"use strict";

const $ = (id) => document.getElementById(id);
const MARKS = "user-data/mpv-remote/marks";
const state = {};
let ws = null;
let seeking = false;
let view = "player";
let token = null;
let files = { loaded: false, path: "", parent: null };

/* ---------- helpers ---------- */

function loadToken() {
  const fromUrl = new URLSearchParams(location.search).get("t");
  try {
    if (fromUrl) {
      localStorage.setItem("mpv-remote-token", fromUrl);
      // Keep the token out of the address bar / history once stored.
      history.replaceState(null, "", location.pathname);
    }
    return fromUrl || localStorage.getItem("mpv-remote-token");
  } catch {
    return fromUrl;
  }
}

function fmt(sec) {
  if (!Number.isFinite(sec)) return "0:00";
  sec = Math.max(0, Math.floor(sec));
  const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec % 60;
  const pad = (n) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

function send(cmd) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ cmd }));
  if (navigator.vibrate) navigator.vibrate(8);
}

function sendMsg(obj) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(obj));
  if (navigator.vibrate) navigator.vibrate(8);
}

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") node.className = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else if (k === "dataset") Object.assign(node.dataset, v);
    else if (k.includes("-")) node.setAttribute(k, v);
    else node[k] = v;
  }
  node.append(...children);
  return node;
}

function basename(p) {
  return String(p || "").split(/[\\/]/).pop();
}

function setList(id, rows, emptyText) {
  const box = $(id);
  box.replaceChildren(...(rows.length ? rows : [el("p", { class: "empty" }, emptyText)]));
}

const asArray = (v) => (Array.isArray(v) ? v : []);

/* ---------- player ---------- */

function renderPlayer() {
  const title = state["media-title"] || state.filename || (state["idle-active"] ? "Nothing playing" : "—");
  $("title").textContent = title;
  $("mini-title").textContent = title;
  const icon = state.pause ? "▶" : "⏸";
  $("play").textContent = icon;
  $("mini-play").textContent = icon;
  $("mute").textContent = state.mute ? "Unmute" : "Mute";
  $("dur").textContent = fmt(state.duration);
  if (!seeking) {
    $("pos").textContent = fmt(state["time-pos"]);
    const d = state.duration;
    $("seek").value = d > 0 ? Math.round((state["time-pos"] / d) * 1000) : 0;
  }
  if (document.activeElement !== $("vol") && typeof state.volume === "number") $("vol").value = state.volume;
  $("volnum").textContent = Math.round(state.volume ?? 0);
  updateMediaSession();
}

/* ---------- lock-screen controls ---------- */
// Phones only show media controls for a page that is playing audio, so we loop a silent clip
// and mirror mpv's state into the Media Session. The clip keeps playing even while mpv is
// paused; the session's playbackState is what the lock screen shows.

const lock = { on: false, audio: null, title: null, lastPos: 0 };

function silentWav(seconds = 10, rate = 8000) {
  const n = seconds * rate, buf = new ArrayBuffer(44 + n), v = new DataView(buf);
  const text = (o, s) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  text(0, "RIFF"); v.setUint32(4, 36 + n, true); text(8, "WAVEfmt ");
  v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate, true); v.setUint16(32, 1, true); v.setUint16(34, 8, true);
  text(36, "data"); v.setUint32(40, n, true);
  new Uint8Array(buf, 44).fill(128); // 8-bit PCM silence
  return URL.createObjectURL(new Blob([buf], { type: "audio/wav" }));
}

function setupMediaSession() {
  if (!("mediaSession" in navigator)) return;
  const ms = navigator.mediaSession;
  const handlers = {
    // Toggle rather than set: if the phone's idea of the state is stale (page throttled while
    // locked), pressing the button still does what the user expects against mpv's real state.
    play: () => send(["cycle", "pause"]),
    pause: () => send(["cycle", "pause"]),
    stop: () => send(["set_property", "pause", true]),
    previoustrack: () => send(["playlist-prev"]),
    nexttrack: () => send(["playlist-next"]),
    seekbackward: () => send(["seek", -10]),
    seekforward: () => send(["seek", 10]),
    seekto: (d) => send(["seek", d.seekTime, "absolute"]),
  };
  for (const [action, fn] of Object.entries(handlers)) {
    try { ms.setActionHandler(action, fn); } catch { /* unsupported action */ }
  }
}

