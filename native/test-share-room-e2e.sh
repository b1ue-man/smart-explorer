#!/usr/bin/env bash
# Room lifecycle and concurrent-transfer end-to-end check over a local Share
# server: two headless CLI clients create and join a Room by its invite code,
# export one folder to the Room only, run every file transaction over
# share://room/<room>/<device>/..., complete three concurrent downloads in both
# directions byte-exact, and lose access after leaving. Linux only (daemon
# discovery reads /proc). The remote task suite runs it; never run it on the
# workstation.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
se_bin="${SMART_EXPLORER_SE_BINARY:-$repo_root/native/target/debug/se}"
server_bin="${SMART_EXPLORER_SHARE_SERVER_BINARY:-$repo_root/share-server/target/debug/se-share-server}"

for tool in jq timeout cmp head awk; do
  command -v "$tool" >/dev/null || {
    echo "Share Room E2E requires $tool" >&2
    exit 1
  }
done
test -x "$se_bin" || {
  echo "se test binary is missing: $se_bin" >&2
  exit 1
}
test -x "$server_bin" || {
  echo "share-server test binary is missing: $server_bin" >&2
  exit 1
}

root="$(mktemp -d "${TMPDIR:-/tmp}/se-share-room.XXXXXX")"
client_c="$root/c"
client_d="$root/d"
server_log="$root/share-server.log"
server_pid=""

trap 'echo "Share Room E2E failed at line $LINENO: $BASH_COMMAND" >&2' ERR

cleanup() {
  local status=$?
  stop_daemon "$client_c" || true
  stop_daemon "$client_d" || true
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ $status -ne 0 ]]; then
    echo "Share Room E2E failed; diagnostics: $root" >&2
    tail -n 40 "$server_log" >&2 2>/dev/null || true
  fi
  if [[ $status -eq 0 && "${SMART_EXPLORER_KEEP_E2E_ROOT:-0}" != 1 ]]; then
    rm -rf "$root"
  fi
  return "$status"
}
trap cleanup EXIT

prepare_client() {
  local client="$1"
  mkdir -p "$client/home" "$client/data" "$client/config" "$client/runtime"
  chmod 700 "$client/home" "$client/data" "$client/config" "$client/runtime"
}

run_client() {
  local client="$1"
  shift
  timeout --foreground --signal=TERM --kill-after=5s 90s env \
    HOME="$client/home" \
    USERPROFILE="$client/home" \
    XDG_DATA_HOME="$client/data" \
    XDG_CONFIG_HOME="$client/config" \
    XDG_RUNTIME_DIR="$client/runtime" \
    APPDATA="$client/data" \
    LOCALAPPDATA="$client/data" \
    SE_SHARE_RELAY_ONLY=1 \
    "$se_bin" "$@"
}

# Use only for a background invocation. `exec` replaces Bash's asynchronous
# function subshell so `$!` is the actual `se` process.
run_client_background() {
  local client="$1"
  shift
  exec env \
    HOME="$client/home" \
    USERPROFILE="$client/home" \
    XDG_DATA_HOME="$client/data" \
    XDG_CONFIG_HOME="$client/config" \
    XDG_RUNTIME_DIR="$client/runtime" \
    APPDATA="$client/data" \
    LOCALAPPDATA="$client/data" \
    SE_SHARE_RELAY_ONLY=1 \
    "$se_bin" "$@"
}

daemon_pids() {
  local client="$1"
  local expected="XDG_DATA_HOME=$client/data"
  local env_file pid command
  for env_file in /proc/[0-9]*/environ; do
    [[ -r "$env_file" ]] || continue
    if tr '\0' '\n' 2>/dev/null <"$env_file" | grep -Fqx "$expected"; then
      pid="${env_file#/proc/}"
      pid="${pid%/environ}"
      command="$(tr '\0' ' ' 2>/dev/null <"/proc/$pid/cmdline" || true)"
      if [[ "$command" == *"--sync-daemon"* ]]; then
        printf '%s\n' "$pid"
      fi
    fi
  done
}

stop_daemon() {
  local client="$1"
  local pid
  while read -r pid; do
    [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
  done < <(daemon_pids "$client")
  local deadline=$((SECONDS + 10))
  while [[ $SECONDS -lt $deadline ]] && [[ -n "$(daemon_pids "$client")" ]]; do
    sleep 0.05
  done
  local remaining
  remaining="$(daemon_pids "$client")"
  if [[ -n "$remaining" ]]; then
    echo "Share daemon did not stop for $client: $remaining" >&2
    return 1
  fi
}

wait_relay_route() {
  local client="$1"
  local expected_relay="$2"
  local deadline=$((SECONDS + 90))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share status --json 2>/dev/null)" \
      && jq -e --arg relay "$expected_relay" \
        '.worker.reachable == true and
         .worker.running == true and
         .worker.connected == true and
         ((.worker.relay_url | rtrimstr("/")) == ($relay | rtrimstr("/"))) and
         (.worker.candidates | length) == 0' >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "relay-only route $expected_relay did not become ready for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_child() {
  local pid="$1"
  local timeout="$2"
  local deadline=$((SECONDS + timeout))
  while kill -0 "$pid" 2>/dev/null && [[ $SECONDS -lt $deadline ]]; do
    sleep 0.1
  done
  if kill -0 "$pid" 2>/dev/null; then
    ps -o pid,ppid,stat,wchan:32,etime,cmd -p "$pid" >&2 || true
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "child $pid did not exit within ${timeout}s" >&2
    return 1
  fi
  set +e
  wait "$pid"
  child_status=$?
  set -e
}

