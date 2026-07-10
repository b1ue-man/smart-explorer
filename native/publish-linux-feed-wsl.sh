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

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"
repo_root="$(cd .. && pwd)"
rel="$repo_root/release-native"
feed="${SMART_EXPLORER_FEED_DIR:-$rel/update-feed}"
share_out="${SMART_EXPLORER_SHARE_DIR:-$rel/share-server}"
linux_target="x86_64-unknown-linux-musl"
write_version=0
build_share_server=1
check_env=0
bootstrap_zig="${SMART_EXPLORER_BOOTSTRAP_ZIG:-1}"
zig_version="${SMART_EXPLORER_ZIG_VERSION:-0.16.0}"
zig_root="${SMART_EXPLORER_ZIG_ROOT:-$HOME/.local/zig}"

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
    --no-bootstrap-zig)
      bootstrap_zig=0
      ;;
    --target)
      shift
      linux_target="${1:?missing value for --target}"
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

if [ "$write_version" = "1" ] && [ "$build_share_server" != "1" ]; then
  echo "--write-version requires the Linux share-server build; remove --skip-share-server." >&2
  exit 1
fi

version="$(sed -nE 's/^version = "([^"]+)".*/\1/p' Cargo.toml | head -1)"
if [ -z "$version" ]; then
  echo "Could not read version from native/Cargo.toml." >&2
  exit 1
fi

echo "Preparing Linux release toolchain for Smart Explorer $version ..."
rustup target add "$linux_target" >/dev/null

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
feed_candidate=""
feed_backup=""
share_stage=""
share_backup=""
version_stage=""
feed_installed=0
feed_had_destination=0
share_installed=0
share_had_destination=0
transaction_active=0

rollback_publication() {
  if [ "$feed_installed" = "1" ] && [ -e "$feed" ]; then
    rm -rf "$feed"
  fi
  if [ "$feed_had_destination" = "1" ] && [ -n "$feed_backup" ] && [ -e "$feed_backup" ]; then
    mv "$feed_backup" "$feed"
  fi
  if [ "$share_installed" = "1" ] && [ -e "$share_out/se-share-server-linux" ]; then
    rm -f "$share_out/se-share-server-linux"
  fi
  if [ "$share_had_destination" = "1" ] && [ -n "$share_backup" ] && [ -e "$share_backup" ]; then
    mv "$share_backup" "$share_out/se-share-server-linux"
  fi
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [ "$transaction_active" = "1" ]; then
    rollback_publication
  fi
  rm -rf "$tool_dir"
  if [ -n "$feed_candidate" ]; then
    rm -rf "$feed_candidate"
  fi
  if [ -n "$share_stage" ]; then
    rm -f "$share_stage"
  fi
  if [ -n "$version_stage" ]; then
    rm -f "$version_stage"
  fi
  exit "$status"
}
trap cleanup EXIT

cat > "$tool_dir/zigcc-gnu" <<EOF
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
exec "$zig_bin" cc -target x86_64-linux-gnu "\${args[@]}"
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
  echo "cargo: $(cargo --version)"
  echo "rustc: $(rustc --version)"
  echo "rustfmt: $(rustfmt --version)"
  echo "clippy: $(cargo clippy --version)"
  echo "target: $linux_target"
  echo "zig: $("$zig_bin" version)"
  echo "rust-lld: $real_rust_lld"
  echo "release environment OK"
  exit 0
fi

echo "Building native Linux payloads for $linux_target ..."
cargo build --release --target "$linux_target" --bin smart_explorer --bin smart_explorer_updater --bin se

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

install -m 0755 "target/$linux_target/release/smart_explorer" "$feed_candidate/smart_explorer"
install -m 0755 "target/$linux_target/release/smart_explorer_updater" "$feed_candidate/smart_explorer_updater"
install -m 0755 "target/$linux_target/release/se" "$feed_candidate/se"

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
  echo "Building Linux share server for $linux_target ..."
  (
    cd "$repo_root/share-server"
    cargo build --release --target "$linux_target" --bin se-share-server
  )
  if [ "$share_in_feed" = "1" ]; then
    install -m 0755 \
      "$repo_root/share-server/target/$linux_target/release/se-share-server" \
      "$feed_candidate/se-share-server-linux"
  else
    mkdir -p "$share_out"
    share_stage="$(mktemp "$share_parent/.se-share-server-linux.stage.XXXXXX")"
    install -m 0755 \
      "$repo_root/share-server/target/$linux_target/release/se-share-server" \
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

file "$feed_candidate/smart_explorer" | grep -Eq 'statically linked|static-pie linked'
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
  if [ -e "$share_destination" ]; then
    share_had_destination=1
    mv "$share_destination" "$share_backup"
  fi
  share_installed=1
  mv "$share_stage" "$share_destination"
  share_stage=""
fi

feed_backup="$feed_parent/.${feed_name}.backup.$$.${RANDOM}"
if [ -e "$feed" ]; then
  feed_had_destination=1
  mv "$feed" "$feed_backup"
fi
feed_installed=1
mv "$feed_candidate" "$feed"
feed_candidate=""

if [ "$write_version" = "1" ]; then
  mv "$version_stage" "$feed/version.txt"
  version_stage=""
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
if [ "$feed_had_destination" = "1" ]; then
  rm -rf "$feed_backup"
fi
if [ "$share_had_destination" = "1" ]; then
  rm -f "$share_backup"
fi

if [ "$write_version" = "1" ]; then
  echo "Complete Linux/Windows feed atomically published: $feed (v$version)"
else
  echo "Linux feed payloads atomically staged: $feed"
  echo "version.txt not changed; the complete release wrapper owns the final version commit."
fi