function updateMediaSession() {
  if (!lock.on || !("mediaSession" in navigator)) return;
  const ms = navigator.mediaSession;
  const title = state["media-title"] || state.filename || "mpv";
  if (title !== lock.title) {
    lock.title = title;
    ms.metadata = new MediaMetadata({ title, artist: "mpv" });
  }
  // Browsers derive the lock-screen play/pause icon from the audio element, not from
  // playbackState, so the silent clip has to follow mpv's pause state.
  if (lock.audio) {
    if (state.pause && !lock.audio.paused) lock.audio.pause();
    else if (!state.pause && lock.audio.paused) lock.audio.play().catch(() => {});
  }
  ms.playbackState = state.pause ? "paused" : "playing";
  const d = state.duration, p = state["time-pos"];
  // Position updates are cheap but pointless more than ~once a second.
  if (d > 0 && Number.isFinite(p) && (Date.now() - lock.lastPos > 1000 || state.pause)) {
    lock.lastPos = Date.now();
    try {
      ms.setPositionState({ duration: d, position: Math.min(p, d), playbackRate: state.pause ? 0.0001 : (state.speed || 1) });
    } catch { /* invalid state while loading */ }
  }
}

async function setLock(on) {
  lock.on = on;
  try { localStorage.setItem("mpv-remote-lock", on ? "1" : "0"); } catch { /* private mode */ }
  $("lock").checked = on;
  if (!on) {
    if (lock.audio) lock.audio.pause();
    if ("mediaSession" in navigator) navigator.mediaSession.metadata = null;
    lock.title = null;
    return;
  }
  setupMediaSession();
  if (!lock.audio) {
    lock.audio = new Audio(silentWav());
    lock.audio.loop = true;
  }
  try {
    await lock.audio.play();
    updateMediaSession(); // pauses the clip again right away if mpv is paused
  } catch {
    // Autoplay needs a tap; resume on the next one.
    document.addEventListener("pointerdown", () => lock.on && lock.audio.play().then(updateMediaSession).catch(() => {}), { once: true });
  }
}

/* ---------- tracks & chapters ---------- */

function trackLabel(t) {
  const parts = [];
  if (t.lang) parts.push(`[${t.lang}]`);
  parts.push(t.title || `Track ${t.id}`);
  if (t.codec) parts.push(`(${t.codec})`);
  return parts.join(" ");
}

function trackRows(type, prop, isSelected, exclude = () => false) {
  const tracks = asArray(state["track-list"]).filter((t) => t.type === type && !exclude(t));
  const none = !tracks.some(isSelected);
  const rows = [el("div", { class: "item" + (none ? " current" : ""), onclick: () => send(["set_property", prop, "no"]) },
    el("span", { class: "label" }, "Off"))];
  for (const t of tracks) {
    rows.push(el("div", { class: "item" + (isSelected(t) ? " current" : ""), onclick: () => send(["set_property", prop, t.id]) },
      el("span", { class: "label" }, trackLabel(t))));
  }
  return rows;
}