wait_room_members() {
  local client="$1"
  local room_id="$2"
  local expected="$3"
  local deadline=$((SECONDS + 90))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share status --json 2>/dev/null)" \
      && jq -e --arg room "$room_id" --argjson members "$expected" \
        '[.rooms[] | select(.room_id == $room)] | length == 1 and .[0].members == $members' \
        >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "room $room_id did not reach $expected member(s) for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_room_access_denied() {
  local client="$1"
  local endpoint="$2"
  local message="$3"
  local deadline=$((SECONDS + 90))
  while [[ $SECONDS -lt $deadline ]]; do
    if ! run_client "$client" ls "$endpoint" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "$message" >&2
  return 1
}

prepare_client "$client_c"
prepare_client "$client_d"

signal_port=$((33000 + ($$ % 12000)))
relay_port=$((signal_port + 1))
working_signal="127.0.0.1:$signal_port"
working_relay="http://127.0.0.1:$relay_port"
"$server_bin" "127.0.0.1:$signal_port" >"$server_log" 2>&1 &
server_pid=$!
sleep 0.5
kill -0 "$server_pid"

# Both headless clients run their Share worker against the local server; the
# device identities come from the CLI itself, never from the harness.
identity_c="$(run_client "$client_c" share identity --json)"
identity_d="$(run_client "$client_d" share identity --json)"
device_c="$(jq -er '.device_id' <<<"$identity_c")"
device_d="$(jq -er '.device_id' <<<"$identity_d")"
[[ -n "$device_c" && -n "$device_d" && "$device_c" != "$device_d" ]]
for client in "$client_c" "$client_d"; do
  run_client "$client" share configure --server "$working_signal" >/dev/null
  stop_daemon "$client"
  run_client "$client" share status --json >/dev/null
  wait_relay_route "$client" "$working_relay" >/dev/null
done

# C creates a Room and D joins it with nothing but the printed invite code.
room_create="$(run_client "$client_c" share room create --name Team)"
room_code="$(awk -F '\t' '$1 == "room_code" { print $2 }' <<<"$room_create")"
room_profile_c="$(awk -F '\t' '$1 == "room_id" { print $2 }' <<<"$room_create")"
[[ "$room_code" == SE-R3-*-* && -n "$room_profile_c" ]]
room_relation_id="${room_code#SE-R3-}"
room_relation_id="${room_relation_id%-*}"
[[ -n "$room_relation_id" && "$room_relation_id" != *-* ]]
run_client "$client_d" connections add-room --code "$room_code" --name Team >/dev/null
run_client "$client_c" share worker refresh >/dev/null
run_client "$client_d" share worker refresh >/dev/null
wait_room_members "$client_c" "$room_relation_id" 1 >/dev/null
wait_room_members "$client_d" "$room_relation_id" 1 >/dev/null
status_c="$(run_client "$client_c" share status --json)"
jq -e --arg room "$room_relation_id" \
  '[.rooms[] | select(.room_id == $room and .name == "Team")] | length == 1' \
  >/dev/null <<<"$status_c"

# Each side exports one folder to the Room only. A Room inherits the default
# Direct exports (a fresh profile exports `Home`) when it is created or joined;
# those inherited roots are removed from the Room scope here, which also proves
# that Room and Direct export configurations are independent.
export_c="$client_c/home/room-export"
export_d="$client_d/home/room-export"
mkdir -p "$export_c" "$export_d"
for client in "$client_c" "$client_d"; do
  run_client "$client" share export add "$client/home/room-export" --label RoomDocs --room Team >/dev/null
  room_exports="$(run_client "$client" share export list --room Team --json)"
  jq -e '.roots | any(.label == "RoomDocs")' >/dev/null <<<"$room_exports"
  while IFS= read -r inherited_label; do
    [[ -n "$inherited_label" ]] || continue
    run_client "$client" share export remove "$inherited_label" --room Team >/dev/null
  done < <(jq -r '.roots[] | select(.label != "RoomDocs") | .label' <<<"$room_exports")
  run_client "$client" share worker refresh >/dev/null
  room_exports="$(run_client "$client" share export list --room Team --json)"
  jq -e '.roots | length == 1 and .[0].label == "RoomDocs"' >/dev/null <<<"$room_exports"
  direct_exports="$(run_client "$client" share export list --json)"
  jq -e '.roots | all(.label != "RoomDocs")' >/dev/null <<<"$direct_exports"
