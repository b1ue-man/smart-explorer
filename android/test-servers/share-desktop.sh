#!/usr/bin/env bash
# Desktop side of the G5 Share check, called by android/test-android-task.sh (emulator-run).
# Follows native/test-share-room-e2e.sh: a local se-share-server (signaling + Iroh relay on the
# next port) and one headless desktop CLI client (`se`) with its own HOME/XDG directories. The
# server binds the runner's non-loopback address so the runner and the emulator reach the same
# signaling and relay endpoints. Linux only (daemon discovery reads /proc).
#
#   share-desktop.sh up ROOT SE_BIN SHARE_SERVER_BIN   start server + client, Room "Team" with a
#                                                      Room-only export; writes ROOT/state.env
#   share-desktop.sh args ROOT                          instrumentation arguments (-e name value)
#   share-desktop.sh members ROOT COUNT                 wait until the desktop sees COUNT members
#   share-desktop.sh exec ROOT [INSTRUMENT_OUT]         desktop side of the exec-host check, while
#                                                      ShareExecTaskTest runs on the phone (adb)
#   share-desktop.sh down ROOT LOGDIR                   stop client daemon and server, keep logs
#
# Each call is its own bash process with errexit, so a failed step always fails the call.
set -Eeuo pipefail
trap 'echo "share-desktop.sh failed at line $LINENO: $BASH_COMMAND" >&2' ERR

SHARE_ROOT=""
SHARE_CLIENT=""
SHARE_SERVER_PID=""
SHARE_SERVER=""
SHARE_RELAY=""
SHARE_ROOM_CODE=""
SHARE_ROOM_RELATION=""
SHARE_DESKTOP_DEVICE=""
SHARE_DESKTOP_DIRECT_CODE=""
SHARE_ROOM_FOLDER=RoomDocs
SHARE_ROOM_FILE=vom-desktop.bin
SHARE_ROOM_FILE_SHA256=""
SE_BIN=""
SE_SHARE_SERVER_BIN=""

# The address of the default route, not 127.0.0.1 and not the Docker bridge.
share_runner_ip() {
  ip -4 route get 1.1.1.1 2>/dev/null | awk '{ for (i = 1; i < NF; i++) if ($i == "src") { print $(i + 1); exit } }'
}

# --preserve-status: a client killed by this limit ends with 143/137, so the CLI's own exit codes
# (e.g. `se exec`: 124 remote time limit, 125 refused/revoked, 130 cancelled) stay unambiguous.
share_client() {
  timeout --foreground --preserve-status --signal=TERM --kill-after=5s "${SHARE_CLIENT_TIMEOUT:-90s}" env \
    HOME="$SHARE_CLIENT/home" \
    USERPROFILE="$SHARE_CLIENT/home" \
    XDG_DATA_HOME="$SHARE_CLIENT/data" \
    XDG_CONFIG_HOME="$SHARE_CLIENT/config" \
    XDG_RUNTIME_DIR="$SHARE_CLIENT/runtime" \
    APPDATA="$SHARE_CLIENT/data" \
    LOCALAPPDATA="$SHARE_CLIENT/data" \
    SE_SHARE_RELAY_ONLY=1 \
    "$SE_BIN" "$@"
}

