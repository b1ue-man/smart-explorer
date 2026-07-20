#!/usr/bin/env sh
set -eu

REPO="${SMART_EXPLORER_REPO:-b1ue-man/smart-explorer}"
REF="${SMART_EXPLORER_REF:-main}"
RELEASE_TAG="${SMART_EXPLORER_RELEASE_TAG:-latest}"
REQUIRE_RELEASE_ASSETS="${SMART_EXPLORER_REQUIRE_RELEASE_ASSETS:-0}"
INSTALL_DIR="${SMART_EXPLORER_INSTALL_DIR:-$HOME/.local/opt/smart-explorer}"
BIN_DIR="${SMART_EXPLORER_BIN_DIR:-$HOME/.local/bin}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$DATA_HOME/applications"
ICON_DIR="$DATA_HOME/icons/hicolor/256x256/apps"
APP_BIN="$INSTALL_DIR/smart_explorer"
UPDATER_BIN="$INSTALL_DIR/smart_explorer_updater"
CLI_BIN="$INSTALL_DIR/se"
case "$RELEASE_TAG" in
  latest) BASE_URL="https://github.com/$REPO/releases/latest/download" ;;
  v[0-9]* )
    case "$RELEASE_TAG" in
      *[!0-9A-Za-z._-]*)
        echo "smart-explorer install: invalid release tag: $RELEASE_TAG" >&2
        exit 2
        ;;
    esac
    BASE_URL="https://github.com/$REPO/releases/download/$RELEASE_TAG"
    ;;
  *)
    echo "smart-explorer install: release tag must be 'latest' or begin with v: $RELEASE_TAG" >&2
    exit 2
    ;;
esac
case "$REQUIRE_RELEASE_ASSETS" in
  0|1) ;;
  *)
    echo "smart-explorer install: SMART_EXPLORER_REQUIRE_RELEASE_ASSETS must be 0 or 1" >&2
    exit 2
    ;;
esac
RAW_BASE_URL="https://raw.githubusercontent.com/$REPO/$REF"
SRC_ARCHIVE_URL="https://github.com/$REPO/archive/refs/heads/$REF.tar.gz"
if [ "$REF" = "main" ]; then
  UPDATE_SOURCE="https://github.com/$REPO"
else
  UPDATE_SOURCE="https://github.com/$REPO/tree/$REF"
fi
TMP_DIR=""
INSTALL_TEMP=""
DRY_RUN=0
CLI_ONLY=0
if SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" 2>/dev/null && pwd -P)"; then
  :
else
  SCRIPT_DIR="$(pwd -P)"
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --cli-only) CLI_ONLY=1 ;;
    *) echo "usage: $0 [--dry-run] [--cli-only]" >&2; exit 2 ;;
  esac
  shift
done

