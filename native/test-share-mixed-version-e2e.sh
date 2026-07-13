#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
se_bin="${SMART_EXPLORER_SE_BINARY:-$repo_root/native/target/debug/se}"
server_bin="${SMART_EXPLORER_SHARE_SERVER_BINARY:-$repo_root/share-server/target/debug/se-share-server}"
legacy_bin="${SMART_EXPLORER_LEGACY_SE_BINARY:-}"

stage="preflight"
failure_line=""
failure_command=""
root=""
server_pid=""

remember_failure() {
  failure_line="$1"
  failure_command="$2"
}
trap 'remember_failure "$LINENO" "$BASH_COMMAND"' ERR

fail() {
  echo "mixed-version Share E2E: $*" >&2
  return 1
}

command -v jq >/dev/null || fail "jq is required"
command -v timeout >/dev/null || fail "GNU timeout is required"
[[ "$(uname -s)" == Linux ]] || fail "this compatibility harness requires Linux"
[[ -n "$legacy_bin" ]] || fail \
  "SMART_EXPLORER_LEGACY_SE_BINARY must point to the published Linux se v0.5.126 binary"

for binary in "$se_bin" "$server_bin" "$legacy_bin"; do
  [[ -x "$binary" ]] || fail "executable is missing: $binary"
done

legacy_version="$("$legacy_bin" --version)"
[[ "$legacy_version" =~ (^|[[:space:]])0\.5\.126($|[[:space:]]) ]] || fail \
  "legacy binary must be v0.5.126, got: $legacy_version"
current_request_help="$("$se_bin" share request --help)"
grep -Fq 'list' <<<"$current_request_help" || fail \
  "SMART_EXPLORER_SE_BINARY does not expose the current request lifecycle CLI"

root="$(mktemp -d "${TMPDIR:-/tmp}/se-share-mixed-version.XXXXXX")"
new_requester="$root/new-requester"
old_target="$root/old-target"
old_requester="$root/old-requester"
new_target="$root/new-target"
old_reject_requester="$root/old-reject-requester"
new_reject_target="$root/new-reject-target"

prepare_profile() {
  local profile="$1"
  mkdir -p \
    "$profile/home" \
    "$profile/data" \
    "$profile/config" \
    "$profile/cache" \
    "$profile/runtime" \
    "$profile/appdata" \
    "$profile/localappdata" \
    "$profile/tmp"
  chmod 700 \
    "$profile/home" \
    "$profile/data" \
    "$profile/config" \
    "$profile/cache" \
    "$profile/runtime" \
    "$profile/appdata" \
    "$profile/localappdata" \
    "$profile/tmp"
}

run_se() {
  local binary="$1"
  local profile="$2"
  shift 2
  timeout --foreground --signal=TERM --kill-after=5s 60s env \
    HOME="$profile/home" \
    USERPROFILE="$profile/home" \
    XDG_DATA_HOME="$profile/data" \
    XDG_CONFIG_HOME="$profile/config" \
    XDG_CACHE_HOME="$profile/cache" \
    XDG_RUNTIME_DIR="$profile/runtime" \
    APPDATA="$profile/appdata" \
    LOCALAPPDATA="$profile/localappdata" \
    TMPDIR="$profile/tmp" \
    NO_COLOR=1 \
    SE_SHARE_RELAY_URL="http://127.0.0.1:$relay_port" \
    "$binary" "$@"
}

complete_se_bash() {
  local profile="$1"
  local index="$2"
  shift 2
  timeout --foreground --signal=TERM --kill-after=5s 60s env \
    HOME="$profile/home" \
    USERPROFILE="$profile/home" \
    XDG_DATA_HOME="$profile/data" \
    XDG_CONFIG_HOME="$profile/config" \
    XDG_CACHE_HOME="$profile/cache" \
    XDG_RUNTIME_DIR="$profile/runtime" \
    APPDATA="$profile/appdata" \
    LOCALAPPDATA="$profile/localappdata" \
    TMPDIR="$profile/tmp" \
    NO_COLOR=1 \
    SE_SHARE_RELAY_URL="http://127.0.0.1:$relay_port" \
    COMPLETE=bash \
    _CLAP_IFS=$'\n' \
    _CLAP_COMPLETE_INDEX="$index" \
    _CLAP_COMPLETE_COMP_TYPE=9 \
    _CLAP_COMPLETE_SPACE=true \
    "$se_bin" -- "$@"
}

