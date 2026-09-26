#!/usr/bin/env bash
# End-to-end check of `se share discoverable` against a local Share server and
# of `se update` on a terminal-only installation fed from a local update feed.
# Linux only (daemon discovery reads /proc). The remote task suite runs it;
# never run it on the workstation.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
se_bin="${SMART_EXPLORER_SE_BINARY:-$repo_root/native/target/debug/se}"
server_bin="${SMART_EXPLORER_SHARE_SERVER_BINARY:-$repo_root/share-server/target/debug/se-share-server}"

for tool in jq timeout sha256sum stat readlink realpath; do
  command -v "$tool" >/dev/null || {
    echo "CLI discoverable/update E2E requires $tool" >&2
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

root="$(mktemp -d "${TMPDIR:-/tmp}/se-cli-discoverable-update.XXXXXX")"
root="$(realpath "$root")"
client_c="$root/c"
client_u="$root/u"
client_d="$root/d"
server_log="$root/share-server.log"
server_pid=""

trap 'echo "CLI discoverable/update E2E failed at line $LINENO: $BASH_COMMAND" >&2' ERR

cleanup() {
  local status=$?
  stop_daemon "$client_c" || true
  stop_daemon "$client_u" || true
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ $status -ne 0 ]]; then
    echo "CLI discoverable/update E2E failed; diagnostics: $root" >&2
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

# run_as CLIENT BINARY ARGS... runs one `se` in the client's private profile.
run_as() {
  local client="$1"
  local binary="$2"
  shift 2
  timeout --foreground --signal=TERM --kill-after=5s 150s env \
    HOME="$client/home" \
    USERPROFILE="$client/home" \
    XDG_DATA_HOME="$client/data" \
    XDG_CONFIG_HOME="$client/config" \
    XDG_RUNTIME_DIR="$client/runtime" \
    APPDATA="$client/data" \
    LOCALAPPDATA="$client/data" \
    SE_SHARE_RELAY_ONLY=1 \
    "$binary" "$@"
}

run_client() {
  local client="$1"
  shift
  run_as "$client" "$se_bin" "$@"
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
  [[ -z "$(daemon_pids "$client")" ]]
}

