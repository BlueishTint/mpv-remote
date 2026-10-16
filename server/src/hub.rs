//! Shared state between the mpv IPC task and phone clients.

use std::sync::Mutex;

use serde_json::{Map, Value, json};
use tokio::sync::{broadcast, mpsc};

pub struct Hub {
    props: Mutex<Map<String, Value>>,
    events: broadcast::Sender<String>,
    commands: mpsc::UnboundedSender<Value>,
}

impl Hub {
    pub fn new(commands: mpsc::UnboundedSender<Value>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self { props: Mutex::default(), events, commands }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.events.subscribe()
    }

    pub fn set_prop(&self, name: &str, data: Value) {
        self.props.lock().unwrap().insert(name.to_owned(), data.clone());
        let _ = self.events.send(json!({ "t": "prop", "name": name, "data": data }).to_string());
    }

    pub fn get_prop(&self, name: &str) -> Option<Value> {
        self.props.lock().unwrap().get(name).cloned()
    }

    /// Queues an already-validated command (the caller did its own checks).
    pub fn send_raw(&self, cmd: Vec<Value>) {
        let _ = self.commands.send(Value::Array(cmd));
    }

    pub fn snapshot(&self) -> String {
        json!({ "t": "state", "props": *self.props.lock().unwrap() }).to_string()
    }

    pub fn announce_gone(&self) {
        let _ = self.events.send(json!({ "t": "gone" }).to_string());
    }

    /// Queues a command for mpv if it passes the allowlist. Returns whether it was accepted.
    pub fn send_command(&self, cmd: Vec<Value>) -> bool {
        if !is_allowed(&cmd) {
            return false;
        }
        self.commands.send(Value::Array(cmd)).is_ok()
    }
}

/// Properties the phone may change. A leaked token must not become code execution on the PC,
/// so `run`, `subprocess`, `load-script`, arbitrary `script-message`s etc. are never forwarded.
const SETTABLE: &[&str] = &[
    "pause", "volume", "mute", "fullscreen", "aid", "sid", "secondary-sid", "sub-delay",
    "audio-delay", "speed", "chapter", "playlist-pos", "loop-file", "loop-playlist",
    "sub-visibility",
];

fn is_allowed(cmd: &[Value]) -> bool {
    let Some(name) = cmd.first().and_then(Value::as_str) else { return false };
    let prop_ok = || cmd.get(1).and_then(Value::as_str).is_some_and(|p| SETTABLE.contains(&p));
    match name {
        "set" | "set_property" | "add" | "cycle" | "cycle-values" => prop_ok(),
        "seek" | "playlist-next" | "playlist-prev" | "playlist-play-index" | "playlist-move"
        | "playlist-remove" | "playlist-shuffle" | "playlist-unshuffle" | "show-text"
        | "sub-seek" | "frame-step" | "frame-back-step" | "stop" => true,
        // Only our own plugin's messages (marks etc.).
        "script-message" => cmd
            .get(1)
            .and_then(Value::as_str)
            .is_some_and(|m| m.starts_with("mpv-remote-")),
        _ => false,
    }
}
