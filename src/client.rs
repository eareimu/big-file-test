use std::{io, path::PathBuf, sync::Arc, time::Duration};

use clap::Parser;
use gm_quic::ToCertificate;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use qlog::telemetry::{
    Log,
    handy::{DefaultSeqLogger, NullLogger},
};
use rustls::RootCertStore;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    task::JoinSet,
};

#[derive(Parser)]
struct Options {
    #[arg(default_value = "localhost:35467")]
    server: String,
    #[arg(short = 'l', long)]
    qlog_dir: Option<PathBuf>,
    #[arg(short = 's', long, default_value = "4")]
    streams: usize,
    #[arg(short = 'f', long, default_value = "rand-file-128M")]
    file: PathBuf,
    #[arg(short = 'p', long)]
    progress: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    let file_size = options.file.metadata()?.len() / (1024 * 1024);
    let output = format!("client-{}*{}M.output", options.streams, file_size);
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(output)?,
        )
        .init();
    run(options)
        .await
        .inspect_err(|error| tracing::error!(?error))
}

async fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let qlogger = options
        .qlog_dir
        .as_ref()
        .map_or_else::<Arc<dyn Log + Send + Sync>, _, _>(
            || Arc::new(NullLogger),
            |dir| Arc::new(DefaultSeqLogger::new(dir.clone())),
        );

    let file = Arc::new(tokio::fs::read(&options.file).await?);

    let pbs = MultiProgress::new();
    if !options.progress {
        pbs.set_draw_target(indicatif::ProgressDrawTarget::hidden());
    }

    let pb_stype = ProgressStyle::default_bar()
        .template("{prefix} {wide_bar} {percent_precise}% {decimal_bytes_per_sec} ETA: {eta} {msg}")
        .unwrap();

    let tx_pbs = (0..options.streams)
        .map(|idx| {
            pbs.add(
                ProgressBar::new(file.len() as u64)
                    .with_style(pb_stype.clone())
                    .with_prefix(format!("流{idx}↑")),
            )
        })
        .collect::<Vec<_>>();

    let total_tx_pb = pbs.add(
        ProgressBar::new((file.len() * options.streams) as u64)
            .with_style(pb_stype.clone())
            .with_prefix("总↑"),
    );

    let rx_pbs = (0..options.streams)
        .map(|idx| {
            pbs.add(
                ProgressBar::new(file.len() as u64)
                    .with_style(pb_stype.clone())
                    .with_prefix(format!("流{idx}↓")),
            )
        })
        .collect::<Vec<_>>();

    let total_rx_pb = pbs.add(
        ProgressBar::new((file.len() * options.streams) as u64)
            .with_style(pb_stype.clone())
            .with_prefix("总↓"),
    );

    let uri = options.server.parse::<http::Uri>()?;
    let server_name = uri.host().ok_or("missing host")?;
    let mut server_addrs = tokio::net::lookup_host(server_name).await?;
    let server_addr = server_addrs.next().ok_or("DNS lookup failed")?;

    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(include_bytes!("../ca.crt").to_certificate());

    let client = Arc::new(
        gm_quic::QuicClient::builder()
            .with_root_certificates(roots)
            .without_cert()
            .with_parameters(client_stream_unlimited_parameters())
            .with_qlog(qlogger)
            .build(),
    );

    let connection = client.connect(server_name, server_addr)?;
    tracing::info!("connecting to {server_name}[{server_addr}]");

    let mut streams = JoinSet::new();
    for (stream_idx, (tx_pb, rx_pb)) in (0..options.streams).zip(tx_pbs.into_iter().zip(rx_pbs)) {
        let (_stream_id, (reader, writer)) = connection.open_bi_stream().await?.unwrap();
        tracing::info!(stream_idx, "opened stream");

        let file = file.clone();
        let total_tx_pb = total_tx_pb.clone();
        let total_rx_pb = total_rx_pb.clone();

        streams.spawn(async move {
            tokio::try_join!(
                upload_stream(file.clone(), writer, tx_pb, total_tx_pb),
                rx_stream(file.clone(), reader, rx_pb, total_rx_pb),
            )
        });
    }

    let ticker = {
        let pbs = pbs.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_millis(33));
            loop {
                pbs.suspend(|| ());
                interval.tick().await;
            }
        }
    };

    tokio::select! {
        all = streams.join_all() => { _ = all.into_iter().collect::<Result<Vec<_>, _>>()? },
        _ = ticker => unreachable!(),
    }

    total_tx_pb.finish_with_message("done");
    total_rx_pb.finish_with_message("done");

    connection.close("no error".into(), 0);

    Ok(())
}

async fn rx_stream(
    file: Arc<Vec<u8>>,
    mut reader: impl AsyncRead + Unpin,
    rx_pb: ProgressBar,
    total_rx_pb: ProgressBar,
) -> io::Result<()> {
    let mut back = vec![];
    loop {
        let n = reader.read_buf(&mut back).await?;
        rx_pb.inc(n as u64);
        total_rx_pb.inc(n as u64);
        if n == 0 {
            break;
        }
    }

    assert_eq!(back, *file);
    rx_pb.finish_with_message("done");
    io::Result::Ok(())
}

async fn upload_stream(
    file: Arc<Vec<u8>>,
    mut writer: impl AsyncWrite + Unpin,
    tx_pb: ProgressBar,
    total_tx_pb: ProgressBar,
) -> Result<(), io::Error> {
    let mut file = file.as_slice();
    while !file.is_empty() {
        let write = writer.write(file).await?;
        tx_pb.inc(write as u64);
        total_tx_pb.inc(write as u64);
        file = &file[write..];
    }
    tx_pb.set_message("shutdown...");
    writer.shutdown().await?;
    tx_pb.finish_with_message("done");
    Ok(())
}

fn client_stream_unlimited_parameters() -> gm_quic::ClientParameters {
    let mut params = gm_quic::ClientParameters::default();

    params.set_initial_max_streams_bidi(100u32);
    params.set_initial_max_streams_uni(100u32);
    params.set_initial_max_data(1u32 << 20);
    params.set_initial_max_stream_data_uni(1u32 << 20);
    params.set_initial_max_stream_data_bidi_local(1u32 << 20);
    params.set_initial_max_stream_data_bidi_remote(1u32 << 20);
    params.set_max_idle_timeout(Duration::from_secs(10));

    params
}
