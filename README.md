# mpv-remote

Control [mpv](https://mpv.io) from your phone. Press **Ctrl+r** in mpv, scan the QR code, and your phone
becomes a remote. There's no app to install: it's a web page served by a small helper that starts and
stops with mpv.

What you can do from the phone:

- Play/pause, seek, volume, previous/next, fullscreen, and skip 85 s (for openings)
- Pick audio and subtitle tracks (including secondary subtitles), adjust subtitle delay and speed, jump to chapters
- Reorder and remove items in the queue, or open files from your PC with the file browser
- Save **marks**: one tap saves the current time, with an optional note, and you can copy them all as text
- Use your phone's lock-screen media controls (optional)

Needs mpv 0.36 or newer, and your phone on the same network as the PC (or [Tailscale](#tailscale)).

## Install

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.ps1 | iex
```

**Linux / macOS**:

```sh
curl -fsSL https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.sh | sh
```

The installer finds your mpv config folder and puts the plugin in `scripts/mpv-remote/`. Run it again to
update. Your settings and marks are kept.

If it picks the wrong folder, or you want something else:

| | Windows | Linux / macOS |
|---|---|---|
| Use a specific mpv config folder | `-ConfigDir PATH` | `--config-dir PATH` |
| Install a specific version | `-Version v0.3.0` | `--version v0.3.0` |
| Uninstall | `-Uninstall` | `--uninstall` |
| Add the Windows firewall rule (run as admin) | `-OpenFirewall` | |

On Windows, options go after the script like this:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.ps1))) -OpenFirewall
```

To install by hand, download the zip for your system from the Releases page and unpack it into
`<mpv config>/scripts/mpv-remote/`.

## Pairing your phone

1. Play something in mpv and press **Ctrl+r**. A QR code appears.
2. Scan it with your phone's camera. The QR code goes away once the phone connects.
3. Optional: add the page to your home screen so it opens like an app.

You only need to do this once per phone.

**Windows firewall.** If the page never loads, Windows is probably blocking it. Either set your Wi-Fi network
to *Private* in Windows settings, or run the installer with `-OpenFirewall`.

## Settings

Settings live in `script-opts/mpv-remote.conf` in your mpv config folder. You usually don't need to change any.

| Setting | Default | What it does |
|---|---|---|
| `browse_roots` | your home folder | Folders the file browser can show. Separate several with `\|`, like `D:\Videos\|E:\Anime`. |
| `port` | `8765` | Port the phone connects to. |
| `host` | detected | The address in the QR code. Set it if the QR code shows the wrong IP. |
| `tailscale_host` | | Address to use while Tailscale is on. See [Tailscale](#tailscale). |
| `autostart` | `yes` | Start the helper with mpv. With `no`, it starts the first time you press Ctrl+r. |
| `pair_seconds` | `60` | How long the QR code stays up. |
| `token` | generated | The pairing code. Delete this line and restart mpv to unpair every phone. |

To mark from the keyboard too, add a key to `input.conf`, for example: `m script-binding mpv_remote/mark`.

Marks are saved in `<mpv config>/mpv-remote/marks/`, in a file named after the video's file name. Two
videos with the same file name in different folders share their marks.

## Tailscale

[Tailscale](https://tailscale.com) lets your phone reach your PC when they aren't on the same network.

1. Install Tailscale on the PC and the phone, signed in to the same account.
2. Add `host=tailscale` to `mpv-remote.conf`. While Tailscale is on, the QR code uses the Tailscale address.
   While it's off, the QR code uses your normal home network address, so you can leave this setting in place.
3. On Windows, run the installer with `-AllowTailscale` (as admin) so the firewall lets Tailscale through.

**Optional: HTTPS.** Tailscale can give the remote a proper `https://` address, which also lets your phone
install it as a full app. Turn on *MagicDNS* and *HTTPS Certificates* in the DNS page of the Tailscale admin
console, then run this on the PC:

```sh
tailscale serve --bg 8765
```

It prints an address like `https://pc.your-tailnet.ts.net`. Put it in the config as `tailscale_host=` (not
`host=`), and pair again.

## Installing as an app

Phones only offer a full app install for `https://` pages. Over plain home Wi-Fi you can still add a home
screen shortcut. Two ways to get the full install:

- Use Tailscale HTTPS (above).
- On Android Chrome, open `chrome://flags`, find *Insecure origins treated as secure*, add your PC's exact
  address (such as `http://192.168.1.20:8765`), and relaunch Chrome. Give your PC a fixed IP in your router
  settings, or this breaks when the IP changes.

## Troubleshooting

| Problem | Try this |
|---|---|
| The page never loads on the phone | Check the [firewall](#pairing-your-phone). Make sure the phone is on the same Wi-Fi, not mobile data or a guest network. Turn off VPNs like Cloudflare WARP. |
| The QR code shows the wrong address | Set `host=` to your PC's IP in `mpv-remote.conf`. |
| Ctrl+r does nothing | Open the mpv console with `` ` `` and look for `mpv-remote` messages. |
| The phone says "Not paired" | Press Ctrl+r and scan again. |
| The file browser can't open a folder | Add it to `browse_roots`. |
| Lock-screen controls disappear | Phones may drop them after a few minutes of pause. Turn the toggle off and on again in the Player tab. |

Only one mpv window can use the remote at a time.

## Good to know about security

- Anyone who has your pairing code and can reach your PC can control mpv. If you think it leaked, delete the
  `token` line and restart mpv to unpair everything.
- The connection is plain HTTP, so other devices on the same network could see the code. Use it on networks
  you trust, or use Tailscale.
- The phone can only control playback and open files inside `browse_roots` or the playing video's folder
  (and their subfolders). It can't run programs or change other mpv settings.

## Building from source

```sh
cargo build --release --manifest-path server/Cargo.toml
```

Copy `main.lua` and `server/target/release/mpv-remote-server[.exe]` into `<mpv config>/scripts/mpv-remote/`.
The phone page in `web/` is built into the helper.

To publish a release, push a tag such as `v0.4.0`. GitHub Actions builds every platform and creates the release.

## AI use disclosure

This entire project was generated using LLMs.
