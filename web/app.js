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
}

/* ---------- tracks & chapters ---------- */

function trackLabel(t) {
  const parts = [];
  if (t.lang) parts.push(`[${t.lang}]`);
  parts.push(t.title || `Track ${t.id}`);
  if (t.codec) parts.push(`(${t.codec})`);
  return parts.join(" ");
}

function trackRows(type, prop, isSelected) {
  const tracks = asArray(state["track-list"]).filter((t) => t.type === type);
  const none = !tracks.some(isSelected);
  const rows = [el("div", { class: "item" + (none ? " current" : ""), onclick: () => send(["set", prop, "no"]) },
    el("span", { class: "label" }, "Off"))];
  for (const t of tracks) {
    rows.push(el("div", { class: "item" + (isSelected(t) ? " current" : ""), onclick: () => send(["set", prop, t.id]) },
      el("span", { class: "label" }, trackLabel(t))));
  }
  return rows;
}

function renderTracks() {
  const main = (t) => t.selected && (t["main-selection"] ?? 0) === 0;
  const second = (t) => t.selected && t["main-selection"] === 1;
  setList("audio-list", trackRows("audio", "aid", (t) => t.selected), "No audio tracks");
  setList("sub-list", trackRows("sub", "sid", main), "No subtitles");
  setList("sub2-list", trackRows("sub", "secondary-sid", second), "No subtitles");
  $("subdelay").textContent = `${(state["sub-delay"] ?? 0).toFixed(1)} s`;

  const speed = $("speed-row");
  if (!speed.children.length) {
    for (const s of [0.75, 1, 1.25, 1.5, 2]) {
      speed.append(el("button", { dataset: { speed: s }, onclick: () => send(["set", "speed", s]) }, `${s}×`));
    }
  }
  for (const b of speed.children) b.style.outline = Number(b.dataset.speed) === state.speed ? "2px solid var(--accent)" : "";

  const chapters = asArray(state["chapter-list"]);
  setList("chapter-list", chapters.map((c, i) =>
    el("div", { class: "item" + (i === state.chapter ? " current" : ""), onclick: () => send(["set", "chapter", i]) },
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
  if (document.activeElement && document.activeElement.closest("#marks")) return; // don't clobber typing
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
$("vol").addEventListener("input", (e) => send(["set", "volume", Number(e.target.value)]));

token = loadToken();
connect();
