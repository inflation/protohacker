use std::{collections::BTreeMap, net::SocketAddr};

use byteorder::{ByteOrder, NetworkEndian};
use bytes::{Buf, BytesMut};
use color_eyre::{eyre::eyre, Result};
use opentelemetry::sdk::trace::Tracer;
use opentelemetry_otlp::WithExportConfig;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, error, info, instrument, metadata::LevelFilter, trace};
use tracing_error::ErrorLayer;
use tracing_subscriber::{prelude::*, EnvFilter};
use uuid::Uuid;

const MSG_BYTES: usize = 9;

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
        // .with(tracing_opentelemetry::layer().with_tracer(init_tracer()?))
        .with(ErrorLayer::default())
        .init();

    info!("Starting server...");

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

#[instrument(name = "New connection", skip(conn), fields(connection_id = %Uuid::new_v4()))]
async fn handle_conn(mut conn: TcpStream, addr: SocketAddr) -> Result<()> {
    const MSG_BYTES: usize = 9;
    let mut db = BTreeMap::new();
    let mut buf = BytesMut::with_capacity(2048);

    while let Some(frame) = read_frame(&mut buf, &mut conn).await? {
        handle_request(frame, &mut db, &mut conn).await?;
    }

    Ok(())
}

async fn read_frame(buf: &mut BytesMut, conn: &mut TcpStream) -> Result<Option<Frame>> {
    loop {
        if let Some(frame) = parse_frame(buf)? {
            return Ok(Some(frame));
        }

        if conn.read_buf(buf).await? == 0 {
            if buf.is_empty() {
                return Ok(None);
            } else {
                return Err(eyre!("Connection closed unexpectedly"));
            }
        };
    }
}

#[derive(Debug)]
enum Frame {
    Insert { timestamp: i32, val: i32 },
    Query { min: i32, max: i32 },
}

fn parse_frame(buf: &mut BytesMut) -> Result<Option<Frame>> {
    if buf.remaining() < MSG_BYTES {
        return Ok(None);
    }

    let method = buf[0];
    let val1 = NetworkEndian::read_i32(&buf[1..5]);
    let val2 = NetworkEndian::read_i32(&buf[5..9]);

    match method {
        b'I' => {
            trace!(
                method = "INSERT",
                timestamp = val1,
                value = val2,
                "Insert {val2} at {val1}"
            );

            buf.advance(MSG_BYTES);
            Ok(Some(Frame::Insert {
                timestamp: val1,
                val: val2,
            }))
        }
        b'Q' => {
            trace!(
                method = "QUERY",
                min = val1,
                max = val2,
                "Query between {val1} and {val2}"
            );

            buf.advance(MSG_BYTES);
            Ok(Some(Frame::Query {
                min: val1,
                max: val2,
            }))
        }
        _ => Err(eyre!("Unknown method: {method}")),
    }
}

#[instrument(name = "Handle request", skip(db, conn))]
async fn handle_request(
    frame: Frame,
    db: &mut BTreeMap<i32, i32>,
    conn: &mut TcpStream,
) -> Result<()> {
    match frame {
        Frame::Insert { timestamp, val } => {
            if db.contains_key(&timestamp) {
                error!(
                    method = "INSERT",
                    timestamp = timestamp,
                    value = val,
                    "Duplicate timestamp: {timestamp}"
                );
                return Err(eyre!("Undefined behavior"));
            }

            db.insert(timestamp, val);
        }
        Frame::Query { min, max } => {
            let (num, sum) = db
                .iter()
                .filter(|(&k, _)| k >= min && k <= max)
                .fold((0, 0), |(count, sum), (_, &v)| (count + 1, sum + v as i64));

            let result = if num == 0 { 0 } else { sum / num as i64 } as i32;
            debug!(
                method = "QUERY",
                num = num,
                sum = sum,
                "Query between {min} and {max} returned {result}"
            );
            conn.write_all(&result.to_be_bytes()).await?;
        }
    }

    Ok(())
}

fn init_tracer() -> Result<Tracer> {
    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_env())
        .install_batch(opentelemetry::runtime::Tokio)
        .map_err(Into::into)
}
