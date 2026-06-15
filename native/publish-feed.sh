#!/usr/bin/env bash
# Build the Windows release and refresh the in-repo git update feed
# (release-native/update-feed/), which installs can pull from over HTTPS via
# raw.githubusercontent.com — i.e. "the git is the update location".
#
# Cross-compiles with the gnu toolchain, so it runs on Linux/macOS/WSL too
# (needs: rustup target add x86_64-pc-windows-gnu + mingw-w64). On Windows use
# publish-update.ps1 instead (also rebuilds the NSIS installer).
#
# Usage:  native/publish-feed.sh
set -euo pipefail
cd "$(dirname "$0")"                       # native/
repo_root="$(cd .. && pwd)"
feed="$repo_root/release-native/update-feed"
target="x86_64-pc-windows-gnu"

version="$(sed -nE 's/^version = "([^"]+)".*/\1/p' Cargo.toml | head -1)"
echo "Building Smart Explorer $version for $target ..."

cargo build --release --target "$target" --bin smart_explorer

mkdir -p "$feed"
# EXE first, version.txt last — clients only see the new version once the new
# binary is fully published (mirrors publish-update.ps1).
cp "target/$target/release/smart_explorer.exe" "$feed/smart_explorer.exe"
printf '%s\n' "$version" > "$feed/version.txt"

echo "Feed updated: $feed (v$version)"
ls -la "$feed"