cleanup() {
  if [ -n "$INSTALL_TEMP" ]; then
    rm -f "$INSTALL_TEMP"
  fi
  if [ -n "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
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

have_cmd() {
  command -v "$1" >/dev/null 2>&1
}

as_root() {
  if [ "$DRY_RUN" = 1 ]; then
    log "dry-run: $*"
    return 0
  fi
  if [ "$(id -u 2>/dev/null || printf 1)" = 0 ]; then
    "$@"
  elif have_cmd sudo; then
    sudo "$@"
  elif have_cmd doas; then
    doas "$@"
  elif have_cmd su; then
    su -c "$(printf '%s ' "$@")"
  else
    echo "smart-explorer install: need root privileges to install missing system packages; install sudo/doas or run as root" >&2
    exit 1
  fi
}

missing_pkg_config_libs() {
  if ! have_cmd pkg-config; then
    printf '%s\n' pkg-config
    return 0
  fi
  missing=0
  for lib in x11 xcb xkbcommon xkbcommon-x11 wayland-client egl gl fontconfig; do
    if ! pkg-config --exists "$lib" >/dev/null 2>&1; then
      printf '%s\n' "$lib"
      missing=1
    fi
  done
  return "$missing"
}

need_system_packages() {
  missing=""
  for cmd in install chmod mkdir mktemp sha256sum tar; do
    if ! have_cmd "$cmd"; then
      missing="$missing $cmd"
    fi
  done
  if ! have_cmd curl && ! have_cmd wget; then
    missing="$missing curl-or-wget"
  fi
  # This check runs only after verified release assets were unavailable. Cargo
  # and desktop libraries are therefore source-build prerequisites, not costs
  # imposed on the normal binary installation path.
  if ! have_cmd cargo; then
    missing="$missing cargo"
  fi
  libs="$(missing_pkg_config_libs || true)"
  if [ -n "$libs" ]; then
    missing="$missing $(printf '%s' "$libs" | tr '\n' ' ')"
  fi
  [ -z "$(printf '%s' "$missing" | tr -d ' ')" ] && return 1
  log "Missing installer/build prerequisites:$missing"
  return 0
}

install_system_packages() {
  if ! need_system_packages; then
    log "System prerequisites already present."
    return 0
  fi

  if have_cmd apt-get; then
    as_root apt-get update
    as_root apt-get install -y --no-install-recommends \
      ca-certificates curl wget tar coreutils findutils util-linux \
      build-essential cargo rustc pkg-config desktop-file-utils xdg-utils \
      libx11-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
      libwayland-dev libgl1-mesa-dev libegl1-mesa-dev libfontconfig1-dev
  elif have_cmd dnf; then
    as_root dnf install -y \
      ca-certificates curl wget tar coreutils findutils util-linux \
      gcc gcc-c++ make cargo rust pkgconf-pkg-config desktop-file-utils xdg-utils \
      libX11-devel libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel \
      wayland-devel mesa-libGL-devel mesa-libEGL-devel fontconfig-devel
  elif have_cmd pacman; then
    as_root pacman -Sy --needed --noconfirm \
      ca-certificates curl wget tar coreutils findutils util-linux \
      base-devel rust pkgconf desktop-file-utils xdg-utils \
      libx11 libxcb libxkbcommon libxkbcommon-x11 wayland mesa fontconfig
  elif have_cmd zypper; then
    as_root zypper --non-interactive install \
      ca-certificates curl wget tar coreutils findutils util-linux \
      gcc gcc-c++ make cargo rust pkg-config desktop-file-utils xdg-utils \
      libX11-devel libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel \
      wayland-devel Mesa-libGL-devel Mesa-libEGL-devel fontconfig-devel
  else
    cat >&2 <<'EOF'
smart-explorer install: unsupported Linux package manager.
Install these prerequisites, then re-run this script:
  curl or wget, tar, coreutils, findutils, util-linux, build tools, rust/cargo,
  pkg-config, desktop-file-utils, xdg-utils, and development packages for
  x11, xcb, xkbcommon, xkbcommon-x11, wayland-client, egl, gl, and fontconfig.
EOF
    exit 1
  fi

  if [ "$DRY_RUN" != 1 ] && need_system_packages; then
    echo "smart-explorer install: some prerequisites are still missing after package installation" >&2
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
need ln
need mv
need basename
need dirname
if ! have_cmd curl && ! have_cmd wget; then
  echo "smart-explorer install: install curl or wget first" >&2
  exit 1
fi
TMP_DIR="$(mktemp -d)"

release_assets_available() {
  log "Trying GitHub Release assets from $BASE_URL ..."
  if [ "$DRY_RUN" = 1 ]; then
    return 0
  fi
  if [ "$CLI_ONLY" = 1 ]; then
    fetch_optional "$BASE_URL/se" "$TMP_DIR/se" && \
    fetch_optional "$BASE_URL/se.sha256" "$TMP_DIR/se.sha256"
    return
  fi
  fetch_optional "$BASE_URL/smart_explorer" "$TMP_DIR/smart_explorer" && \
  fetch_optional "$BASE_URL/smart_explorer.sha256" "$TMP_DIR/smart_explorer.sha256" && \
  fetch_optional "$BASE_URL/smart_explorer_updater" "$TMP_DIR/smart_explorer_updater" && \
  fetch_optional "$BASE_URL/smart_explorer_updater.sha256" "$TMP_DIR/smart_explorer_updater.sha256" && \
  fetch_optional "$BASE_URL/se" "$TMP_DIR/se" && \
  fetch_optional "$BASE_URL/se.sha256" "$TMP_DIR/se.sha256"
}

use_release_assets() {
  if [ "$DRY_RUN" = 1 ]; then
    log "dry-run: verify downloaded release SHA-256 sidecars"
    return 0
  fi
  if [ "$CLI_ONLY" = 1 ]; then
    verify_payload_sha256 "$TMP_DIR/se" "$TMP_DIR/se.sha256"
    return
  fi
  verify_payload_sha256 "$TMP_DIR/smart_explorer" "$TMP_DIR/smart_explorer.sha256"
  verify_payload_sha256 "$TMP_DIR/smart_explorer_updater" "$TMP_DIR/smart_explorer_updater.sha256"
  verify_payload_sha256 "$TMP_DIR/se" "$TMP_DIR/se.sha256"
}

verify_payload_sha256() {
  verify_sidecar_line=""
  IFS= read -r verify_sidecar_line < "$2" || [ -n "$verify_sidecar_line" ] || {
    echo "smart-explorer install: empty SHA256 sidecar: $2" >&2
    return 1
  }
  verify_expected=${verify_sidecar_line%% *}
  verify_actual_line=$(sha256sum "$1")
  verify_actual=${verify_actual_line%% *}
  if [ "${#verify_expected}" -ne 64 ]; then
    echo "smart-explorer install: invalid SHA256 sidecar: $2" >&2
    return 1
  fi
  case "$verify_expected" in
    *[!0-9a-fA-F]*)
      echo "smart-explorer install: invalid SHA256 sidecar: $2" >&2
      return 1
      ;;
  esac
  if [ "$verify_expected" != "$verify_actual" ]; then
    echo "smart-explorer install: SHA256 mismatch for $1" >&2
    return 1
  fi
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
    if [ "$CLI_ONLY" = 1 ]; then
      log "dry-run: cargo build --release --bin se (in $src/native)"
    else
      log "dry-run: cargo build --release --bin smart_explorer --bin smart_explorer_updater --bin se (in $src/native)"
    fi
  elif [ "$CLI_ONLY" = 1 ]; then
    # One codegen job keeps fallback builds within modest workstation memory.
    (cd "$src/native" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}" cargo build --release --bin se)
  else
    # One codegen job keeps fallback builds within modest workstation memory.
    (cd "$src/native" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}" cargo build --release --bin smart_explorer --bin smart_explorer_updater --bin se)
  fi
  printf '%s\n' "$src/native/target/release"
}

install_files() {
  src_dir="$1"
  if [ "$CLI_ONLY" = 1 ]; then
    run mkdir -p "$INSTALL_DIR" "$BIN_DIR"
  else
    run mkdir -p "$INSTALL_DIR" "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
    install_executable_atomic "$src_dir/smart_explorer" "$APP_BIN"
    install_executable_atomic "$src_dir/smart_explorer_updater" "$UPDATER_BIN"
  fi
  install_executable_atomic "$src_dir/se" "$CLI_BIN"
  if [ "$CLI_ONLY" != 1 ]; then
    if [ "$DRY_RUN" = 1 ]; then
      log "dry-run: write $INSTALL_DIR/update_source.txt"
    else
      printf '%s\n' "$UPDATE_SOURCE" > "$INSTALL_DIR/update_source.txt"
    fi
    run ln -sf "$APP_BIN" "$BIN_DIR/smart_explorer"
  fi
  run ln -sf "$CLI_BIN" "$BIN_DIR/se"
  if [ "$CLI_ONLY" = 1 ]; then
    return 0
  fi
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

install_executable_atomic() {
  source_path="$1"
  destination_path="$2"
  destination_dir=$(dirname "$destination_path")
  destination_name=$(basename "$destination_path")
  if [ "$DRY_RUN" = 1 ]; then
    log "dry-run: install -m 755 $source_path -> $destination_path (atomic rename)"
    return 0
  fi
  INSTALL_TEMP=$(mktemp "$destination_dir/.$destination_name.install.XXXXXX")
  install -m 755 "$source_path" "$INSTALL_TEMP"
  mv -f "$INSTALL_TEMP" "$destination_path"
  INSTALL_TEMP=""
}

if release_assets_available; then
  use_release_assets
  install_files "$TMP_DIR"
else
  if [ "$REQUIRE_RELEASE_ASSETS" = 1 ]; then
    echo "smart-explorer install: required release assets are unavailable at $BASE_URL" >&2
    exit 1
  fi
  if [ "$CLI_ONLY" = 1 ]; then
    log "Requested release does not have the Linux se asset; falling back to a source build."
  else
    log "Requested release does not have Linux desktop assets; falling back to a source build."
  fi
  install_system_packages
  build_dir="$(prepare_source_build)"
  install_files "$build_dir"
fi

if [ "$DRY_RUN" = 1 ]; then
  if [ "$CLI_ONLY" = 1 ]; then
    log "dry-run: se install path would be: $CLI_BIN"
  else
    log "dry-run: Smart Explorer install path would be: $APP_BIN"
  fi
else
  "$CLI_BIN" --version >&2
  if [ "$CLI_ONLY" != 1 ]; then
    log "Smart Explorer installed: $APP_BIN"
    log "Run it from your app launcher or with: $BIN_DIR/smart_explorer"
  fi
  log "Terminal companion installed: $BIN_DIR/se"
  case ":${PATH:-}:" in
    *":$BIN_DIR:"*) ;;
    *)
      log "WARNING: $BIN_DIR is not in PATH. Add it with:"
      log "  export PATH=\"$BIN_DIR:\$PATH\""
      ;;
  esac
  resolved_se="$(command -v se 2>/dev/null || true)"
  if [ -n "$resolved_se" ] && [ "$resolved_se" != "$BIN_DIR/se" ]; then
    log "WARNING: 'se' currently resolves to $resolved_se, not the managed $BIN_DIR/se."
    log "Put $BIN_DIR first in PATH and run: hash -r"
  fi
fi
