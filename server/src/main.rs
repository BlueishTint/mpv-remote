mod browse;
mod cli;
mod hub;
mod ipc;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use browse::Roots;
use hub::Hub;

#[derive(RustEmbed)]
#[folder = "../web/"]
struct Web;

struct App {
    hub: Arc<Hub>,
    token: String,
    roots: Roots,
}

impl App {
    /// Folder of the file mpv is playing, if it is a local file.
    fn current_dir(&self) -> Option<PathBuf> {
        let path = self.hub.get_prop("path")?;
        Path::new(path.as_str()?).parent().map(Path::to_path_buf).filter(|p| p.is_dir())
    }
}

struct Args {
    pipe: String,
    port: u16,
    token: String,
    bind: String,
    roots: String,
}

fn flag_map(args: impl Iterator<Item = String>) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut it = args;
    while let Some(k) = it.next() {
        let v = it.next().ok_or_else(|| format!("missing value for {k}"))?;
        m.insert(k, v);
    }
    Ok(m)
}

fn parse_args(m: HashMap<String, String>) -> Result<Args, String> {
    let get = |k: &str| m.get(k).cloned().ok_or_else(|| format!("missing {k}"));
    Ok(Args {
        pipe: get("--pipe")?,
        port: get("--port")?.parse().map_err(|_| "bad --port".to_string())?,
        token: get("--token")?,
        bind: m.get("--bind").cloned().unwrap_or_else(|| "0.0.0.0".into()),
        roots: m.get("--roots").cloned().unwrap_or_default(),
    })
}

#[tokio::main]
async fn main() {
    const USAGE: &str = "usage:\n  mpv-remote-server serve --pipe <path> --port <n> --token <t> [--bind <addr>]\n  mpv-remote-server pair --port <n> --token <t> [--host <addr>] [--tailscale-host <addr>]\n  mpv-remote-server token";
    let mut argv = std::env::args().skip(1);
    let sub = argv.next().unwrap_or_default();
    let flags = flag_map(argv).unwrap_or_else(|e| {
        eprintln!("{USAGE}\n{e}");
        std::process::exit(2);
    });
    match sub.as_str() {
        "token" => return println!("{}", cli::token()),
        "--version" | "version" => return println!("mpv-remote-server {}", env!("CARGO_PKG_VERSION")),
        "pair" => {
            let port = flags.get("--port").and_then(|p| p.parse().ok());
            let (Some(port), Some(token)) = (port, flags.get("--token")) else {
                eprintln!("{USAGE}");
                std::process::exit(2);
            };
            if let Err(e) = cli::pair(flags.get("--host").cloned(), flags.get("--tailscale-host").cloned(), port, token) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        "serve" => {}
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
    let args = match parse_args(flags) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{USAGE}\n{e}");
            std::process::exit(2);
        }
    };

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let app = Arc::new(App {
        hub: Arc::new(Hub::new(cmd_tx)),
        token: args.token,
        roots: Roots::new(&args.roots),
    });

    let router = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/browse", get(browse_handler))
        .fallback(static_handler)
        .with_state(app.clone());

    let listener = match tokio::net::TcpListener::bind((args.bind.as_str(), args.port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {}:{}: {e}", args.bind, args.port);
            std::process::exit(1);
        }
    };

    // The server's lifetime follows mpv's: when the IPC connection ends, tell clients and exit.
    let hub_app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = ipc::run(&args.pipe, hub_app.hub.clone(), cmd_rx).await {
            eprintln!("mpv ipc: {e}");
        }
        hub_app.hub.announce_gone();
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::process::exit(0);
    });

    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("server: {e}");
    }
}

fn authorized(app: &App, q: &HashMap<String, String>) -> bool {
    q.get("t").is_some_and(|t| t.as_bytes() == app.token.as_bytes())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(q): Query<HashMap<String, String>>,
    State(app): State<Arc<App>>,
) -> Response {
    if !authorized(&app, &q) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| client(socket, app))
}

async fn client(socket: WebSocket, app: Arc<App>) {
    let (mut tx, mut rx) = socket.split();
    // Subscribe before snapshotting so no update falls between the two.
    let mut events = app.hub.subscribe();
    if tx.send(Message::Text(app.hub.snapshot().into())).await.is_err() {
        return;
    }
    // Lets main.lua dismiss the pairing QR as soon as a phone gets through.
    app.hub.send_raw(vec![json!("script-message"), json!("mpv-remote-connected")]);
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(text) => {
                    if tx.send(Message::Text(text.into())).await.is_err() { break; }
                }
                // Slow client missed events: resync with a fresh snapshot.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    if tx.send(Message::Text(app.hub.snapshot().into())).await.is_err() { break; }
                }
                Err(_) => break,
            },
            msg = rx.next() => match msg {
                // Keepalive from the phone: lets it tell an idle connection from a silently dead one.
                Some(Ok(Message::Text(text))) if text.as_str() == r#"{"ping":1}"# => {
                    if tx.send(Message::Text(r#"{"t":"pong"}"#.into())).await.is_err() { break; }
                }
                Some(Ok(Message::Text(text))) => handle_client_message(&app, &text),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
        }
    }
}

fn handle_client_message(app: &App, text: &str) {
    let Ok(Value::Object(msg)) = serde_json::from_str::<Value>(text) else { return };
    if let Some(Value::Array(cmd)) = msg.get("cmd") {
        if !app.hub.send_command(cmd.clone()) {
            eprintln!("rejected command: {cmd:?}");
        }
    }
    if let Some(load) = msg.get("load") {
        let path = load.get("path").and_then(Value::as_str).unwrap_or_default();
        let mode = match load.get("mode").and_then(Value::as_str) {
            Some("append") => "append",
            Some("append-play") => "append-play",
            _ => "replace",
        };
        let file = app.roots.allows(Path::new(path), app.current_dir().as_deref()).filter(|p| p.is_file());
        match file {
            Some(f) => app.hub.send_raw(vec![json!("loadfile"), json!(browse::display(&f)), json!(mode)]),
            None => eprintln!("rejected load: {path}"),
        }
    }
}

async fn browse_handler(
    Query(q): Query<HashMap<String, String>>,
    State(app): State<Arc<App>>,
) -> Response {
    if !authorized(&app, &q) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let current = app.current_dir();
    let start = match q.get("path").filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => current.clone().or_else(|| app.roots.first().map(Path::to_path_buf)).unwrap_or_default(),
    };
    match app.roots.list(&start, current.as_deref()) {
        Some(listing) => Json(listing).into_response(),
        None => StatusCode::FORBIDDEN.into_response(),
    }
}

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Web::get(path) {
        Some(file) => {
            let mime = if path.ends_with(".webmanifest") {
                "application/manifest+json".to_owned()
            } else {
                mime_guess::from_path(path).first_or_octet_stream().as_ref().to_owned()
            };
            ([(header::CONTENT_TYPE, mime)], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
