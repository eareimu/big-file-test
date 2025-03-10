use std::{io, net::ToSocketAddrs, path::PathBuf, sync::Arc, time::Instant};

use clap::Parser;
use gm_quic::ToCertificate;
use qlog::telemetry::handy::{DefaultSeqLogger, NullLogger};
use rustls::RootCertStore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Parser)]
struct Opt {
    #[arg(long, default_value = "0.0.0.0:0")]
    server: String,
    #[arg(long, default_value = ".")]
    qlog_dir: PathBuf,
    #[arg(long)]
    file: PathBuf,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let option = Opt::parse();

    let qlogger = Arc::new(DefaultSeqLogger::new(option.qlog_dir));
    // let qlogger = Arc::new(NullLogger);

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

    let (_stream_id, (mut reader, mut writer)) = connection.open_bi_stream().await?.unwrap();
    tracing::info!("opened stream");
    let file = Arc::new(tokio::fs::read(&option.file).await?);
    // let file = Arc::new(
    //     std::iter::repeat_n(
    //         [0x0, 0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9].into_iter(),
    //         1024 * 1024,
    //     )
    //     .flatten()
    //     .collect::<Vec<u8>>(),
    // );

    let task = tokio::spawn({
        let start_read = Instant::now();
        let file = file.clone();
        async move {
            let mut back = vec![];
            while let Ok(n) = reader.read_buf(&mut back).await {
                tracing::info!(
                    "read {n} bytes ({}/{})({:.2}%)",
                    back.len(),
                    file.len(),
                    back.len() as f64 / file.len() as f64 * 100.0
                );
                if n == 0 {
                    break;
                }
            }

            assert_eq!(back, *file);
            start_read
        }
    });

    let start_write = Instant::now();
    writer.write_all(&file).await?;
    writer.shutdown().await?;
    let upload_sec = start_write.elapsed().as_secs_f64();

    let start_read = task.await?;
    let download_sec = start_read.elapsed().as_secs_f64();

    tracing::info!(
        "done! ↑ {:.4}s({:.4}MB/S), ↓ {:.4}s({:.4}MB/S)",
        upload_sec,
        file.len() as f64 / upload_sec / 1024u32.pow(2) as f64,
        download_sec,
        file.len() as f64 / download_sec / 1024u32.pow(2) as f64
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
