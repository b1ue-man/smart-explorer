#!/usr/bin/env bash
# Build the Windows release and refresh the in-repo git update feed
# (release-native/update-feed/), which installs can pull from over HTTPS via
# raw.githubusercontent.com — i.e. "the git is the update location".
#
# Cross-compiles with the gnu toolchain, so it runs on Linux/macOS/WSL too
# (needs: rustup target add x86_64-pc-windows-gnu + mingw-w64; makensis for the
# installer). Builds the feed, the portable exe AND the NSIS installer.
#
# Usage:  native/publish-feed.sh
set -euo pipefail
cd "$(dirname "$0")"                       # native/
repo_root="$(cd .. && pwd)"
rel="$repo_root/release-native"
feed="$rel/update-feed"
target="x86_64-pc-windows-gnu"

version="$(sed -nE 's/^version = "([^"]+)".*/\1/p' Cargo.toml | head -1)"
echo "Building Smart Explorer $version for $target ..."

cargo build --release --target "$target" --bin smart_explorer --bin smart_explorer_updater
exe="target/$target/release/smart_explorer.exe"
updater="target/$target/release/smart_explorer_updater.exe"

mkdir -p "$feed"
# EXE first, version.txt last — clients only see the new version once the new
# binary is fully published (mirrors publish-update.ps1).
cp "$exe" "$feed/smart_explorer.exe"
cp "$updater" "$feed/smart_explorer_updater.exe"
( cd "$feed"
  sha256sum smart_explorer.exe > smart_explorer.exe.sha256
  sha256sum smart_explorer_updater.exe > smart_explorer_updater.exe.sha256
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
