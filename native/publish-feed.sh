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
check_env=0
bootstrap_zig=1

while [ "$#" -gt 0 ]; do
  case "$1" in
    --check-env)
      check_env=1
      ;;
    --no-bootstrap-zig)
      bootstrap_zig=0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      echo "Usage: native/publish-feed.sh [--check-env] [--no-bootstrap-zig]" >&2
      exit 2
      ;;
  esac
  shift
done

if [ "$(uname -s 2>/dev/null || echo unknown)" != "Linux" ]; then
  echo "publish-feed.sh requires Linux/WSL for a complete release." >&2
  echo "On Windows use native\\publish-release-local.ps1." >&2
  exit 1
fi
if [ "$check_env" != "1" ] && [ -z "${SMART_EXPLORER_RELEASE_LOCK_TOKEN:-}" ]; then
  echo "A complete release may only run through native/publish-release-local.ps1." >&2
  echo "Run 'pwsh native/publish-release-local.ps1' so one wrapper owns the version, lock, build, publication, and final verification." >&2
  exit 1
fi
for tool in cargo rustc rustup git curl x86_64-w64-mingw32-gcc \
  x86_64-w64-mingw32-objdump makensis sha256sum file install 7z; do
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
source_commit="$(git -C "$repo_root" rev-parse 'HEAD^{commit}')"
if ! [[ "$source_commit" =~ ^[0-9a-fA-F]{40,64}$ ]]; then
  echo "Could not bind the release build to one source commit." >&2
  exit 1
fi
source_commit="${source_commit,,}"
dokany_msi="$("$script_dir/fetch-dokany-runtime.sh")"
test -s "$dokany_msi" || {
  echo "Pinned Dokany installer dependency missing: $dokany_msi" >&2
  exit 1
}
# Canonical release resource limits are fixed rather than ambient defaults.
export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_RELEASE_LTO=thin
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8
export CARGO_PROFILE_RELEASE_DEBUG=0

linux_release_args=()
if [ "$bootstrap_zig" != "1" ]; then
  linux_release_args+=(--no-bootstrap-zig)
fi

if [ "$check_env" = "1" ]; then
  rustup target add "$windows_target" >/dev/null
  cargo fmt --version >/dev/null
  cargo clippy --version >/dev/null
  "$script_dir/build-agent-bundles.sh" --check-env
  "$script_dir/publish-linux-feed-wsl.sh" --check-env "${linux_release_args[@]}"
  echo "Complete Linux-host release environment OK for Smart Explorer $version."
  exit 0
fi

mkdir -p "$rel"
# Resolved relative to this script at runtime.
# shellcheck source=release-lock.sh
# shellcheck disable=SC1091
. "$script_dir/release-lock.sh"
release_lock_acquire "$rel" "native/publish-feed.sh"

stage_root=""
feed_stage=""
share_stage=""
portable_stage=""
transaction_active=0
release_complete=0
feed_backup=""
feed_install=""
feed_had_destination=0
feed_installed=0
promoted_destinations=()
promoted_backups=()
promoted_had_destination=()
promotion_temporaries=()