share_daemon_pids() {
  local expected="XDG_DATA_HOME=$SHARE_CLIENT/data" env_file pid command
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

share_stop_daemon() {
  local pid deadline=$((SECONDS + 10))
  while read -r pid; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; fi
  done < <(share_daemon_pids)
  while ((SECONDS < deadline)) && [[ -n "$(share_daemon_pids)" ]]; do
    sleep 0.1
  done
  [[ -z "$(share_daemon_pids)" ]] || {
    echo "desktop Share daemon did not stop" >&2
    return 1
  }
}

share_wait_relay_route() {
  local deadline=$((SECONDS + 120)) value=""
  while ((SECONDS < deadline)); do
    if value="$(share_client share status --json 2>/dev/null)" &&
      jq -e --arg relay "$SHARE_RELAY" \
        '.worker.reachable == true and .worker.running == true and .worker.connected == true and
         ((.worker.relay_url | rtrimstr("/")) == ($relay | rtrimstr("/")))' >/dev/null <<<"$value"; then
      return 0
    fi
    sleep 0.5
  done
  echo "desktop client did not reach the relay $SHARE_RELAY" >&2
  [[ -z "$value" ]] || printf '%s\n' "$value" >&2
  return 1
}

# Room members as the desktop sees them (a count in the CLI's JSON).
share_desktop_members() {
  share_client share status --json | jq -r --arg room "$SHARE_ROOM_RELATION" \
    '[.rooms[] | select(.room_id == $room)] | if length == 1 then .[0].members else -1 end'
}

share_wait_members() {
  local expected=$1 deadline=$((SECONDS + 180)) members=""
  while ((SECONDS < deadline)); do
    share_client share worker refresh >/dev/null 2>&1 || true
    members="$(share_desktop_members 2>/dev/null || true)"
    if [[ "$members" == "$expected" ]]; then
      echo "desktop sees $members member(s) in room $SHARE_ROOM_RELATION"
      return 0
    fi
    sleep 1
  done
  echo "desktop did not see $expected member(s) in room $SHARE_ROOM_RELATION (last: '$members')" >&2
  return 1
}

# Server, client, Room "Team" with a Room-only export RoomDocs holding one test file.
share_desktop_up() {
  local root=$1 port ip identity room_create export_dir
  SE_BIN=$2
  SE_SHARE_SERVER_BIN=$3
  SHARE_ROOT=$root
  SHARE_CLIENT="$root/desktop"
  mkdir -p "$SHARE_CLIENT/home" "$SHARE_CLIENT/data" "$SHARE_CLIENT/config" "$SHARE_CLIENT/runtime"
  chmod 700 "$SHARE_CLIENT/home" "$SHARE_CLIENT/data" "$SHARE_CLIENT/config" "$SHARE_CLIENT/runtime"
  ip="$(share_runner_ip)"
  [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ && "$ip" != 127.* ]] || {
    echo "no non-loopback IPv4 address on the runner (got '$ip')" >&2
    return 1
  }
  port=$((34000 + ($$ % 12000)))
  SHARE_SERVER="$ip:$port"
  SHARE_RELAY="http://$ip:$((port + 1))"
  "$SE_SHARE_SERVER_BIN" "$SHARE_SERVER" >"$root/share-server.log" 2>&1 &
  SHARE_SERVER_PID=$!
  share_save_state
  sleep 1
  kill -0 "$SHARE_SERVER_PID" || {
    cat "$root/share-server.log" >&2
    return 1
  }

  identity="$(share_client share identity --json)"
  SHARE_DESKTOP_DEVICE="$(jq -er '.device_id' <<<"$identity")"
  SHARE_DESKTOP_DIRECT_CODE="$(jq -er '.direct_code' <<<"$identity")"
  share_client share configure --server "$SHARE_SERVER" >/dev/null
  share_stop_daemon
  share_client share status --json >/dev/null
  share_wait_relay_route

  room_create="$(share_client share room create --name Team)"
  SHARE_ROOM_CODE="$(awk -F '\t' '$1 == "room_code" { print $2 }' <<<"$room_create")"
  [[ "$SHARE_ROOM_CODE" == SE-R3-*-* ]] || {
    echo "room create printed no invite code: $room_create" >&2
    return 1
  }
  SHARE_ROOM_RELATION="${SHARE_ROOM_CODE#SE-R3-}"
  SHARE_ROOM_RELATION="${SHARE_ROOM_RELATION%-*}"

  # Room-only export: remove the Direct defaults a new Room inherits (see the E2E script).
  export_dir="$SHARE_CLIENT/home/room-export"
  mkdir -p "$export_dir"
  head -c 1048576 /dev/urandom >"$export_dir/$SHARE_ROOM_FILE"
  SHARE_ROOM_FILE_SHA256="$(sha256sum "$export_dir/$SHARE_ROOM_FILE" | awk '{ print $1 }')"
  share_client share export add "$export_dir" --label "$SHARE_ROOM_FOLDER" --room Team >/dev/null
  local inherited
  while IFS= read -r inherited; do
    [[ -n "$inherited" ]] || continue
    share_client share export remove "$inherited" --room Team >/dev/null
  done < <(share_client share export list --room Team --json | jq -r --arg keep "$SHARE_ROOM_FOLDER" '.roots[] | select(.label != $keep) | .label')
  share_client share worker refresh >/dev/null
  share_client share export list --room Team --json |
    jq -e --arg keep "$SHARE_ROOM_FOLDER" '.roots | length == 1 and .[0].label == $keep' >/dev/null
  share_save_state
  echo "desktop Share ready: server $SHARE_SERVER, room $SHARE_ROOM_RELATION, device $SHARE_DESKTOP_DEVICE"
}

