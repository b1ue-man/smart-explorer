#!/usr/bin/env sh
set -eu

REPO="${SMART_EXPLORER_REPO:-b1ue-man/smart-explorer}"
INSTALL_DIR="${SMART_EXPLORER_INSTALL_DIR:-$HOME/.local/opt/smart-explorer}"
BIN_DIR="${SMART_EXPLORER_BIN_DIR:-$HOME/.local/bin}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$DATA_HOME/applications"
ICON_DIR="$DATA_HOME/icons/hicolor/256x256/apps"
APP_BIN="$INSTALL_DIR/smart_explorer"
UPDATER_BIN="$INSTALL_DIR/smart_explorer_updater"
BASE_URL="https://github.com/$REPO/releases/latest/download"
RAW_BASE_URL="https://raw.githubusercontent.com/$REPO/main"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "smart-explorer install: missing required command: $1" >&2
    exit 1
  fi
}

fetch() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 3 --connect-timeout 15 -o "$dest" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -O "$dest" "$url"
  else
    echo "smart-explorer install: install curl or wget first" >&2
    exit 1
  fi
}

case "$(uname -s)" in
  Linux) ;;
  *) echo "smart-explorer install: this installer is for Linux desktops only" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) ;;
  *) echo "smart-explorer install: only x86_64 Linux release assets are published right now" >&2; exit 1 ;;
esac

need chmod
need mkdir
need mktemp
need sha256sum

fetch "$BASE_URL/smart_explorer" "$TMP_DIR/smart_explorer"
fetch "$BASE_URL/smart_explorer.sha256" "$TMP_DIR/smart_explorer.sha256"
fetch "$BASE_URL/smart_explorer_updater" "$TMP_DIR/smart_explorer_updater"
fetch "$BASE_URL/smart_explorer_updater.sha256" "$TMP_DIR/smart_explorer_updater.sha256"

(
  cd "$TMP_DIR"
  sha256sum -c smart_explorer.sha256
  sha256sum -c smart_explorer_updater.sha256
)

mkdir -p "$INSTALL_DIR" "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 "$TMP_DIR/smart_explorer" "$APP_BIN"
install -m 755 "$TMP_DIR/smart_explorer_updater" "$UPDATER_BIN"
ln -sf "$APP_BIN" "$BIN_DIR/smart_explorer"
fetch "$RAW_BASE_URL/native/assets/smart-explorer-logo-256.png" "$ICON_DIR/smart-explorer.png" >/dev/null 2>&1 || true

cat > "$APP_DIR/smart-explorer.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Smart Explorer
Comment=Fast native file explorer with deep filtering
Exec=$APP_BIN
Icon=smart-explorer
Terminal=false
Categories=Utility;FileManager;
StartupNotify=true
DESKTOP
chmod +x "$APP_DIR/smart-explorer.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi

echo "Smart Explorer installed: $APP_BIN"
echo "Run it from your app launcher or with: $BIN_DIR/smart_explorer"
