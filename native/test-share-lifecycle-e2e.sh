#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
se_bin="${SMART_EXPLORER_SE_BINARY:-$repo_root/native/target/debug/se}"
server_bin="${SMART_EXPLORER_SHARE_SERVER_BINARY:-$repo_root/share-server/target/debug/se-share-server}"

command -v jq >/dev/null || {
  echo "share lifecycle E2E requires jq" >&2
  exit 1
}
command -v timeout >/dev/null || {
  echo "share lifecycle E2E requires GNU timeout" >&2
  exit 1
}
test -x "$se_bin" || {
  echo "se test binary is missing: $se_bin" >&2
  exit 1
}
test -x "$server_bin" || {
  echo "share-server test binary is missing: $server_bin" >&2
  exit 1
}

root="$(mktemp -d "${TMPDIR:-/tmp}/se-share-lifecycle.XXXXXX")"
client_a="$root/a"
client_b="$root/b"
client_c="$root/c"
client_d="$root/d"
server_log="$root/share-server.log"
server_pid=""

cleanup() {
  local status=$?
  stop_daemon "$client_a" || true
  stop_daemon "$client_b" || true
  stop_daemon "$client_c" || true
  stop_daemon "$client_d" || true
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ $status -ne 0 ]]; then
    echo "Share lifecycle E2E failed; diagnostics: $root" >&2
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
    SE_SHARE_RELAY_URL="http://127.0.0.1:$((signal_port + 1))" \
    "$se_bin" "$@"
}

# Use only for a background invocation. `exec` replaces Bash's asynchronous
# function subshell so `$!` is the actual `se` process, not a killable wrapper
# which could orphan the CLI under test.
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
    SE_SHARE_RELAY_URL="http://127.0.0.1:$((signal_port + 1))" \
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
}

wait_request_inbox() {
  local client="$1"
  local deadline=$((SECONDS + 40))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request 2>/dev/null)" \
      && grep -Fqx $'pending_requests\t1' <<<"$value" \
      && grep -F $'delivery=received\tdecision=pending\tauthorization=inactive' \
        >/dev/null <<<"$value" \
      && grep -Fqx $'next\tse share request accept' <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "no received pending request appeared in the bare inbox for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_request_inbox_json() {
  local client="$1"
  local deadline=$((SECONDS + 45))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request --json 2>/dev/null)" \
      && jq -e '.count == 1 and (.requests | length) == 1 and .requests[0].direction == "incoming" and .requests[0].delivery.state == "received" and .requests[0].decision.state == "pending" and .requests[0].authorization.active == false' \
        >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "no single received pending JSON request appeared for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_empty_request_inbox() {
  local client="$1"
  local deadline=$((SECONDS + 45))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request --json 2>/dev/null)" \
      && jq -e '.count == 0 and (.requests | length) == 0' >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "pending inbox did not become empty for $client" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

wait_request_state() {
  local client="$1"
  local request_id="$2"
  local jq_filter="$3"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request show "$request_id" --json 2>/dev/null)" \
      && jq -e "$jq_filter" >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "request $request_id never satisfied: $jq_filter" >&2
  return 1
}

wait_exec_state() {
  local client="$1"
  local direction="$2"
  local state="$3"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share exec --json 2>/dev/null)" \
      && jq -e --arg direction "$direction" --arg state "$state" \
        '[.[] | select(.direction == $direction and .job.state == $state)] | length == 1' >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.1
  done
  echo "no $direction Exec reached $state for $client" >&2
  return 1
}

wait_exec_history() {
  local client="$1"
  local direction="$2"
  local state="$3"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share exec history --json 2>/dev/null)" \
      && jq -e --arg direction "$direction" --arg state "$state" \
        'any(.[]; .direction == $direction and .job.state == $state)' >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.1
  done
  echo "no $direction Exec history reached $state for $client" >&2
  return 1
}

wait_exec_history_id() {
  local client="$1"
  local direction="$2"
  local state="$3"
  local exec_id="$4"
  local deadline=$((SECONDS + 45))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share exec history --json 2>/dev/null)" \
      && jq -e --arg direction "$direction" --arg state "$state" --arg exec_id "$exec_id" \
        'any(.[]; .direction == $direction and .job.state == $state and .job.exec_id == $exec_id)' \
        >/dev/null <<<"$value"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.1
  done
  echo "Exec $exec_id did not reach $direction/$state history for $client" >&2
  return 1
}

