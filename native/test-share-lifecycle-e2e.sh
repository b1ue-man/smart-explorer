#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
se_bin="${SMART_EXPLORER_SE_BINARY:-$repo_root/native/target/debug/se}"
server_bin="${SMART_EXPLORER_SHARE_SERVER_BINARY:-$repo_root/share-server/target/debug/se-share-server}"

command -v jq >/dev/null || {
  echo "share lifecycle E2E requires jq" >&2
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
server_log="$root/share-server.log"
server_pid=""

cleanup() {
  local status=$?
  stop_daemon "$client_a" || true
  stop_daemon "$client_b" || true
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
  env \
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

wait_request() {
  local client="$1"
  local request_id="$2"
  local deadline=$((SECONDS + 40))
  local value
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share request show "$request_id" --json 2>/dev/null)"; then
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.25
  done
  echo "request $request_id did not become visible for $client" >&2
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
        'any(.[]; .direction == $direction and .job.state == $state)' >/dev/null <<<"$value"; then
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
add_output="$(run_client "$client_a" connections add-peer --code "$direct_code_b" --name Target)"
contact_id="$(sed -nE 's/.*peer contact ([^;]+);.*/\1/p' <<<"$add_output")"
request_id="$(sed -nE 's/.*request_id=([^;]+);.*/\1/p' <<<"$add_output")"
[[ -n "$contact_id" && -n "$request_id" ]]

wait_request_state "$client_a" "$request_id" '.relay.outcome == "target_offline" and .peer_receipt.request.state == "unconfirmed"' >/dev/null
shown="$(run_client "$client_a" share request show --json)"
jq -e --arg id "$request_id" '.request_id == $id' >/dev/null <<<"$shown"

run_client "$client_b" share configure --server "127.0.0.1:$signal_port" >/dev/null
retry="$(run_client "$client_a" share request retry --json)"
jq -e --arg id "$request_id" '.request.request_id == $id' >/dev/null <<<"$retry"
incoming="$(wait_request_state "$client_b" "$request_id" '.direction == "incoming" and .delivery.state == "received"')"
jq -e '.peer.fingerprint and .decision.state == "pending"' >/dev/null <<<"$incoming"

# The target discovers everything it needs from one bare inbox command. No
# request ID, device ID, or fingerprint is supplied out of band.
inbox="$(run_client "$client_b" share request)"
inbox_request_id="$(awk -F '\t' '$1 == "pending_request" { print $2 }' <<<"$inbox")"
[[ "$inbox_request_id" == "$request_id" ]]
grep -Fqx $'pending_requests\t1' <<<"$inbox"
grep -F $'delivery=received\tdecision=pending\tauthorization=inactive' >/dev/null <<<"$inbox"
grep -Fqx $'next\tse share request accept' <<<"$inbox"

# Pending inbox survives a full target daemon restart.
stop_daemon "$client_b"
run_client "$client_b" share status --json >/dev/null
wait_request_state "$client_b" "$request_id" '.direction == "incoming" and .decision.state == "pending"' >/dev/null

# The requester is offline while B accepts. B retains and retries the signed
# decision; A applies it after restart and returns a signed decision receipt.
stop_daemon "$client_a"
accepted="$(run_client "$client_b" share request accept --json)"
jq -e '.request.decision.state == "accepted" and .request.authorization.active == true' >/dev/null <<<"$accepted"
run_client "$client_a" share status --json >/dev/null
wait_request_state "$client_a" "$request_id" '.decision.state == "accepted" and .authorization.active == true' >/dev/null
wait_request_state "$client_b" "$request_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null

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
run_client "$client_a" ls "share://direct/$contact_id/" >/dev/null

# Exec remains a separate exact-device grant. The target discovers the only
# accepted device from its own CLI; no request/device/fingerprint fixture is
# passed to the grant or execution commands.
exec_grants="$(run_client "$client_b" share grants exec --json)"
jq -e 'length == 1 and .[0].enabled == false' >/dev/null <<<"$exec_grants"
enabled="$(run_client "$client_b" share grants exec enable --yes --json)"
jq -e '.persisted == true and .applied == true' >/dev/null <<<"$enabled"

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
timeout_marker="$client_b/home/timed-out-exec-must-not-run"
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

# The target can find and cancel the sole incoming active command without an
# externally supplied Exec ID. Both endpoints must converge on Cancelled.
cancel_stdout="$root/cancel.stdout"
cancel_stderr="$root/cancel.stderr"
cancel_marker="$client_b/home/cancelled-exec-must-not-run"
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


# Killing only the foreground CLI closes local IPC; the worker must cancel the
# exact remote cgroup, and its delayed descendant must never reach the marker.
disconnect_marker="$client_b/home/disconnected-cli-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 3; touch "$1") & wait' sh "$disconnect_marker" \
  </dev/null >"$root/disconnect.stdout" 2>"$root/disconnect.stderr" &
disconnect_pid=$!
disconnect_jobs="$(wait_exec_state "$client_b" incoming running)"
disconnect_id="$(jq -er '.[] | select(.direction == "incoming" and .job.state == "running") | .job.exec_id' <<<"$disconnect_jobs" | tail -1)"
kill -KILL "$disconnect_pid"
wait "$disconnect_pid" 2>/dev/null || true
wait_exec_history_id "$client_b" incoming cancelled "$disconnect_id" >/dev/null
sleep 3.5
[[ ! -e "$disconnect_marker" ]]

# A hard target-worker crash must rely on kernel/systemd containment, not on a
# cooperative remote Cancel. The transient unit and owner-only socket must go
# away before the delayed child could write its marker.
crash_marker="$client_b/home/crashed-worker-must-not-run"
run_client_background "$client_a" exec -- sh -c '(sleep 5; touch "$1") & wait' sh "$crash_marker" \
  </dev/null >"$root/crash.stdout" 2>"$root/crash.stderr" &
crash_cli_pid=$!
crash_jobs="$(wait_exec_state "$client_b" incoming running)"
crash_id="$(jq -er '.[] | select(.direction == "incoming" and .job.state == "running") | .job.exec_id' <<<"$crash_jobs" | tail -1)"
crash_unit="smart-explorer-exec-$crash_id.service"
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
wait_exec_history_id "$client_a" outgoing disconnected "$crash_id" >/dev/null
wait_exec_unit_stopped "$crash_id"
wait_cgroup_empty_or_gone "$crash_cgroup"
uid="$(id -u)"
[[ ! -e "/run/user/$uid/smart-explorer-exec/$crash_id.sock" ]]
[[ ! -e "/tmp/smart-explorer-runtime-$uid/smart-explorer-exec/$crash_id.sock" ]]
sleep 5.5
[[ ! -e "$crash_marker" ]]
run_client "$client_b" share status --json >/dev/null

# Disabling the exact Exec grant terminates an already running process and
# prevents a later command from starting any payload.
revoke_marker="$client_b/home/revoked-exec-must-not-run"
revoke_child_marker="$client_b/home/revoked-active-child-must-not-run"
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
wait_request_state "$client_b" "$request_id" '.decision_delivery.state == "received" and .peer_receipt.decision.state == "received"' >/dev/null
deleted="$(run_client "$client_b" share request delete --json)"
jq -e --arg id "$request_id" '.action == "deleted" and .request_id == $id and .persisted == true' >/dev/null <<<"$deleted"

if run_client "$client_a" ls "share://direct/$contact_id/" >/dev/null 2>&1; then
  echo "revoked direct authorization still allowed file access" >&2
  exit 1
fi

# Peer removal consumes the canonical selector emitted by the CLI itself.
connections="$(run_client "$client_a" connections list --json)"
peer_selector="$(jq -er '.[] | select(.kind == "direct") | .selector' <<<"$connections")"
run_client "$client_a" connections remove-peer "$peer_selector" >/dev/null
after_remove="$(run_client "$client_a" connections list --json)"
jq -e 'all(.[]; .kind != "direct")' >/dev/null <<<"$after_remove"

echo "tracked Share lifecycle E2E passed: $request_id"
