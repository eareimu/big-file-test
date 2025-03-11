#![feature(never_type)]

use std::{io, net::SocketAddr, path::PathBuf, sync::Arc};

use clap::Parser;
use gm_quic::{Connection, StreamReader, StreamWriter};
use qlog::telemetry::{
    Log,
    handy::{DefaultSeqLogger, NullLogger},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::Instrument;

#[derive(Parser)]
struct Opt {
    #[arg(long, default_value = "0.0.0.0:0")]
    bind: SocketAddr,
    #[arg(long, default_value = "None")]
    qlog_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let option = Opt::parse();

    let qlogger = option
        .qlog_dir
        .as_ref()
        .map_or_else::<Arc<dyn Log + Send + Sync>, _, _>(
            || Arc::new(NullLogger),
            |dir| Arc::new(DefaultSeqLogger::new(dir.clone())),
        );

    let server = gm_quic::QuicServer::builder()
        .without_cert_verifier()
        .with_single_cert(
            include_bytes!("../server.crt"),
            include_bytes!("../server.key"),
        )
        .with_parameters(server_stream_unlimited_parameters())
        .with_qlog(qlogger)
        .listen(option.bind)?;

    tracing::info!("listening on {:?}", server.addresses());

    async fn for_each_stream(mut reader: StreamReader, mut writer: StreamWriter) -> io::Result<()> {
        let mut buffer = [0; 4096];

        let mut echoed = 0;
        loop {
            match reader.read(&mut buffer).await? {
                0 => break,
                n => {
                    echoed += n;
                    writer.write_all(&buffer[..n]).await?
                }
            }
        }

        // let mut buf = vec![];
        // reader.read_to_end(&mut buf).await?;
        // writer.write_all(&buf).await?;

        tracing::info!(echoed, "transfer completed, waiting for ack");
        writer.shutdown().await?;
        tracing::info!("done");
        Ok(())
    }

    async fn for_each_conn(conn: Arc<Connection>) -> io::Result<!> {
        loop {
            let (stream_id, (reader, writer)) = conn.accept_bi_stream().await?.unwrap();

            tokio::spawn(
                for_each_stream(reader, writer)
                    .instrument(tracing::info_span!("stream", %stream_id)),
            );
        }
    }

    while let Ok((connection, pathway)) = server.accept().await {
        tracing::info!(%pathway, "new connection");
        tokio::spawn(
            for_each_conn(connection)
                .instrument(tracing::info_span!("conn", from = %pathway.remote())),
        );
    }

    Ok(())
}

pub fn server_stream_unlimited_parameters() -> gm_quic::ServerParameters {
    let mut params = gm_quic::ServerParameters::default();

    params.set_initial_max_streams_bidi(100);
    params.set_initial_max_streams_uni(100);
    params.set_initial_max_data((1u32 << 20).into());
    params.set_initial_max_stream_data_uni((1u32 << 20).into());
    params.set_initial_max_stream_data_bidi_local((1u32 << 20).into());
    params.set_initial_max_stream_data_bidi_remote((1u32 << 20).into());

    params
}
