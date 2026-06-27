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
feed="$rel/update-feed"
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
trap 'rm -rf "$tool_dir"' EXIT

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
cargo build --release --target "$linux_target" --bin smart_explorer --bin smart_explorer_updater

mkdir -p "$feed"
cp "target/$linux_target/release/smart_explorer" "$feed/smart_explorer"
cp "target/$linux_target/release/smart_explorer_updater" "$feed/smart_explorer_updater"

if [ "$build_share_server" = "1" ] && [ -d "$repo_root/share-server" ]; then
  echo "Building Linux share server for $linux_target ..."
  (
    cd "$repo_root/share-server"
    cargo build --release --target "$linux_target" --bin se-share-server
  )
  mkdir -p "$rel/share-server"
  cp "$repo_root/share-server/target/$linux_target/release/se-share-server" "$rel/share-server/se-share-server-linux"
fi

(
  cd "$feed"
  sha256sum smart_explorer > smart_explorer.sha256
  sha256sum smart_explorer_updater > smart_explorer_updater.sha256
)

if [ "$write_version" = "1" ]; then
  printf '%s\n' "$version" > "$feed/version.txt"
  echo "Linux feed payloads staged and version.txt updated: $feed (v$version)"
else
  echo "Linux feed payloads staged: $feed"
  echo "version.txt not changed; pass --write-version from the full release wrapper."
fi
