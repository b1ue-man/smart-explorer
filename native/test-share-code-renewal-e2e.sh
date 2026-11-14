#!/usr/bin/env bash
# Direct-code renewal across real processes (E17, plan 2026-11-08 M19).
#
# One share server (default mode: key proof, no legacy clients) and five devices, each a
# separate profile with its own background worker:
#
#   host      the current build; its direct code is renewed when it removes a contact
#   online    the current build, online while the host removes a contact and renews
#   offline   the current build, stopped during the removal, started again later
#   old195    the published v0.5.195 `se` (no `code_renewal_v1`), downloaded from the
#             GitHub Release page and checked against its SHA-256
#   gone      the current build; the contact the host removes
#
# Proved:
#   1. no contact is lost: online and offline take the new data (online while the host runs,
#      offline when it returns), old195 keeps working on the old data for good, and every one
#      of the three still lists the host's files afterwards and keeps an accepted grant;
#   2. the removed device does not receive the new code: its access fails, it is never listed
#      as pending, and the new lookup id appears nowhere in its profile;
#   3. (E4) the removal runs in a second process of the host profile while the host worker keeps
#      running; the removed device's first connection after it is refused;
#   4. (E2) on a fresh host, 63 removals leave 63 retiring generations, each held by one offline
#      contact; the 64th removal waits (`deferred`) and no pending entry disappears.
#
# Every value (ports, paths, ids, selectors, the new lookup id) comes from the programs' own
# output; nothing is passed in from outside. Runs on Linux only. Optional inputs:
#   SMART_EXPLORER_SE_BINARY            current `se` (default native/target/debug/se)
#   SMART_EXPLORER_SHARE_SERVER_BINARY  current share server (default share-server/target/debug/se-share-server)
#   SMART_EXPLORER_LEGACY_SE_BINARY     an already downloaded v0.5.195 `se` (its SHA-256 is checked anyway)
#   SMART_EXPLORER_E2E_LOG_DIR          directory that receives the logs of a failed run
#   SMART_EXPLORER_KEEP_E2E_ROOT=1      keep the working directory of a passed run
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
se_bin="${SMART_EXPLORER_SE_BINARY:-$repo_root/native/target/debug/se}"
server_bin="${SMART_EXPLORER_SHARE_SERVER_BINARY:-$repo_root/share-server/target/debug/se-share-server}"
legacy_bin="${SMART_EXPLORER_LEGACY_SE_BINARY:-}"
legacy_repository="${SMART_EXPLORER_LEGACY_REPOSITORY:-b1ue-man/smart-explorer}"
legacy_tag="v0.5.195"
# Published `se` (Linux x86_64) of v0.5.195: the digest GitHub computed for the release asset.
legacy_sha256="826383058dc9130648705c2ac7ec3166aeb723eac8503f4627ae54fad2f4d195"
# Seconds each convergence may take (relay round trips, server presence, reciprocal exchange).
converge_limit=150

stage="preflight"
failure_line=""
failure_command=""
root=""
download_dir=""
server_pid=""
signal_port=""

remember_failure() {
  failure_line="$1"
  failure_command="$2"
}
trap 'remember_failure "$LINENO" "$BASH_COMMAND"' ERR

fail() {
  echo "code renewal E2E: $*" >&2
  return 1
}

command -v jq >/dev/null || fail "jq is required"
command -v curl >/dev/null || fail "curl is required"
command -v sha256sum >/dev/null || fail "sha256sum is required"
command -v timeout >/dev/null || fail "GNU timeout is required"
[[ "$(uname -s)" == Linux ]] || fail "this harness requires Linux"
for binary in "$se_bin" "$server_bin"; do
  [[ -x "$binary" ]] || fail "executable is missing: $binary"
done

root="$(mktemp -d "${TMPDIR:-/tmp}/se-renewal.XXXXXX")"
download_dir="$(mktemp -d "${TMPDIR:-/tmp}/se-renewal-dl.XXXXXX")"
host="$root/host"
online="$root/online"
offline="$root/offline"
old195="$root/old195"
gone="$root/gone"
profiles=("$host" "$online" "$offline" "$old195" "$gone")

