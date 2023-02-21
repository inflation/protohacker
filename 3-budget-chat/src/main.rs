use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use color_eyre::Result;
use opentelemetry::sdk::trace::Tracer;
use opentelemetry_otlp::WithExportConfig;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    signal,
    sync::broadcast,
    task::JoinHandle,
};
use tracing::{debug, error, info, info_span, instrument, metadata::LevelFilter, Instrument};
use tracing_error::ErrorLayer;
use tracing_subscriber::{prelude::*, EnvFilter};
use uuid::Uuid;

fn init_tracer() -> Result<Tracer> {
    let headers: Option<HashMap<_, _>> =
        std::env::var("OTEL_EXPORTER_OTLP_HEADERS").ok().map(|e| {
            e.split(',')
                .filter_map(|h| h.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect()
        });

    let mut exporter = opentelemetry_otlp::new_exporter().http().with_env();
    if let Some(headers) = headers {
        exporter = exporter.with_headers(headers);
    }

    // let exporter = opentelemetry_otlp::new_exporter().tonic().with_env();

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

    Ok(())
}

async fn run(listen: TcpListener) {
    info!("Starting server...");

    let (tx, rx) = broadcast::channel(128);
    let chatroom = Arc::new(Chatroom {
        users: Mutex::new(Vec::new()),
        _rx: rx,
        tx,
    });

    while let Ok((conn, addr)) = listen.accept().await {
        let ch = chatroom.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(conn, addr, ch).await {
                error!("{e}");
            }
        });
    }
}

struct Chatroom {
    users: Mutex<Vec<String>>,
    _rx: broadcast::Receiver<Message>,
    tx: broadcast::Sender<Message>,
}

#[derive(Debug, Clone)]
struct Message {
    is_system: bool,
    from: String,
    msg: String,
}

#[instrument(name = "New Connection", skip(conn, chatroom), fields(connection_id = %Uuid::new_v4()))]
async fn handle_conn(mut conn: TcpStream, addr: SocketAddr, chatroom: Arc<Chatroom>) -> Result<()> {
    let Some(name) = create_user(&mut conn).await? else {
        return Ok(());
    };

    let user_list = chatroom.users.lock().unwrap().join(", ");
    conn.write_all(format!("* The chatroom contains: {user_list}\n").as_bytes())
        .await?;
    chatroom.users.lock().unwrap().push(name.clone());
    debug!("New user connected: {}", name.clone());

    let rx = chatroom.tx.subscribe();
    let (r, w) = conn.into_split();
    let handle = tokio::spawn(
        handle_recv(r, name.clone(), chatroom.clone())
            .instrument(info_span!("Handle Receive", name)),
    );
    tokio::spawn(
        handle_send(w, name.clone(), rx, handle).instrument(info_span!("Handle Send", name)),
    );

    chatroom.tx.send(Message {
        is_system: true,
        from: name.clone(),
        msg: format!("* {} has entered the room\n", name.clone()),
    })?;

    Ok(())
}

#[instrument(name = "Create user", skip(conn))]
async fn create_user(conn: &mut TcpStream) -> Result<Option<String>> {
    conn.write_all(b"Welcome to our chatroom! What shall I call you?\n")
        .await?;

    let mut buf = [0; 128];
    let len = conn.read(&mut buf).await?;
    if let Some(name) = &buf[..len].split(|&b| b == b'\n').next() {
        if !name.is_empty() && name.iter().all(u8::is_ascii_alphanumeric) {
            return Ok(Some(String::from_utf8_lossy(name).to_string()));
        }
    }

    conn.write_all(b"Invalid username. Closing the connection.\n")
        .await?;
    error!("Invalid username. Closing the connection.");

    Ok(None)
}

async fn handle_recv(r: OwnedReadHalf, user: String, chatroom: Arc<Chatroom>) {
    let r = BufReader::with_capacity(2048, r);
    let tx = chatroom.tx.clone();
    let mut lines = r.lines();

    while let Ok(Some(msg)) = lines.next_line().await {
        let from = user.to_string();
        debug!(msg, "Received msg");
        tx.send(Message {
            is_system: false,
            from,
            msg,
        })
        .unwrap();
    }

    chatroom.users.lock().unwrap().retain(|u| u != &user);
    debug!("User disconnected: {}", user);
    tx.send(Message {
        is_system: true,
        from: user.clone(),
        msg: format!("* {user} has left the room\n"),
    })
    .unwrap();
}

async fn handle_send(
    mut w: OwnedWriteHalf,
    user: String,
    mut rx: broadcast::Receiver<Message>,
    mut recv_handle: JoinHandle<()>,
) {
    while let Some(Message {
        is_system,
        from,
        msg,
    }) = tokio::select! {
        _ = &mut recv_handle => { None },
        msg = rx.recv() => {
            msg.ok()
        }
    } {
        if from == user {
            continue;
        }

        let msg = if is_system {
            msg
        } else {
            format!("[{from}] {msg}\n")
        };
        if let Err(e) = w.write_all(msg.as_bytes()).await {
            error!("Unable to send message to user: {}", e);
            break;
        }
    }
}