function renderTracks() {
  const main = (t) => t.selected && (t["main-selection"] ?? 0) === 0;
  const second = (t) => t.selected && t["main-selection"] === 1;
  setList("audio-list", trackRows("audio", "aid", (t) => t.selected), "No audio tracks");
  setList("sub-list", trackRows("sub", "sid", main), "No subtitles");
  // mpv refuses the same track as both primary and secondary, so don't offer it.
  setList("sub2-list", trackRows("sub", "secondary-sid", second, main), "No subtitles");
  $("subdelay").textContent = `${(state["sub-delay"] ?? 0).toFixed(1)} s`;

  const speed = $("speed-row");
  if (!speed.children.length) {
    for (const s of [0.75, 1, 1.25, 1.5, 2]) {
      speed.append(el("button", { dataset: { speed: s }, onclick: () => send(["set_property", "speed", s]) }, `${s}×`));
    }
  }
  for (const b of speed.children) b.style.outline = Number(b.dataset.speed) === state.speed ? "2px solid var(--accent)" : "";

  const chapters = asArray(state["chapter-list"]);
  setList("chapter-list", chapters.map((c, i) =>
    el("div", { class: "item" + (i === state.chapter ? " current" : ""), onclick: () => send(["set_property", "chapter", i]) },
      el("span", { class: "label" }, c.title || `Chapter ${i + 1}`),
      el("span", { class: "meta" }, fmt(c.time)))), "No chapters");
}

/* ---------- playlist ---------- */

function renderPlaylist() {
  const items = asArray(state.playlist);
  setList("playlist", items.map((it, i) => el("div", { class: "item" + (it.current ? " current" : "") },
    el("span", { class: "label", onclick: () => send(["playlist-play-index", i]) }, it.title || basename(it.filename)),
    i > 0 ? el("button", { "aria-label": "Move up", onclick: () => send(["playlist-move", i, i - 1]) }, "↑") : "",
    i < items.length - 1 ? el("button", { "aria-label": "Move down", onclick: () => send(["playlist-move", i, i + 2]) }, "↓") : "",
    el("button", { class: "danger", "aria-label": "Remove", onclick: () => send(["playlist-remove", i]) }, "✕"))),
  "Playlist is empty");
}

/* ---------- file browser ---------- */

async function browse(path) {
  try {
    const q = new URLSearchParams({ t: token, path: path || "" });
    const res = await fetch(`/api/browse?${q}`);
    if (!res.ok) throw new Error(res.status === 403 ? "That folder isn't allowed (see browse_roots)." : `Error ${res.status}`);
    const data = await res.json();
    files = { loaded: true, path: data.path, parent: data.parent };
    $("files-path").textContent = data.path;
    $("files-up").disabled = !data.parent;
    const sep = data.path.includes("\\") ? "\\" : "/";
    const full = (name) => data.path.replace(/[\\/]$/, "") + sep + name;
    setList("files", data.entries.map((e) => e.dir
      ? el("div", { class: "item", onclick: () => browse(full(e.name)) }, el("span", { class: "label" }, `📁 ${e.name}`))
      : el("div", { class: "item" },
          el("span", { class: "label", onclick: () => sendMsg({ load: { path: full(e.name), mode: "replace" } }) }, e.name),
          el("button", { "aria-label": "Add to queue", onclick: () => sendMsg({ load: { path: full(e.name), mode: "append" } }) }, "+"))),
    "No media files here");
  } catch (err) {
    setList("files", [], String(err.message || err));
  }
}

/* ---------- marks ---------- */

function marksList() {
  return asArray(state[MARKS]);
}

function renderMarks() {
  // Don't rebuild the list under a note being typed; buttons holding focus (e.g. a just-tapped ✕) are fine.
  if (document.activeElement && document.activeElement.matches("#marks input")) return;
  const marks = marksList();
  setList("marks", marks.map((m) => {
    const note = el("input", { type: "text", value: m.note || "", placeholder: "Add a note", "aria-label": "Note" });
    note.addEventListener("change", () => send(["script-message", "mpv-remote-mark-note", String(m.id), note.value]));
    note.addEventListener("blur", () => renderMarks());
    return el("div", { class: "item" },
      el("button", { class: "time", onclick: () => send(["seek", m.t, "absolute"]) }, fmt(m.t)),
      note,
      el("button", { class: "danger", "aria-label": "Delete mark", onclick: () => send(["script-message", "mpv-remote-mark-delete", String(m.id)]) }, "✕"));
  }), "No marks for this file yet");
}