prepare_profile() {
  local profile="$1"
  mkdir -p "$profile/home" "$profile/data" "$profile/config" "$profile/cache" "$profile/runtime" "$profile/tmp"
  chmod 700 "$profile/home" "$profile/data" "$profile/config" "$profile/cache" "$profile/runtime" "$profile/tmp"
}

# as BINARY PROFILE ARGS...: one program invocation inside one profile. The route is the server's
# relay (nothing but loopback exists between the processes), as in test-share-lifecycle-e2e.sh.
as() {
  local binary="$1" profile="$2"
  shift 2
  timeout --foreground --signal=TERM --kill-after=5s 90s env \
    HOME="$profile/home" \
    USERPROFILE="$profile/home" \
    XDG_DATA_HOME="$profile/data" \
    XDG_CONFIG_HOME="$profile/config" \
    XDG_CACHE_HOME="$profile/cache" \
    XDG_RUNTIME_DIR="$profile/runtime" \
    APPDATA="$profile/data" \
    LOCALAPPDATA="$profile/data" \
    TMPDIR="$profile/tmp" \
    NO_COLOR=1 \
    SE_SHARE_RELAY_ONLY=1 \
    "$binary" "$@"
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
  local -a pids=()
  local pid deadline
  mapfile -t pids < <(daemon_pids "$profile")
  for pid in "${pids[@]}"; do
    kill -TERM "$pid" 2>/dev/null || true
  done
  deadline=$((SECONDS + 10))
  while [[ $SECONDS -lt $deadline && -n "$(daemon_pids "$profile")" ]]; do
    sleep 0.1
  done
  mapfile -t pids < <(daemon_pids "$profile")
  for pid in "${pids[@]}"; do
    kill -KILL "$pid" 2>/dev/null || true
  done
  deadline=$((SECONDS + 5))
  while [[ $SECONDS -lt $deadline && -n "$(daemon_pids "$profile")" ]]; do
    sleep 0.1
  done
  [[ -z "$(daemon_pids "$profile")" ]] || fail "worker did not stop for $profile"
}

dump_diagnostics() {
  echo "stage: $stage" >&2
  [[ -z "$failure_line" ]] || echo "failed at line $failure_line: $failure_command" >&2
  if [[ -n "$server_pid" ]]; then
    ps -o pid,ppid,stat,etime,cmd -p "$server_pid" >&2 || true
  fi
  if [[ -d "$root" ]]; then
    local file
    while IFS= read -r file; do
      echo "--- ${file#"$root"/}" >&2
      tail -n 60 "$file" >&2 || true
    done < <(find "$root" -type f \( -name '*.log' -o -name '*.txt' \) -print | sort)
    echo "preserved diagnostics: $root" >&2
    if [[ -n "${SMART_EXPLORER_E2E_LOG_DIR:-}" ]]; then
      mkdir -p "$SMART_EXPLORER_E2E_LOG_DIR"
      (cd "$root" && find . -type f \( -name '*.log' -o -name '*.txt' -o -name '*.json' \) -print0 \
        | xargs -0 -r cp --parents -t "$SMART_EXPLORER_E2E_LOG_DIR") 2>/dev/null || true
    fi
  fi
}

cleanup() {
  local status=$?
  trap - EXIT ERR
  set +e
  local profile
  for profile in "${profiles[@]}"; do
    [[ -d "$profile" ]] && stop_daemon "$profile"
  done
  if [[ -n "$server_pid" ]]; then
    kill -TERM "$server_pid" 2>/dev/null
    wait "$server_pid" 2>/dev/null
  fi
  rm -rf "$download_dir"
  if [[ $status -ne 0 ]]; then
    dump_diagnostics
  elif [[ "${SMART_EXPLORER_KEEP_E2E_ROOT:-0}" != 1 ]]; then
    rm -rf "$root"
  else
    echo "code renewal E2E root retained: $root"
  fi
  exit "$status"
}
trap cleanup EXIT