daemon_pids() {
  local profile="$1"
  local expected="XDG_DATA_HOME=$profile/data"
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
  local profile="$1"
  local required="${2:-optional}"
  local -a pids=()
  local pid deadline
  mapfile -t pids < <(daemon_pids "$profile")
  if [[ "$required" == required && ${#pids[@]} -eq 0 ]]; then
    fail "no daemon was found for restart profile $profile"
  fi
  for pid in "${pids[@]}"; do
    kill -TERM "$pid" 2>/dev/null || true
  done
  deadline=$((SECONDS + 10))
  while [[ $SECONDS -lt $deadline && -n "$(daemon_pids "$profile")" ]]; do
    sleep 0.05
  done
  mapfile -t pids < <(daemon_pids "$profile")
  for pid in "${pids[@]}"; do
    kill -KILL "$pid" 2>/dev/null || true
  done
  deadline=$((SECONDS + 5))
  while [[ $SECONDS -lt $deadline && -n "$(daemon_pids "$profile")" ]]; do
    sleep 0.05
  done
  [[ -z "$(daemon_pids "$profile")" ]] || fail "daemon did not stop for $profile"
}

dump_diagnostics() {
  echo "stage: $stage" >&2
  [[ -z "$failure_line" ]] || echo "failed at line $failure_line: $failure_command" >&2
  if [[ -n "$server_pid" ]]; then
    ps -o pid,ppid,stat,etime,cmd -p "$server_pid" >&2 || true
  fi
  if [[ -n "$root" && -d "$root" ]]; then
    local file
    while IFS= read -r file; do
      echo "--- ${file#"$root"/}" >&2
      tail -n 80 "$file" >&2 || true
    done < <(find "$root" -type f \( -name '*.log' -o -name '*.last' -o -name '*.json' -o -name '*.txt' \) -print | sort)
    echo "preserved diagnostics: $root" >&2
  fi
}

cleanup() {
  local status=$?
  trap - EXIT ERR
  set +e
  if [[ -n "$root" ]]; then
    stop_daemon "$new_requester" optional
    stop_daemon "$old_target" optional
    stop_daemon "$old_requester" optional
    stop_daemon "$new_target" optional
    stop_daemon "$old_reject_requester" optional
    stop_daemon "$new_reject_target" optional
  fi
  if [[ -n "$server_pid" ]]; then
    kill -TERM "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ $status -ne 0 ]]; then
    dump_diagnostics
  elif [[ "${SMART_EXPLORER_KEEP_E2E_ROOT:-0}" != 1 ]]; then
    rm -rf "$root"
  else
    echo "mixed-version Share E2E root retained: $root"
  fi
  exit "$status"
}
trap cleanup EXIT

start_server() {
  local attempt candidate log
  for attempt in $(seq 0 15); do
    candidate=$((30000 + (( $$ + attempt * 97) % 15000)))
    log="$root/share-server-$candidate.log"
    "$server_bin" "127.0.0.1:$candidate" >"$log" 2>&1 &
    server_pid=$!
    sleep 0.5
    if kill -0 "$server_pid" 2>/dev/null; then
      signal_port="$candidate"
      relay_port=$((candidate + 1))
      return 0
    fi
    wait "$server_pid" 2>/dev/null || true
    server_pid=""
  done
  fail "share server did not start; inspect $root/share-server-*.log"
}

wait_connected() {
  local binary="$1"
  local profile="$2"
  local output_file="$3"
  local label="$4"
  local deadline=$((SECONDS + 45))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_se "$binary" "$profile" share status --json 2>/dev/null)"; then
      printf '%s\n' "$value" >"$output_file"
      if jq -e '.running == true and .connected == true' >/dev/null <<<"$value"; then
        return 0
      fi
    fi
    sleep 0.2
  done
  fail "$label did not connect; last status: $output_file"
}

wait_current_request() {
  local profile="$1"
  local request_id="$2"
  local filter="$3"
  local output_file="$4"
  local label="$5"
  local deadline=$((SECONDS + 60))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_se "$se_bin" "$profile" share request show "$request_id" --json 2>/dev/null)"; then
      printf '%s\n' "$value" >"$output_file"
      if jq -e --arg id "$request_id" ".request_id == \$id and ($filter)" >/dev/null <<<"$value"; then
        printf '%s\n' "$value"
        return 0
      fi
    fi
    sleep 0.2
  done
  fail "$label did not reach the expected lifecycle state; last request: $output_file"
}

wait_old_pending_status() {
  local profile="$1"
  local output_file="$2"
  local deadline=$((SECONDS + 60))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_se "$legacy_bin" "$profile" share status --json 2>/dev/null)"; then
      printf '%s\n' "$value" >"$output_file"
      if jq -e '
        .running == true and .connected == true and
        (.pending_requests | length) == 1 and
        (.pending_requests[0].device_id | type == "string" and length > 0) and
        (.pending_requests[0].fingerprint | type == "string" and length > 0)
      ' >/dev/null <<<"$value"; then
        printf '%s\n' "$value"
        return 0
      fi
    fi
    sleep 0.2
  done
  fail "legacy target did not expose exactly one pending request in 'se share status'; last status: $output_file"
}

