#!/usr/bin/env bash
set -euo pipefail

export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
windows_target="x86_64-pc-windows-gnu"

command -v cargo >/dev/null 2>&1 || {
    echo "cargo is required" >&2
    exit 1
}

echo "remote-drive task suite: release memory boundary"
for release_leaf in \
    native/publish-feed.sh \
    native/publish-linux-feed-wsl.sh \
    native/build-agent-bundles.sh; do
    grep -Fxq 'export CARGO_BUILD_JOBS=1' "$repo_root/$release_leaf"
    grep -Fxq 'export CARGO_PROFILE_RELEASE_LTO=thin' "$repo_root/$release_leaf"
    grep -Fxq 'export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1' "$repo_root/$release_leaf"
done
grep -Fq 'lto = "thin"' "$repo_root/native/Cargo.toml"
grep -Fq '$env:CARGO_BUILD_JOBS = "1"' "$repo_root/native/publish-release-local.ps1"
grep -Fq '$env:CARGO_PROFILE_RELEASE_LTO = "thin"' "$repo_root/native/publish-update.ps1"
test "$(grep -Fc 'run-release-memory-bounded.sh' "$repo_root/native/publish-release-local.ps1")" -eq 2
grep -Fxq '  -p MemoryHigh=4G' "$repo_root/native/run-release-memory-bounded.sh"
grep -Fxq '  -p MemoryMax=5G' "$repo_root/native/run-release-memory-bounded.sh"
grep -Fxq '  -p MemorySwapMax=2G' "$repo_root/native/run-release-memory-bounded.sh"
memory_settings="$("$repo_root/native/run-release-memory-bounded.sh" \
    bash -c 'printf "%s:%s:%s" "$CARGO_BUILD_JOBS" "$CARGO_PROFILE_RELEASE_LTO" "$CARGO_PROFILE_RELEASE_CODEGEN_UNITS"')"
case "$memory_settings" in
    *"1:thin:1") ;;
    *)
        echo "unexpected release memory settings: $memory_settings" >&2
        exit 1
        ;;
esac

if command -v rustup >/dev/null 2>&1; then
    rustup target list --installed | grep -Fxq "$windows_target" || {
        echo "missing Rust target: $windows_target" >&2
        exit 1
    }
fi

echo "remote-drive task suite: native behavior"
(
    cd "$repo_root/native"
    cargo test --locked --lib remote_drive_task_ -- --test-threads=1
)

echo "remote-drive task suite: confined agent process"
(
    cd "$repo_root/se-agent"
    cargo test --locked --test remote_drive_task remote_drive_task_agent_ -- --test-threads=1
)

echo "remote-drive task suite: Windows host boundary"
(
    cd "$repo_root/native"
    cargo check --locked --lib --bin se --target "$windows_target"
)