# Exec host (phase A2): markers on the phone's primary volume coordinate with ShareExecTaskTest.
SHARE_EXEC_MARKERS=/sdcard/SmartExplorerTask/exec-host
# Output of the phone's `am instrument` run: once it ends, no marker will come.
SHARE_EXEC_INSTRUMENT=""
SHARE_EXEC_TARGET=""

share_marker_wait() {
  local name=$1 seconds=$2 deadline=$((SECONDS + $2)) value=""
  while ((SECONDS < deadline)); do
    value="$(adb shell cat "$SHARE_EXEC_MARKERS/$name" 2>/dev/null | tr -d '\r' || true)"
    if [[ -n "$value" ]]; then
      printf '%s\n' "$value"
      return 0
    fi
    if [[ -n "$SHARE_EXEC_INSTRUMENT" ]] && grep -q 'INSTRUMENTATION_CODE' "$SHARE_EXEC_INSTRUMENT" 2>/dev/null; then
      echo "the phone test ended before writing marker $name" >&2
      return 1
    fi
    sleep 2
  done
  echo "phone marker $name did not appear within ${seconds}s" >&2
  return 1
}

# A command the phone ends (cancel, time limit, revocation): the CLI must return the matching
# exit code (cli/exec.rs exit_code) well before this script's own limit.
share_exec_expect_end() {
  local step=$1 what=$2 limit=$3 expected=$4 code=0
  shift 4
  SHARE_CLIENT_TIMEOUT=$limit share_client exec "$SHARE_EXEC_TARGET" "$@" \
    >"$SHARE_ROOT/exec-$step.out" 2>"$SHARE_ROOT/exec-$step.err" || code=$?
  echo "exec $step ($what): exit $code"
  [[ "$code" -eq "$expected" ]] || {
    echo "exec $step was not ended on the phone as expected (exit $code, expected $expected)" >&2
    cat "$SHARE_ROOT/exec-$step.out" "$SHARE_ROOT/exec-$step.err" >&2
    return 1
  }
}

share_exec_check() {
  local phone target out code deadline verdict=not-refused
  phone="$(share_marker_wait granted 900)"
  phone="${phone%%$'\n'*}"
  [[ "$phone" =~ ^[^/[:space:]]+$ ]] || {
    echo "unexpected phone device id '$phone'" >&2
    return 1
  }
  target="share://room/$SHARE_ROOM_RELATION/$phone"
  SHARE_EXEC_TARGET=$target
  echo "exec target $target"

  # 1. Allowed: the command runs in the phone's shell (retried while presence and grant spread).
  deadline=$((SECONDS + 240))
  while ((SECONDS < deadline)); do
    code=0
    out="$(share_client exec "$target" --timeout 60 --shell 'echo exec-ok-$((6*7))' 2>"$SHARE_ROOT/exec-1.err")" || code=$?
    [[ "$code" -eq 0 && "$out" == "exec-ok-42" ]] && break
    sleep 3
  done
  echo "exec 1 (allowed): exit $code, stdout '$out'"
  [[ "$code" -eq 0 && "$out" == "exec-ok-42" ]] || {
    echo "the allowed command did not run on the phone" >&2
    cat "$SHARE_ROOT/exec-1.err" >&2
    return 1
  }

  # 2. A shell that leaves a setsid child and a double-forked orphan; the phone cancels it and
  #    must end the whole tree (ShareExecTaskTest checks /proc).
  share_exec_expect_end 2 "cancelled on the phone" 300s 130 --shell 'setsid sleep 302 & (sleep 303 &); sleep 301'

  # 3. The same kind of tree with a remote time limit.
  share_exec_expect_end 3 "timed out on the phone" 120s 124 --timeout 5 --shell 'setsid sleep 304 & (sleep 305 &); sleep 306'

  # 4. A running command while the phone revokes the grant.
  share_exec_expect_end 4 "revoked on the phone" 300s 125 --shell 'setsid sleep 307 & sleep 308'

  # 5. Revoked: the phone refuses the next attempt (not a transport failure).
  share_marker_wait revoked 300 >/dev/null
  deadline=$((SECONDS + 120))
  while ((SECONDS < deadline)); do
    code=0
    share_client exec "$target" --timeout 30 --shell 'echo darf-nicht-laufen' \
      >"$SHARE_ROOT/exec-5.out" 2>"$SHARE_ROOT/exec-5.err" || code=$?
    if [[ "$code" -eq 0 ]]; then
      verdict=ran
      break
    fi
    if grep -Eq 'permission_denied|exec authentication failed' "$SHARE_ROOT/exec-5.err"; then
      verdict=refused
      break
    fi
    sleep 3
  done
  echo "exec 5 (after the revocation): exit $code, verdict $verdict"
  adb shell "echo $verdict >$SHARE_EXEC_MARKERS/host-done"
  [[ "$verdict" == refused ]] || {
    echo "the command after the revocation was not refused" >&2
    cat "$SHARE_ROOT/exec-5.out" "$SHARE_ROOT/exec-5.err" >&2
    return 1
  }
}

