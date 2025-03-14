use std::{
    io,
    net::ToSocketAddrs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Instant,
};

use clap::Parser;
use gm_quic::ToCertificate;
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
    let connection = client.connect("test0.genmeta.net", server)?;
    tracing::info!("connecting to {}", server);

    let file = Arc::new(tokio::fs::read(&option.file).await?);
    let rcvd = Arc::new(AtomicUsize::new(0));

    let mut streams = JoinSet::new();

    for stream_idx in 0..option.streams {
        let (_stream_id, (mut reader, mut writer)) = connection.open_bi_stream().await?.unwrap();
        tracing::info!(stream_idx, "opened stream");

        let file = file.clone();
        let read = rcvd.clone();
        streams.spawn(async move {
            let recv = tokio::spawn({
                let file = file.clone();
                let start_read = Instant::now();
                async move {
                    let mut back = vec![];
                    loop {
                        let n = reader.read_buf(&mut back).await?;
                        read.fetch_add(n, Relaxed);
                        if n == 0 {
                            break;
                        }
                    }

                    assert_eq!(back, *file);
                    io::Result::Ok(start_read)
                }
            });

            let start_write = Instant::now();
            writer.write_all(&file).await?;
            writer.shutdown().await?;
            let upload_sec = start_write.elapsed().as_secs_f64();

            let start_read = recv.await??;
            let download_sec = start_read.elapsed().as_secs_f64();

            io::Result::Ok((upload_sec, download_sec))
        });
    }

    let (mut upload_sec, mut download_sec) = streams
        .join_all()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .reduce(|(u1, d1), (u2, d2)| (u1 + u2, d1 + d2))
        .unwrap();

    let total_data = option.streams * file.len();
    upload_sec /= option.streams as f64;
    download_sec /= option.streams as f64;

    tracing::info!(
        "done! ↑ {:.4}s({:.4}MB/S), ↓ {:.4}s({:.4}MB/S)",
        upload_sec,
        total_data as f64 / upload_sec / 1024u32.pow(2) as f64,
        download_sec,
        total_data as f64 / download_sec / 1024u32.pow(2) as f64
    );

    connection.close("no error".into(), 0);

    Ok(())
}

fn client_stream_unlimited_parameters() -> gm_quic::ClientParameters {
    let mut params = gm_quic::ClientParameters::default();

    params.set_initial_max_streams_bidi(100);
    params.set_initial_max_streams_uni(100);
    params.set_initial_max_data((1u32 << 20).into());
    params.set_initial_max_stream_data_uni((1u32 << 20).into());
    params.set_initial_max_stream_data_bidi_local((1u32 << 20).into());
    params.set_initial_max_stream_data_bidi_remote((1u32 << 20).into());

    params
}
