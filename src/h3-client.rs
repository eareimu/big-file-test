use std::{sync::Arc, time::Instant};

use clap::Parser;
use gm_quic::ToCertificate;
use http::Uri;
use indicatif::{ProgressBar, ProgressStyle};
use rustls::RootCertStore;
use tokio::task::JoinSet;
use tracing::{Instrument, info_span};

#[derive(Parser, Clone)]
struct Opt {
    #[arg(long, short = 'r', default_value = "64")]
    reqs: usize,
    #[arg(long, short = 'c', default_value = "64")]
    conns: usize,
    #[arg(default_value = "https://localhost:4433/rand-file-15K")]
    uri: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        // .with_ansi(false)
        // .with_writer(
        //     std::fs::OpenOptions::new()
        //         .create(true)
        //         .truncate(true)
        //         .write(true)
        //         .open("h3-client.log")
        //         .unwrap(),
        // )
        .init();
    if let Err(error) = run(Opt::parse()).await {
        tracing::error!(?error);
        panic!("{error:?}");
    };
}

type Error = Box<dyn std::error::Error + Send + Sync>;

async fn run(option: Opt) -> Result<(), Error> {
    let uri = option.uri.parse::<Uri>()?;
    let auth = uri.authority().unwrap();
    let addr = tokio::net::lookup_host((auth.host(), auth.port_u16().unwrap_or(443)))
        .await?
        .next()
        .ok_or("dns found no addresses")?;
    tracing::info!("DNS lookup for {:?}: {:?}", uri, addr);

    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(include_bytes!("../ca.crt").to_certificate());

    let client = Arc::new(
        gm_quic::QuicClient::builder()
            .with_root_certificates(roots)
            .without_cert()
            .with_parameters(client_parameters())
            .with_alpns([b"h3".to_vec(), b"hq-29".to_vec()])
            .build(),
    );

    let start_time = Instant::now();
    let pb = ProgressBar::new(0)
        .with_style(ProgressStyle::with_template("{wide_bar} {pos}/{len} {eta}").unwrap());

    let mut connections = JoinSet::new();
    for conn_id in 0..option.conns {
        pb.inc_length(option.reqs as u64);
        let connection = client.connect(auth.host(), addr)?;
        let uri = uri.clone();

        connections.spawn(
            for_each_connection(connection, uri, option.reqs)
                .instrument(info_span!("connection", conn_id)),
        );
    }

    let mut success_queries = 0;
    let mut conn_counting = 0;
    while let Some(res) = connections.join_next().await {
        conn_counting += 1;
        match res {
            Ok(Ok(queries)) => {
                success_queries += queries;
                pb.inc(queries as u64);
            }
            Ok(Err(err)) => {
                pb.dec_length(option.reqs as u64);
                tracing::error!(error = ?err,"conenction failed");
            }
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(err) => panic!("{err}"),
        }
        if conn_counting == option.conns {
            tracing::info!(target: "counting", "this should done");
        }
    }

    let total_time = start_time.elapsed().as_secs_f64();
    let qps = success_queries as f64 / total_time;

    tracing::info!(target: "counting",success_queries,total_time,qps, "done!");

    Ok(())
}

fn client_parameters() -> gm_quic::ClientParameters {
    let mut params = gm_quic::ClientParameters::default();
    params.set_initial_max_streams_bidi(100);
    params.set_initial_max_streams_uni(100);
    params.set_initial_max_data((1u32 << 20).into());
    params.set_initial_max_stream_data_uni((1u32 << 20).into());
    params.set_initial_max_stream_data_bidi_local((1u32 << 20).into());
    params.set_initial_max_stream_data_bidi_remote((1u32 << 20).into());
    params
}

async fn for_each_connection(
    connection: Arc<gm_quic::Connection>,
    uri: Uri,
    reqs: usize,
) -> Result<usize, Error> {
    let connection = h3_shim::QuicConnection::new(connection).await;
    let (mut conn, send_request) = h3::client::new(connection).await?;
    tracing::info!("conenction established");
    let driver = async move {
        core::future::poll_fn(|cx| conn.poll_close(cx))
            .await
            .map_err(Error::from)
    };

    let _driver = tokio::spawn(driver);

    let mut requests = JoinSet::new();
    for req_id in 0..reqs {
        let request = http::Request::builder().uri(uri.clone()).body(())?;
        let mut send_request = send_request.clone();

        requests.spawn(
            async move {
                let mut request_stream = send_request.send_request(request).await?;
                request_stream.finish().await?;
                let _resp = request_stream.recv_response().await?;
                while request_stream.recv_data().await?.is_some() {}
                Result::<(), Error>::Ok(())
            }
            .instrument(info_span!("request", req_id)),
        );
    }

    let mut error = None;

    Ok(requests
        .join_all()
        .await
        .into_iter()
        .filter_map(|result| match result {
            Ok(()) => Some(()),
            Err(err) => {
                error = Some(err);
                None
            }
        })
        .count())
    .and_then(|n| match n {
        0 => Err(error.unwrap()),
        n => Ok(n),
    })
}
