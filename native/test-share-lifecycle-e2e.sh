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
  stop_daemon "$client_a" || true
  stop_daemon "$client_b" || true
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$root"
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

run_client "$client_b" share configure --server "127.0.0.1:$signal_port" >/dev/null
retry="$(run_client "$client_a" share request retry "$request_id" --json)"
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

# An accepted authorization is operational, not merely a UI flag.
run_client "$client_a" ls "share://direct/$contact_id/" >/dev/null

revoked="$(run_client "$client_b" share grants revoke --json)"
jq -e '.request.decision.state == "revoked" and .request.authorization.active == false' >/dev/null <<<"$revoked"
wait_request_state "$client_a" "$request_id" '.decision.state == "revoked" and .authorization.active == false' >/dev/null

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
