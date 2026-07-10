#!/usr/bin/env bash
# Build the two static SSH-agent payloads that are embedded into the desktop
# app. Run this before any native app build so agent source/protocol changes can
# never ship with stale committed binaries.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
agent_dir="$repo_root/se-agent"
bundle_dir="$script_dir/agent-bin"
x86_target="x86_64-unknown-linux-musl"
arm_target="aarch64-unknown-linux-musl"
check_env=0

if [ "${1:-}" = "--check-env" ]; then
  check_env=1
  shift
fi
if [ "$#" -ne 0 ]; then
  echo "Usage: native/build-agent-bundles.sh [--check-env]" >&2
  exit 2
fi

command -v cargo >/dev/null 2>&1 || { echo "cargo not found" >&2; exit 1; }
command -v rustc >/dev/null 2>&1 || { echo "rustc not found" >&2; exit 1; }
command -v rustup >/dev/null 2>&1 || { echo "rustup not found" >&2; exit 1; }
test -d "$agent_dir" || { echo "se-agent source missing: $agent_dir" >&2; exit 1; }

rustup target add "$x86_target" "$arm_target" >/dev/null
host_triple="$(rustc -vV | sed -n 's/^host: //p')"
rust_lld="$(rustc --print sysroot)/lib/rustlib/$host_triple/bin/rust-lld"
test -x "$rust_lld" || { echo "rust-lld not found: $rust_lld" >&2; exit 1; }

export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$rust_lld"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$rust_lld"

if [ "$check_env" = "1" ]; then
  echo "SSH-agent bundle environment OK ($x86_target, $arm_target; $rust_lld)"
  exit 0
fi

(
  cd "$agent_dir"
  cargo build --release --target "$x86_target" --bin se-agent
  cargo build --release --target "$arm_target" --bin se-agent
)

mkdir -p "$bundle_dir"
x86_tmp="$bundle_dir/.se-agent-x86_64-linux-musl.$$.tmp"
arm_tmp="$bundle_dir/.se-agent-aarch64-linux-musl.$$.tmp"
trap 'rm -f "$x86_tmp" "$arm_tmp"' EXIT
install -m 0755 "$agent_dir/target/$x86_target/release/se-agent" "$x86_tmp"
install -m 0755 "$agent_dir/target/$arm_target/release/se-agent" "$arm_tmp"
mv -f "$x86_tmp" "$bundle_dir/se-agent-x86_64-linux-musl"
mv -f "$arm_tmp" "$bundle_dir/se-agent-aarch64-linux-musl"

test -s "$bundle_dir/se-agent-x86_64-linux-musl"
test -s "$bundle_dir/se-agent-aarch64-linux-musl"
"$bundle_dir/se-agent-x86_64-linux-musl" --version | grep -Eq '^proto=[0-9]+ ver=[0-9]'
echo "SSH-agent bundles refreshed: $bundle_dir"
