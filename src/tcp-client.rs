use std::{io, path::PathBuf, sync::Arc};

use clap::Parser;
use indicatif::{MultiProgress, ProgressBar};
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

    let pbs = MultiProgress::new();

    let download = tokio::spawn({
        let file = file.clone();
        let download_pb = ProgressBar::new(file.len() as u64);
        pbs.add(download_pb.clone());
        async move {
            let mut back = vec![];
            while let Ok(n) = reader.read_buf(&mut back).await {
                download_pb.inc(n as u64);
                if n == 0 {
                    break;
                }
            }
            download_pb.finish_with_message("done!");

            assert_eq!(back, *file);
        }
    });

    let upload_pb = ProgressBar::new(file.len() as u64);
    pbs.add(upload_pb.clone());

    let mut file = file.as_slice();
    while !file.is_empty() {
        let write = writer.write(file).await?;
        upload_pb.inc(write as u64);
        file = &file[write..];
    }
    upload_pb.set_message("shutdown...");
    writer.shutdown().await?;
    upload_pb.finish_with_message("done");
    download.await?;

    // tracing::info!(
    //     "done! ↑ {:.4}s({:.4}MB/S), ↓ {:.4}s({:.4}MB/S)",
    //     upload_sec,
    //     file.len() as f64 / upload_sec / 1024u32.pow(2) as f64,
    //     download_sec,
    //     file.len() as f64 / download_sec / 1024u32.pow(2) as f64
    // );

    Ok(())
}