done
room_endpoint_c="share://room/$room_relation_id/$device_c"
room_endpoint_d="share://room/$room_relation_id/$device_d"

# Read side: listing, metadata, cat and a download over the Room relation.
printf 'room alpha\n' >"$export_c/alpha.txt"
room_root_listing="$(run_client "$client_d" ls "$room_endpoint_c")"
grep -Fq 'RoomDocs' <<<"$room_root_listing"
room_docs_listing="$(run_client "$client_d" ls "$room_endpoint_c/RoomDocs")"
grep -Fq 'alpha.txt' <<<"$room_docs_listing"
[[ "$(run_client "$client_d" cat "$room_endpoint_c/RoomDocs/alpha.txt")" == 'room alpha' ]]
run_client "$client_d" stat "$room_endpoint_c/RoomDocs/alpha.txt" >/dev/null
run_client "$client_d" cp "$room_endpoint_c/RoomDocs/alpha.txt" "$root/room-alpha.txt" >/dev/null
cmp "$export_c/alpha.txt" "$root/room-alpha.txt"

# Write side: upload, folder creation, recursive copy, rename, search, delete.
head -c 3145728 /dev/urandom >"$root/room-upload.bin"
run_client "$client_d" cp "$root/room-upload.bin" "$room_endpoint_c/RoomDocs/upload.bin" >/dev/null
cmp "$root/room-upload.bin" "$export_c/upload.bin"
run_client "$client_d" mkdir "$room_endpoint_c/RoomDocs/nested" >/dev/null
test -d "$export_c/nested"
mkdir -p "$root/room-tree/sub"
printf 'one' >"$root/room-tree/one.txt"
printf 'two' >"$root/room-tree/sub/two.txt"
run_client "$client_d" cp --recursive "$root/room-tree" "$room_endpoint_c/RoomDocs/nested/tree" >/dev/null
cmp "$root/room-tree/sub/two.txt" "$export_c/nested/tree/sub/two.txt"
run_client "$client_d" mv "$room_endpoint_c/RoomDocs/nested/tree/one.txt" \
  "$room_endpoint_c/RoomDocs/nested/tree/renamed.txt" >/dev/null
test -f "$export_c/nested/tree/renamed.txt"
test ! -e "$export_c/nested/tree/one.txt"
room_search="$(run_client "$client_d" search "$room_endpoint_c/RoomDocs" two)"
grep -Fq 'two.txt' <<<"$room_search"
run_client "$client_d" rm --recursive --force "$room_endpoint_c/RoomDocs/nested" >/dev/null
test ! -e "$export_c/nested"

# Concurrency: two downloads from C plus one download from D run at the same
# time over the same Room relation and must all complete byte-exact.
head -c 8388608 /dev/urandom >"$export_c/big-1.bin"
head -c 8388608 /dev/urandom >"$export_c/big-2.bin"
head -c 8388608 /dev/urandom >"$export_d/big-3.bin"
run_client_background "$client_d" cp "$room_endpoint_c/RoomDocs/big-1.bin" "$root/big-1.bin" \
  </dev/null >"$root/parallel-1.out" 2>&1 &
parallel_1=$!
run_client_background "$client_d" cp "$room_endpoint_c/RoomDocs/big-2.bin" "$root/big-2.bin" \
  </dev/null >"$root/parallel-2.out" 2>&1 &
parallel_2=$!
run_client_background "$client_c" cp "$room_endpoint_d/RoomDocs/big-3.bin" "$root/big-3.bin" \
  </dev/null >"$root/parallel-3.out" 2>&1 &
parallel_3=$!
wait_child "$parallel_1" 180
[[ $child_status -eq 0 ]] || { cat "$root/parallel-1.out" >&2; false; }
wait_child "$parallel_2" 180
[[ $child_status -eq 0 ]] || { cat "$root/parallel-2.out" >&2; false; }
wait_child "$parallel_3" 180
[[ $child_status -eq 0 ]] || { cat "$root/parallel-3.out" >&2; false; }
cmp "$export_c/big-1.bin" "$root/big-1.bin"
cmp "$export_c/big-2.bin" "$root/big-2.bin"
cmp "$export_d/big-3.bin" "$root/big-3.bin"

# Leaving the Room removes the local profile and its export; the remaining
# member must no longer reach the leaver's files. Both denials become stable
# once the leaver's worker dropped the Room and the RoomLeft event reached the
# remaining member, so they are awaited rather than asserted immediately.
run_client "$client_d" connections remove-room Team >/dev/null
after_leave_d="$(run_client "$client_d" share status --json)"
jq -e --arg room "$room_relation_id" '[.rooms[] | select(.room_id == $room)] | length == 0' \
  >/dev/null <<<"$after_leave_d"
wait_room_access_denied "$client_d" "$room_endpoint_c" \
  "a device that left the Room still reached the Room files"
wait_room_access_denied "$client_c" "$room_endpoint_d/RoomDocs" \
  "the remaining member still reached the leaver's Room export"
run_client "$client_c" connections remove-room Team >/dev/null
echo "Room lifecycle passed: $room_relation_id"
