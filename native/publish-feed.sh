#!/usr/bin/env bash
# Build and publish the complete local Windows + Linux release feed from Linux
# or WSL. Every payload is built and verified before version.txt is replaced.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"
repo_root="$(cd .. && pwd)"
rel="$repo_root/release-native"
feed="$rel/update-feed"
share_out="$rel/share-server"
windows_target="x86_64-pc-windows-gnu"

if [ "$(uname -s 2>/dev/null || echo unknown)" != "Linux" ]; then
  echo "publish-feed.sh requires Linux/WSL for a complete release." >&2
  echo "On Windows use native\\publish-release-local.ps1." >&2
  exit 1
fi
for tool in cargo rustc rustup x86_64-w64-mingw32-gcc \
  x86_64-w64-mingw32-objdump makensis sha256sum file; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Required release tool missing: $tool" >&2
    exit 1
  }
done

version="$(sed -nE 's/^version = "([^"]+)".*/\1/p' Cargo.toml | head -1)"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid or missing native version: $version" >&2
  exit 1
fi
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

mkdir -p "$rel"
stage_root="$(mktemp -d "$rel/.release-stage.XXXXXX")"
feed_stage="$stage_root/update-feed"
share_stage="$stage_root/share-server"
portable_stage="$stage_root/portable"
mkdir -p "$feed_stage" "$share_stage" "$portable_stage"
transaction_active=0
feed_backup=""
feed_had_destination=0
feed_installed=0
promoted_destinations=()
promoted_backups=()
promoted_had_destination=()

rollback_publication() {
  if [ "$feed_installed" = "1" ] && [ -e "$feed" ]; then
    rm -rf "$feed"
  fi
  if [ "$feed_had_destination" = "1" ] && [ -n "$feed_backup" ] && [ -e "$feed_backup" ]; then
    mv "$feed_backup" "$feed"
  fi
  local index
  for ((index = ${#promoted_destinations[@]} - 1; index >= 0; index--)); do
    rm -f "${promoted_destinations[$index]}"
    if [ "${promoted_had_destination[$index]}" = "1" ] && [ -e "${promoted_backups[$index]}" ]; then
      mv "${promoted_backups[$index]}" "${promoted_destinations[$index]}"
    fi
  done
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [ "$transaction_active" = "1" ]; then
    rollback_publication
  fi
  rm -rf "$stage_root"
  exit "$status"
}
trap cleanup EXIT

echo "Building complete Smart Explorer v$version release ..."

# The desktop app embeds these files, so refresh them before either native app
# target is compiled.
"$script_dir/build-agent-bundles.sh"

rustup target add "$windows_target" >/dev/null
cargo build --release --target "$windows_target" \
  --bin smart_explorer --bin smart_explorer_updater --bin se

SMART_EXPLORER_FEED_DIR="$feed_stage" \
SMART_EXPLORER_SHARE_DIR="$share_stage" \
  "$script_dir/publish-linux-feed-wsl.sh"

(
  cd "$repo_root/share-server"
  cargo build --release --target "$windows_target" --bin se-share-server
)
(
  cd "$script_dir/explorer-command"
  cargo build --release --target "$windows_target"
)

windows_dir="$script_dir/target/$windows_target/release"
dll="$script_dir/explorer-command/target/$windows_target/release/smart_explorer_command.dll"
installer="$portable_stage/Smart Explorer Setup $version.exe"

install -m 0755 "$windows_dir/smart_explorer.exe" "$feed_stage/smart_explorer.exe"
install -m 0755 "$windows_dir/smart_explorer_updater.exe" "$feed_stage/smart_explorer_updater.exe"
install -m 0755 "$windows_dir/se.exe" "$feed_stage/se.exe"
install -m 0755 "$repo_root/share-server/target/$windows_target/release/se-share-server.exe" \
  "$share_stage/se-share-server.exe"

install -m 0755 "$windows_dir/smart_explorer.exe" "$portable_stage/Smart Explorer.exe"
install -m 0755 "$windows_dir/smart_explorer_updater.exe" "$portable_stage/Smart Explorer Updater.exe"
install -m 0755 "$windows_dir/se.exe" "$portable_stage/se.exe"
install -m 0755 "$dll" "$portable_stage/smart_explorer_command.dll"

feed_payloads=(
  smart_explorer.exe smart_explorer_updater.exe se.exe
  smart_explorer smart_explorer_updater se
)
for payload in "${feed_payloads[@]}"; do
  test -s "$feed_stage/$payload" || {
    echo "Required staged feed payload missing: $payload" >&2
    exit 1
  }
  (
    cd "$feed_stage"
    sha256sum "$payload" > "$payload.sha256"
  )
done

{
  printf 'version=%s\n' "$version"
  for payload in smart_explorer.exe smart_explorer_updater.exe se.exe; do
    printf '%s=%s\n' "$payload" "$(sha256sum "$feed_stage/$payload" | awk '{print $1}')"
  done
} > "$feed_stage/windows-build.manifest"

for share in se-share-server.exe se-share-server-linux; do
  test -s "$share_stage/$share" || {
    echo "Required staged share-server payload missing: $share" >&2
    exit 1
  }
done
for portable in "Smart Explorer.exe" "Smart Explorer Updater.exe" se.exe; do
  test -s "$portable_stage/$portable" || {
    echo "Required portable payload missing: $portable" >&2
    exit 1
  }
done
test -s "$portable_stage/smart_explorer_command.dll" || {
  echo "Context-menu DLL missing: $portable_stage/smart_explorer_command.dll" >&2
  exit 1
}
dll_exports="$(x86_64-w64-mingw32-objdump -p "$portable_stage/smart_explorer_command.dll")"
grep -q 'DllGetClassObject' <<<"$dll_exports" || {
  echo "Context-menu DLL lacks DllGetClassObject" >&2
  exit 1
}
grep -q 'DllCanUnloadNow' <<<"$dll_exports" || {
  echo "Context-menu DLL lacks DllCanUnloadNow" >&2
  exit 1
}
test -x "$repo_root/install-linux.sh" || {
  echo "Linux installer missing or not executable: $repo_root/install-linux.sh" >&2
  exit 1
}
file "$feed_stage/smart_explorer" | grep -Eq 'statically linked|static-pie linked'
file "$share_stage/se-share-server-linux" | grep -Eq 'statically linked|static-pie linked'

(
  cd "$feed_stage"
  for payload in "${feed_payloads[@]}"; do
    sha256sum -c "$payload.sha256"
  done
)
test "$(sed -n 's/^version=//p' "$feed_stage/windows-build.manifest")" = "$version"
for payload in smart_explorer.exe smart_explorer_updater.exe se.exe; do
  expected_manifest_line="$payload=$(sha256sum "$feed_stage/$payload" | awk '{print $1}')"
  test "$(grep -Fxc "$expected_manifest_line" "$feed_stage/windows-build.manifest")" = "1"
done

makensis \
  -DVERSION="$version" \
  -DEXE_SRC="$windows_dir/smart_explorer.exe" \
  -DUPDATER_SRC="$windows_dir/smart_explorer_updater.exe" \
  -DCLI_SRC="$windows_dir/se.exe" \
  -DINSTALLER_OUT="$installer" \
  installer.nsi >/dev/null
test -s "$installer" || { echo "Installer was not produced: $installer" >&2; exit 1; }

# Publish every ancillary file with a rollback copy, swap the complete feed
# directory, and move version.txt last. Builds and staged verification above do
# not mutate the live release tree.
promote_file() {
  local source=$1
  local destination=$2
  local destination_parent
  local backup
  local had_destination=0
  destination_parent="$(dirname "$destination")"
  mkdir -p "$destination_parent"
  backup="$destination_parent/.$(basename "$destination").release-backup.$$.${RANDOM}"
  if [ -e "$destination" ]; then
    had_destination=1
    mv "$destination" "$backup"
  fi
  promoted_destinations+=("$destination")
  promoted_backups+=("$backup")
  promoted_had_destination+=("$had_destination")
  mv "$source" "$destination"
}

version_stage="$stage_root/version.txt"
printf '%s\n' "$version" > "$version_stage"
transaction_active=1

promote_file "$portable_stage/Smart Explorer.exe" "$rel/Smart Explorer.exe"
promote_file "$portable_stage/Smart Explorer Updater.exe" "$rel/Smart Explorer Updater.exe"
promote_file "$portable_stage/se.exe" "$rel/se.exe"
promote_file "$portable_stage/smart_explorer_command.dll" "$rel/smart_explorer_command.dll"
promote_file "$installer" "$rel/Smart Explorer Setup $version.exe"
promote_file "$share_stage/se-share-server.exe" "$share_out/se-share-server.exe"
promote_file "$share_stage/se-share-server-linux" "$share_out/se-share-server-linux"

feed_backup="$rel/.update-feed.release-backup.$$.${RANDOM}"
if [ -e "$feed" ]; then
  feed_had_destination=1
  mv "$feed" "$feed_backup"
fi
feed_installed=1
mv "$feed_stage" "$feed"
mv "$version_stage" "$feed/version.txt"

test "$(tr -d '\r\n' < "$feed/version.txt")" = "$version"
(
  cd "$feed"
  for payload in "${feed_payloads[@]}"; do
    sha256sum -c "$payload.sha256"
  done
)
test "$(sed -n 's/^version=//p' "$feed/windows-build.manifest")" = "$version"
for payload in smart_explorer.exe smart_explorer_updater.exe se.exe; do
  expected_manifest_line="$payload=$(sha256sum "$feed/$payload" | awk '{print $1}')"
  test "$(grep -Fxc "$expected_manifest_line" "$feed/windows-build.manifest")" = "1"
done
test -s "$rel/Smart Explorer Setup $version.exe"
test -s "$rel/Smart Explorer.exe"
test -s "$rel/Smart Explorer Updater.exe"
test -s "$rel/se.exe"
test -s "$rel/smart_explorer_command.dll"
test -s "$share_out/se-share-server.exe"
test -s "$share_out/se-share-server-linux"
file "$feed/smart_explorer" | grep -Eq 'statically linked|static-pie linked'
file "$share_out/se-share-server-linux" | grep -Eq 'statically linked|static-pie linked'

transaction_active=0
if [ "$feed_had_destination" = "1" ]; then
  rm -rf "$feed_backup"
fi
for ((index = 0; index < ${#promoted_destinations[@]}; index++)); do
  if [ "${promoted_had_destination[$index]}" = "1" ]; then
    rm -f "${promoted_backups[$index]}"
  fi
done

echo "Complete local release staged and verified: v$version"
echo "Installer: $rel/Smart Explorer Setup $version.exe"
echo "Feed: $feed"
