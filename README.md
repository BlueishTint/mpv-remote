# mpv-remote

Control [mpv](https://mpv.io) from your phone over your home Wi-Fi. It's an mpv script plus a tiny
helper server (about 1 MB, written in Rust); the phone side is a web page, so there's no app to install.

- Play/pause, seek, volume, previous/next, fullscreen, skip-OP (+85 s)
- Audio / subtitle / secondary-subtitle tracks, subtitle delay, playback speed, chapters
- Queue: jump, reorder, remove
- File browser: open or queue files on the PC from your phone
- **Marks:** one tap saves the current timestamp, with an optional note. They're stored per file and can be copied as text
- Lock-screen / notification-shade media controls (best effort, see below)
- Pair by scanning a QR code shown in mpv. The helper starts and stops with mpv.

## Install

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.ps1 | iex
```

**Linux / macOS**:

```sh
curl -fsSL https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.sh | sh
```

The installer finds your mpv config folder (scoop, portable\_config, `%APPDATA%\mpv`, `~/.config/mpv`,
Flatpak, Snap), downloads the matching release, verifies its checksum, and installs into
`scripts/mpv-remote/`. Re-running it upgrades; your settings and marks are never touched. Options:

| | Windows | Linux / macOS |
|---|---|---|
| Choose the config folder | `-ConfigDir PATH` | `--config-dir PATH` |
| Pin a version | `-Version v0.1.0` | `--version v0.1.0` |
| Remove | `-Uninstall` | `--uninstall` |
| Add a firewall rule (admin) | `-OpenFirewall` | – |
| Same, also admitting [Tailscale](#tailscale) | `-AllowTailscale` | – |

To pass options to the Windows one-liner:
`& ([scriptblock]::Create((irm <url>/install.ps1))) -OpenFirewall`.

Manual install: download the zip for your platform from the Releases page and unpack it into
`<mpv config>/scripts/mpv-remote/`, so `main.lua` and `mpv-remote-server` sit side by side.

## Pairing

1. Start mpv and play something, then press **Ctrl+r**. A QR code appears; it closes by itself as soon as
   a phone connects (or after 60 s, or when you press Ctrl+r again).
2. Scan it with your phone's camera. Your phone must be on the same network as the PC.
3. Use *Add to Home Screen* in your phone browser for an app-like icon. The phone remembers the token,
   so you only pair once.

### Windows firewall

Windows blocks incoming connections on networks marked **Public**. Either mark your Wi-Fi as Private, or
run this once in an admin PowerShell. It only admits devices on your own subnet:

```powershell
New-NetFirewallRule -DisplayName mpv-remote -Direction Inbound -Action Allow -Protocol TCP -LocalPort 8765 -RemoteAddress LocalSubnet -Profile Any
```

## Settings

`script-opts/mpv-remote.conf` in your mpv config folder:

| Option | Default | |
|---|---|---|
| `token` | generated | Pairing secret. Delete the line to invalidate all paired phones. |
| `port` | `8765` | |
| `bind` | `0.0.0.0` | Interface to listen on. `127.0.0.1` disables phone access. |
| `host` | auto | Address in the QR code. Empty = auto-detected LAN IP. `tailscale` = this PC's [Tailscale](#tailscale) address. Or an IP/name, or a full URL such as `https://pc.example.ts.net`. |
| `browse_roots` | your home folder | Folders the phone may browse and open files from, separated by `\|`. |
| `autostart` | `yes` | Start the helper when mpv starts. |
| `pair_seconds` | `60` | How long the QR code stays up. |
| `server_path` | next to `main.lua` | Override the helper location (development). |

Key bindings: **Ctrl+r** shows the pairing QR. To bind a key for marking from the keyboard, add to
`input.conf`: `m script-binding mpv_remote/mark`.

Marks are saved as JSON in `<mpv config>/mpv-remote/marks/<file name>.json`.

## Tailscale

[Tailscale](https://tailscale.com) is a free VPN that gives your devices private addresses that work from
anywhere. Use it to control mpv when your phone and PC can't see each other on the same network, or to get a
trusted HTTPS address.

1. Install Tailscale on the PC and on the phone, and sign in to the same account on both.
2. In `mpv-remote.conf` set `host=tailscale`, so the QR code carries the PC's Tailscale address.
3. Let Tailscale traffic through the Windows firewall. The default rule only admits your local subnet, so run
   the installer with `-AllowTailscale`, or in an admin PowerShell:

   ```powershell
   New-NetFirewallRule -DisplayName mpv-remote -Direction Inbound -Action Allow -Protocol TCP -LocalPort 8765 -RemoteAddress LocalSubnet,'100.64.0.0/10' -Profile Any
   ```

4. Press Ctrl+r and scan the QR code with the phone as usual.

**HTTPS and a full app install.** Tailscale can also publish the helper under a real certificate, which lets the
page install as an app without the Chrome flag. This needs MagicDNS and HTTPS Certificates enabled once on the
DNS page of the Tailscale admin console. Then, on the PC run:

```sh
tailscale serve --bg 8765
```

It prints an address like `https://pc.your-tailnet.ts.net`. Put that in the config as
`host=https://pc.your-tailnet.ts.net` and pair again. This route is reached through Tailscale itself, so the
firewall rule above isn't needed for it, and traffic is encrypted.

Tailscale's addresses are in `100.64.0.0/10`. The QR code's automatic detection ignores virtual adapters on
purpose, which is why `host=tailscale` has to be set explicitly.

## Security

Anyone on your network who has the token can control mpv, so treat it like a password and don't pair
on networks you don't trust. The connection is plain HTTP, which is visible to other devices on the same network.

- Every request needs the token; the phone UI itself is the only thing served without it.
- The phone can only send a fixed list of commands (playback, tracks, playlist, marks). It can't run
  programs, load scripts or change arbitrary mpv settings.
- File browsing and `loadfile` are limited to `browse_roots`, plus the folder of the playing file.

## Limitations

- **Full app install needs HTTPS.** Browsers only offer it from a secure origin, and a LAN address over
  HTTP doesn't qualify. *Add to Home Screen* still gives you a shortcut. The page ships a web app manifest,
  icons and a service worker, so it becomes installable wherever the browser trusts the origin: for example
  behind [Tailscale Serve](https://tailscale.com/kb/1242/tailscale-serve) (real HTTPS certificate), or, as a
  on Android Chrome, by adding the exact address (`http://192.168.x.x:8765`, with scheme and port) under
  `chrome://flags` → "Insecure origins treated as secure" and relaunching. This has been confirmed to give
  a full install. Reserve a fixed IP for the PC on your router, because the flag only matches that address.
- **Lock-screen controls** work by playing a silent clip in the page, which phones need before they show
  media controls. Android Chrome and iOS Safari behave differently, and a paused page may lose the controls
  after a few minutes. Turn the toggle on under the Player tab.
- One mpv window at a time: a second mpv finds the port in use and its helper exits.
- VPNs such as Cloudflare WARP can interfere. The QR code ignores virtual adapters, but if your phone
  can't connect, try turning the VPN off while using the remote.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| Phone page never loads | Firewall (see above), Wi-Fi "client isolation" / guest network, or phone on mobile data |
| QR shows the wrong IP | Set `host=` in `mpv-remote.conf` |
| Ctrl+r does nothing | Open the mpv console (`` ` ``) and look for `mpv-remote` messages; check `mpv-remote-server` sits next to `main.lua` |
| "Not paired" on the phone | Scan the QR again (the token may have been reset) |
| Marks or browse list empty | Check `browse_roots`; marks are keyed by file name |

## Building from source

Needs a recent stable Rust toolchain.

```sh
cargo build --release --manifest-path server/Cargo.toml
```

Copy `main.lua` and `server/target/release/mpv-remote-server[.exe]` into
`<mpv config>/scripts/mpv-remote/`. The web UI in `web/` is embedded into the binary at build time
(debug builds read it from disk, so UI edits show up on refresh).

### Layout

| | |
|---|---|
| `main.lua` | mpv side: starts/stops the helper, token, pairing QR, marks storage |
| `server/` | Rust helper: mpv JSON-IPC client, WebSocket state sync, file browsing, command allowlist |
| `web/` | Phone UI (plain HTML/CSS/JS, no build step) |
| `install.ps1`, `install.sh` | Installers |
| `.github/workflows/release.yml` | Builds Windows/Linux/macOS binaries on a `v*` tag |

### Releasing

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The workflow builds `mpv-remote-{windows,linux,macos}-{x86_64,aarch64}.zip` (Windows: x86\_64 only),
plus `SHA256SUMS`, and publishes them as a GitHub release. The installers download from there.
