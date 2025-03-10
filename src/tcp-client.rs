use std::{io, path::PathBuf, sync::Arc, time::Instant};

use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Parser)]
struct Opt {
    #[arg(long, default_value = "0.0.0.0:0")]
    server: String,
    #[arg(long)]
    file: PathBuf,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let option = Opt::parse();
    let file = Arc::new(tokio::fs::read(&option.file).await?);

    let (mut reader, mut writer) = tokio::net::TcpStream::connect(option.server)
        .await?
        .into_split();

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

    Ok(())
}
