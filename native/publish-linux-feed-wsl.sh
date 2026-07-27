#!/usr/bin/env bash
# Build the Linux release payloads from Linux/WSL and stage them into the
# in-repo update feed. This script is intentionally Linux-only; on Windows use
# publish-release-local.ps1 so Windows and Linux artifacts are built together.
#
# It bootstraps the local Rust target and, when needed, a user-local Zig binary
# under ~/.local/zig. The Zig wrappers are temporary and live outside the repo.
#
# Usage:
#   native/publish-linux-feed-wsl.sh
#   native/publish-linux-feed-wsl.sh --write-version
#   native/publish-linux-feed-wsl.sh --check-env
#   native/publish-linux-feed-wsl.sh --check-gui

set -euo pipefail

# This leaf is also callable from the Windows/WSL release path, where Cargo
# settings from the parent PowerShell process are not reliably inherited.
export CARGO_BUILD_JOBS="$(nproc)"
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_RELEASE_LTO=off
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
export CARGO_PROFILE_RELEASE_DEBUG=0

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"
repo_root="$(cd .. && pwd)"
rel="$repo_root/release-native"
feed="${SMART_EXPLORER_FEED_DIR:-$rel/update-feed}"
share_out="${SMART_EXPLORER_SHARE_DIR:-$rel/share-server}"
# winit loads X11/Wayland client libraries at runtime, which a static-musl
# executable cannot do. Ship the desktop app as a dynamic GNU/glibc binary and
# keep the headless payloads static-musl for standalone portability.
linux_gui_target="x86_64-unknown-linux-gnu"
linux_static_target="x86_64-unknown-linux-musl"
write_version=0
build_share_server=1
check_env=0
check_gui=0
bootstrap_zig="${SMART_EXPLORER_BOOTSTRAP_ZIG:-1}"
zig_version="${SMART_EXPLORER_ZIG_VERSION:-0.15.2}"
zig_root="${SMART_EXPLORER_ZIG_ROOT:-$HOME/.local/zig}"
# Keep linker-heavy Cargo output on WSL's native filesystem. Building it under
# /mnt/c can leave parallel Zig linkers blocked in Windows filesystem I/O.
linux_target_dir="${SMART_EXPLORER_LINUX_TARGET_DIR:-$HOME/.cache/smart-explorer/linux-target}"
share_target_dir="${SMART_EXPLORER_SHARE_TARGET_DIR:-$HOME/.cache/smart-explorer/share-target}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --write-version)
      write_version=1
      ;;
    --skip-share-server)
      build_share_server=0
      ;;
    --check-env)
      check_env=1
      ;;
    --check-gui)
      check_gui=1
      ;;
    --no-bootstrap-zig)
      bootstrap_zig=0
      ;;
    --gui-target)
      shift
      linux_gui_target="${1:?missing value for --gui-target}"
      ;;
    --static-target|--target)
      shift
      linux_static_target="${1:?missing value for --static-target}"
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
  shift
done

if [ "$(uname -s 2>/dev/null || echo unknown)" != "Linux" ]; then
  echo "publish-linux-feed-wsl.sh must run on Linux/WSL." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
fi
command -v cargo >/dev/null 2>&1 || { echo "cargo not found. Install rustup/Rust in WSL first." >&2; exit 1; }
command -v rustup >/dev/null 2>&1 || { echo "rustup not found. Install rustup in WSL first." >&2; exit 1; }
command -v sha256sum >/dev/null 2>&1 || { echo "sha256sum not found." >&2; exit 1; }
command -v file >/dev/null 2>&1 || { echo "file not found." >&2; exit 1; }
if [ "$write_version" = "1" ]; then
  command -v git >/dev/null 2>&1 || { echo "git is required to verify build provenance." >&2; exit 1; }
fi

if [ "$write_version" = "1" ] && [ "$build_share_server" != "1" ]; then
  echo "--write-version requires the Linux share-server build; remove --skip-share-server." >&2
  exit 1
fi
if [ "$check_env" = "1" ] && [ "$check_gui" = "1" ]; then
  echo "--check-env and --check-gui are mutually exclusive." >&2
  exit 2
fi
if [ "$write_version" = "1" ] && [ "$check_gui" = "1" ]; then
  echo "--check-gui is a non-publishing test build and cannot write the feed version." >&2
  exit 2
fi
if [ "$linux_gui_target" != "x86_64-unknown-linux-gnu" ]; then
  echo "The release GUI must target x86_64-unknown-linux-gnu for dynamic display-library loading." >&2
  exit 2