# ---------------------------------------------------------------------------------------------
# The published v0.5.195 `se`: downloaded from the GitHub Release page, SHA-256 checked against the
# digest pinned here and against the release's own `se.sha256` sidecar, then proven to lack the
# renewal feature the premise of this test needs.
# ---------------------------------------------------------------------------------------------
fetch_legacy() {
  local base="https://github.com/$legacy_repository/releases/download/$legacy_tag"
  if [[ -z "$legacy_bin" ]]; then
    legacy_bin="$download_dir/se"
    curl --fail --silent --show-error --location --retry 5 --retry-delay 5 --retry-all-errors \
      --max-time 900 --output "$legacy_bin" "$base/se"
    chmod 755 "$legacy_bin"
  fi
  curl --fail --silent --show-error --location --retry 5 --retry-delay 5 --retry-all-errors \
    --max-time 120 --output "$download_dir/se.sha256" "$base/se.sha256"
  local actual sidecar
  actual="$(sha256sum "$legacy_bin" | cut -d' ' -f1)"
  sidecar="$(awk 'NR == 1 { print tolower($1) }' "$download_dir/se.sha256")"
  [[ "$actual" == "$legacy_sha256" ]] || fail \
    "the v0.5.195 se has SHA-256 $actual, expected $legacy_sha256"
  [[ "$sidecar" == "$legacy_sha256" ]] || fail \
    "the release sidecar se.sha256 names $sidecar, expected $legacy_sha256"
  [[ -x "$legacy_bin" ]] || fail "the v0.5.195 se is not executable: $legacy_bin"
  local version
  version="$("$legacy_bin" --version)"
  [[ "$version" =~ (^|[[:space:]])0\.5\.195($|[[:space:]]) ]] || fail \
    "the downloaded se reports '$version', expected 0.5.195"
  # The premise: the old peer cannot negotiate the renewal; the current build can.
  if grep -aFq 'code_renewal_v1' "$legacy_bin"; then
    fail "the v0.5.195 se contains code_renewal_v1; it is no peer without renewal support"
  fi
  grep -aFq 'code_renewal_v1' "$se_bin" || fail "the current se lacks code_renewal_v1"
  echo "legacy peer: $legacy_tag se, sha256 $actual, without code_renewal_v1"
}

start_server() {
  local attempt candidate log
  for attempt in $(seq 0 15); do
    candidate=$((31000 + (( $$ + attempt * 97) % 15000)))
    log="$root/share-server-$candidate.log"
    "$server_bin" "127.0.0.1:$candidate" >"$log" 2>&1 &
    server_pid=$!
    sleep 0.5
    if kill -0 "$server_pid" 2>/dev/null; then
      signal_port="$candidate"
      return 0
    fi
    wait "$server_pid" 2>/dev/null || true
    server_pid=""
  done
  fail "the share server did not start; inspect $root/share-server-*.log"
}

wait_connected() {
  local binary="$1" profile="$2" label="$3"
  local deadline=$((SECONDS + 60))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(as "$binary" "$profile" share status --json 2>/dev/null)" \
      && jq -e '.running == true and .connected == true' >/dev/null <<<"$value"; then
      return 0
    fi
    sleep 0.5
  done
  printf '%s\n' "$value" >"$root/$label-not-connected.txt"
  fail "$label did not connect to the share server (last status: $root/$label-not-connected.txt)"
}

# bring_online BINARY PROFILE LABEL: server address set, worker (re)started and connected.
bring_online() {
  local binary="$1" profile="$2" label="$3"
  as "$binary" "$profile" share configure --server "tcp://127.0.0.1:$signal_port" --allow-plaintext \
    >"$root/$label-configure.txt"
  stop_daemon "$profile"
  as "$binary" "$profile" share status --json >/dev/null
  wait_connected "$binary" "$profile" "$label"
}

wait_request_state() {
  local binary="$1" profile="$2" request_id="$3" filter="$4" label="$5"
  local deadline=$((SECONDS + 90))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(as "$binary" "$profile" share request show "$request_id" --json 2>/dev/null)" \
      && jq -e "$filter" >/dev/null <<<"$value"; then
      return 0
    fi
    sleep 0.5
  done
  printf '%s\n' "$value" >"$root/$label-request.txt"
  fail "$label: request $request_id never satisfied: $filter (last: $root/$label-request.txt)"
}

# list_works BINARY PROFILE ENDPOINT LABEL: the contact lists the host's shared folder.
list_works() {
  local binary="$1" profile="$2" endpoint="$3" label="$4"
  local deadline=$((SECONDS + 90))
  while [[ $SECONDS -lt $deadline ]]; do
    if as "$binary" "$profile" ls "$endpoint" >"$root/$label-ls.txt" 2>"$root/$label-ls.err.txt"; then
      grep -q . "$root/$label-ls.txt" || fail "$label listed the host, but the listing is empty"
      return 0
    fi
    sleep 1
  done
  fail "$label could not list $endpoint (see $root/$label-ls.err.txt)"
}

