use std::{io, net::SocketAddr, path::PathBuf, sync::Arc};

use clap::Parser;
use gm_quic::ToCertificate;
use qlog::telemetry::handy::DefaultSeqLogger;
use rustls::RootCertStore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Parser)]
struct Opt {
    #[arg(long, default_value = "0.0.0.0:0")]
    server: SocketAddr,
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

    let connection = client.connect("test0.genmeta.net", option.server)?;
    tracing::info!("connecting to {}", option.server);

    let (_stream_id, (mut reader, mut writer)) = connection.open_bi_stream().await?.unwrap();
    tracing::info!("opened stream");

    let file = tokio::fs::read(&option.file).await?;
    writer.write_all(&file).await?;
    writer.shutdown().await?;

    let mut back = Vec::new();
    while let Ok(n) = reader.read_buf(&mut back).await {
        tracing::info!("read {n} bytes");
        if n == 0 {
            break;
        }
    }

    assert_eq!(file, back);

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