fi
if [ "$linux_static_target" != "x86_64-unknown-linux-musl" ]; then
  echo "The Linux updater, CLI, and share server must target x86_64-unknown-linux-musl." >&2
  exit 2
fi

version="$(sed -nE 's/^version = "([^"]+)".*/\1/p' Cargo.toml | head -1)"
if [ -z "$version" ]; then
  echo "Could not read version from native/Cargo.toml." >&2
  exit 1
fi

tool_dir=""
feed_candidate=""
feed_backup=""
feed_install=""
share_stage=""
share_backup=""
share_install=""
version_stage=""
feed_installed=0
feed_had_destination=0
share_installed=0
share_had_destination=0
transaction_active=0
publication_complete=0
preserve_failed_stage=0

# Resolved relative to this script at runtime.
# shellcheck source=release-lock.sh
# shellcheck disable=SC1091
. "$script_dir/release-lock.sh"

rollback_publication() {
  local failed=0
  if [ "$feed_installed" = "1" ] && [ -e "$feed" ]; then
    rm -rf -- "$feed" || failed=1
  fi
  if [ "$feed_had_destination" = "1" ] && [ -n "$feed_backup" ] && [ -e "$feed_backup" ]; then
    mv -- "$feed_backup" "$feed" || failed=1
  fi
  if [ "$share_installed" = "1" ] && [ -e "$share_out/se-share-server-linux" ]; then
    rm -f -- "$share_out/se-share-server-linux" || failed=1
  fi
  if [ "$share_had_destination" = "1" ] && [ -n "$share_backup" ] && [ -e "$share_backup" ]; then
    mv -- "$share_backup" "$share_out/se-share-server-linux" || failed=1
  fi
  return "$failed"
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [ "$transaction_active" = "1" ]; then
    if ! rollback_publication; then
      echo "Linux release rollback encountered an error; inspect the preserved candidate and backups." >&2
      status=1
    fi
  fi
  [ -n "$tool_dir" ] && rm -rf -- "$tool_dir"
  [ -n "$feed_install" ] && rm -rf "$feed_install"
  [ -n "$share_install" ] && rm -f "$share_install"
  if [ "$publication_complete" = "1" ] || [ "$preserve_failed_stage" != "1" ]; then
    [ -n "$feed_candidate" ] && rm -rf "$feed_candidate"
    [ -n "$share_stage" ] && rm -f "$share_stage"
    [ -n "$version_stage" ] && rm -f "$version_stage"
  elif [ "$status" -ne 0 ]; then
    echo "Linux release stage failed; preserved candidate paths:" >&2
    [ -n "$feed_candidate" ] && [ -e "$feed_candidate" ] && echo "  feed: $feed_candidate" >&2
    [ -n "$share_stage" ] && [ -e "$share_stage" ] && echo "  share server: $share_stage" >&2
    [ -n "$version_stage" ] && [ -e "$version_stage" ] && echo "  version marker: $version_stage" >&2
    echo "No automatic resume is assumed. Inspect these bytes and rerun only through a verified release script." >&2
  fi
  if ! release_lock_release; then
    status=1
  fi
  exit "$status"
}
trap cleanup EXIT

if [ "$check_env" != "1" ] && [ "$check_gui" != "1" ]; then
  release_lock_acquire "$rel" "native/publish-linux-feed-wsl.sh"
  preserve_failed_stage=1
fi

echo "Preparing Linux release toolchain for Smart Explorer $version ..."
rustup target add "$linux_gui_target" "$linux_static_target" >/dev/null

find_zig() {
  if command -v zig >/dev/null 2>&1; then
    command -v zig
    return 0
  fi
  local candidate="$zig_root/zig-x86_64-linux-$zig_version/zig"
  if [ -x "$candidate" ]; then
    printf '%s\n' "$candidate"
    return 0
  fi
  return 1
}

download_zig() {
  if [ "$(uname -m)" != "x86_64" ]; then
    echo "Automatic Zig bootstrap currently supports x86_64 Linux/WSL only." >&2
    return 1
  fi
  command -v curl >/dev/null 2>&1 || { echo "curl is required to bootstrap Zig." >&2; return 1; }
  command -v tar >/dev/null 2>&1 || { echo "tar is required to bootstrap Zig." >&2; return 1; }

  mkdir -p "$zig_root"
  local archive="$zig_root/zig-x86_64-linux-$zig_version.tar.xz"
  local dir="$zig_root/zig-x86_64-linux-$zig_version"
  if [ ! -x "$dir/zig" ]; then
    echo "Downloading Zig $zig_version to $zig_root ..." >&2
    curl -L --fail -o "$archive" "https://ziglang.org/download/$zig_version/zig-x86_64-linux-$zig_version.tar.xz"
    tar -C "$zig_root" -xf "$archive"
  fi
  printf '%s\n' "$dir/zig"
}

zig_bin="$(find_zig || true)"
if [ -z "$zig_bin" ]; then
  if [ "$bootstrap_zig" = "1" ]; then
    zig_bin="$(download_zig)"
  else
    echo "zig not found. Install zig or rerun without --no-bootstrap-zig." >&2
    exit 1
  fi
fi

tool_dir="$(mktemp -d "${TMPDIR:-/tmp}/smart-explorer-release.XXXXXX")"

cat > "$tool_dir/zigcc-gnu" <<EOF
#!/usr/bin/env bash
set -e
args=()
for arg in "\$@"; do
  case "\$arg" in
    --target=x86_64-unknown-linux-gnu) args+=(--target=x86_64-linux-gnu.2.17) ;;
    --target=x86_64-unknown-linux-musl) args+=(--target=x86_64-linux-musl) ;;
    *) args+=("\$arg") ;;
  esac