STATE_VARS=(SHARE_ROOT SHARE_CLIENT SHARE_SERVER_PID SHARE_SERVER SHARE_RELAY SHARE_ROOM_CODE SHARE_ROOM_RELATION
  SHARE_DESKTOP_DEVICE SHARE_DESKTOP_DIRECT_CODE SHARE_ROOM_FOLDER SHARE_ROOM_FILE SHARE_ROOM_FILE_SHA256 SE_BIN SE_SHARE_SERVER_BIN)

share_save_state() {
  local name
  for name in "${STATE_VARS[@]}"; do
    printf '%s=%q\n' "$name" "${!name}"
  done >"$SHARE_ROOT/state.env"
}

share_load_state() {
  local root=$1
  [[ -f "$root/state.env" ]] || {
    echo "no desktop Share state in $root" >&2
    return 1
  }
  # shellcheck disable=SC1091
  source "$root/state.env"
}

share_instrumentation_args() {
  printf '%s\n' \
    -e seShareServer "$SHARE_SERVER" \
    -e seRoomCode "$SHARE_ROOM_CODE" \
    -e seDesktopDevice "$SHARE_DESKTOP_DEVICE" \
    -e seDesktopDirectCode "$SHARE_DESKTOP_DIRECT_CODE" \
    -e seRoomFolder "$SHARE_ROOM_FOLDER" \
    -e seRoomFile "$SHARE_ROOM_FILE" \
    -e seRoomFileSha256 "$SHARE_ROOM_FILE_SHA256"
}

share_desktop_down() {
  local logs=$1
  mkdir -p "$logs"
  if [[ -n "$SHARE_CLIENT" && -d "$SHARE_CLIENT" ]]; then
    share_client share status --json >"$logs/desktop-share-status.json" 2>&1 || true
    share_stop_daemon || true
  fi
  if [[ -n "$SHARE_SERVER_PID" ]]; then
    kill "$SHARE_SERVER_PID" 2>/dev/null || true
    local deadline=$((SECONDS + 10))
    while kill -0 "$SHARE_SERVER_PID" 2>/dev/null && ((SECONDS < deadline)); do sleep 0.2; done
  fi
  [[ -z "$SHARE_ROOT" ]] || cp -- "$SHARE_ROOT/share-server.log" "$logs/" 2>/dev/null || true
}

case "${1:-}" in
  up)
    [[ "$#" -eq 4 ]] || { sed -n '8,14p' "$0" >&2; exit 2; }
    mkdir -p "$2"
    share_desktop_up "$2" "$3" "$4"
    ;;
  args)
    [[ "$#" -eq 2 ]] || exit 2
    share_load_state "$2"
    share_instrumentation_args
    ;;
  members)
    [[ "$#" -eq 3 ]] || exit 2
    share_load_state "$2"
    share_wait_members "$3"
    ;;
  exec)
    [[ "$#" -eq 2 || "$#" -eq 3 ]] || exit 2
    share_load_state "$2"
    SHARE_EXEC_INSTRUMENT="${3:-}"
    share_exec_check
    ;;
  down)
    [[ "$#" -eq 3 ]] || exit 2
    share_load_state "$2" || exit 0
    share_desktop_down "$3"
    ;;
  *)
    sed -n '8,14p' "$0" >&2
    exit 2
    ;;
esac
