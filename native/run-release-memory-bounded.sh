#!/usr/bin/env bash
# Run one release build tree inside a bounded cgroup when the host supports it.
# Compiler-level limits remain active on hosts without a usable systemd scope.
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "Usage: native/run-release-memory-bounded.sh COMMAND [ARG ...]" >&2
  exit 2
fi

export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_RELEASE_LTO=thin
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8
export CARGO_PROFILE_RELEASE_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0

scope_properties=(
  --scope
  --quiet
  --collect
  -p MemoryHigh=3G
  -p MemoryMax=4G
  -p MemorySwapMax=1G
  -p OOMPolicy=kill
)

if command -v systemd-run >/dev/null 2>&1; then
  if systemd-run --user "${scope_properties[@]}" true >/dev/null 2>&1; then
    echo "Build memory scope: user cgroup (high 3G, max 4G, swap max 1G)."
    exec systemd-run --user "${scope_properties[@]}" "$@"
  fi
  if [ "$(id -u)" -eq 0 ] && \
      systemd-run "${scope_properties[@]}" true >/dev/null 2>&1; then
    echo "Build memory scope: system cgroup (high 3G, max 4G, swap max 1G)."
    exec systemd-run "${scope_properties[@]}" "$@"
  fi
fi

echo "Warning: no usable systemd cgroup scope; compiler memory limits remain active." >&2
exec "$@"
