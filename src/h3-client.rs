use std::{sync::Arc, time::Instant};

use clap::Parser;
use gm_quic::ToCertificate;
use http::Uri;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rustls::RootCertStore;
use tokio::task::JoinSet;
use tracing::{Instrument, info_span};

#[derive(Parser, Clone)]
struct Opt {
    #[arg(long, short = 'r', default_value = "64")]
    reqs: usize,
    #[arg(long, short = 'c', default_value = "64")]
    conns: usize,
    #[arg(default_value = "https://localhost:4431/rand-file-15K")]
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

    let pbs = MultiProgress::new();
    let conns_pb = pbs.add(ProgressBar::new(0).with_prefix("connections").with_style(
        ProgressStyle::with_template("{prefix} {wide_bar} {pos}/{len}")?,
    ));
    let total_pb = pbs.add(ProgressBar::new(0).with_prefix("requests").with_style(
        ProgressStyle::with_template("{prefix} {wide_bar} {pos}/{len} {per_sec} {eta}")?,
    ));

    let start_time = Instant::now();
    let mut connections = JoinSet::new();
    for idx in 0..option.conns {
        conns_pb.inc_length(1);

        let connection = client.connect(auth.host(), addr)?;
        let uri = uri.clone();

        connections.spawn(
            for_each_connection(connection, uri, option.reqs, total_pb.clone(), pbs.clone())
                .instrument(info_span!("connection", idx)),
        );
    }

    let mut success_queries = 0;
    while let Some(res) = connections.join_next().await {
        match res {
            Ok(Ok(queries)) => {
                success_queries += queries;
                conns_pb.inc(1);
            }
            Ok(Err(err)) => {
                tracing::error!(error = ?err,"conenction failed");
                conns_pb.dec_length(1);
            }
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(err) => panic!("{err}"),
        }
    }

    conns_pb.finish();
    total_pb.finish();

    let total_time = start_time.elapsed().as_secs_f64();
    let qps = success_queries as f64 / total_time;

    tracing::info!(target: "counting" ,success_queries ,total_time ,qps, "done!");

    Ok(())
}

fn client_parameters() -> gm_quic::ClientParameters {
    let mut params = gm_quic::ClientParameters::default();

    params.set_initial_max_streams_bidi(100u32);
    params.set_initial_max_streams_uni(100u32);
    params.set_initial_max_data(1u32 << 20);
    params.set_initial_max_stream_data_uni(1u32 << 20);
    params.set_initial_max_stream_data_bidi_local(1u32 << 20);
    params.set_initial_max_stream_data_bidi_remote(1u32 << 20);

    params
}

async fn for_each_connection(
    connection: Arc<gm_quic::Connection>,
    uri: Uri,
    reqs: usize,
    total_pb: ProgressBar,
    pbs: MultiProgress,
) -> Result<usize, Error> {
    // let origin_dcid = connection.origin_dcid()?;
    let conn_pb = pbs.insert_after(
        &total_pb,
        ProgressBar::new(0)
            // .with_prefix(format!("{origin_dcid:x}"))
            .with_message("connecting")
            .with_style(ProgressStyle::with_template("{prefix} {spinner} {msg}")?),
    );

    let connection = h3_shim::QuicConnection::new(connection).await;
    let (mut conn, send_request) = h3::client::new(connection).await?;
    tracing::info!("conenction established");

    total_pb.inc_length(reqs as u64);
    conn_pb.set_style(ProgressStyle::with_template(
        "{prefix} {wide_bar} {pos}/{len}",
    )?);

    let driver = async move {
        core::future::poll_fn(|cx| conn.poll_close(cx))
            .await
            .map_err(Error::from)
    };

    let _driver = tokio::spawn(driver);

    let mut requests = JoinSet::new();
    for req_id in 0..reqs {
        let conn_pb = conn_pb.clone();
        let request = http::Request::builder().uri(uri.clone()).body(())?;
        let mut send_request = send_request.clone();

        requests.spawn(
            async move {
                let mut request_stream = send_request.send_request(request).await?;
                let request = async {
                    request_stream.finish().await?;
                    conn_pb.inc_length(1);
                    let _resp = request_stream.recv_response().await?;
                    while request_stream.recv_data().await?.is_some() {}
                    Result::<(), Error>::Ok(())
                };
                request
                    .await
                    .inspect(|()| conn_pb.inc(1))
                    .inspect_err(|_| conn_pb.dec_length(1))
            }
            .instrument(info_span!("request", req_id)),
        );
    }

    let mut error = None;

    let mut success_queries = 0;
    while let Some(res) = requests.join_next().await {
        match res {
            Ok(Ok(())) => {
                success_queries += 1;
                total_pb.inc(1);
            }
            Ok(Err(err)) => {
                total_pb.dec_length(1);
                error = Some(err);
            }
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(err) => panic!("{err}"),
        }
    }
    conn_pb.finish_and_clear();
    if success_queries != 0 {
        Ok(success_queries)
    } else {
        Err(error.unwrap())
    }
}
