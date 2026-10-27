//! mpv JSON IPC client: connects to mpv's pipe/socket, mirrors observed
//! properties into the hub, and writes commands queued by phone clients.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::hub::Hub;

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Properties mirrored to the phone. `time-pos` is throttled in `run`.
const OBSERVED: &[&str] = &[
    "pause",
    "time-pos",
    "duration",
    "volume",
    "mute",
    "speed",
    "media-title",
    "path",
    "filename",
    "fullscreen",
    "idle-active",
    "track-list",
    "chapter-list",
    "chapter",
    "playlist",
    "playlist-pos",
    "sub-delay",
    // Maintained by main.lua; the phone's marks list.
    "user-data/mpv-remote/marks",
];

const TIME_POS_STEP: f64 = 0.25;

async fn connect(path: &str) -> io::Result<Box<dyn Stream>> {
    let mut last_err = io::Error::other("no attempts made");
    // mpv creates the pipe during startup; give it a moment.
    for _ in 0..50 {
        #[cfg(windows)]
        let attempt = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(path)
            .map(|c| Box::new(c) as Box<dyn Stream>);
        #[cfg(unix)]
        let attempt = tokio::net::UnixStream::connect(path)
            .await
            .map(|c| Box::new(c) as Box<dyn Stream>);
        match attempt {
            Ok(s) => return Ok(s),
            Err(e) => last_err = e,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(last_err)
}

/// Runs until mpv closes the connection.
pub async fn run(path: &str, hub: Arc<Hub>, mut commands: tokio::sync::mpsc::UnboundedReceiver<Value>) -> io::Result<()> {
    let stream = connect(path).await?;
    let (rd, mut wr) = tokio::io::split(stream);

    for (i, name) in OBSERVED.iter().enumerate() {
        let line = json!({ "command": ["observe_property", i + 1, name] });
        wr.write_all(format!("{line}\n").as_bytes()).await?;
    }

    let writer = tokio::spawn(async move {
        while let Some(cmd) = commands.recv().await {
            let line = json!({ "command": cmd });
            if wr.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(rd).lines();
    let mut last_time: Option<(f64, Instant)> = None;
    while let Some(line) = lines.next_line().await? {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        match msg.get("event").and_then(Value::as_str) {
            Some("property-change") => {
                let Some(name) = msg.get("name").and_then(Value::as_str) else { continue };
                let data = msg.get("data").cloned().unwrap_or(Value::Null);
                if name == "time-pos" {
                    if let Some(t) = data.as_f64() {
                        // Skip sub-step changes while playing; always pass seeks/pauses through.
                        if let Some((prev, at)) = last_time {
                            if (t - prev).abs() < TIME_POS_STEP && at.elapsed() < Duration::from_secs(1) {
                                continue;
                            }
                        }
                        last_time = Some((t, Instant::now()));
                    }
                }
                hub.set_prop(name, data);
            }
            Some("shutdown") => break,
            _ => {}
        }
    }
    writer.abort();
    Ok(())
}