# list_denied BINARY PROFILE ENDPOINT LABEL: a real refusal, not a timeout.
list_denied() {
  local binary="$1" profile="$2" endpoint="$3" label="$4"
  local status=0
  as "$binary" "$profile" ls "$endpoint" >"$root/$label-denied.txt" 2>"$root/$label-denied.err.txt" || status=$?
  [[ $status -ne 0 ]] || fail "$label still lists the host after its removal"
  [[ $status -ne 124 && $status -ne 137 ]] || fail \
    "$label: the refused listing timed out instead of returning a refusal"
}

# pair LABEL BINARY PROFILE: the device becomes a contact of the host with an accepted grant.
# Sets endpoint_of[LABEL] and request_of[LABEL].
declare -A endpoint_of
pair() {
  local label="$1" binary="$2" profile="$3"
  # pair_host and pair_code, when set, name another host profile and its direct code (E2 below).
  local host_profile="${pair_host:-$host}" code="${pair_code:-$host_code}"
  prepare_profile "$profile"
  as "$binary" "$profile" share identity --set-name "$label" >"$root/$label-name.txt" 2>&1
  bring_online "$binary" "$profile" "$label"
  local added request_id
  added="$(as "$binary" "$profile" connections add-peer --code "$code" --name HostOf-"$label" --json)"
  printf '%s\n' "$added" >"$root/$label-add-peer.json"
  request_id="$(jq -er '.request_id' <<<"$added")"
  endpoint_of[$label]="$(jq -er '.endpoint' <<<"$added")"
  [[ "${endpoint_of[$label]}" == share://direct/* ]] || fail "$label: add-peer emitted no direct endpoint"
  wait_request_state "$se_bin" "$host_profile" "$request_id" \
    '.direction == "incoming" and .decision.state == "pending"' "host-sees-$label"
  as "$se_bin" "$host_profile" share request accept "$request_id" --json >"$root/host-accept-$label.json"
  jq -e '.request.decision.state == "accepted" and .request.authorization.active == true' \
    >/dev/null <"$root/host-accept-$label.json" || fail "the host did not accept $label"
  wait_request_state "$binary" "$profile" "$request_id" \
    '.decision.state == "accepted" and .authorization.active == true' "$label-accepted"
  list_works "$binary" "$profile" "${endpoint_of[$label]}" "$label-before"
}

lookup_of() {
  sed -E 's/^SE-D3-([0-9a-f]+)-.*/\1/' <<<"$1"
}

# profile_file PROFILE: the one share_profiles.json of a profile.
profile_file() {
  local -a found=()
  mapfile -t found < <(find "$1" -type f -name share_profiles.json -print)
  [[ ${#found[@]} -eq 1 ]] || fail "expected one share_profiles.json below $1, found ${#found[@]}"
  printf '%s\n' "${found[0]}"
}

# contact_lookup PROFILE: the lookup id this device stores for the host's direct contact.
contact_lookup() {
  jq -er '[.direct_contacts[] | .lookup_id] | if length == 1 then .[0] else error("expected one direct contact") end' \
    "$(profile_file "$1")"
}

host_status() {
  as "$se_bin" "$host" share status --json
}

# wait_host FILTER LABEL [NUDGE-FUNCTION]: the host worker's status satisfies FILTER; the nudge
# (contacts that are online use the host) runs every 20 s while waiting.
wait_host() {
  local filter="$1" label="$2" nudge="${3:-}"
  local deadline=$((SECONDS + converge_limit))
  local next_nudge=$((SECONDS + 5))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(host_status 2>/dev/null)" && jq -e "$filter" >/dev/null <<<"$value"; then
      printf '%s\n' "$value" >"$root/host-$label.json"
      return 0
    fi
    if [[ -n "$nudge" && $SECONDS -ge $next_nudge ]]; then
      "$nudge" || true
      next_nudge=$((SECONDS + 20))
    fi
    sleep 1
  done
  printf '%s\n' "$value" >"$root/host-$label-timeout.json"
  fail "host status never satisfied '$filter' ($label; last: $root/host-$label-timeout.json)"
}

nudge_online_contacts() {
  as "$se_bin" "$online" ls "${endpoint_of[online]}" >/dev/null 2>&1 || true
  as "$legacy_bin" "$old195" ls "${endpoint_of[old195]}" >/dev/null 2>&1 || true
  as "$se_bin" "$gone" ls "${endpoint_of[gone]}" >/dev/null 2>&1 || true
}

nudge_returned_contact() {
  as "$se_bin" "$offline" ls "${endpoint_of[offline]}" >/dev/null 2>&1 || true
  as "$legacy_bin" "$old195" ls "${endpoint_of[old195]}" >/dev/null 2>&1 || true
}

# ---------------------------------------------------------------------------------------------
stage="legacy peer"
fetch_legacy

for profile in "${profiles[@]}"; do
  prepare_profile "$profile"
done

stage="start the share server (default mode)"
start_server
echo "current se: $("$se_bin" --version); share server on 127.0.0.1:$signal_port"

stage="host: identity, export, server"
as "$se_bin" "$host" share identity --set-name host >"$root/host-name.txt" 2>&1
host_identity="$(as "$se_bin" "$host" share identity --json)"
printf '%s\n' "$host_identity" >"$root/host-identity.json"
host_code="$(jq -er '.direct_code' <<<"$host_identity")"
old_lookup="$(lookup_of "$host_code")"
[[ "$old_lookup" =~ ^[0-9a-f]+$ ]] || fail "cannot read the lookup id from the host's direct code"
jq -e '.code_renewal == null' >/dev/null <<<"$host_identity" || fail "the host shows a renewal before any removal"
mkdir -p "$root/export"
echo hello >"$root/export/hello.txt"
as "$se_bin" "$host" share export add "$root/export" --label Files >"$root/host-export.txt"
bring_online "$se_bin" "$host" host
jq -e '.code_renewal == null' >/dev/null <<<"$(host_status)" || fail "the host worker shows a renewal before any removal"

stage="pair the four contacts"
pair online "$se_bin" "$online"
pair offline "$se_bin" "$offline"
pair old195 "$legacy_bin" "$old195"
pair gone "$se_bin" "$gone"

grants="$(as "$se_bin" "$host" share grants --json)"
printf '%s\n' "$grants" >"$root/host-grants-before.json"
for label in online offline old195 gone; do
  jq -e --arg name "$label" '[.grants[] | select(.device_name == $name and .grant_state == "accepted" and .authorization.active == true)] | length == 1' \
    >/dev/null <<<"$grants" || fail "the host holds no accepted grant for $label"
done
gone_id="$(jq -er '.grants[] | select(.device_name == "gone") | .device_id' <<<"$grants")"
for label in online offline old195 gone; do
  [[ "$(contact_lookup "$root/$label")" == "$old_lookup" ]] || fail "$label does not hold the host's lookup id"
done

stage="take one contact offline, then remove another"
stop_daemon "$offline"
as "$se_bin" "$host" share grants delete "$gone_id" --json >"$root/host-delete-gone.json"
jq -e '.action == "removed_device" or .action == "deleted_grant"' >/dev/null <"$root/host-delete-gone.json" \
  || fail "the host did not remove the contact"

# E4: the removal above ran in a second process of the host profile while the host worker kept running.
# The removed device's first connection after it is refused by that running worker, before any other
# device has spoken to the host (the nudges below come later).
stage="the removed contact is refused at its first attempt (E4)"
list_denied "$se_bin" "$gone" "${endpoint_of[gone]}" gone-first-attempt

stage="the removal renews the host's direct code"
renewal_filter='.code_renewal != null and .code_renewal.active == true and .code_renewal.pending_devices == 3
  and (.code_renewal.pending_names | index("online") != null and index("offline") != null and index("old195") != null)
  and (.code_renewal.pending_names | index("gone") == null)'
fallback="none"
renewed=0
for phase in worker-alone cli-load worker-restart; do
  case "$phase" in
    cli-load) as "$se_bin" "$host" share identity --json >"$root/host-identity-after.json" ;;
    worker-restart)
      stop_daemon "$host"
      as "$se_bin" "$host" share status --json >/dev/null
      wait_connected "$se_bin" "$host" host-restarted
      ;;
  esac
  limit=45
  deadline=$((SECONDS + limit))
  while [[ $SECONDS -lt $deadline ]]; do
    if host_status >"$root/host-renewal.json" 2>/dev/null && jq -e "$renewal_filter" >/dev/null <"$root/host-renewal.json"; then
      renewed=1
      break
    fi
    sleep 1
  done
  if [[ $renewed -eq 1 ]]; then
    fallback="$phase"
    break
  fi
done
[[ $renewed -eq 1 ]] || fail "no renewal appeared for the three remaining contacts (last: $root/host-renewal.json)"
# A renewal the worker only learns after a CLI load or a restart is a finding for the evaluation of
# this log; the contacts' outcome below is what fails the run.
echo "E2E-RENEWAL started=$fallback"

new_identity="$(as "$se_bin" "$host" share identity --json)"
printf '%s\n' "$new_identity" >"$root/host-identity-renewed.json"
new_code="$(jq -er '.direct_code' <<<"$new_identity")"
new_lookup="$(lookup_of "$new_code")"
[[ "$new_code" != "$host_code" && "$new_lookup" != "$old_lookup" && "$new_lookup" =~ ^[0-9a-f]+$ ]] || fail \
  "the host's direct code did not change"
jq -e '.code_renewal.active == true' >/dev/null <<<"$new_identity" || fail "the identity view shows no renewal"

stage="the online contact and the old peer, while the offline one is away"
wait_host '.code_renewal.pending_names | index("online") == null' online-took-over nudge_online_contacts
wait_host '.code_renewal.legacy_devices >= 1 and (.code_renewal.pending_names | index("old195") != null)' old195-flagged nudge_online_contacts
wait_host '.code_renewal.active == true and .code_renewal.pending_devices == 2
  and (.code_renewal.pending_names | index("offline") != null and index("old195") != null)' offline-still-pending
list_works "$se_bin" "$online" "${endpoint_of[online]}" online-after
list_works "$legacy_bin" "$old195" "${endpoint_of[old195]}" old195-after
[[ "$(contact_lookup "$online")" == "$new_lookup" ]] || fail "the online contact did not take the new lookup id"
[[ "$(contact_lookup "$old195")" == "$old_lookup" ]] || fail "the old peer changed its lookup id without renewal support"
list_denied "$se_bin" "$gone" "${endpoint_of[gone]}" gone-after-removal

stage="the offline contact returns"
# The offline contact still holds the old data: it is neither lost nor renewed while it is away.
[[ "$(contact_lookup "$offline")" == "$old_lookup" ]] || fail "the offline contact's data changed while it was offline"
as "$se_bin" "$offline" share status --json >/dev/null
wait_connected "$se_bin" "$offline" offline-returned
list_works "$se_bin" "$offline" "${endpoint_of[offline]}" offline-returned
wait_host '.code_renewal.active == true and .code_renewal.pending_devices == 1 and .code_renewal.legacy_devices == 1
  and .code_renewal.pending_names == ["old195"]' offline-took-over nudge_returned_contact
[[ "$(contact_lookup "$offline")" == "$new_lookup" ]] || fail "the returned contact did not take the new lookup id"

stage="no contact was lost, the removed contact got nothing"
final_grants="$(as "$se_bin" "$host" share grants --json)"
printf '%s\n' "$final_grants" >"$root/host-grants-after.json"
for label in online offline old195; do
  jq -e --arg name "$label" '[.grants[] | select(.device_name == $name and .grant_state == "accepted" and .authorization.active == true)] | length == 1' \
    >/dev/null <<<"$final_grants" || fail "the grant of $label is gone or no longer active"
done
jq -e '[.grants[] | select(.device_name == "gone")] | length == 0' >/dev/null <<<"$final_grants" \
  || fail "the removed contact still has a grant"
list_works "$se_bin" "$online" "${endpoint_of[online]}" online-final
list_works "$se_bin" "$offline" "${endpoint_of[offline]}" offline-final
list_works "$legacy_bin" "$old195" "${endpoint_of[old195]}" old195-final
list_denied "$se_bin" "$gone" "${endpoint_of[gone]}" gone-final

# The search is meaningful only if the old lookup id is found where the new one would be: the
# removed device's own profile still names the host's old data.
grep -rqF "$old_lookup" "$gone" || fail "positive control failed: the removed device's profile does not hold the old lookup id"
if grep -rqF "$new_lookup" "$gone"; then
  fail "the removed device holds the host's new lookup id"
fi
if grep -rqF "$new_code" "$gone"; then
  fail "the removed device holds the host's new direct code"
fi
final_status="$(host_status)"
jq -e '(.code_renewal.pending_names | index("gone")) == null' >/dev/null <<<"$final_status" \
  || fail "the removed device is listed as pending"

# E2, the bound (E17; MAX_RETIRING_CODES in code_renewal_state.rs = 63): on a fresh host every generation
# is the one of its own removals. Each step pairs a holder, which is stopped at once and so keeps the
# generation it took and stays pending, and a removal target, which is removed at once and holds nothing
# after its grant is gone. The 63 holders keep 63 retiring generations; the removal after them must wait
# (`deferred`), and no pending entry may disappear. The holders and targets are separate profiles.
stage="E2: the bound of 63 retiring generations"
e2_bound=63
e2_host="$root/e2-host"
prepare_profile "$e2_host"
profiles+=("$e2_host")
as "$se_bin" "$e2_host" share identity --set-name e2-host >"$root/e2-host-name.txt" 2>&1
as "$se_bin" "$e2_host" share export add "$root/export" --label Files >"$root/e2-host-export.txt"
bring_online "$se_bin" "$e2_host" e2-host

e2_status() {
  as "$se_bin" "$e2_host" share status --json
}

# e2_wait FILTER LABEL: the E2 host's status satisfies FILTER within converge_limit seconds.
e2_wait() {
  local filter="$1" label="$2"
  local deadline=$((SECONDS + converge_limit))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(e2_status 2>/dev/null)" && jq -e "$filter" >/dev/null <<<"$value"; then
      printf '%s\n' "$value" >"$root/e2-$label.json"
      return 0
    fi
    sleep 1
  done
  printf '%s\n' "$value" >"$root/e2-$label-timeout.json"
  fail "E2 host status never satisfied '$filter' ($label; last: $root/e2-$label-timeout.json)"
}

# e2_pair LABEL PROFILE: a fresh device paired with the E2 host at the host's current direct code; its
# worker is stopped again once the pairing is checked.
e2_pair() {
  local label="$1" profile="$2" code
  code="$(jq -er '.direct_code' <<<"$(as "$se_bin" "$e2_host" share identity --json)")"
  profiles+=("$profile")
  pair_host="$e2_host"
  pair_code="$code"
  pair "$label" "$se_bin" "$profile"
  unset pair_host pair_code
  stop_daemon "$profile"
}

# e2_remove LABEL: the E2 host removes the contact LABEL, in a second process of its profile.
e2_remove() {
  local label="$1" device_id
  device_id="$(as "$se_bin" "$e2_host" share grants --json \
    | jq -er --arg name "$label" '.grants[] | select(.device_name == $name and .grant_state == "accepted") | .device_id')"
  as "$se_bin" "$e2_host" share grants delete "$device_id" --json >"$root/e2-delete-$label.json"
  jq -e '.action == "removed_device" or .action == "deleted_grant"' >/dev/null <"$root/e2-delete-$label.json" \
    || fail "E2: the host did not remove $label"
}

for step in $(seq 1 "$e2_bound"); do
  e2_pair "e2-holder-$step" "$root/e2-holder-$step"
  e2_pair "e2-target-$step" "$root/e2-target-$step"
  e2_remove "e2-target-$step"
  e2_wait ".code_renewal.pending_devices == $step and .code_renewal.deferred == false" "step-$step"
done
e2_last=$((e2_bound + 1))
e2_pair "e2-target-$e2_last" "$root/e2-target-$e2_last"
e2_remove "e2-target-$e2_last"
e2_wait ".code_renewal.deferred == true and .code_renewal.pending_devices == $e2_bound" deferred
sleep 10
e2_wait ".code_renewal.deferred == true and .code_renewal.pending_devices == $e2_bound" deferred-settled
echo "E2-BOUND limit=$e2_bound pending=$e2_bound deferred=true (removal $e2_last waits)"

stage="complete"
echo "code renewal E2E passed: online and offline contacts took the new data, v0.5.195 kept the old data, the removed contact received nothing (renewal started by: $fallback)"
