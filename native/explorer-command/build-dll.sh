#!/usr/bin/env bash
# Cross-compile the IExplorerCommand handler DLL (windows-gnu). Output:
#   target/x86_64-pc-windows-gnu/release/smart_explorer_command.dll
# Packaging + signing happen on Windows — see ../../docs/WIN11_CONTEXT_MENU.md.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --target x86_64-pc-windows-gnu
dll="target/x86_64-pc-windows-gnu/release/smart_explorer_command.dll"
echo "Built: $dll"
# Sanity-check the COM exports are present.
if command -v x86_64-w64-mingw32-objdump >/dev/null 2>&1; then
  echo "Exports:"
  x86_64-w64-mingw32-objdump -p "$dll" | grep -iE "DllGetClassObject|DllCanUnloadNow" || true
fi
