use color_eyre::Result;
use dashmap::DashMap;
use opentelemetry::sdk::trace::Tracer;
use opentelemetry_otlp::WithExportConfig;
use tokio::{net::UdpSocket, signal};
use tracing::{debug, error, info, info_span, metadata::LevelFilter};
use tracing_error::ErrorLayer;
use tracing_subscriber::{prelude::*, EnvFilter};

fn init_tracer() -> Result<Tracer> {
    let exporter = opentelemetry_otlp::new_exporter().tonic().with_env();

    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .install_batch(opentelemetry::runtime::Tokio)
        .map_err(Into::into)
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    color_eyre::install()?;
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

    let listen = UdpSocket::bind(std::env::var("SERVER_ADDR").unwrap()).await?;
    tokio::spawn(run(listen));

    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Shutting down...")
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }

    Ok(())
}

async fn run(listen: UdpSocket) {
    info!(addr = ?listen.local_addr(), "Starting server...");

    let mut buf = [0; 1024];
    let map = DashMap::new();
    map.insert(
        "version".to_string(),
        "Ken's Key-Value Store 1.0".to_string(),
    );

    while let Ok((n, addr)) = listen.recv_from(&mut buf).await {
        let span = info_span!("Request", %addr);
        let _span = span.enter();
        let msg = String::from_utf8_lossy(&buf[..n]);

        match msg.split_once('=') {
            Some((key, val)) => {
                if key == "version" {
                    continue;
                }

                debug!(?key, ?val, "Inserting key-value pair");
                map.insert(key.to_string(), val.to_string());
            }
            None => {
                if let Some(val) = map.get(msg.as_ref()) {
                    debug!(key = ?msg, val = ?*val, "Retrieving value");
                    let msg = format!("{}={}", msg, *val);
                    let _ = listen.send_to(msg.as_bytes(), addr).await;
                }
            }
        }
    }
}
