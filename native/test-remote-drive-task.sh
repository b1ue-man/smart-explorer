#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
windows_target="x86_64-pc-windows-gnu"

command -v cargo >/dev/null 2>&1 || {
    echo "cargo is required" >&2
    exit 1
}

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
