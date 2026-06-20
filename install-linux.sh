#!/usr/bin/env sh
set -eu

REPO="${SMART_EXPLORER_REPO:-b1ue-man/smart-explorer}"
REF="${SMART_EXPLORER_REF:-main}"
INSTALL_DIR="${SMART_EXPLORER_INSTALL_DIR:-$HOME/.local/opt/smart-explorer}"
BIN_DIR="${SMART_EXPLORER_BIN_DIR:-$HOME/.local/bin}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$DATA_HOME/applications"
ICON_DIR="$DATA_HOME/icons/hicolor/256x256/apps"
APP_BIN="$INSTALL_DIR/smart_explorer"
UPDATER_BIN="$INSTALL_DIR/smart_explorer_updater"
BASE_URL="https://github.com/$REPO/releases/latest/download"
RAW_BASE_URL="https://raw.githubusercontent.com/$REPO/$REF"
SRC_ARCHIVE_URL="https://github.com/$REPO/archive/refs/heads/$REF.tar.gz"
TMP_DIR="$(mktemp -d)"
DRY_RUN=0
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd -P || pwd)"

case "${1:-}" in
  --dry-run) DRY_RUN=1 ;;
  "") ;;
  *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
esac

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

log() {
  printf '%s\n' "$*" >&2
}

run() {
  if [ "$DRY_RUN" = 1 ]; then
    log "dry-run: $*"
  else
    "$@"
  fi
}

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

fetch_optional() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 1 --connect-timeout 15 -o "$dest" "$url" >/dev/null 2>&1
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$dest" "$url" >/dev/null 2>&1
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
  *) echo "smart-explorer install: only x86_64 Linux desktops are supported by this installer right now" >&2; exit 1 ;;
esac

need chmod
need mkdir
need mktemp
need sha256sum
need install

release_assets_available() {
  log "Trying latest GitHub Release assets from $BASE_URL ..."
  fetch_optional "$BASE_URL/smart_explorer" "$TMP_DIR/smart_explorer" && \
  fetch_optional "$BASE_URL/smart_explorer.sha256" "$TMP_DIR/smart_explorer.sha256" && \
  fetch_optional "$BASE_URL/smart_explorer_updater" "$TMP_DIR/smart_explorer_updater" && \
  fetch_optional "$BASE_URL/smart_explorer_updater.sha256" "$TMP_DIR/smart_explorer_updater.sha256"
}

use_release_assets() {
  (
    cd "$TMP_DIR"
    sha256sum -c smart_explorer.sha256
    sha256sum -c smart_explorer_updater.sha256
  )
}

find_local_source() {
  if [ -f "$SCRIPT_DIR/native/Cargo.toml" ]; then
    printf '%s\n' "$SCRIPT_DIR"
    return 0
  fi
  if [ -f "$PWD/native/Cargo.toml" ]; then
    printf '%s\n' "$PWD"
    return 0
  fi
  return 1
}

prepare_source_build() {
  need cargo
  if src="$(find_local_source)"; then
    log "Using local source checkout: $src"
  else
    need tar
    log "Release assets are unavailable; downloading source from $SRC_ARCHIVE_URL ..."
    fetch "$SRC_ARCHIVE_URL" "$TMP_DIR/source.tar.gz"
    mkdir -p "$TMP_DIR/source"
    tar -xzf "$TMP_DIR/source.tar.gz" -C "$TMP_DIR/source" --strip-components=1
    src="$TMP_DIR/source"
  fi

  if [ "$DRY_RUN" = 1 ]; then
    log "dry-run: cargo build --release --bin smart_explorer --bin smart_explorer_updater (in $src/native)"
  else
    (cd "$src/native" && cargo build --release --bin smart_explorer --bin smart_explorer_updater)
  fi
  printf '%s\n' "$src/native/target/release"
}

install_files() {
  src_dir="$1"
  run mkdir -p "$INSTALL_DIR" "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
  run install -m 755 "$src_dir/smart_explorer" "$APP_BIN"
  run install -m 755 "$src_dir/smart_explorer_updater" "$UPDATER_BIN"
  run ln -sf "$APP_BIN" "$BIN_DIR/smart_explorer"
  if [ "$DRY_RUN" = 1 ]; then
    log "dry-run: fetch icon $RAW_BASE_URL/native/assets/smart-explorer-logo-256.png -> $ICON_DIR/smart-explorer.png"
  else
    fetch "$RAW_BASE_URL/native/assets/smart-explorer-logo-256.png" "$ICON_DIR/smart-explorer.png" >/dev/null 2>&1 || true
  fi

  if [ "$DRY_RUN" = 1 ]; then
    log "dry-run: write $APP_DIR/smart-explorer.desktop"
  else
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
  fi

  if command -v update-desktop-database >/dev/null 2>&1; then
    run update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
  fi
}

if release_assets_available; then
  use_release_assets
  install_files "$TMP_DIR"
else
  log "Latest release does not have Linux desktop assets yet; falling back to a source build."
  build_dir="$(prepare_source_build)"
  install_files "$build_dir"
fi

if [ "$DRY_RUN" = 1 ]; then
  log "dry-run: Smart Explorer install path would be: $APP_BIN"
else
  log "Smart Explorer installed: $APP_BIN"
  log "Run it from your app launcher or with: $BIN_DIR/smart_explorer"
fi