function addMark() {
  send(["script-message", "mpv-remote-mark-add", ""]);
  const t = fmt(state["time-pos"]);
  $("mark-toast").textContent = `Marked ${t}`;
  $("marks-hint").textContent = "";
  setTimeout(() => { $("mark-toast").textContent = ""; }, 2500);
}

function copyMarks() {
  const marks = marksList();
  if (!marks.length) return;
  const text = marks.map((m) => `${fmt(m.t)}${m.note ? ` — ${m.note}` : ""}`).join("\n");
  // The page is served over plain http, so navigator.clipboard is unavailable; use a textarea.
  const ta = $("marks-text");
  ta.value = text;
  ta.select();
  let ok = false;
  try { ok = document.execCommand("copy"); } catch { /* fall through */ }
  $("marks-hint").textContent = ok ? "Copied." : "Couldn't copy automatically.";
}

/* ---------- views ---------- */

function showView(name) {
  view = name;
  document.querySelectorAll("[data-view]").forEach((s) => { s.hidden = s.dataset.view !== name; });
  document.querySelectorAll("[data-tab]").forEach((b) => b.classList.toggle("active", b.dataset.tab === name));
  $("mini").hidden = name === "player";
  if (name === "files" && !files.loaded) browse("");
  renderAll();
}

function renderAll() {
  renderPlayer();
  if (view === "tracks") renderTracks();
  if (view === "playlist") renderPlaylist();
  if (view === "marks") renderMarks();
}

function onProps(names) {
  renderPlayer();
  const hit = (...keys) => names.some((n) => keys.includes(n));
  if (view === "tracks" && hit("track-list", "chapter-list", "chapter", "sub-delay", "speed")) renderTracks();
  if (view === "playlist" && hit("playlist", "playlist-pos")) renderPlaylist();
  if (view === "marks" && hit(MARKS)) renderMarks();
}

/* ---------- connection ---------- */

function connect() {
  if (!token) {
    $("status").textContent = "Not paired. Scan the QR code shown in mpv (Ctrl+r).";
    return;
  }
  ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws?t=${encodeURIComponent(token)}`);
  ws.onopen = () => { $("status").textContent = ""; };
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.t === "state") {
      Object.assign(state, msg.props);
      renderAll();
    } else if (msg.t === "prop") {
      state[msg.name] = msg.data;
      onProps([msg.name]);
    } else if (msg.t === "gone") {
      $("status").textContent = "mpv closed.";
    }
  };
  ws.onclose = () => {
    if ($("status").textContent !== "mpv closed.") $("status").textContent = "Reconnecting…";
    setTimeout(connect, 1500);
  };
}

/* ---------- wiring ---------- */

document.querySelectorAll("[data-cmd]").forEach((b) =>
  b.addEventListener("click", () => send(JSON.parse(b.dataset.cmd))));
document.querySelectorAll("[data-tab]").forEach((b) =>
  b.addEventListener("click", () => showView(b.dataset.tab)));
for (const id of ["play", "mini-play"]) $(id).addEventListener("click", () => send(["cycle", "pause"]));
for (const id of ["mark", "mini-mark", "marks-add"]) $(id).addEventListener("click", addMark);
$("marks-copy").addEventListener("click", copyMarks);
$("files-up").addEventListener("click", () => files.parent && browse(files.parent));

const seek = $("seek");
seek.addEventListener("input", () => {
  seeking = true;
  $("pos").textContent = fmt((seek.value / 1000) * (state.duration || 0));
});
seek.addEventListener("change", () => {
  send(["seek", (seek.value / 1000) * (state.duration || 0), "absolute"]);
  seeking = false;
});
$("vol").addEventListener("input", (e) => send(["set_property", "volume", Number(e.target.value)]));

$("lock").addEventListener("change", (e) => setLock(e.target.checked));
try { if (localStorage.getItem("mpv-remote-lock") === "1") setLock(true); } catch { /* ignore */ }

token = loadToken();
connect();
