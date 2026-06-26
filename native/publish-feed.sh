#!/usr/bin/env bash
# Build the Windows release and refresh the in-repo git update feed
# (release-native/update-feed/), which installs can pull from over HTTPS via
# raw.githubusercontent.com — i.e. "the git is the update location".
#
# Cross-compiles with the gnu toolchain (needs: rustup target add
# x86_64-pc-windows-gnu + mingw-w64; makensis for the installer). Run on
# Linux/WSL for the complete Windows+Linux feed; non-Linux hosts must opt into a
# Windows-only feed with SMART_EXPLORER_ALLOW_PARTIAL_FEED=1.
#
# Usage:  native/publish-feed.sh
set -euo pipefail
cd "$(dirname "$0")"                       # native/
repo_root="$(cd .. && pwd)"
rel="$repo_root/release-native"
feed="$rel/update-feed"
target="x86_64-pc-windows-gnu"
host_os="$(uname -s 2>/dev/null || echo unknown)"
allow_partial_feed="${SMART_EXPLORER_ALLOW_PARTIAL_FEED:-0}"

version="$(sed -nE 's/^version = "([^"]+)".*/\1/p' Cargo.toml | head -1)"
echo "Building Smart Explorer $version for $target ..."

if [ "$host_os" != "Linux" ] && [ "$allow_partial_feed" != "1" ]; then
  echo "Refusing to update the shared feed from $host_os: Linux payloads require a Linux/WSL host." >&2
  echo "Run this on Linux/WSL for a complete feed, or set SMART_EXPLORER_ALLOW_PARTIAL_FEED=1 for a Windows-only feed." >&2
  exit 1
fi

cargo build --release --target "$target" --bin smart_explorer --bin smart_explorer_updater
exe="target/$target/release/smart_explorer.exe"
updater="target/$target/release/smart_explorer_updater.exe"
linux_exe=""
linux_updater=""
if [ "$host_os" = "Linux" ]; then
  echo "Building Smart Explorer $version for Linux host ..."
  cargo build --release --bin smart_explorer --bin smart_explorer_updater
  linux_exe="target/release/smart_explorer"
  linux_updater="target/release/smart_explorer_updater"
else
  echo "Linux desktop feed payloads skipped by explicit partial-feed opt-in."
fi

mkdir -p "$feed"
# EXE first, version.txt last — clients only see the new version once the new
# binary is fully published (mirrors publish-update.ps1).
cp "$exe" "$feed/smart_explorer.exe"
cp "$updater" "$feed/smart_explorer_updater.exe"
if [ -n "$linux_exe" ]; then
  cp "$linux_exe" "$feed/smart_explorer"
  cp "$linux_updater" "$feed/smart_explorer_updater"
else
  rm -f "$feed/smart_explorer" "$feed/smart_explorer.sha256"
  rm -f "$feed/smart_explorer_updater" "$feed/smart_explorer_updater.sha256"
fi
( cd "$feed"
  sha256sum smart_explorer.exe > smart_explorer.exe.sha256
  sha256sum smart_explorer_updater.exe > smart_explorer_updater.exe.sha256
  if [ -f smart_explorer ]; then
    sha256sum smart_explorer > smart_explorer.sha256
    sha256sum smart_explorer_updater > smart_explorer_updater.sha256
  fi
)
printf '%s\n' "$version" > "$feed/version.txt"

# Standalone share rendezvous server (Linux + Windows) — routes peer-sharing
# discovery only; ships alongside the app so users can self-host it.
share_src="$repo_root/share-server"
if [ -d "$share_src" ]; then
  share_out="$rel/share-server"
  mkdir -p "$share_out"
  ( cd "$share_src"
    cargo build --release --target "$target" --bin se-share-server
    cargo build --release --bin se-share-server )
  cp "$share_src/target/$target/release/se-share-server.exe" "$share_out/se-share-server.exe"
  cp "$share_src/target/release/se-share-server" "$share_out/se-share-server-linux"
  echo "Share server staged: $share_out (windows + linux)"
fi

# Portable exe + NSIS installer (per-user, sets up update source + context menu).
cp "$exe" "$rel/Smart Explorer.exe"
cp "$updater" "$rel/Smart Explorer Updater.exe"
if command -v makensis >/dev/null 2>&1; then
  makensis -DVERSION="$version" -DEXE_SRC="$exe" -DUPDATER_SRC="$updater" installer.nsi >/dev/null
  echo "Installer: $rel/Smart Explorer Setup $version.exe"
else
  echo "makensis not found — installer skipped (apt-get install nsis)" >&2
fi

echo "Feed updated: $feed (v$version)"
ls -la "$feed" "$rel"