wait_exec_unit_stopped() {
  local exec_id="$1"
  local unit="smart-explorer-exec-$exec_id.service"
  local deadline=$((SECONDS + 20))
  local active
  while [[ $SECONDS -lt $deadline ]]; do
    if [[ $(id -u) -eq 0 ]]; then
      active="$(systemctl is-active "$unit" 2>/dev/null || true)"
    else
      active="$(systemctl --user is-active "$unit" 2>/dev/null || true)"
    fi
    if [[ "$active" != active && "$active" != activating && "$active" != deactivating ]]; then
      return 0
    fi
    sleep 0.1
  done
  echo "Exec unit $unit remained active after worker death" >&2
  return 1
}

wait_cgroup_empty_or_gone() {
  local cgroup="$1"
  local deadline=$((SECONDS + 20))
  while [[ $SECONDS -lt $deadline ]]; do
    if [[ ! -e "$cgroup" ]] || grep -Fqx 'populated 0' "$cgroup/cgroup.events" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  echo "Exec cgroup remained populated: $cgroup" >&2
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

prepare_client "$client_a"
prepare_client "$client_b"
prepare_client "$client_c"
prepare_client "$client_d"

signal_port=$((31000 + ($$ % 12000)))
"$server_bin" "127.0.0.1:$signal_port" >"$server_log" 2>&1 &
server_pid=$!
sleep 0.5
kill -0 "$server_pid"

identity_b="$(run_client "$client_b" share identity --json)"
direct_code_b="$(jq -er '.direct_code' <<<"$identity_b")"

# Queue while the target is offline. The requester must report relay state,
# never peer receipt, until B durably receives the signed envelope.
run_client "$client_a" share configure --server "127.0.0.1:$signal_port" >/dev/null
add_output="$(run_client "$client_a" connections add-peer --code "$direct_code_b" --name Target --json)"
peer_selector="$(jq -er '.selector' <<<"$add_output")"
direct_endpoint="$(jq -er '.endpoint' <<<"$add_output")"
request_id="$(jq -er '.request_id' <<<"$add_output")"
[[ -n "$peer_selector" && "$direct_endpoint" == share://direct/* && -n "$request_id" ]]
jq -e '
  .request.request_id == .request_id and
  .request.direction == "outgoing" and
  (.request.delivery.state | type == "string") and
  (.request.relay | has("outcome")) and
  .request.decision.state == "pending" and
  .request.authorization.active == false and
  (.worker_refresh.state | type == "string") and
  (.worker_refresh | has("error"))
' >/dev/null <<<"$add_output"

wait_request_state "$client_a" "$request_id" '.relay.outcome == "target_offline" and .peer_receipt.request.state == "unconfirmed"' >/dev/null
shown="$(run_client "$client_a" share request show --json)"
jq -e --arg id "$request_id" '.request_id == $id' >/dev/null <<<"$shown"

run_client "$client_b" share configure --server "127.0.0.1:$signal_port" >/dev/null
retry="$(run_client "$client_a" share request retry --json)"
jq -e --arg id "$request_id" '.request.request_id == $id' >/dev/null <<<"$retry"

# The target discovers everything it needs from one bare inbox command. No
# request ID, device ID, or fingerprint is supplied out of band.
inbox="$(wait_request_inbox "$client_b")"
mapfile -t inbox_request_lines < <(awk -F '\t' '$1 == "pending_request"' <<<"$inbox")
[[ ${#inbox_request_lines[@]} -eq 1 ]]
IFS=$'\t' read -r inbox_kind inbox_request_id inbox_name inbox_device_id inbox_fingerprint \
  inbox_delivery inbox_decision inbox_authorization <<<"${inbox_request_lines[0]}"
[[ "$inbox_kind" == pending_request && -n "$inbox_request_id" ]]
[[ "$inbox_name" == device_name=?* && "$inbox_device_id" == device_id=?* ]]
[[ "$inbox_fingerprint" == fingerprint=?* && "$inbox_delivery" == delivery=received ]]
[[ "$inbox_decision" == decision=pending && "$inbox_authorization" == authorization=inactive ]]
[[ "$inbox_request_id" == "$request_id" ]]
wait_request_state "$client_a" "$request_id" '.delivery.state == "received" and .peer_receipt.request.state == "received"' >/dev/null

# Pending inbox survives a full target daemon restart.
stop_daemon "$client_b"
run_client "$client_b" share status --json >/dev/null
wait_request_state "$client_b" "$inbox_request_id" '.direction == "incoming" and .decision.state == "pending"' >/dev/null

# The requester is offline while B accepts. B retains and retries the signed
# decision; A applies it after restart and returns a signed decision receipt.
stop_daemon "$client_a"
accepted="$(run_client "$client_b" share request accept --json)"
jq -e '.request.decision.state == "accepted" and .request.authorization.active == true' >/dev/null <<<"$accepted"
run_client "$client_a" share status --json >/dev/null
wait_request_state "$client_a" "$request_id" '.decision.state == "accepted" and .authorization.active == true' >/dev/null
wait_request_state "$client_b" "$inbox_request_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null

grants="$(run_client "$client_b" share grants --json)"
jq -e '.grants | any(.authorization.active == true)' >/dev/null <<<"$grants"

# Deleting the signed authorization basis while its grant is active must be
# refused. The user first revokes, waits for the peer receipt, then may delete
# the now-inactive history without supplying a hidden selector.
set +e
run_client "$client_b" share request delete --json \
  >"$root/delete-active.stdout" 2>"$root/delete-active.stderr"
delete_active_status=$?
set -e
[[ $delete_active_status -ne 0 ]]
run_client "$client_b" share request show --json >/dev/null

# An accepted authorization is operational, not merely a UI flag.
run_client "$client_a" ls "$direct_endpoint" >/dev/null

# Exec remains a separate exact-device grant. The target discovers the only
# accepted device from its own CLI; no request/device/fingerprint fixture is
# passed to the grant or execution commands.
exec_grants="$(run_client "$client_b" share grants exec --json)"
jq -e 'length == 1 and .[0].enabled == false' >/dev/null <<<"$exec_grants"
enabled="$(run_client "$client_b" share grants exec enable --yes --json)"
jq -e '.persisted == true and .applied == true' >/dev/null <<<"$enabled"

# Learn the target home from the remote shell itself; every later target path
# is derived from this earlier CLI output, never from the test harness layout.
remote_home="$(run_client "$client_a" exec -- sh -c 'printf %s "$HOME"')"
[[ "$remote_home" == /* ]]

# A healthy silent command outlives the application heartbeat deadline.
silent_started=$SECONDS
silent_output="$(run_client "$client_a" exec -- sh -c 'sleep 25; printf LINUX_HEARTBEAT_OK')"
[[ "$silent_output" == LINUX_HEARTBEAT_OK ]]
[[ $((SECONDS - silent_started)) -ge 24 ]]

# Literal argv, arbitrary binary stdin/stdout, stderr, and a non-zero remote
# exit code cross the actual daemon IPC and Iroh Exec protocol.
exec_input="$root/exec-input.bin"
exec_output="$root/exec-output.bin"
exec_error="$root/exec-error.txt"
printf 'binary\000stdin\377\200\n' >"$exec_input"
set +e
run_client "$client_a" exec -- sh -c 'cat; printf "remote-stderr\n" >&2; exit 7' \
  <"$exec_input" >"$exec_output" 2>"$exec_error"
exec_status=$?
set -e
[[ $exec_status -eq 7 ]]
cmp "$exec_input" "$exec_output"
grep -Fqx 'remote-stderr' "$exec_error"
wait_exec_history "$client_a" outgoing exited >/dev/null
wait_exec_history "$client_b" incoming exited >/dev/null

# An explicit output cap truncates bytes without losing the terminal result.
set +e
run_client "$client_a" exec --max-output 5 -- sh -c 'printf 1234567890' \
  </dev/null >"$root/limited.stdout" 2>"$root/limited.stderr"
limited_status=$?
set -e
[[ $limited_status -eq 0 ]]
[[ "$(cat "$root/limited.stdout")" == 12345 ]]
grep -F 'remote output was truncated' "$root/limited.stderr" >/dev/null

# Timeout must kill a stubborn descendant before it can leave a delayed marker.
timeout_marker="$remote_home/timed-out-exec-must-not-run"
set +e
run_client "$client_a" exec --timeout 1 -- sh -c \
  '(sleep 3; touch "$1") & wait' sh "$timeout_marker" \
  </dev/null >"$root/timeout.stdout" 2>"$root/timeout.stderr"
timeout_status=$?
set -e
[[ $timeout_status -eq 124 ]]
wait_exec_history "$client_a" outgoing timed_out >/dev/null
wait_exec_history "$client_b" incoming timed_out >/dev/null
sleep 3.5
[[ ! -e "$timeout_marker" ]]
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$timeout_marker" >/dev/null

# The target can find and cancel the sole incoming active command without an
# externally supplied Exec ID. Both endpoints must converge on Cancelled.
cancel_stdout="$root/cancel.stdout"
cancel_stderr="$root/cancel.stderr"
cancel_marker="$remote_home/cancelled-exec-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 5; touch "$1") & wait' sh "$cancel_marker" \
  </dev/null >"$cancel_stdout" 2>"$cancel_stderr" &
cancel_pid=$!
wait_exec_state "$client_a" outgoing running >/dev/null
wait_exec_state "$client_b" incoming running >/dev/null
cancelled="$(run_client "$client_b" share exec cancel --json)"
jq -e '.cancel_requested == true and (.exec_id | length == 32)' >/dev/null <<<"$cancelled"
wait_child "$cancel_pid" 30
[[ $child_status -eq 130 ]]
wait_exec_history "$client_a" outgoing cancelled >/dev/null
wait_exec_history "$client_b" incoming cancelled >/dev/null
sleep 5.5
[[ ! -e "$cancel_marker" ]]
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$cancel_marker" >/dev/null


# Killing only the foreground CLI closes local IPC; the worker must cancel the
# exact remote cgroup, and its delayed descendant must never reach the marker.
disconnect_marker="$remote_home/disconnected-cli-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 3; touch "$1") & wait' sh "$disconnect_marker" \
  </dev/null >"$root/disconnect.stdout" 2>"$root/disconnect.stderr" &
disconnect_pid=$!
disconnect_jobs="$(wait_exec_state "$client_b" incoming running)"
disconnect_id="$(jq -er '[.[] | select(.direction == "incoming" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous incoming Exec") end' <<<"$disconnect_jobs")"
kill -KILL "$disconnect_pid"
wait "$disconnect_pid" 2>/dev/null || true
wait_exec_history_id "$client_b" incoming cancelled "$disconnect_id" >/dev/null
sleep 3.5
[[ ! -e "$disconnect_marker" ]]
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$disconnect_marker" >/dev/null

# A hard target-worker crash must rely on kernel/systemd containment, not on a
# cooperative remote Cancel. The transient unit and owner-only socket must go
# away before the delayed child could write its marker.
crash_marker="$remote_home/crashed-worker-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 5; touch "$1") & wait' sh "$crash_marker" \
  </dev/null >"$root/crash.stdout" 2>"$root/crash.stderr" &
crash_cli_pid=$!
crash_outgoing_jobs="$(wait_exec_state "$client_a" outgoing running)"
crash_incoming_jobs="$(wait_exec_state "$client_b" incoming running)"
crash_outgoing_id="$(jq -er '[.[] | select(.direction == "outgoing" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous outgoing Exec") end' <<<"$crash_outgoing_jobs")"
crash_incoming_id="$(jq -er '[.[] | select(.direction == "incoming" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous incoming Exec") end' <<<"$crash_incoming_jobs")"
[[ "$crash_outgoing_id" == "$crash_incoming_id" ]]
crash_unit="smart-explorer-exec-$crash_incoming_id.service"
if [[ $(id -u) -eq 0 ]]; then
  crash_control_group="$(systemctl show "$crash_unit" -p ControlGroup --value)"
else
  crash_control_group="$(systemctl --user show "$crash_unit" -p ControlGroup --value)"
fi
[[ "$crash_control_group" == /* ]]
crash_cgroup="/sys/fs/cgroup${crash_control_group}"
[[ -f "$crash_cgroup/cgroup.events" ]]
mapfile -t target_daemons < <(daemon_pids "$client_b")
[[ ${#target_daemons[@]} -eq 1 ]]
kill -KILL "${target_daemons[0]}"
wait_child "$crash_cli_pid" 30
[[ $child_status -eq 125 ]]
wait_exec_history_id "$client_a" outgoing disconnected "$crash_outgoing_id" >/dev/null
wait_exec_unit_stopped "$crash_incoming_id"
wait_cgroup_empty_or_gone "$crash_cgroup"
uid="$(id -u)"
[[ ! -e "/run/user/$uid/smart-explorer-exec/$crash_incoming_id.sock" ]]
[[ ! -e "/tmp/smart-explorer-runtime-$uid/smart-explorer-exec/$crash_incoming_id.sock" ]]
sleep 5.5
[[ ! -e "$crash_marker" ]]
run_client "$client_b" share status --json >/dev/null
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$crash_marker" >/dev/null

# A hard requester-worker crash closes local IPC immediately, but the target
# must independently notice the missing authenticated Pings and kill the whole
# remote process tree. The command itself has no runtime limit.
requester_crash_marker="$remote_home/crashed-requester-worker-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 30; touch "$1") & wait' sh "$requester_crash_marker" \
  </dev/null >"$root/requester-crash.stdout" 2>"$root/requester-crash.stderr" &
requester_crash_cli_pid=$!
requester_outgoing_jobs="$(wait_exec_state "$client_a" outgoing running)"
requester_incoming_jobs="$(wait_exec_state "$client_b" incoming running)"
requester_outgoing_id="$(jq -er '[.[] | select(.direction == "outgoing" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous outgoing Exec") end' <<<"$requester_outgoing_jobs")"
requester_incoming_id="$(jq -er '[.[] | select(.direction == "incoming" and .job.state == "running")] | if length == 1 then .[0].job.exec_id else error("ambiguous incoming Exec") end' <<<"$requester_incoming_jobs")"
[[ "$requester_outgoing_id" == "$requester_incoming_id" ]]
mapfile -t requester_daemons < <(daemon_pids "$client_a")
[[ ${#requester_daemons[@]} -eq 1 ]]
kill -KILL "${requester_daemons[0]}"
wait_child "$requester_crash_cli_pid" 15
[[ $child_status -eq 125 ]]
wait_exec_history_id "$client_b" incoming disconnected "$requester_incoming_id" >/dev/null
# Wait beyond the descendant's own 30-second delay. Checking earlier would
# pass even if containment had leaked the process tree.
sleep 31
[[ ! -e "$requester_crash_marker" ]]
run_client "$client_a" share status --json >/dev/null
run_client "$client_a" exec -- sh -c 'test ! -e "$1"' sh "$requester_crash_marker" >/dev/null

# Disabling the exact Exec grant terminates an already running process and
# prevents a later command from starting any payload.
revoke_marker="$remote_home/revoked-exec-must-not-run"
revoke_child_marker="$remote_home/revoked-active-child-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 5; touch "$1") & wait' sh "$revoke_child_marker" \
  </dev/null >"$root/revoke.stdout" 2>"$root/revoke.stderr" &
revoke_pid=$!
wait_exec_state "$client_b" incoming running >/dev/null
disabled="$(run_client "$client_b" share grants exec disable --json)"
jq -e '.persisted == true and .applied == true' >/dev/null <<<"$disabled"
wait_child "$revoke_pid" 30
[[ $child_status -eq 125 ]]
wait_exec_history "$client_a" outgoing revoked >/dev/null
wait_exec_history "$client_b" incoming revoked >/dev/null
sleep 5.5
[[ ! -e "$revoke_child_marker" ]]
set +e
run_client "$client_a" exec -- sh -c "touch '$revoke_marker'" \
  >"$root/denied.stdout" 2>"$root/denied.stderr"
denied_status=$?
set -e
[[ $denied_status -eq 125 ]]
[[ ! -e "$revoke_marker" ]]

revoked="$(run_client "$client_b" share grants revoke --json)"
jq -e '.request.decision.state == "revoked" and .request.authorization.active == false' >/dev/null <<<"$revoked"
wait_request_state "$client_a" "$request_id" '.decision.state == "revoked" and .authorization.active == false' >/dev/null
wait_request_state "$client_b" "$inbox_request_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null
deleted="$(run_client "$client_b" share request delete --json)"
jq -e --arg id "$inbox_request_id" '.action == "deleted" and .request_id == $id and .persisted == true' >/dev/null <<<"$deleted"

if run_client "$client_a" ls "$direct_endpoint" >/dev/null 2>&1; then
  echo "revoked direct authorization still allowed file access" >&2
  exit 1
fi

# Peer removal auto-selects the only peer without requiring a hidden selector.
connections="$(run_client "$client_a" connections list --json)"
jq -e --arg selector "$peer_selector" --arg endpoint "$direct_endpoint" \
  'length == 1 and .[0].kind == "direct" and .[0].selector == $selector and .[0].endpoint == $endpoint' \
  >/dev/null <<<"$connections"
run_client "$client_a" connections remove-peer >/dev/null
after_remove="$(run_client "$client_a" connections list --json)"
jq -e 'all(.[]; .kind != "direct")' >/dev/null <<<"$after_remove"

# A fresh third device avoids inheriting B's intentionally revoked grant and
# exercises bare rejection plus rejected-history deletion from a true pending
# state.
identity_c="$(run_client "$client_c" share identity --json)"
direct_code_c="$(jq -er '.direct_code' <<<"$identity_c")"
run_client "$client_c" share configure --server "127.0.0.1:$signal_port" >/dev/null
reject_add="$(run_client "$client_a" connections add-peer --code "$direct_code_c" --name RejectTarget --json)"
reject_request_id="$(jq -er '.request_id' <<<"$reject_add")"
reject_inbox="$(wait_request_inbox_json "$client_c")"
reject_inbox_id="$(jq -er '.requests[0].request_id' <<<"$reject_inbox")"
[[ "$reject_inbox_id" == "$reject_request_id" ]]
wait_request_state "$client_a" "$reject_request_id" '.delivery.state == "received" and .peer_receipt.request.state == "received"' >/dev/null
rejected="$(run_client "$client_c" share request reject --json)"
jq -e --arg id "$reject_inbox_id" '.request.request_id == $id and .request.decision.state == "rejected" and .request.authorization.active == false' >/dev/null <<<"$rejected"
wait_request_state "$client_a" "$reject_request_id" '.decision.state == "rejected" and .authorization.active == false' >/dev/null
wait_request_state "$client_c" "$reject_inbox_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null
run_client "$client_c" share request delete --json >/dev/null
run_client "$client_a" connections remove-peer >/dev/null

# A fourth fresh device supplies a genuinely pending request. Two full worker
# restarts prove the local dismissal tombstone remains durable and hidden.
identity_d="$(run_client "$client_d" share identity --json)"
direct_code_d="$(jq -er '.direct_code' <<<"$identity_d")"
run_client "$client_d" share configure --server "127.0.0.1:$signal_port" >/dev/null
pending_add="$(run_client "$client_a" connections add-peer --code "$direct_code_d" --name TombstoneTarget --json)"
pending_request_id="$(jq -er '.request_id' <<<"$pending_add")"
pending_inbox="$(wait_request_inbox_json "$client_d")"
pending_inbox_id="$(jq -er '.requests[0].request_id' <<<"$pending_inbox")"
[[ "$pending_inbox_id" == "$pending_request_id" ]]
pending_deleted="$(run_client "$client_d" share request delete --json)"
jq -e --arg id "$pending_inbox_id" '.action == "deleted" and .request_id == $id and .persisted == true' >/dev/null <<<"$pending_deleted"
stop_daemon "$client_d"
run_client "$client_d" share status --json >/dev/null
wait_empty_request_inbox "$client_d" >/dev/null
stop_daemon "$client_d"
run_client "$client_d" share status --json >/dev/null
wait_empty_request_inbox "$client_d" >/dev/null
run_client "$client_a" connections remove-peer >/dev/null

echo "tracked Share lifecycle E2E passed: $request_id"
