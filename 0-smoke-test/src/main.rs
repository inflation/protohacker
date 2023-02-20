use std::net::SocketAddr;

use color_eyre::Result;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    run().await
}

async fn run() -> Result<()> {
    let listen = TcpListener::bind("0.0.0.0:1729").await?;
    while let Ok((conn, addr)) = listen.accept().await {
        tokio::spawn(handle_conn(conn, addr));
    }

    Ok(())
}

async fn handle_conn(mut conn: TcpStream, addr: SocketAddr) -> Result<()> {
    let mut buf = Vec::with_capacity(1024);
    let len = conn.read_to_end(&mut buf).await?;
    tracing::info!("Received {len} bytes from {addr}: {buf:?}",);

    conn.write_all(&buf).await?;

    Ok(())
}
