use std::{io, net::SocketAddr};

use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Parser)]
struct Options {
    #[arg(long, default_value = "0.0.0.0:0")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let options = Options::parse();

    let listener = tokio::net::TcpListener::bind(options.bind).await?;

    tracing::info!("listening on {:?}", listener.local_addr()?);

    loop {
        let (stream, peer) = listener.accept().await?;
        tokio::spawn(async move {
            let (mut reader, mut writer) = stream.into_split();
            let mut buffer = [0; 4096];

            let mut echoed = 0;
            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(n) => {
                        echoed += n;
                        writer.write_all(&buffer[..n]).await?;
                    }
                    Err(e) => {
                        tracing::error!("failed to read from socket; err = {:?}", e);
                        break;
                    }
                }
            }

            tracing::info!("echoed {} bytes to {}", echoed, peer);
            io::Result::Ok(())
        });
    }
}