wait_connected() {
  local client="$1"
  local deadline=$((SECONDS + 90))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    if value="$(run_client "$client" share status --json 2>/dev/null)" \
      && jq -e '.worker.reachable and .worker.running and .worker.connected' \
        >/dev/null <<<"$value"; then
      return 0
    fi
    sleep 0.25
  done
  echo "Share worker of $client did not connect" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

# wait_offer CLIENT OFFER_ID STATE: the offer is listed with that state.
wait_offer() {
  local client="$1"
  local offer_id="$2"
  local state="$3"
  local deadline=$((SECONDS + 60))
  local value=""
  while [[ $SECONDS -lt $deadline ]]; do
    value="$(run_client "$client" share discoverable list --json)"
    if jq -e --arg id "$offer_id" --arg state "$state" \
      '[.offers[] | select(.offer_id == $id and .state == $state)] | length == 1' \
      >/dev/null <<<"$value"; then
      return 0
    fi
    sleep 0.5
  done
  echo "offer $offer_id did not reach state $state" >&2
  printf '%s\n' "$value" >&2
  return 1
}

# expect_failure OUTPUT_FILE MESSAGE COMMAND...: the command fails and its
# stderr names the expected reason.
expect_failure() {
  local output="$1"
  local message="$2"
  shift 2
  if "$@" >"$output.out" 2>"$output"; then
    echo "command unexpectedly succeeded: $*" >&2
    return 1
  fi
  grep -Fq -- "$message" "$output" || {
    echo "missing \"$message\" in the error of: $*" >&2
    cat "$output" >&2
    return 1
  }
}

no_update_leftovers() {
  local dir="$1"
  if compgen -G "$dir/*.update-*" >/dev/null; then
    ls -la "$dir" >&2
    echo "update left temporary files in $dir" >&2
    return 1
  fi
}

# ---------------------------------------------------------------------------
# se share discoverable
# ---------------------------------------------------------------------------
prepare_client "$client_c"
signal_port=$((34000 + ($$ % 12000)))
"$server_bin" "127.0.0.1:$signal_port" >"$server_log" 2>&1 &
server_pid=$!
sleep 0.5
kill -0 "$server_pid"

identity_c="$(run_client "$client_c" share identity --json)"
device_name="$(jq -er '.device_name' <<<"$identity_c")"
run_client "$client_c" share configure --server "127.0.0.1:$signal_port" >/dev/null
wait_connected "$client_c"

# Without a terminal and without --pin/--pin-stdin there is no PIN to use.
expect_failure "$root/no-pin.err" "no PIN given" \
  run_client "$client_c" share discoverable </dev/null

# The owner's one-line request: this device, 5 minutes, PIN 1454.
direct="$(run_client "$client_c" share discoverable --minutes 5 --pin 1454 --json)"
direct_id="$(jq -er '.offer.offer_id' <<<"$direct")"
jq -e --arg name "$device_name" '
  .action == "discoverable"
  and .offer.target.kind == "direct"
  and .offer.name == $name
  and .offer.remaining_secs > 270 and .offer.remaining_secs <= 300
  and (.offer.discoverable_until_local | length) > 0
  and (.offer.state == "published" or .offer.state == "prepared")' \
  >/dev/null <<<"$direct"
wait_offer "$client_c" "$direct_id" published

# The offer list is the daemon's own state, not the event stream: status and
# list agree, however often the events were read.
run_client "$client_c" share status --json >/dev/null
status_c="$(run_client "$client_c" share status --json)"
jq -e --arg id "$direct_id" '[.discoverable[] | select(.offer_id == $id)] | length == 1' \
  >/dev/null <<<"$status_c"
listed="$(run_client "$client_c" share discoverable list)"
grep -Fq "discoverable"$'\t'"$direct_id"$'\t'"state=published"$'\t'"target=direct" <<<"$listed"
grep -Fq "remaining=" <<<"$listed"

# One running offer per target, across all clients.
expect_failure "$root/conflict.err" "already discoverable" \
  run_client "$client_c" share discoverable --pin 1454

# A room, with the PIN from stdin instead of the command line. Only the first
# line is the PIN; a distinctive value lets the end of this part prove that it
# was written to no output, log or file.
room_create="$(run_client "$client_c" share room create --name Team)"
room_profile="$(awk -F '\t' '$1 == "room_id" { print $2 }' <<<"$room_create")"
[[ -n "$room_profile" ]]
probe_pin="pin-probe-5f3a9c"
room="$(printf '%s\nnot the pin\n' "$probe_pin" \
  | run_client "$client_c" share discoverable --room Team --pin-stdin --json 2>"$root/room.err")"
printf '%s\n' "$room" >"$root/room.out"
room_id="$(jq -er '.offer.offer_id' <<<"$room")"
jq -e --arg profile "$room_profile" '
  .offer.target.kind == "room"
  and .offer.target.room_profile_id == $profile
  and .offer.target.room_name == "Team"
  and .offer.name == "Team"' >/dev/null <<<"$room"
wait_offer "$client_c" "$room_id" published

# Stop one offer by a unique id prefix, then everything that is left.
stopped="$(run_client "$client_c" share discoverable stop "${room_id:0:8}" --json)"
jq -e --arg id "$room_id" '.stopped == [$id] and .failed == []' >/dev/null <<<"$stopped"
remaining="$(run_client "$client_c" share discoverable list --json)"
jq -e --arg id "$direct_id" '[.offers[].offer_id] == [$id]' >/dev/null <<<"$remaining"
stopped="$(run_client "$client_c" share discoverable stop --all --json)"
jq -e --arg id "$direct_id" '.stopped == [$id]' >/dev/null <<<"$stopped"
jq -e '.offers == []' >/dev/null <<<"$(run_client "$client_c" share discoverable list --json)"
nothing="$(run_client "$client_c" share discoverable stop)"
grep -Fq "no offer is discoverable" <<<"$nothing"

# An explicitly empty PIN is accepted with a warning; stopping the worker
# ends the offer instead of leaving it listed.
run_client "$client_c" share discoverable --pin "" --json >/dev/null 2>"$root/empty-pin.err"
grep -Fq "trivial to guess" "$root/empty-pin.err"
run_client "$client_c" share worker stop >/dev/null
# `worker stop` also turns Auto-Connect off; configuring starts it again.
run_client "$client_c" share configure --server "127.0.0.1:$signal_port" >/dev/null
wait_connected "$client_c"
jq -e '.offers == []' >/dev/null <<<"$(run_client "$client_c" share discoverable list --json)"

# The PIN never reaches any output, the daemon log, the profile/data files or
# the Share server log.
run_client "$client_c" share status >"$root/status.out"
for evidence in "$root/room.out" "$root/room.err" "$root/status.out" "$server_log"; do
  if grep -Fq "$probe_pin" "$evidence"; then
    echo "the PIN appeared in $evidence" >&2
    exit 1
  fi
done
if grep -rFq "$probe_pin" "$client_c/data" "$client_c/config" "$client_c/home"; then
  grep -rFl "$probe_pin" "$client_c/data" "$client_c/config" "$client_c/home" >&2
  echo "the PIN was written to a file of the client" >&2
  exit 1
fi
echo "se share discoverable passed"

# ---------------------------------------------------------------------------
# se update (terminal-only installation, local feed)
# ---------------------------------------------------------------------------
prepare_client "$client_u"
version="$("$se_bin" --version | awk '{ print $2 }')"
[[ -n "$version" ]]
install_dir="$root/u/opt/smart-explorer"
bin_dir="$root/u/bin"
installed="$install_dir/se"
mkdir -p "$install_dir" "$bin_dir"

# The installed file differs from the feed's bytes, so a replacement shows.
install_marked_old_se() {
  cp "$se_bin" "$installed.tmp"
  printf 'OLD-SE-MARKER' >>"$installed.tmp"
  chmod 755 "$installed.tmp"
  mv "$installed.tmp" "$installed"
}
install_marked_old_se
ln -sf "$installed" "$bin_dir/se"
old_sha="$(sha256sum "$installed" | awk '{ print $1 }')"

make_feed() {
  local dir="$1"
  local feed_version="$2"
  mkdir -p "$dir"
  printf '%s\n' "$feed_version" >"$dir/version.txt"
  cp "$se_bin" "$dir/se"
  # A download or checkout may lack execute bits; the installed mode wins.
  chmod 644 "$dir/se"
  (cd "$dir" && sha256sum se >se.sha256)
}
make_feed "$root/feed" "$version"
feed_sha="$(awk '{ print $1 }' "$root/feed/se.sha256")"

expect_failure "$root/no-feed.err" "no update feed is configured" \
  run_as "$client_u" "$bin_dir/se" update --check

checked="$(run_as "$client_u" "$bin_dir/se" update --source "$root/feed" --check --json)"
jq -e --arg version "$version" --arg cli "$installed" '
  .status == "up_to_date"
  and .update_available == false
  and .current_version == $version
  and .feed_version == $version
  and .installation.kind == "terminal_only"
  and .installation.cli == $cli' >/dev/null <<<"$checked"
plain="$(run_as "$client_u" "$bin_dir/se" update)"
grep -Fq "status"$'\t'"up_to_date" <<<"$plain"
[[ "$(sha256sum "$installed" | awk '{ print $1 }')" == "$old_sha" ]]

# A source that does not answer is refused and does not replace the saved one.
expect_failure "$root/bad-source.err" "is not usable" \
  run_as "$client_u" "$bin_dir/se" update --source "$root/missing-feed" --check
kept="$(run_as "$client_u" "$bin_dir/se" update --check --json)"
jq -e --arg source "$root/feed" '.source == $source' >/dev/null <<<"$kept"

# One se update at a time per installed file.
printf '%s' "$$" >"$install_dir/se.update-lock"
expect_failure "$root/locked.err" "another se update" \
  run_as "$client_u" "$bin_dir/se" update --reinstall
rm "$install_dir/se.update-lock"
[[ "$(sha256sum "$installed" | awk '{ print $1 }')" == "$old_sha" ]]

# Leftovers of an update process that no longer runs are cleared.
printf 'x' >"$install_dir/se.update-old.4294967294.1"
printf 'x' >"$install_dir/se.update-pending.4294967294.2"

# Reinstall through the link: the resolved file is replaced, the link stays.
reinstalled="$(run_as "$client_u" "$bin_dir/se" update --reinstall --json)"
jq -e --arg cli "$installed" --arg sha "$feed_sha" '
  .status == "reinstalled"
  and .installed.path == $cli
  and .installed.sha256 == $sha
  and .installed.worker == "not_running"
  and .installed.worker_error == null' >/dev/null <<<"$reinstalled"
[[ "$(sha256sum "$installed" | awk '{ print $1 }')" == "$feed_sha" ]]
[[ -L "$bin_dir/se" && "$(readlink "$bin_dir/se")" == "$installed" ]]
[[ "$(stat -c '%a' "$installed")" == 755 ]]
no_update_leftovers "$install_dir"
[[ -z "$(daemon_pids "$client_u")" ]]

# A running worker of the same version stays; the new se reports it current.
run_as "$client_u" "$bin_dir/se" share status --json >/dev/null
[[ -n "$(daemon_pids "$client_u")" ]]
current="$(run_as "$client_u" "$bin_dir/se" update --reinstall --json)"
jq -e '.installed.worker == "current" and .installed.worker_error == null' \
  >/dev/null <<<"$current"
[[ -n "$(daemon_pids "$client_u")" ]]
stop_daemon "$client_u"

# A payload that does not match its published hash never replaces anything.
make_feed "$root/feed-bad" "$version"
printf '%064d  se\n' 0 >"$root/feed-bad/se.sha256"
install_marked_old_se
expect_failure "$root/bad-hash.err" "passt nicht" \
  run_as "$client_u" "$bin_dir/se" update --source "$root/feed-bad" --reinstall
[[ "$(sha256sum "$installed" | awk '{ print $1 }')" == "$old_sha" ]]
no_update_leftovers "$install_dir"

# A new se that does not start is replaced by the previous file again.
mkdir -p "$root/feed-broken"
printf '%s\n' "$version" >"$root/feed-broken/version.txt"
printf '#!/bin/sh\nexit 1\n' >"$root/feed-broken/se"
(cd "$root/feed-broken" && sha256sum se >se.sha256)
expect_failure "$root/broken.err" "did not start correctly" \
  run_as "$client_u" "$bin_dir/se" update --source "$root/feed-broken" --reinstall
grep -Fq "the previous se was restored" "$root/broken.err"
[[ "$(sha256sum "$installed" | awk '{ print $1 }')" == "$old_sha" ]]
[[ "$(stat -c '%a' "$installed")" == 755 ]]
no_update_leftovers "$install_dir"

# The feed announces a version its se does not report: the new file is
# rejected after the swap and the previous file comes back.
make_feed "$root/feed-next" 999.0.0
next="$(run_as "$client_u" "$bin_dir/se" update --source "$root/feed-next" --check --json)"
jq -e '.status == "update_available" and .feed_version == "999.0.0"' >/dev/null <<<"$next"
expect_failure "$root/mismatch.err" "the previous se was restored" \
  run_as "$client_u" "$bin_dir/se" update
grep -Fq "instead of 999.0.0" "$root/mismatch.err"
[[ "$(sha256sum "$installed" | awk '{ print $1 }')" == "$old_sha" ]]
[[ "$(stat -c '%a' "$installed")" == 755 ]]
no_update_leftovers "$install_dir"
[[ -z "$(daemon_pids "$client_u")" ]]

# se beside the desktop app is a desktop installation (checked, not applied).
prepare_client "$client_d"
desktop_dir="$root/d/opt/smart-explorer"
mkdir -p "$desktop_dir"
cp "$se_bin" "$desktop_dir/se"
printf 'app' >"$desktop_dir/smart_explorer"
desktop="$(run_as "$client_d" "$desktop_dir/se" update --source "$root/feed" --check --json)"
jq -e --arg app "$desktop_dir/smart_explorer" --arg cli "$desktop_dir/se" '
  .installation.kind == "desktop"
  and .installation.app == $app
  and .installation.cli == $cli' >/dev/null <<<"$desktop"
expect_failure "$root/desktop-reinstall.err" "terminal-only installation" \
  run_as "$client_d" "$desktop_dir/se" update --reinstall
# Without a graphical session the helper could not restart the app, so the
# desktop update is refused before anything is downloaded.
expect_failure "$root/desktop-headless.err" "no graphical session" \
  run_as "$client_d" env -u DISPLAY -u WAYLAND_DISPLAY \
  "$desktop_dir/se" update --source "$root/feed-next"
if compgen -G "$client_d/data/smart_explorer/*_download_*" >/dev/null; then
  echo "a refused desktop update staged payloads" >&2
  exit 1
fi
echo "se update passed"