rollback_publication() {
  local failed=0
  if [ "$feed_installed" = "1" ] && [ -e "$feed" ]; then
    rm -rf -- "$feed" || failed=1
  fi
  if [ "$feed_had_destination" = "1" ] && [ -n "$feed_backup" ] && [ -e "$feed_backup" ]; then
    mv -- "$feed_backup" "$feed" || failed=1
  fi
  local index
  for ((index = ${#promoted_destinations[@]} - 1; index >= 0; index--)); do
    if [ "${promoted_had_destination[$index]}" = "1" ]; then
      if [ -e "${promoted_backups[$index]}" ]; then
        rm -f -- "${promoted_destinations[$index]}" || failed=1
        mv -- "${promoted_backups[$index]}" "${promoted_destinations[$index]}" || failed=1
      elif [ ! -e "${promoted_destinations[$index]}" ]; then
        failed=1
      fi
    else
      rm -f -- "${promoted_destinations[$index]}" || failed=1
    fi
  done
  return "$failed"
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [ "$transaction_active" = "1" ]; then
    if ! rollback_publication; then
      echo "Complete-release rollback encountered an error; inspect the preserved stage and backup files." >&2
      status=1
    fi
  fi
  local temporary
  for temporary in "${promotion_temporaries[@]}"; do
    [ -n "$temporary" ] && rm -rf -- "$temporary"
  done
  if [ -n "$feed_install" ]; then
    rm -rf -- "$feed_install"
  fi
  if [ "$release_complete" = "1" ] && [ -n "$stage_root" ]; then
    rm -rf -- "$stage_root"
  elif [ -n "$stage_root" ] && [ -d "$stage_root" ]; then
    echo "Complete release failed; preserved stage: $stage_root" >&2
    echo "No automatic resume is assumed. Inspect the stage and rerun only through a verified release script." >&2
  fi
  if ! release_lock_release; then
    status=1
  fi
  exit "$status"
}
trap cleanup EXIT

stage_root="$(mktemp -d "$rel/.release-stage.XXXXXX")"
feed_stage="$stage_root/update-feed"
share_stage="$stage_root/share-server"
portable_stage="$stage_root/portable"
mkdir -p "$feed_stage" "$share_stage" "$portable_stage"

echo "Building complete Smart Explorer v$version release ..."

# The desktop app embeds these files, so refresh them before either native app
# target is compiled.
"$script_dir/build-agent-bundles.sh"

rustup target add "$windows_target" >/dev/null
cargo build --locked --release --target-dir "$script_dir/target" --target "$windows_target" \
  --bin smart_explorer --bin smart_explorer_updater --bin se

SMART_EXPLORER_FEED_DIR="$feed_stage" \
SMART_EXPLORER_SHARE_DIR="$share_stage" \
  "$script_dir/publish-linux-feed-wsl.sh" "${linux_release_args[@]}"

(
  cd "$repo_root/share-server"
  cargo build --locked --release \
    --target-dir "$repo_root/share-server/target" \
    --target "$windows_target" --bin se-share-server
)
(
  cd "$script_dir/explorer-command"
  cargo build --locked --release \
    --target-dir "$script_dir/explorer-command/target" \
    --target "$windows_target"
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
  printf 'source_commit=%s\n' "$source_commit"
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
file "$feed_stage/smart_explorer" | grep -Fq 'dynamically linked'
file "$share_stage/se-share-server-linux" | grep -Eq 'statically linked|static-pie linked'

(
  cd "$feed_stage"
  for payload in "${feed_payloads[@]}"; do
    sha256sum -c "$payload.sha256"
  done
)
test "$(sed -n 's/^version=//p' "$feed_stage/windows-build.manifest")" = "$version"
test "$(sed -n 's/^source_commit=//p' "$feed_stage/windows-build.manifest")" = "$source_commit"
test "$(awk 'NF { count++ } END { print count + 0 }' "$feed_stage/windows-build.manifest")" = "5"
for payload in smart_explorer.exe smart_explorer_updater.exe se.exe; do
  expected_manifest_line="$payload=$(sha256sum "$feed_stage/$payload" | awk '{print $1}')"
  test "$(grep -Fxc "$expected_manifest_line" "$feed_stage/windows-build.manifest")" = "1"
done

makensis \
  -DVERSION="$version" \
  -DEXE_SRC="$windows_dir/smart_explorer.exe" \
  -DUPDATER_SRC="$windows_dir/smart_explorer_updater.exe" \
  -DCLI_SRC="$windows_dir/se.exe" \
  -DDOKANY_MSI_SRC="$dokany_msi" \
  -DINSTALLER_OUT="$installer" \
  installer.nsi >/dev/null
test -s "$installer" || { echo "Installer was not produced: $installer" >&2; exit 1; }
installer_dokany_entry="\$PLUGINSDIR/$(basename "$dokany_msi")"
embedded_dokany_size="$(7z e -so "$installer" "$installer_dokany_entry" | wc -c | tr -d '[:space:]')"
embedded_dokany_sha256="$(7z e -so "$installer" "$installer_dokany_entry" | sha256sum | awk '{print $1}')"
test "$embedded_dokany_size" = "$(wc -c < "$dokany_msi" | tr -d '[:space:]')" || {
  echo "Installer contains the wrong Dokany MSI size." >&2
  exit 1
}
test "$embedded_dokany_sha256" = "$(sha256sum "$dokany_msi" | awk '{print $1}')" || {
  echo "Installer contains the wrong Dokany MSI hash." >&2
  exit 1
}

# Publish every ancillary file with a rollback copy, swap the complete feed
# directory, and move version.txt last. Builds and staged verification above do
# not mutate the live release tree.
promote_file() {
  local source=$1
  local destination=$2
  local destination_parent
  local backup
  local candidate
  local had_destination=0
  destination_parent="$(dirname "$destination")"
  mkdir -p "$destination_parent"
  backup="$destination_parent/.$(basename "$destination").release-backup.$$.${RANDOM}"
  candidate="$destination_parent/.$(basename "$destination").release-new.$$.${RANDOM}"
  promotion_temporaries+=("$candidate")
  cp -p -- "$source" "$candidate"
  if [ "$(sha256sum "$source" | awk '{print $1}')" != "$(sha256sum "$candidate" | awk '{print $1}')" ]; then
    echo "Release candidate copy verification failed: $destination" >&2
    return 1
  fi
  if [ -e "$destination" ]; then
    had_destination=1
  fi
  promoted_destinations+=("$destination")
  promoted_backups+=("$backup")
  promoted_had_destination+=("$had_destination")
  if [ "$had_destination" = "1" ]; then
    mv "$destination" "$backup"
  fi
  mv "$candidate" "$destination"
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
feed_install="$rel/.update-feed.release-new.$$.${RANDOM}"
promotion_temporaries+=("$feed_install")
cp -a -- "$feed_stage" "$feed_install"
(
  cd "$feed_install"
  for payload in "${feed_payloads[@]}"; do
    sha256sum -c "$payload.sha256"
  done
)
if [ -e "$feed" ]; then
  feed_had_destination=1
  mv "$feed" "$feed_backup"
fi
feed_installed=1
mv "$feed_install" "$feed"
feed_install=""
version_install="$feed/.version.release-new.$$.${RANDOM}"
install -m 0644 "$version_stage" "$version_install"
mv "$version_install" "$feed/version.txt"

test "$(tr -d '\r\n' < "$feed/version.txt")" = "$version"
(
  cd "$feed"
  for payload in "${feed_payloads[@]}"; do
    sha256sum -c "$payload.sha256"
  done
)
test "$(sed -n 's/^version=//p' "$feed/windows-build.manifest")" = "$version"
test "$(sed -n 's/^source_commit=//p' "$feed/windows-build.manifest")" = "$source_commit"
test "$(awk 'NF { count++ } END { print count + 0 }' "$feed/windows-build.manifest")" = "5"
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
file "$feed/smart_explorer" | grep -Fq 'dynamically linked'
file "$share_out/se-share-server-linux" | grep -Eq 'statically linked|static-pie linked'

transaction_active=0
release_complete=1
backup_cleanup_incomplete=0
echo "Complete local release committed and verified: v$version"
echo "Installer: $rel/Smart Explorer Setup $version.exe"
echo "Feed: $feed"
if [ "$feed_had_destination" = "1" ]; then
  if ! rm -rf -- "$feed_backup"; then
    echo "WARNING: Complete release v$version is committed and verified at $feed, but the old feed backup could not be removed: $feed_backup" >&2
    backup_cleanup_incomplete=1
  fi
fi
for ((index = 0; index < ${#promoted_destinations[@]}; index++)); do
  if [ "${promoted_had_destination[$index]}" = "1" ]; then
    if ! rm -f -- "${promoted_backups[$index]}"; then
      echo "WARNING: Complete release v$version is committed and verified, but an old artifact backup could not be removed: ${promoted_backups[$index]}" >&2
      backup_cleanup_incomplete=1
    fi
  fi
done

if [ "$backup_cleanup_incomplete" = "1" ]; then
  echo "Release v$version remains committed; backup cleanup is best-effort and does not require rebuilding or rerunning the release." >&2
fi