wait_old_accepted() {
  local profile="$1"
  local output_file="$2"
  local deadline=$((SECONDS + 60))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_se "$legacy_bin" "$profile" share status --json 2>/dev/null)"; then
      printf '%s\n' "$value" >"$output_file"
      if jq -e '
        .running == true and .connected == true and
        (.contacts | length) == 1 and .contacts[0].access == "Freigegeben"
      ' >/dev/null <<<"$value"; then
        return 0
      fi
    fi
    sleep 0.2
  done
  fail "legacy requester did not observe accepted access; last status: $output_file"
}

wait_old_rejected() {
  local profile="$1"
  local output_file="$2"
  local deadline=$((SECONDS + 60))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_se "$legacy_bin" "$profile" share status --json 2>/dev/null)"; then
      printf '%s\n' "$value" >"$output_file"
      if jq -e '
        .running == true and .connected == true and
        (.contacts | length) == 1 and .contacts[0].access == "Ignoriert"
      ' >/dev/null <<<"$value"; then
        return 0
      fi
    fi
    sleep 0.2
  done
  fail "legacy requester did not observe rejected access; last status: $output_file"
}

wait_legacy_inbox() {
  local profile="$1"
  local output_file="$2"
  local deadline=$((SECONDS + 60))
  local value=""
  local legacy_count tracked_count
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_se "$se_bin" "$profile" share request 2>/dev/null)"; then
      printf '%s\n' "$value" >"$output_file"
      legacy_count="$(awk -F '\t' '$1 == "pending_legacy_request" { count++ } END { print count + 0 }' <<<"$value")"
      tracked_count="$(awk -F '\t' '$1 == "pending_request" { count++ } END { print count + 0 }' <<<"$value")"
      if grep -Fqx $'pending_requests\t1' <<<"$value" \
        && [[ "$legacy_count" == 1 && "$tracked_count" == 0 ]] \
        && grep -Fqx $'next\tse share request accept' <<<"$value"; then
        printf '%s\n' "$value"
        return 0
      fi
    fi
    sleep 0.2
  done
  fail "current target did not expose one durable legacy request plus the context-free accept command; last inbox: $output_file"
}

wait_current_grant() {
  local profile="$1"
  local device_id="$2"
  local selector="$3"
  local output_file="$4"
  local deadline=$((SECONDS + 60))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_se "$se_bin" "$profile" share grants --json 2>/dev/null)"; then
      printf '%s\n' "$value" >"$output_file"
      if jq -e --arg id "$device_id" --arg selector "$selector" '
        .count == 1 and (.grants | length) == 1 and
        .grants[0].device_id == $id and
        .grants[0].grant_state == "accepted" and
        .grants[0].authorization.active == true and
        .grants[0].tracked == false and
        (.grants[0].requests | length) == 0 and
        (.grants[0].legacy_requests | length) == 1 and
        .grants[0].legacy_requests[0].selector == $selector and
        .grants[0].legacy_requests[0].decision.state == "accepted" and
        .grants[0].legacy_requests[0].authorization.active == true
      ' >/dev/null <<<"$value"; then
        return 0
      fi
    fi
    sleep 0.2
  done
  fail "current target did not persist the active legacy grant; last grants: $output_file"
}

