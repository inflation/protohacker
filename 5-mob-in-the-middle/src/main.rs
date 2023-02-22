use color_eyre::Result;
use once_cell::sync::Lazy;
use opentelemetry::sdk::trace::Tracer;
use opentelemetry_otlp::WithExportConfig;
use regex::bytes::{Captures, Regex};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    signal,
};
use tracing::{debug, error, info, info_span, instrument, metadata::LevelFilter, Instrument};
use tracing_error::ErrorLayer;
use tracing_subscriber::{prelude::*, EnvFilter};

static BOGUSCOIN_ADDR: Lazy<Regex> =
    Lazy::new(|| Regex::new("(7[a-zA-Z0-9]{25,34})( |\n)").unwrap());

const TONY_ADDR: &[u8] = b"7YWHMfk9JZe0LM0g1ZauHuiSxhI";

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

    let listen = TcpListener::bind("0.0.0.0:1729").await?;

    tokio::spawn(run(listen));

    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Shutting down...")
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }

    opentelemetry::global::shutdown_tracer_provider();
    Ok(())
}

async fn run(listen: TcpListener) {
    info!("Starting server...");

    while let Ok((conn, addr)) = listen.accept().await {
        tokio::spawn(async move {
            if let Err(e) = handle_conn(conn)
                .instrument(info_span!("Connection", %addr))
                .await
            {
                error!("{e}");
            }
        });
    }
}

async fn handle_conn(conn: TcpStream) -> Result<()> {
    let proxy = TcpStream::connect("chat.protohackers.com:16963").await?;
    debug!("Connected to proxy");

    let (pr, pw) = proxy.into_split();
    let (r, w) = conn.into_split();

    let r_h = tokio::spawn(handle_proxy(pr, w).instrument(info_span!("User Read")));
    let w_h = tokio::spawn(handle_proxy(r, pw).instrument(info_span!("User Write")));

    match tokio::try_join!(r_h, w_h)? {
        (Err(e), _) | (_, Err(e)) => {
            error!("{e:?}");
        }
        _ => {}
    }

    debug!("Connection closed");
    Ok(())
}

async fn handle_proxy(r: OwnedReadHalf, mut w: OwnedWriteHalf) -> Result<()> {
    let mut l = BufReader::new(r);
    let mut buf = Vec::with_capacity(1024);

    loop {
        let len = l.read_until(b'\n', &mut buf).await?;
        if len == 0 {
            return Ok(());
        }

        handle_msg(&buf[..len], &mut w).await?;
        buf.clear();
    }
}

#[instrument(skip_all, fields(msg = %String::from_utf8_lossy(l)))]
async fn handle_msg(l: &[u8], w: &mut OwnedWriteHalf) -> Result<()> {
    let result = BOGUSCOIN_ADDR
        .replace_all(l, |cap: &Captures| {
            debug!(
                "Found address: {}, replacing...",
                String::from_utf8_lossy(&cap[1])
            );

            let mut result = TONY_ADDR.to_vec();
            result.extend_from_slice(&cap[2]);
            result
        })
        .into_owned();

    w.write_all(&result).await?;

    Ok(())
}
