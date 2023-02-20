use std::{collections::HashMap, net::SocketAddr};

use color_eyre::{eyre::Context, Result};
use opentelemetry::sdk::trace::Tracer;
use opentelemetry_otlp::WithExportConfig;
use serde::Deserialize;
use serde_json::{json, Number};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use tracing::{error, info, metadata::LevelFilter, warn};
use tracing_error::ErrorLayer;
use tracing_subscriber::{prelude::*, EnvFilter};
use uuid::Uuid;

const DATASET_NAME: &str = "prime-time";
const API_KEY: &str = "QLLy20BDjNUOmUcfk2kf6K"; // cspell:disable-line

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_opentelemetry::layer().with_tracer(init_tracer()?))
        .with(ErrorLayer::default())
        .init();
    color_eyre::install()?;

    let listen = TcpListener::bind("0.0.0.0:1729").await?;
    while let Ok((conn, addr)) = listen.accept().await {
        tokio::spawn(async move {
            if let Err(e) = handle_conn(conn, addr).await {
                error!("{e:?}");
            }
        });
    }

    Ok(())
}

#[tracing::instrument(name = "New request", skip(conn), fields(connection_id = %Uuid::new_v4()))]
async fn handle_conn(mut conn: TcpStream, addr: SocketAddr) -> Result<()> {
    let (r, mut w) = conn.split();
    let mut lines = BufReader::with_capacity(1024, r).lines();

    while let Some(line) = lines.next_line().await? {
        let span = tracing::info_span!("Request", request_id = %Uuid::new_v4()).or_current();
        let _guard = span.enter();
        info!("Received {} bytes: {line}", line.len());

        if let Ok(req) = serde_json::from_str::<PrimeRequest>(&line) {
            info!("Parsed request: {req:?}");
            if req.method == "isPrime" {
                let prime = if let Some(num) = req.number.as_u64() {
                    primes::is_prime(num)
                } else {
                    false
                };
                let mut resp = serde_json::to_vec(&json!({
                    "method": "isPrime",
                    "prime": prime,
                }))?;
                resp.push(b'\n');

                info!("Sending response: {:?}", String::from_utf8_lossy(&resp));
                w.write_all(&resp)
                    .await
                    .wrap_err("Failed to write to socket")?;

                continue;
            }
        }

        warn!("Malformed request");
        w.write_all(b"Malformed request\n")
            .await
            .wrap_err("Failed to write to socket")?;
    }

    Ok(())
}

#[derive(Deserialize, Debug)]
struct PrimeRequest {
    method: String,
    number: Number,
}

fn init_tracer() -> Result<Tracer> {
    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .http()
                .with_endpoint("https://api.honeycomb.io/v1/traces")
                // .with_http_client(reqwest::Client::default())
                .with_headers(HashMap::from([
                    ("x-honeycomb-dataset".into(), DATASET_NAME.into()),
                    ("x-honeycomb-team".into(), API_KEY.into()),
                ]))
                .with_timeout(std::time::Duration::from_secs(2)),
        )
        .install_batch(opentelemetry::runtime::Tokio)
        .map_err(Into::into)
}