done
exec "$zig_bin" cc -target x86_64-linux-gnu.2.17 "\${args[@]}"
EOF

cat > "$tool_dir/zigcc-musl" <<EOF
#!/usr/bin/env bash
set -e
args=()
for arg in "\$@"; do
  case "\$arg" in
    --target=x86_64-unknown-linux-gnu) args+=(--target=x86_64-linux-gnu) ;;
    --target=x86_64-unknown-linux-musl) args+=(--target=x86_64-linux-musl) ;;
    *) args+=("\$arg") ;;
  esac
done
exec "$zig_bin" cc -target x86_64-linux-musl "\${args[@]}"
EOF

cat > "$tool_dir/zigar" <<EOF
#!/usr/bin/env bash
exec "$zig_bin" ar "\$@"
EOF

host_triple="$(rustc -vV | sed -n 's/^host: //p')"
real_rust_lld="$(rustc --print sysroot)/lib/rustlib/$host_triple/bin/rust-lld"
if [ ! -x "$real_rust_lld" ]; then
  echo "rust-lld not found at $real_rust_lld." >&2
  exit 1
fi

# The wrapper must be named rust-lld so rustc keeps using the LLD linker flavor.
# Some desktop dependencies pass -ldl even for musl, where dlopen is provided by
# libc; filtering that flag avoids requiring a separate libdl archive.
cat > "$tool_dir/rust-lld" <<EOF
#!/usr/bin/env bash
set -e
args=()
for arg in "\$@"; do
  case "\$arg" in
    -ldl) ;;
    *) args+=("\$arg") ;;
  esac
done
exec "$real_rust_lld" "\${args[@]}"
EOF

chmod +x "$tool_dir/zigcc-gnu" "$tool_dir/zigcc-musl" "$tool_dir/zigar" "$tool_dir/rust-lld"

export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$tool_dir/zigcc-gnu"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$tool_dir/rust-lld"
export CC="$tool_dir/zigcc-gnu"
export CC_x86_64_unknown_linux_musl="$tool_dir/zigcc-musl"
export AR="$tool_dir/zigar"
export AR_x86_64_unknown_linux_musl="$tool_dir/zigar"
export PKG_CONFIG_ALLOW_CROSS=1

if [ "$check_env" = "1" ]; then
  "$script_dir/build-agent-bundles.sh" --check-env
  for tool in file ldd readelf xvfb-run xauth xwininfo; do
    command -v "$tool" >/dev/null 2>&1 || {
      echo "Required Linux GUI smoke-test tool missing: $tool" >&2
      exit 1
    }
  done
  command -v rustfmt >/dev/null 2>&1 || {
    echo "rustfmt is missing from the active Rust toolchain." >&2
    exit 1
  }
  "$zig_bin" version >/dev/null
  echo "cargo: $(cargo --version)"
  echo "rustc: $(rustc --version)"
  echo "rustfmt: $(rustfmt --version)"
  echo "Linux GUI target: $linux_gui_target"
  echo "Linux static target: $linux_static_target"
  echo "zig: $("$zig_bin" version)"
  echo "rust-lld: $real_rust_lld"
  echo "release environment OK"
  exit 0
