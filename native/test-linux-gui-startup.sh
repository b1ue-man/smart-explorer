#!/usr/bin/env bash
# Verify that a Linux desktop release can load its display libraries and create
# a real X11 window. Static-musl binaries cannot satisfy winit's dlopen-based
# X11 backend, so this deliberately checks the exact staged executable.

set -euo pipefail
export LC_ALL=C

strict_glibc_baseline=1
if [ "${1:-}" = "--runtime-only" ]; then
  strict_glibc_baseline=0
  shift
fi
if [ "$#" -ne 1 ]; then
  echo "Usage: $0 [--runtime-only] <smart_explorer-linux-binary>" >&2
  exit 2
fi

binary="$(realpath "$1")"
test -x "$binary" || {
  echo "Linux GUI binary is missing or not executable: $binary" >&2
  exit 1
}

for tool in file ldd readelf xvfb-run xauth xwininfo; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Required Linux GUI smoke-test tool missing: $tool" >&2
    exit 1
  }
done

file_description="$(file -Lb "$binary")"
if [[ "$file_description" != *"dynamically linked"* ]]; then
  echo "Linux GUI must be dynamically linked so winit can load X11/Wayland libraries." >&2
  echo "$file_description" >&2
  exit 1
fi
if ! readelf -l "$binary" | grep -Fq '/ld-linux-x86-64.so.2'; then
  echo "Linux GUI must target the x86_64 GNU/glibc runtime." >&2
  readelf -l "$binary" >&2
  exit 1
fi

# Rust's x86_64 GNU target promises glibc 2.17 compatibility, and the Zig
# linker is pinned to that same baseline. Guard the actual ELF version needs so
# a future native dependency cannot silently raise the shipped requirement.
glibc_versions="$({
  readelf --version-info "$binary" \
    | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)*' \
    | sed 's/^GLIBC_//' \
    | sort -Vu
} || true)"
if [ -z "$glibc_versions" ]; then
  echo "Linux GUI does not expose any verifiable GLIBC_* requirements." >&2
  exit 1
fi
glibc_max="$(tail -n 1 <<<"$glibc_versions")"
if [ "$strict_glibc_baseline" = "1" ] \
  && [ "$(printf '%s\n%s\n' '2.17' "$glibc_max" | sort -V | tail -n 1)" != "2.17" ]; then
  echo "Linux GUI requires GLIBC_$glibc_max, newer than the supported GLIBC_2.17 baseline." >&2
  echo "Required GLIBC versions: $(tr '\n' ' ' <<<"$glibc_versions")" >&2
  exit 1
fi

ldd_output="$(ldd "$binary" 2>&1)" || {
  echo "Unable to resolve Linux GUI runtime libraries:" >&2
  echo "$ldd_output" >&2
  exit 1
}
if grep -Fq 'not found' <<<"$ldd_output"; then
  echo "Linux GUI has unresolved runtime libraries:" >&2
  echo "$ldd_output" >&2
  exit 1
fi

smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/smart-explorer-gui-smoke.XXXXXX")"
smoke_token="gui-smoke-$$-$RANDOM-$RANDOM"
stdout_log="$smoke_root/stdout.log"
stderr_log="$smoke_root/stderr.log"

cleanup_marked_processes() {
  local signal=$1
  local environ
  local pid
  for environ in /proc/[0-9]*/environ; do
    if grep -zFqx "SMART_EXPLORER_LINUX_GUI_SMOKE_TOKEN=$smoke_token" "$environ" 2>/dev/null; then
      pid="${environ#/proc/}"
      pid="${pid%/environ}"
      if [[ "$pid" =~ ^[0-9]+$ ]]; then
        kill "-$signal" "$pid" 2>/dev/null || true
      fi
    fi
  done
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  cleanup_marked_processes TERM
  sleep 0.2
  cleanup_marked_processes KILL
  rm -rf "$smoke_root"
  exit "$status"
}
trap cleanup EXIT

mkdir -p "$smoke_root/home" "$smoke_root/config" "$smoke_root/data" \
  "$smoke_root/cache" "$smoke_root/runtime"
chmod 0700 "$smoke_root/runtime"

set +e
# Positional parameters are deliberately passed into the single-quoted script.
# shellcheck disable=SC2016
env \
  SMART_EXPLORER_LINUX_GUI_SMOKE_TOKEN="$smoke_token" \
  HOME="$smoke_root/home" \
  XDG_CONFIG_HOME="$smoke_root/config" \
  XDG_DATA_HOME="$smoke_root/data" \
  XDG_CACHE_HOME="$smoke_root/cache" \
  XDG_RUNTIME_DIR="$smoke_root/runtime" \
  LIBGL_ALWAYS_SOFTWARE=1 \
  xvfb-run -a -s '-screen 0 1440x900x24' \
    bash -c '
      binary=$1
      stdout_log=$2
      stderr_log=$3
      "$binary" >"$stdout_log" 2>"$stderr_log" &
      app_pid=$!
      for ((attempt = 0; attempt < 200; attempt++)); do
        if ! kill -0 "$app_pid" 2>/dev/null; then
          wait "$app_pid"
          app_status=$?
          if [ "$app_status" -eq 0 ]; then
            exit 1
          fi
          exit "$app_status"
        fi
        if xwininfo -root -tree 2>/dev/null | grep -Fq '"'"'Smart Explorer'"'"'; then
          kill -TERM "$app_pid" 2>/dev/null || true
          wait "$app_pid" 2>/dev/null || true
          exit 0
        fi
        sleep 0.1
      done
      kill -TERM "$app_pid" 2>/dev/null || true
      wait "$app_pid" 2>/dev/null || true
      exit 124
    ' _ "$binary" "$stdout_log" "$stderr_log"
smoke_status=$?
set -e

if [ "$smoke_status" -ne 0 ]; then
  echo "Linux GUI failed to create a Smart Explorer window (status $smoke_status)." >&2
  if [ -s "$stderr_log" ]; then
    echo "--- stderr ---" >&2
    sed -n '1,160p' "$stderr_log" >&2
  fi
  if [ -s "$stdout_log" ]; then
    echo "--- stdout ---" >&2
    sed -n '1,80p' "$stdout_log" >&2
  fi
  exit 1
fi

if [ "$strict_glibc_baseline" = "1" ]; then
  echo "Linux GUI startup verified: $binary (maximum GLIBC_$glibc_max)"
else
  echo "Linux GUI runtime startup verified: $binary (host maximum GLIBC_$glibc_max)"
fi
