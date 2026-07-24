#!/usr/bin/env bash
# Run one focused development task inside a conservative memory boundary.
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "Usage: native/run-task-memory-bounded.sh COMMAND [ARG ...]" >&2
  exit 2
fi

export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0

full_scope_properties=(
  --scope
  --quiet
  --collect
  -p MemoryHigh=1792M
  -p MemoryMax=2G
  -p MemorySwapMax=256M
  -p OOMPolicy=kill
)

essential_scope_properties=(
  --scope
  --quiet
  -p MemoryHigh=1792M
  -p MemoryMax=2G
)

hard_scope_properties=(
  --scope
  --quiet
  -p MemoryMax=2G
)

systemd_safety_options=(
  --expand-environment=no
)

memory_limit_probe='set -eu
cgroup_path=""
while IFS=: read -r hierarchy controllers path; do
  if [ "$hierarchy" = "0" ] && [ -z "$controllers" ]; then
    cgroup_path=$path
    break
  fi
done < /proc/self/cgroup
case "$cgroup_path" in
  /*) ;;
  *) exit 1 ;;
esac
memory_max_path="/sys/fs/cgroup${cgroup_path%/}/memory.max"
[ -r "$memory_max_path" ]
memory_max=$(cat "$memory_max_path")
case "$memory_max" in
  ""|*[!0-9]*) exit 1 ;;
esac
[ "$memory_max" -le 2147483648 ]'

memory_scope_guard="${memory_limit_probe}
exec \"\$@\""

scope_limit_is_effective() {
  local manager=$1
  shift
  if [ "$manager" = user ]; then
    systemd-run --user "${systemd_safety_options[@]}" "$@" \
      sh -c "$memory_limit_probe" >/dev/null 2>&1
  else
    systemd-run "${systemd_safety_options[@]}" "$@" \
      sh -c "$memory_limit_probe" >/dev/null 2>&1
  fi
}

if command -v systemd-run >/dev/null 2>&1; then
  if scope_limit_is_effective user "${full_scope_properties[@]}"; then
    echo "Task memory scope: user cgroup (high 1792M, max 2G, swap max 256M)."
    exec systemd-run --user "${systemd_safety_options[@]}" \
      "${full_scope_properties[@]}" \
      sh -c "$memory_scope_guard" task-memory-scope "$@"
  fi
  if scope_limit_is_effective user "${essential_scope_properties[@]}"; then
    echo "Task memory scope: user cgroup (high 1792M, hard max 2G)."
    exec systemd-run --user "${systemd_safety_options[@]}" \
      "${essential_scope_properties[@]}" \
      sh -c "$memory_scope_guard" task-memory-scope "$@"
  fi
  if scope_limit_is_effective user "${hard_scope_properties[@]}"; then
    echo "Task memory scope: user cgroup (hard max 2G)."
    exec systemd-run --user "${systemd_safety_options[@]}" \
      "${hard_scope_properties[@]}" \
      sh -c "$memory_scope_guard" task-memory-scope "$@"
  fi
  if [ "$(id -u)" -eq 0 ] && \
      scope_limit_is_effective system "${full_scope_properties[@]}"; then
    echo "Task memory scope: system cgroup (high 1792M, max 2G, swap max 256M)."
    exec systemd-run "${systemd_safety_options[@]}" \
      "${full_scope_properties[@]}" \
      sh -c "$memory_scope_guard" task-memory-scope "$@"
  fi
  if [ "$(id -u)" -eq 0 ] && \
      scope_limit_is_effective system "${essential_scope_properties[@]}"; then
    echo "Task memory scope: system cgroup (high 1792M, hard max 2G)."
    exec systemd-run "${systemd_safety_options[@]}" \
      "${essential_scope_properties[@]}" \
      sh -c "$memory_scope_guard" task-memory-scope "$@"
  fi
  if [ "$(id -u)" -eq 0 ] && \
      scope_limit_is_effective system "${hard_scope_properties[@]}"; then
    echo "Task memory scope: system cgroup (hard max 2G)."
    exec systemd-run "${systemd_safety_options[@]}" \
      "${hard_scope_properties[@]}" \
      sh -c "$memory_scope_guard" task-memory-scope "$@"
  fi
fi

echo "No usable aggregate cgroup memory boundary is available; refusing to run." >&2
exit 1