fi

echo "Building Linux desktop app for $linux_gui_target ..."
cargo build --locked --release \
  --target-dir "$linux_target_dir" \
  --target "$linux_gui_target" --bin smart_explorer
if [ "$check_gui" = "1" ]; then
  "$script_dir/test-linux-gui-startup.sh" \
    "$linux_target_dir/$linux_gui_target/release/smart_explorer"
  echo "Targeted Linux GUI build/start check passed; no feed files were changed."
  exit 0
fi
echo "Building static Linux updater and CLI for $linux_static_target ..."
cargo build --locked --release \
  --target-dir "$linux_target_dir" \
  --target "$linux_static_target" --bin smart_explorer_updater --bin se

feed_parent="$(dirname "$feed")"
mkdir -p "$feed_parent"
feed_name="$(basename "$feed")"
feed_candidate="$(mktemp -d "$feed_parent/.${feed_name}.linux-candidate.XXXXXX")"
if [ -d "$feed" ]; then
  cp -a "$feed/." "$feed_candidate/"
elif [ -e "$feed" ]; then
  echo "Feed destination exists but is not a directory: $feed" >&2
  exit 1
fi

# A standalone --write-version invocation must commit the version only after
# the complete candidate has passed every Windows and Linux verification.
if [ "$write_version" = "1" ]; then
  rm -f "$feed_candidate/version.txt"
fi

install -m 0755 "$linux_target_dir/$linux_gui_target/release/smart_explorer" "$feed_candidate/smart_explorer"
install -m 0755 "$linux_target_dir/$linux_static_target/release/smart_explorer_updater" "$feed_candidate/smart_explorer_updater"
install -m 0755 "$linux_target_dir/$linux_static_target/release/se" "$feed_candidate/se"

mkdir -p "$(dirname "$share_out")"
feed_abs="$(cd "$feed_parent" && pwd)/$feed_name"
share_parent="$(dirname "$share_out")"
mkdir -p "$share_parent"
share_abs="$(cd "$share_parent" && pwd)/$(basename "$share_out")"
share_in_feed=0
if [ "$share_abs" = "$feed_abs" ]; then
  share_in_feed=1
elif [[ "$share_abs/" == "$feed_abs/"* ]]; then
  echo "SMART_EXPLORER_SHARE_DIR may equal the feed or be outside it, but cannot be nested inside it." >&2
  exit 1
fi

if [ "$build_share_server" = "1" ] && [ -d "$repo_root/share-server" ]; then
  echo "Building static Linux share server for $linux_static_target ..."
  (
    cd "$repo_root/share-server"
    cargo build --locked --release \
      --target-dir "$share_target_dir" \
      --target "$linux_static_target" --bin se-share-server
  )
  if [ "$share_in_feed" = "1" ]; then
    install -m 0755 \
      "$share_target_dir/$linux_static_target/release/se-share-server" \
      "$feed_candidate/se-share-server-linux"
  else
    mkdir -p "$share_out"
    share_stage="$(mktemp "$share_parent/.se-share-server-linux.stage.XXXXXX")"
    install -m 0755 \
      "$share_target_dir/$linux_static_target/release/se-share-server" \
      "$share_stage"
  fi
elif [ "$build_share_server" = "1" ]; then
  echo "Share-server source missing: $repo_root/share-server" >&2
  exit 1
fi

(
  cd "$feed_candidate"
  sha256sum smart_explorer > smart_explorer.sha256
  sha256sum smart_explorer_updater > smart_explorer_updater.sha256
  sha256sum se > se.sha256
  sha256sum -c smart_explorer.sha256
  sha256sum -c smart_explorer_updater.sha256
  sha256sum -c se.sha256
)

file "$feed_candidate/smart_explorer_updater" | grep -Eq 'statically linked|static-pie linked'
file "$feed_candidate/se" | grep -Eq 'statically linked|static-pie linked'
if [ "$build_share_server" = "1" ]; then
  if [ "$share_in_feed" = "1" ]; then
    file "$feed_candidate/se-share-server-linux" | grep -Eq 'statically linked|static-pie linked'
  else
    file "$share_stage" | grep -Eq 'statically linked|static-pie linked'
  fi
fi