prove_remote_filesystem() {
  local binary="$1"
  local profile="$2"
  local endpoint="$3"
  local prefix="$4"
  local deadline=$((SECONDS + 60))
  local listing=""
  local mount=""
  local target stat_value
  while [[ $SECONDS -lt $deadline ]]; do
    if listing="$(run_se "$binary" "$profile" ls "$endpoint" 2>/dev/null)"; then
      printf '%s\n' "$listing" >"$root/$prefix-root-listing.txt"
      mount="$(awk -F '\t' '$1 ~ /^d/ && NF >= 3 { print $3; exit }' <<<"$listing")"
      if [[ -n "$mount" && "$mount" != */* && "$mount" != '.' && "$mount" != '..' ]]; then
        break
      fi
    fi
    sleep 0.2
  done
  [[ -n "$mount" ]] || fail \
    "$prefix could not learn a remote mount name from 'se ls' at the CLI-emitted endpoint"

  # The endpoint came from connections/add-peer output and the mount component
  # came from the preceding remote `se ls`. No harness filesystem path becomes
  # a remote target fixture.
  target="${endpoint%/}/$mount"
  stat_value="$(run_se "$binary" "$profile" stat "$target")"
  printf '%s\n' "$stat_value" >"$root/$prefix-remote-stat.txt"
  grep -Fqx $'type\tdir' <<<"$stat_value" || fail \
    "$prefix did not stat the CLI-discovered remote mount as a directory"
}

for profile in "$new_requester" "$old_target" "$old_requester" "$new_target" \
  "$old_reject_requester" "$new_reject_target"; do
  prepare_profile "$profile"
done

stage="start isolated Share server"
start_server
echo "current=$($se_bin --version) legacy=$legacy_version server=$server_bin" >"$root/versions.txt"

stage="NEW to OLD: learn the legacy invite and connect both workers"
old_identity="$(run_se "$legacy_bin" "$old_target" share identity --json)"
printf '%s\n' "$old_identity" >"$root/new-to-old-legacy-identity.json"
old_direct_code="$(jq -er '.direct_code' <<<"$old_identity")"
run_se "$legacy_bin" "$old_target" share configure \
  --server "127.0.0.1:$signal_port" \
  >"$root/new-to-old-legacy-configure.txt"
wait_connected "$legacy_bin" "$old_target" "$root/new-to-old-legacy-connected.json" \
  "legacy target"

run_se "$se_bin" "$new_requester" share configure \
  --server "127.0.0.1:$signal_port" \
  >"$root/new-to-old-current-configure.txt"
wait_connected "$se_bin" "$new_requester" "$root/new-to-old-current-connected.json" \
  "current requester"

stage="NEW to OLD: bridge the signed request to a legacy target"
new_add="$(run_se "$se_bin" "$new_requester" connections add-peer \
  --code "$old_direct_code" --json)"
printf '%s\n' "$new_add" >"$root/new-to-old-add-peer.json"
new_request_id="$(jq -er '.request_id' <<<"$new_add")"
new_endpoint="$(jq -er '.endpoint' <<<"$new_add")"
[[ "$new_endpoint" == share://direct/* ]] || fail \
  "current add-peer did not emit a direct endpoint"

wait_current_request "$new_requester" "$new_request_id" '
  .direction == "outgoing" and
  .delivery.state == "server_queued" and
  .relay.outcome == "legacy_forwarded" and
  .peer_receipt.request.state == "unconfirmed" and
  .decision.state == "pending" and
  .authorization.active == false
' "$root/new-to-old-initial-lifecycle.json" \
  "NEW to OLD request" >/dev/null

stage="NEW to OLD: lose the legacy RAM inbox, restart it, and manually retry"
stop_daemon "$old_target" required
run_se "$legacy_bin" "$old_target" share status --json \
  >"$root/new-to-old-legacy-restart.json"
wait_connected "$legacy_bin" "$old_target" "$root/new-to-old-legacy-reconnected.json" \
  "restarted legacy target"
jq -e '(.pending_requests | length) == 0' \
  >/dev/null <"$root/new-to-old-legacy-reconnected.json" || fail \
  "legacy target unexpectedly retained its RAM-only inbox across a full daemon restart"

new_retry="$(run_se "$se_bin" "$new_requester" share request retry --json)"
printf '%s\n' "$new_retry" >"$root/new-to-old-manual-retry.json"
jq -e --arg id "$new_request_id" '
  .action == "retry_due_now" and .request.request_id == $id
' >/dev/null <<<"$new_retry"

old_pending="$(wait_old_pending_status "$old_target" "$root/new-to-old-old-status-pending.json")"
old_request_device_id="$(jq -er '.pending_requests[0].device_id' <<<"$old_pending")"
old_request_fingerprint="$(jq -er '.pending_requests[0].fingerprint' <<<"$old_pending")"

stage="NEW to OLD: accept using only values emitted by legacy share status"
run_se "$legacy_bin" "$old_target" share request accept \
  --fingerprint "$old_request_fingerprint" "$old_request_device_id" \
  >"$root/new-to-old-legacy-accept.txt"

wait_current_request "$new_requester" "$new_request_id" '
  .direction == "outgoing" and
  .delivery.state == "server_queued" and
  .relay.outcome == "legacy_forwarded" and
  .peer_receipt.request.state == "unconfirmed" and
  .decision.state == "pending" and
  .decision.effective_state == "accepted" and
  .decision.evidence == "legacy_relation" and
  .authorization.state == "active" and
  .authorization.active == true and
  .authorization.basis == "legacy_contact_projection"
' "$root/new-to-old-accepted-lifecycle.json" \
  "NEW to OLD legacy acceptance projection" >/dev/null

stage="NEW to OLD: prove authorized filesystem access via CLI-discovered target"
prove_remote_filesystem "$se_bin" "$new_requester" "$new_endpoint" "new-to-old"
stop_daemon "$new_requester" required
stop_daemon "$old_target" required

stage="OLD to NEW: learn the current invite and connect both workers"
new_identity="$(run_se "$se_bin" "$new_target" share identity --json)"
printf '%s\n' "$new_identity" >"$root/old-to-new-current-identity.json"
new_direct_code="$(jq -er '.direct_code' <<<"$new_identity")"
run_se "$se_bin" "$new_target" share configure \
  --server "127.0.0.1:$signal_port" \
  >"$root/old-to-new-current-configure.txt"
wait_connected "$se_bin" "$new_target" "$root/old-to-new-current-connected.json" \
  "current target"

run_se "$legacy_bin" "$old_requester" share configure \
  --server "127.0.0.1:$signal_port" \
  >"$root/old-to-new-legacy-configure.txt"
wait_connected "$legacy_bin" "$old_requester" "$root/old-to-new-legacy-connected.json" \
  "legacy requester"

stage="OLD to NEW: send the legacy request and prove its durable bare inbox"
run_se "$legacy_bin" "$old_requester" connections add-peer \
  --code "$new_direct_code" \
  >"$root/old-to-new-add-peer.txt"
old_connections="$(run_se "$legacy_bin" "$old_requester" connections list --json)"
printf '%s\n' "$old_connections" >"$root/old-to-new-legacy-connections.json"
old_endpoint="$(jq -er '
  [.[] | select(.kind == "direct")]
  | if length == 1 then .[0].account else error("expected exactly one direct endpoint") end
' <<<"$old_connections")"
[[ "$old_endpoint" == share://direct/* ]] || fail \
  "legacy connections list did not emit a direct endpoint"

first_inbox="$(wait_legacy_inbox "$new_target" "$root/old-to-new-inbox-before-restart.txt")"
first_legacy_line="$(awk -F '\t' '$1 == "pending_legacy_request" { print; exit }' <<<"$first_inbox")"
IFS=$'\t' read -r first_kind first_selector first_name first_device_field \
  first_fingerprint first_delivery first_delivery_scope first_decision \
  first_decision_channel first_decision_delivery first_authorization first_receipt \
  first_identity_conflict <<<"$first_legacy_line"
[[ "$first_kind" == pending_legacy_request && -n "$first_selector" ]] || fail \
  "bare current inbox did not emit a usable legacy selector"
[[ "$first_name" == device_name=?* && "$first_device_field" == device_id=?* \
  && "$first_fingerprint" == fingerprint=?* ]] || fail \
  "bare current inbox omitted legacy identity fields"
first_device_id="${first_device_field#device_id=}"
[[ -n "$first_device_id" ]] || fail \
  "bare current inbox emitted an empty legacy device id"
[[ "$first_delivery" == delivery=received \
  && "$first_delivery_scope" == delivery_scope=local_persisted \
  && "$first_decision" == decision=pending \
  && "$first_decision_channel" == decision_channel=not_applicable \
  && "$first_decision_delivery" == decision_delivery=not_started ]] || fail \
  "bare current inbox did not describe the pending delivery state"
[[ "$first_authorization" == authorization=inactive ]] || fail \
  "bare current inbox incorrectly reported active authorization before acceptance"
[[ "$first_receipt" == receipt=unsupported ]] || fail \
  "bare current inbox incorrectly claimed a tracked legacy peer receipt"
[[ "$first_identity_conflict" == identity_conflict=false ]] || fail \
  "bare current inbox did not expose a conflict-free legacy identity"
pending_completion="$(complete_se_bash "$new_target" 4 \
  se share request accept '')"
grep -Fqx "$first_selector" <<<"$pending_completion" || fail \
  "live Bash completion did not expose the pending legacy selector"

# Keep the legacy sender offline while restarting the current target. This
# makes the second inbox observation proof of local durability: no peer exists
# that could silently redeliver the request after the restart.
stop_daemon "$old_requester" required
stop_daemon "$new_target" required
run_se "$se_bin" "$new_target" share status --json \
  >"$root/old-to-new-current-restart.json"
wait_connected "$se_bin" "$new_target" "$root/old-to-new-current-reconnected.json" \
  "restarted current target"
jq -e --arg selector "$first_selector" --arg id "$first_device_id" '
  .worker.reachable == true and .running == true and .connected == true and
  ([.pending_requests[] |
    select(.selector == $selector and .device_id == $id and
      .delivery.state == "received" and .delivery.scope == "local_persisted" and
      .decision.state == "pending" and
      .decision.delivery.channel == "not_applicable" and
      .decision.delivery.state == "not_started" and
      .authorization.active == false and .identity_conflict == false)] | length) == 1 and
  ([.legacy_requests[] |
    select(.selector == $selector and .device_id == $id and
      .delivery.state == "received" and .delivery.scope == "local_persisted" and
      .decision.state == "pending" and
      .decision.delivery.channel == "not_applicable" and
      .decision.delivery.state == "not_started" and
      .authorization.active == false and .identity_conflict == false)] | length) == 1
' >/dev/null <"$root/old-to-new-current-reconnected.json" || fail \
  "durable legacy request was not complete in share status after restart"
second_inbox="$(wait_legacy_inbox "$new_target" "$root/old-to-new-inbox-after-restart.txt")"
second_legacy_line="$(awk -F '\t' '$1 == "pending_legacy_request" { print; exit }' <<<"$second_inbox")"
[[ "$second_legacy_line" == "$first_legacy_line" ]] || fail \
  "durable legacy inbox changed identity or lifecycle state across daemon restart"

run_se "$legacy_bin" "$old_requester" share status --json \
  >"$root/old-to-new-legacy-restart.json"
wait_connected "$legacy_bin" "$old_requester" "$root/old-to-new-legacy-reconnected.json" \
  "restarted legacy requester"

stage="OLD to NEW: accept context-free, then prove history and grant persistence"
legacy_accept="$(run_se "$se_bin" "$new_target" share request accept --json)"
printf '%s\n' "$legacy_accept" >"$root/old-to-new-context-free-accept.json"
jq -e --arg selector "$first_selector" --arg id "$first_device_id" '
  .action == "accepted" and .legacy == true and
  .request.selector == $selector and .request.device_id == $id and
  .request.delivery.state == "received" and
  .request.delivery.scope == "local_persisted" and
  .request.decision.state == "accepted" and
  .request.decision.delivery.channel == "legacy_signaling_untracked" and
  .request.decision.delivery.state == "attempted_untracked" and
  .request.decision.delivery.attempt_count >= 1 and
  .request.decision.delivery.last_error == null and
  .request.authorization.state == "active" and
  .request.authorization.active == true and
  .worker_refresh.state == "refreshed" and .worker_refresh.error == null
' >/dev/null <<<"$legacy_accept"
legacy_accept_attempts="$(jq -er '.request.decision.delivery.attempt_count' \
  <<<"$legacy_accept")"

stage="OLD to NEW: retry the untracked decision context-free"
retry_completion="$(complete_se_bash "$new_target" 4 \
  se share request retry '')"
grep -Fqx "$first_selector" <<<"$retry_completion" || fail \
  "live Bash completion did not expose the retryable legacy selector"
legacy_retry="$(run_se "$se_bin" "$new_target" share request retry --json)"
printf '%s\n' "$legacy_retry" >"$root/old-to-new-context-free-retry.json"
jq -e --arg selector "$first_selector" --argjson prior "$legacy_accept_attempts" '
  .action == "retry_queued" and .legacy == true and
  .request.selector == $selector and
  .request.decision.state == "accepted" and
  .request.decision.delivery.channel == "legacy_signaling_untracked" and
  .request.decision.delivery.state == "attempted_untracked" and
  .request.decision.delivery.attempt_count > $prior and
  .request.decision.delivery.last_error == null and
  .request.authorization.active == true and
  .worker_refresh.state == "refreshed" and .worker_refresh.error == null
' >/dev/null <<<"$legacy_retry"

wait_current_grant "$new_target" "$first_device_id" "$first_selector" \
  "$root/old-to-new-grants-active.json"
legacy_history="$(run_se "$se_bin" "$new_target" share request list --json)"
printf '%s\n' "$legacy_history" >"$root/old-to-new-history-accepted.json"
jq -e --arg selector "$first_selector" --arg id "$first_device_id" \
  --argjson attempts "$legacy_accept_attempts" '
  .count == 1 and (.legacy_requests | length) == 1 and
  .legacy_requests[0].legacy == true and
  .legacy_requests[0].selector == $selector and
  .legacy_requests[0].device_id == $id and
  .legacy_requests[0].delivery.state == "received" and
  .legacy_requests[0].decision.state == "accepted" and
  .legacy_requests[0].decision.delivery.channel == "legacy_signaling_untracked" and
  .legacy_requests[0].decision.delivery.state == "attempted_untracked" and
  .legacy_requests[0].decision.delivery.attempt_count > $attempts and
  .legacy_requests[0].authorization.active == true
' >/dev/null <<<"$legacy_history"

stage="OLD to NEW: prove authorization and filesystem access"
wait_old_accepted "$old_requester" "$root/old-to-new-legacy-accepted.json"
prove_remote_filesystem "$legacy_bin" "$old_requester" "$old_endpoint" "old-to-new"

stage="OLD to NEW: active history cannot be deleted; revoke and delete context-free"
grant_completion="$(complete_se_bash "$new_target" 4 \
  se share grants revoke '')"
grep -Fqx "$first_device_id" <<<"$grant_completion" || fail \
  "live Bash completion did not expose the active legacy grant device"
set +e
run_se "$se_bin" "$new_target" share request delete --json \
  >"$root/old-to-new-delete-active.txt" 2>&1
delete_active_status=$?
set -e
[[ $delete_active_status -ne 0 ]] || fail \
  "active legacy authorization history was deletable before revocation"
if ! grep -Fq 'active authorization' "$root/old-to-new-delete-active.txt" \
  || ! grep -Fq "se share grants revoke $first_selector" \
    "$root/old-to-new-delete-active.txt"; then
  fail "active legacy history deletion failed without the exact actionable revoke command"
fi

legacy_revoke="$(run_se "$se_bin" "$new_target" share grants revoke --json)"
printf '%s\n' "$legacy_revoke" >"$root/old-to-new-context-free-revoke.json"
jq -e --arg selector "$first_selector" --arg id "$first_device_id" '
  .action == "revoked" and .legacy == true and
  .request.selector == $selector and .request.device_id == $id and
  .request.decision.state == "revoked" and
  .request.decision.delivery.channel == "local_only_untracked" and
  .request.decision.delivery.state == "local_only_untracked" and
  .request.authorization.active == false
' >/dev/null <<<"$legacy_revoke"

if run_se "$legacy_bin" "$old_requester" ls "$old_endpoint" \
  >"$root/old-to-new-access-after-revoke.txt" 2>&1; then
  fail "revoked legacy grant still authorized remote filesystem access"
fi

delete_completion="$(complete_se_bash "$new_target" 4 \
  se share request delete '')"
grep -Fqx "$first_selector" <<<"$delete_completion" || fail \
  "live Bash completion did not expose the deletable legacy selector"
legacy_delete="$(run_se "$se_bin" "$new_target" share request delete --json)"
printf '%s\n' "$legacy_delete" >"$root/old-to-new-context-free-delete.json"
jq -e --arg selector "$first_selector" '
  .action == "deleted" and .legacy == true and
  .selector == $selector and .persisted == true
' >/dev/null <<<"$legacy_delete"

empty_history="$(run_se "$se_bin" "$new_target" share request list --json)"
printf '%s\n' "$empty_history" >"$root/old-to-new-history-empty.json"
jq -e '
  .count == 0 and (.requests | length) == 0 and (.legacy_requests | length) == 0
' >/dev/null <<<"$empty_history"

stage="OLD to NEW reject: learn the target invite and connect fresh workers"
reject_target_identity="$(run_se "$se_bin" "$new_reject_target" share identity --json)"
printf '%s\n' "$reject_target_identity" >"$root/reject-current-identity.json"
reject_direct_code="$(jq -er '.direct_code' <<<"$reject_target_identity")"
run_se "$se_bin" "$new_reject_target" share configure \
  --server "127.0.0.1:$signal_port" >"$root/reject-current-configure.txt"
wait_connected "$se_bin" "$new_reject_target" "$root/reject-current-connected.json" \
  "reject current target"

run_se "$legacy_bin" "$old_reject_requester" share configure \
  --server "127.0.0.1:$signal_port" >"$root/reject-legacy-configure.txt"
wait_connected "$legacy_bin" "$old_reject_requester" \
  "$root/reject-legacy-connected.json" "reject legacy requester"
run_se "$legacy_bin" "$old_reject_requester" connections add-peer \
  --code "$reject_direct_code" >"$root/reject-legacy-add-peer.txt"

stage="OLD to NEW reject: reject and retry using only the bare inbox"
reject_inbox="$(wait_legacy_inbox "$new_reject_target" "$root/reject-inbox.txt")"
reject_legacy_line="$(awk -F '\t' '$1 == "pending_legacy_request" { print; exit }' \
  <<<"$reject_inbox")"
IFS=$'\t' read -r reject_kind reject_selector reject_name reject_device_field \
  reject_fingerprint reject_delivery reject_delivery_scope reject_decision \
  reject_decision_channel reject_decision_delivery reject_authorization reject_receipt \
  reject_identity_conflict <<<"$reject_legacy_line"
[[ "$reject_kind" == pending_legacy_request && -n "$reject_selector" \
  && "$reject_name" == device_name=?* && "$reject_device_field" == device_id=?* \
  && "$reject_fingerprint" == fingerprint=?* \
  && "$reject_delivery" == delivery=received \
  && "$reject_delivery_scope" == delivery_scope=local_persisted \
  && "$reject_decision" == decision=pending \
  && "$reject_decision_channel" == decision_channel=not_applicable \
  && "$reject_decision_delivery" == decision_delivery=not_started \
  && "$reject_authorization" == authorization=inactive \
  && "$reject_receipt" == receipt=unsupported \
  && "$reject_identity_conflict" == identity_conflict=false ]] || fail \
  "reject inbox did not emit a complete conflict-free legacy request"
reject_device_id="${reject_device_field#device_id=}"
[[ -n "$reject_device_id" ]] || fail "reject inbox emitted an empty device id"

legacy_reject="$(run_se "$se_bin" "$new_reject_target" share request reject --json)"
printf '%s\n' "$legacy_reject" >"$root/reject-context-free.json"
jq -e --arg selector "$reject_selector" --arg id "$reject_device_id" '
  .action == "rejected" and .legacy == true and
  .request.selector == $selector and .request.device_id == $id and
  .request.decision.state == "rejected" and
  .request.decision.delivery.channel == "legacy_signaling_untracked" and
  .request.decision.delivery.state == "attempted_untracked" and
  .request.decision.delivery.attempt_count >= 1 and
  .request.decision.delivery.last_error == null and
  .request.authorization.active == false and
  .worker_refresh.state == "refreshed" and .worker_refresh.error == null
' >/dev/null <<<"$legacy_reject"
reject_attempts="$(jq -er '.request.decision.delivery.attempt_count' \
  <<<"$legacy_reject")"

reject_retry="$(run_se "$se_bin" "$new_reject_target" share request retry --json)"
printf '%s\n' "$reject_retry" >"$root/reject-context-free-retry.json"
jq -e --arg selector "$reject_selector" --arg id "$reject_device_id" \
  --argjson prior "$reject_attempts" '
  .action == "retry_queued" and .legacy == true and
  .request.selector == $selector and .request.device_id == $id and
  .request.decision.state == "rejected" and
  .request.decision.delivery.channel == "legacy_signaling_untracked" and
  .request.decision.delivery.state == "attempted_untracked" and
  .request.decision.delivery.attempt_count > $prior and
  .request.decision.delivery.last_error == null and
  .request.authorization.active == false and
  .worker_refresh.state == "refreshed" and .worker_refresh.error == null
' >/dev/null <<<"$reject_retry"
wait_old_rejected "$old_reject_requester" "$root/reject-legacy-observed.json"

reject_delete="$(run_se "$se_bin" "$new_reject_target" share request delete --json)"
printf '%s\n' "$reject_delete" >"$root/reject-context-free-delete.json"
jq -e --arg selector "$reject_selector" '
  .action == "deleted" and .legacy == true and
  .selector == $selector and .persisted == true
' >/dev/null <<<"$reject_delete"
reject_empty="$(run_se "$se_bin" "$new_reject_target" share request list --json)"
printf '%s\n' "$reject_empty" >"$root/reject-history-empty.json"
jq -e '.count == 0 and (.legacy_requests | length) == 0' \
  >/dev/null <<<"$reject_empty"

stage="complete"
echo "mixed-version Share E2E passed: NEW->OLD request $new_request_id; OLD->NEW accepted $first_selector; rejected $reject_selector"
