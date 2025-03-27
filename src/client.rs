use std::{io, net::ToSocketAddrs, path::PathBuf, sync::Arc, time::Duration};

use clap::Parser;
use gm_quic::ToCertificate;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use qlog::telemetry::{
    Log,
    handy::{DefaultSeqLogger, NullLogger},
};
use rustls::RootCertStore;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
};

#[derive(Parser)]
struct Opt {
    #[arg(long, default_value = "[::1]:35467")]
    server: String,
    #[arg(long)]
    qlog_dir: Option<PathBuf>,
    #[arg(long, default_value = "4")]
    streams: usize,
    #[arg(long, default_value = "rand-file-128M")]
    file: PathBuf,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let option = Opt::parse();
    let file_size = option.file.metadata()?.len() / (1024 * 1024);
    let output = format!("client-{}*{}M.output", option.streams, file_size);
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
    run(option)
        .await
        .inspect_err(|error| tracing::error!(?error))
}

async fn run(option: Opt) -> io::Result<()> {
    let qlogger = option
        .qlog_dir
        .as_ref()
        .map_or_else::<Arc<dyn Log + Send + Sync>, _, _>(
            || Arc::new(NullLogger),
            |dir| Arc::new(DefaultSeqLogger::new(dir.clone())),
        );

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

    let server = option.server.to_socket_addrs()?.next().unwrap();
    let connection = client.connect("localhost", server)?;
    tracing::info!("connecting to {}", server);

    let file = Arc::new(tokio::fs::read(&option.file).await?);
    let pbs = MultiProgress::new();

    let pb_stype = ProgressStyle::default_bar()
        .template("{prefix} {wide_bar} {percent_precise}% {decimal_bytes_per_sec} ETA: {eta} {msg}")
        .unwrap();

    let mut streams = JoinSet::new();

    let upload_pbs = (0..option.streams)
        .map(|idx| {
            pbs.add(
                ProgressBar::new(file.len() as u64)
                    .with_style(pb_stype.clone())
                    .with_prefix(format!("流{idx}↑")),
            )
        })
        .collect::<Vec<_>>();

    let totoal_upload_pb = pbs.add(
        ProgressBar::new((file.len() * option.streams) as u64)
            .with_style(pb_stype.clone())
            .with_prefix("总↑"),
    );

    let download_pbs = (0..option.streams)
        .map(|idx| {
            pbs.add(
                ProgressBar::new(file.len() as u64)
                    .with_style(pb_stype.clone())
                    .with_prefix(format!("流{idx}↓")),
            )
        })
        .collect::<Vec<_>>();

    let totoal_download_pb = pbs.add(
        ProgressBar::new((file.len() * option.streams) as u64)
            .with_style(pb_stype.clone())
            .with_prefix("总↓"),
    );

    for (stream_idx, (upload_pb, download_pb)) in
        (0..option.streams).zip(upload_pbs.into_iter().zip(download_pbs))
    {
        let (_stream_id, (mut reader, mut writer)) = connection.open_bi_stream().await?.unwrap();
        tracing::info!(stream_idx, "opened stream");

        let file = file.clone();

        let totoal_upload_pb = totoal_upload_pb.clone();
        let totoal_download_pb = totoal_download_pb.clone();

        streams.spawn(async move {
            let recv = tokio::spawn({
                let file = file.clone();
                async move {
                    let mut back = vec![];
                    loop {
                        let n = reader.read_buf(&mut back).await?;
                        download_pb.inc(n as u64);
                        totoal_download_pb.inc(n as u64);
                        if n == 0 {
                            break;
                        }
                    }

                    assert_eq!(back, *file);
                    download_pb.finish_with_message("done");
                    io::Result::Ok(())
                }
            });

            {
                let mut file = file.as_slice();
                while !file.is_empty() {
                    let write = writer.write(file).await?;
                    upload_pb.inc(write as u64);
                    totoal_upload_pb.inc(write as u64);
                    file = &file[write..];
                }
                upload_pb.set_message("shutdown...");
                writer.shutdown().await?;
                upload_pb.finish_with_message("done");
            }

            recv.await.unwrap()
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

    totoal_upload_pb.finish_with_message("done");
    totoal_download_pb.finish_with_message("done");

    connection.close("no error".into(), 0);

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

    params
}