verify_sha256_binding() {
  local payload=$1
  local sidecar=$2
  local expected
  local actual
  expected="$(awk 'NR == 1 { print $1 }' "$sidecar")"
  if ! [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "Invalid SHA256 sidecar token: $sidecar" >&2
    return 1
  fi
  actual="$(sha256sum "$payload" | awk '{ print $1 }')"
  if [ "${expected,,}" != "$actual" ]; then
    echo "SHA256 sidecar does not bind payload: $payload" >&2
    return 1
  fi
}

for linux_payload in smart_explorer smart_explorer_updater se; do
  verify_sha256_binding \
    "$feed_candidate/$linux_payload" \
    "$feed_candidate/$linux_payload.sha256"
done

manifest_value() {
  local manifest_path=$1
  local manifest_key=$2
  awk -F= -v key="$manifest_key" '
    $1 == key { count++; value = substr($0, index($0, "=") + 1) }
    END { sub(/\r$/, "", value); if (count != 1) exit 1; print value }
  ' "$manifest_path"
}

verify_windows_manifest() {
  local manifest="$feed_candidate/windows-build.manifest"
  local manifest_version
  local manifest_source_commit
  local expected_source_commit
  local windows_payload
  local manifest_hash
  local actual_hash
  test -s "$manifest" || {
    echo "Current-version Windows build manifest missing: $manifest" >&2
    return 1
  }
  manifest_version="$(manifest_value "$manifest" version)" || {
    echo "Windows build manifest must contain exactly one version entry." >&2
    return 1
  }
  if [ "$manifest_version" != "$version" ]; then
    echo "Windows build manifest version '$manifest_version' does not match '$version'." >&2
    return 1
  fi
  manifest_source_commit="$(manifest_value "$manifest" source_commit)" || {
    echo "Windows build manifest must contain exactly one source_commit entry." >&2
    return 1
  }
  expected_source_commit="$(git -C "$repo_root" rev-parse 'HEAD^{commit}')"
  if ! [[ "$manifest_source_commit" =~ ^[0-9a-fA-F]{40,64}$ ]] ||
    [ "${manifest_source_commit,,}" != "${expected_source_commit,,}" ]; then
    echo "Windows build manifest source commit '$manifest_source_commit' does not match '$expected_source_commit'." >&2
    return 1
  fi
  if [ "$(awk 'NF { count++ } END { print count + 0 }' "$manifest")" != "5" ]; then
    echo "Windows build manifest must contain exactly version, source_commit, and three hashes." >&2
    return 1
  fi
  for windows_payload in smart_explorer.exe smart_explorer_updater.exe se.exe; do
    test -s "$feed_candidate/$windows_payload" || {
      echo "Required Windows payload missing: $feed_candidate/$windows_payload" >&2
      return 1
    }
    test -s "$feed_candidate/$windows_payload.sha256" || {
      echo "Required Windows hash missing: $feed_candidate/$windows_payload.sha256" >&2
      return 1
    }
    manifest_hash="$(manifest_value "$manifest" "$windows_payload")" || {
      echo "Windows build manifest must contain exactly one $windows_payload entry." >&2
      return 1
    }
    if ! [[ "$manifest_hash" =~ ^[0-9a-fA-F]{64}$ ]]; then
      echo "Invalid manifest SHA256 for $windows_payload." >&2
      return 1
    fi
    actual_hash="$(sha256sum "$feed_candidate/$windows_payload" | awk '{print $1}')"
    if [ "${manifest_hash,,}" != "$actual_hash" ]; then
      echo "Windows build manifest SHA256 mismatch for $windows_payload." >&2
      return 1
    fi
    verify_sha256_binding \
      "$feed_candidate/$windows_payload" \
      "$feed_candidate/$windows_payload.sha256"
  done
}

if [ "$write_version" = "1" ]; then
  verify_windows_manifest
  for name in \
    smart_explorer.exe smart_explorer.exe.sha256 \
    smart_explorer_updater.exe smart_explorer_updater.exe.sha256 \
    se.exe se.exe.sha256 \
    smart_explorer smart_explorer.sha256 \
    smart_explorer_updater smart_explorer_updater.sha256 \
    se se.sha256; do
    test -s "$feed_candidate/$name" || {
      echo "Required feed file missing: $feed_candidate/$name" >&2
      exit 1
    }
  done
  (
    cd "$feed_candidate"
    sha256sum -c smart_explorer.exe.sha256
    sha256sum -c smart_explorer_updater.exe.sha256
    sha256sum -c se.exe.sha256
    sha256sum -c smart_explorer.sha256
    sha256sum -c smart_explorer_updater.sha256
    sha256sum -c se.sha256
  )
  test -x "$repo_root/install-linux.sh" || {
    echo "Linux installer missing or not executable: $repo_root/install-linux.sh" >&2
    exit 1
  }
  version_stage="$(mktemp "$feed_parent/.version.$$.XXXXXX")"
  printf '%s\n' "$version" > "$version_stage"
fi

# Promote the ancillary share server first, then swap the complete feed tree.
# On ordinary failures the EXIT trap restores every prior destination. The
# version file is moved into the installed feed only after the swap succeeds.
transaction_active=1
if [ "$build_share_server" = "1" ] && [ "$share_in_feed" != "1" ]; then
  share_destination="$share_out/se-share-server-linux"
  share_backup="$share_parent/.se-share-server-linux.backup.$$.${RANDOM}"
  share_install="$share_parent/.se-share-server-linux.release-new.$$.${RANDOM}"
  cp -p -- "$share_stage" "$share_install"
  if [ "$(sha256sum "$share_stage" | awk '{print $1}')" != "$(sha256sum "$share_install" | awk '{print $1}')" ]; then
    echo "Linux Share-server candidate copy verification failed." >&2
    exit 1
  fi
  if [ -e "$share_destination" ]; then
    share_had_destination=1
    mv "$share_destination" "$share_backup"
  fi
  share_installed=1
  mv "$share_install" "$share_destination"
  share_install=""
fi

feed_backup="$feed_parent/.${feed_name}.backup.$$.${RANDOM}"
feed_install="$feed_parent/.${feed_name}.release-new.$$.${RANDOM}"
cp -a -- "$feed_candidate" "$feed_install"
(
  cd "$feed_install"
  sha256sum -c smart_explorer.sha256
  sha256sum -c smart_explorer_updater.sha256
  sha256sum -c se.sha256
  if [ "$write_version" = "1" ]; then
    sha256sum -c smart_explorer.exe.sha256
    sha256sum -c smart_explorer_updater.exe.sha256
    sha256sum -c se.exe.sha256
  fi
)
if [ -e "$feed" ]; then
  feed_had_destination=1
  mv "$feed" "$feed_backup"
fi
feed_installed=1
mv "$feed_install" "$feed"
feed_install=""

if [ "$write_version" = "1" ]; then
  version_install="$feed/.version.release-new.$$.${RANDOM}"
  install -m 0644 "$version_stage" "$version_install"
  mv "$version_install" "$feed/version.txt"
fi

(
  cd "$feed"
  sha256sum -c smart_explorer.sha256
  sha256sum -c smart_explorer_updater.sha256
  sha256sum -c se.sha256
  if [ "$write_version" = "1" ]; then
    sha256sum -c smart_explorer.exe.sha256
    sha256sum -c smart_explorer_updater.exe.sha256
    sha256sum -c se.exe.sha256
  fi
)
if [ "$write_version" = "1" ]; then
  test "$(tr -d '\r\n' < "$feed/version.txt")" = "$version"
fi
if [ "$build_share_server" = "1" ]; then
  if [ "$share_in_feed" = "1" ]; then
    test -s "$feed/se-share-server-linux"
  else
    test -s "$share_out/se-share-server-linux"
  fi
fi

transaction_active=0
publication_complete=1
backup_cleanup_incomplete=0
if [ "$write_version" = "1" ]; then
  echo "Complete Linux/Windows feed atomically published: $feed (v$version)"
else
  echo "Linux feed payloads atomically staged: $feed"
  echo "version.txt not changed; the complete release wrapper owns the final version commit."
fi
if [ "$feed_had_destination" = "1" ]; then
  if ! rm -rf -- "$feed_backup"; then
    echo "WARNING: Linux feed promotion is committed and verified at $feed, but the old feed backup could not be removed: $feed_backup" >&2
    backup_cleanup_incomplete=1
  fi
fi
if [ "$share_had_destination" = "1" ]; then
  if ! rm -f -- "$share_backup"; then
    echo "WARNING: Linux feed promotion is committed and verified, but the old Share-server backup could not be removed: $share_backup" >&2
    backup_cleanup_incomplete=1
  fi
fi

if [ "$backup_cleanup_incomplete" = "1" ]; then
  echo "The promoted destination remains committed; backup cleanup is best-effort and does not require rebuilding or rerunning this promotion." >&2
fi
