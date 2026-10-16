#!/bin/sh
# Installs (or upgrades / removes) the mpv-remote plugin for mpv on Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --config-dir ~/.config/mpv --version v0.1.0
#
# Options (or the MPV_REMOTE_* environment variables):
#   --config-dir DIR   mpv config folder (auto-detected when omitted)
#   --version TAG      release tag, default: latest
#   --repo OWNER/NAME  GitHub repository to download from
#   --source DIR|URL   folder or base URL holding the release assets (offline / testing)
#   --uninstall        remove the plugin; settings and saved marks are kept
set -eu

REPO="${MPV_REMOTE_REPO:-BlueishTint/mpv-remote}"
VERSION="${MPV_REMOTE_VERSION:-latest}"
CONFIG_DIR="${MPV_REMOTE_CONFIG_DIR:-}"
SOURCE="${MPV_REMOTE_SOURCE:-}"
UNINSTALL=0
PORT=8765

die() { printf 'Error: %s\n' "$*" >&2; exit 1; }
step() { printf '==> %s\n' "$*"; }
need() { command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not installed."; }

while [ $# -gt 0 ]; do
  case "$1" in
    --config-dir) [ $# -ge 2 ] || die "$1 needs a value"; CONFIG_DIR=$2; shift 2 ;;
    --version)    [ $# -ge 2 ] || die "$1 needs a value"; VERSION=$2; shift 2 ;;
    --repo)       [ $# -ge 2 ] || die "$1 needs a value"; REPO=$2; shift 2 ;;
    --source)     [ $# -ge 2 ] || die "$1 needs a value"; SOURCE=$2; shift 2 ;;
    --uninstall)  UNINSTALL=1; shift ;;
    -h|--help)    sed -n '2,13p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)            die "unknown option: $1" ;;
  esac
done

# ---- platform ----
case "$(uname -s)" in
  Linux)  OS=linux ;;
  Darwin) OS=macos ;;
  *)      die "unsupported OS: $(uname -s) (use install.ps1 on Windows)" ;;
esac
case "$(uname -m)" in
  x86_64|amd64)  ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *)             die "unsupported CPU: $(uname -m)" ;;
esac
ASSET="mpv-remote-$OS-$ARCH.zip"

# ---- config dir ----
candidates() {
  [ -d "${XDG_CONFIG_HOME:-$HOME/.config}/mpv" ] && printf '%s\n' "${XDG_CONFIG_HOME:-$HOME/.config}/mpv"
  [ -d "$HOME/.config/mpv" ] && [ "${XDG_CONFIG_HOME:-$HOME/.config}" != "$HOME/.config" ] && printf '%s\n' "$HOME/.config/mpv"
  [ -d "$HOME/.var/app/io.mpv.Mpv/config/mpv" ] && printf '%s\n' "$HOME/.var/app/io.mpv.Mpv/config/mpv"
  [ -d "$HOME/snap/mpv/current/.config/mpv" ] && printf '%s\n' "$HOME/snap/mpv/current/.config/mpv"
  [ "$OS" = macos ] && [ -d "$HOME/Library/Preferences/mpv" ] && printf '%s\n' "$HOME/Library/Preferences/mpv"
  return 0
}

choose_config_dir() {
  [ -n "$CONFIG_DIR" ] && return 0
  found=$(candidates)
  if [ -z "$found" ]; then
    CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/mpv"
    return 0
  fi
  count=$(printf '%s\n' "$found" | wc -l | tr -d ' ')
  if [ "$count" -eq 1 ] || [ ! -r /dev/tty ]; then
    CONFIG_DIR=$(printf '%s\n' "$found" | head -n 1)
    return 0
  fi
  echo "Found several mpv config folders:"
  printf '%s\n' "$found" | awk '{ printf "  [%d] %s\n", NR, $0 }'
  printf 'Install into which one? [1] '
  read -r answer </dev/tty || answer=
  answer=${answer:-1}
  CONFIG_DIR=$(printf '%s\n' "$found" | sed -n "${answer}p")
  [ -n "$CONFIG_DIR" ] || die "invalid choice: $answer"
}

choose_config_dir
TARGET="$CONFIG_DIR/scripts/mpv-remote"

# ---- uninstall ----
if [ "$UNINSTALL" -eq 1 ]; then
  step "Removing $TARGET"
  rm -rf "$TARGET"
  echo "Removed. Settings (script-opts/mpv-remote.conf) and marks (mpv-remote/marks) were left in $CONFIG_DIR."
  exit 0
fi

# ---- download ----
need unzip
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q "$1" -O "$2"; }
else
  die "curl or wget is required."
fi

if [ -n "$SOURCE" ] && [ -d "$SOURCE" ]; then
  get() { cp "$SOURCE/$1" "$2"; }
else
  if [ -n "$SOURCE" ]; then BASE=${SOURCE%/}
  elif [ "$VERSION" = latest ]; then BASE="https://github.com/$REPO/releases/latest/download"
  else BASE="https://github.com/$REPO/releases/download/$VERSION"
  fi
  get() { fetch "$BASE/$1" "$2"; }
fi

if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
else die "sha256sum or shasum is required."
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

step "Downloading $ASSET ($VERSION)"
get "$ASSET" "$TMP/$ASSET" || die "download failed: $ASSET"
get SHA256SUMS "$TMP/SHA256SUMS" || die "download failed: SHA256SUMS"

step "Verifying checksum"
expected=$(grep " $ASSET\$" "$TMP/SHA256SUMS" | head -n 1 | cut -d' ' -f1)
[ -n "$expected" ] || die "SHA256SUMS has no entry for $ASSET"
actual=$(sha "$TMP/$ASSET")
[ "$expected" = "$actual" ] || die "checksum mismatch for $ASSET (expected $expected, got $actual)"

step "Installing to $TARGET"
mkdir -p "$TARGET"
unzip -oq "$TMP/$ASSET" -d "$TARGET"
chmod +x "$TARGET/mpv-remote-server"

CONF="$CONFIG_DIR/script-opts/mpv-remote.conf"
if [ ! -f "$CONF" ]; then
  mkdir -p "$CONFIG_DIR/script-opts"
  cat >"$CONF" <<'EOF'
# mpv-remote settings. The pairing token is added automatically on first run.
# port=8765
# host=            # LAN IP shown in the QR code, if the wrong one is picked
# browse_roots=    # folders the phone may browse, separated by |  (default: your home folder)
# autostart=yes
EOF
fi

printf '\nInstalled mpv-remote %s into %s\n' "$(cat "$TARGET/VERSION" 2>/dev/null || true)" "$CONFIG_DIR"
echo "Next: start mpv, play something, press Ctrl+r and scan the QR code with your phone."
if command -v ufw >/dev/null 2>&1; then
  echo "If your phone can't connect and ufw is active:  sudo ufw allow from 192.168.0.0/16 to any port $PORT proto tcp"
fi
