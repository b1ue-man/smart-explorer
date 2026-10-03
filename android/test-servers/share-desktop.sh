#!/usr/bin/env bash
# Desktop side of the G5 Share check, called by android/test-android-task.sh (emulator-run).
# Uses a local TLS se-share-server (signaling + HTTPS Iroh relay on the
# next port) and one headless desktop CLI client (`se`) with its own HOME/XDG directories. The
# server binds the runner's non-loopback address so the runner and the emulator reach the same
# signaling and relay endpoints. Linux only (daemon discovery reads /proc).
#
#   share-desktop.sh up ROOT SE_BIN SHARE_SERVER_BIN   start server + client, Room "Team" with a
#                                                      Room-only export; writes ROOT/state.env
#   share-desktop.sh args ROOT                          instrumentation arguments (-e name value)
#   share-desktop.sh accept ROOT SECONDS INSTRUMENT_OUT accept exactly the current RV1 phone request
#   share-desktop.sh members ROOT COUNT                 wait until the desktop sees COUNT members
#   share-desktop.sh exec ROOT [INSTRUMENT_OUT]         desktop side of the exec-host check, while
#                                                      ShareExecTaskTest runs on the phone (adb)
#   share-desktop.sh reach ROOT PHONE_DEVICE SECONDS    the desktop lists the phone's Room exports
#                                                      (phone in the background / Doze); retries
#                                                      until SECONDS while routes settle
#   share-desktop.sh down ROOT LOGDIR                   stop client daemon and server, keep logs
#
# Each call is its own bash process with errexit, so a failed step always fails the call.
set -Eeuo pipefail
umask 077
trap 'share_failed "$?" "$LINENO" "$BASH_COMMAND"' ERR

SHARE_ROOT=""
SHARE_CLIENT=""
SHARE_SERVER_PID=""
SHARE_SERVER_STAMP=""
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
SHARE_STATE_VERSION=2
SHARE_READY=0
SHARE_UP_OWNER=""

share_failed() {
  local status=$1 line=$2 command=$3
  echo "share-desktop.sh failed at line $line: $command" >&2
  if [[ -n "$SHARE_UP_OWNER" && "$SHARE_UP_OWNER" == "$BASHPID" ]]; then
    SHARE_UP_OWNER=""
    share_desktop_down "$SHARE_ROOT/failure-logs" || true
  fi
  exit "$status"
}

# A PID alone may be reused between up/down; retain its Linux start-time identity.
share_process_stamp() {
  local pid=$1 stat
  local -a fields
  [[ "$pid" =~ ^[1-9][0-9]*$ && -r "/proc/$pid/stat" ]] || return 0
  IFS= read -r stat <"/proc/$pid/stat" 2>/dev/null || return 0
  stat="${stat##*) }"
  read -r -a fields <<<"$stat"
  [[ "${fields[0]:-Z}" != Z ]] || return 0
  printf '%s\n' "${fields[19]:-}"
}

share_signal_process() {
  local signal=$1 pid=$2 stamp=$3 stat
  [[ "$pid" != "$BASHPID" ]] || return 0
  [[ -n "$stamp" && "$(share_process_stamp "$pid")" == "$stamp" ]] || return 0
  IFS= read -r stat <"/proc/$pid/stat" 2>/dev/null || return 0
  stat="${stat##*) }"
  local -a fields
  read -r -a fields <<<"$stat"
  if [[ "${fields[2]:-}" == "$pid" ]]; then
    kill -s "$signal" -- "-$pid" 2>/dev/null || true
  else
    kill -s "$signal" "$pid" 2>/dev/null || true
  fi
}

share_stop_process() {
  local pid=$1 stamp=$2 label=$3 deadline=$((SECONDS + 10))
  [[ -n "$stamp" ]] || return 0
  share_signal_process TERM "$pid" "$stamp"
  while [[ "$(share_process_stamp "$pid")" == "$stamp" ]] && ((SECONDS < deadline)); do sleep 0.1; done
  if [[ "$(share_process_stamp "$pid")" == "$stamp" ]]; then
    share_signal_process KILL "$pid" "$stamp"
    deadline=$((SECONDS + 3))
    while [[ "$(share_process_stamp "$pid")" == "$stamp" ]] && ((SECONDS < deadline)); do sleep 0.1; done
  fi
  [[ "$(share_process_stamp "$pid")" != "$stamp" ]] || {
    echo "$label did not stop" >&2
    return 1
  }
}

share_register_helper() {
  local pid=$BASHPID stamp owner
  stamp="$(share_process_stamp "$pid")"
  [[ -n "$stamp" ]] || return 1
  owner="$SHARE_ROOT/helper-owner-$pid-$stamp"
  mkdir -m 700 "$owner"
  printf '%s %s\n' "$pid" "$stamp" >"$owner/process"
}

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
    SE_SHARE_FIXTURE_ROOT="$SHARE_ROOT" \
    SE_SHARE_RELAY_ONLY=1 \
    "$SE_BIN" "$@"
}

share_daemon_pids() {
  local expected="XDG_DATA_HOME=$SHARE_CLIENT/data" env_file pid command
  for env_file in /proc/[0-9]*/environ; do
    [[ -r "$env_file" ]] || continue
    if tr '\0' '\n' 2>/dev/null <"$env_file" | grep -Fx "$expected" >/dev/null; then
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
  local pid stamp failed=0
  while read -r pid; do
    [[ -n "$pid" ]] || continue
    stamp="$(share_process_stamp "$pid")"
    share_stop_process "$pid" "$stamp" "desktop Share daemon" || failed=1
  done < <(share_daemon_pids)
  return "$failed"
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
  local root=$1 port ip identity room_create room_policy export_dir cert key pin bind tool deadline
  for tool in ip jq timeout openssl python3 setsid sha256sum realpath; do
    command -v "$tool" >/dev/null || { echo "required fixture tool missing: $tool" >&2; return 1; }
  done
  SE_BIN="$(realpath -- "$2")"
  SE_SHARE_SERVER_BIN="$(realpath -- "$3")"
  [[ -x "$SE_BIN" && -x "$SE_SHARE_SERVER_BIN" ]] || return 1
  root="$(cd -- "$root" && pwd -P)"
  [[ ! -e "$root/state.env" && ! -L "$root/state.env" && ! -e "$root/desktop" && ! -L "$root/desktop" &&
     ! -e "$root/tls" && ! -L "$root/tls" ]] || {
    echo "up needs a fresh fixture root; existing state is preserved in $root" >&2
    return 1
  }
  SHARE_ROOT=$root
  SHARE_CLIENT="$root/desktop"
  mkdir -m 700 "$SHARE_CLIENT" "$root/tls"
  SHARE_UP_OWNER=$BASHPID
  mkdir -m 700 "$SHARE_CLIENT/home" "$SHARE_CLIENT/data" "$SHARE_CLIENT/config" "$SHARE_CLIENT/runtime"
  ip="$(share_runner_ip)"
  [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ && "$ip" != 127.* ]] || {
    echo "no non-loopback IPv4 address on the runner (got '$ip')" >&2
    return 1
  }
  # Discover an available adjacent pair. A later bind race fails with retained logs.
  port="$(python3 - "$ip" <<'PY'
import socket, sys
for _ in range(128):
    with socket.socket() as signal, socket.socket() as relay:
        signal.bind((sys.argv[1], 0))
        port = signal.getsockname()[1]
        if port == 65535:
            continue
        try:
            relay.bind((sys.argv[1], port + 1))
        except OSError:
            continue
        print(port)
        break
else:
    raise SystemExit("no available adjacent signaling/relay ports")
PY
)"
  cert="$root/tls/server.pem"
  key="$root/tls/server-key.pem"
  openssl req -x509 -newkey rsa:2048 -sha256 -days 2 -nodes \
    -keyout "$key" -out "$cert" -subj '/CN=Smart Explorer RV1 fixture' \
    -addext "subjectAltName=IP:$ip" -addext 'basicConstraints=critical,CA:FALSE' \
    >"$root/tls-generation.log" 2>&1
  pin="$(openssl x509 -in "$cert" -outform DER | sha256sum | awk '{ print $1 }')"
  [[ "$pin" =~ ^[0-9a-f]{64}$ ]] || return 1
  bind="$ip:$port"
  SHARE_SERVER="wss://$bind/#sha256=$pin"
  SHARE_RELAY="https://$ip:$((port + 1))"
  share_save_state
  # The shortest idle keepalive the server accepts, so the device suite sees several server
  # keepalives of the backgrounded phone within its time budget (default 180 s).
  setsid env SE_SHARE_IDLE_KEEPALIVE_SECS=30 SE_SHARE_ALLOW_PLAINTEXT=0 \
    SE_SHARE_REQUIRE_KEY_LOGIN=0 SE_IROH_RELAY_DISABLE=0 SE_IROH_RELAY_BIND="$ip:$((port + 1))" \
    SE_SHARE_FIXTURE_ROOT="$root" "$SE_SHARE_SERVER_BIN" "$bind" \
    --tls-cert "$cert" --tls-key "$key" --state-file "$root/server-bindings.json" \
    >"$root/share-server.log" 2>&1 &
  SHARE_SERVER_PID=$!
  SHARE_SERVER_STAMP="$(share_process_stamp "$SHARE_SERVER_PID")"
  [[ -n "$SHARE_SERVER_STAMP" ]] || { cat "$root/share-server.log" >&2; return 1; }
  share_save_state
  deadline=$((SECONDS + 30))
  until grep -Fq "signaling on $bind (TLS WebSocket" "$root/share-server.log" &&
        grep -Fq "relay listening on $SHARE_RELAY" "$root/share-server.log"; do
    [[ "$(share_process_stamp "$SHARE_SERVER_PID")" == "$SHARE_SERVER_STAMP" && SECONDS -lt deadline ]] || {
      cat "$root/share-server.log" >&2
      return 1
    }
    sleep 0.1
  done

  identity="$(share_client share identity --json)"
  SHARE_DESKTOP_DEVICE="$(jq -er '.device_id' <<<"$identity")"
  SHARE_DESKTOP_DIRECT_CODE="$(jq -er '.direct_code' <<<"$identity")"
  share_client share configure --server "$SHARE_SERVER" >/dev/null
  share_stop_daemon
  share_client share status --json >/dev/null
  share_wait_relay_route
  share_client share request list --json >"$root/direct-initial.json"
  jq -e '.count == 0 and (.requests | length) == 0 and (.legacy_requests | length) == 0' \
    "$root/direct-initial.json" >/dev/null

  room_create="$(share_client share room create --name Team)"
  SHARE_ROOM_CODE="$(awk -F '\t' '$1 == "room_code" { print $2 }' <<<"$room_create")"
  [[ "$SHARE_ROOM_CODE" == SE-R3-*-* ]] || {
    echo "room create printed no invite code: $room_create" >&2
    return 1
  }
  SHARE_ROOM_RELATION="${SHARE_ROOM_CODE#SE-R3-}"
  SHARE_ROOM_RELATION="${SHARE_ROOM_RELATION%-*}"
  # Explicit fixture admission grants Room reading; leave write and exec unchanged.
  room_policy="$(share_client share export policy --room Team --confirm-new-members false)"
  printf '%s\n' "$room_policy" >"$root/room-policy.txt"
  awk '$1 == "room_policy" && $3 == "members_may_write=false" &&
    $4 ~ /^confirm_new_members=false/ { found=1 } END { exit !found }' "$root/room-policy.txt"

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
    jq -e --arg keep "$SHARE_ROOM_FOLDER" \
      '.roots | length == 1 and .[0].label == $keep and .[0].access == "read_only" and
       .[0].allow_system_writes != true' >/dev/null
  SHARE_READY=1
  share_save_state
  SHARE_UP_OWNER=""
  echo "desktop Share ready: server $SHARE_SERVER, room $SHARE_ROOM_RELATION, device $SHARE_DESKTOP_DEVICE"
}

# One bounded foreground companion of ReviewShareTaskTest. The suite owns stop/wait.
share_accept_phone_request() {
  local seconds=$1 instrument=$2 deadline request fingerprint remaining inbox
  [[ "$seconds" =~ ^[1-9][0-9]{0,3}$ && -n "$instrument" && "$SHARE_READY" == 1 ]] || return 2
  [[ ! -e "$SHARE_ROOT/direct-accepted.json" ]] || {
    echo "this fixture has already accepted its phone request" >&2
    return 1
  }
  mkdir -m 700 "$SHARE_ROOT/direct-accept-owner"
  jq -e '.count == 0' "$SHARE_ROOT/direct-initial.json" >/dev/null
  deadline=$((SECONDS + seconds))
  inbox="$SHARE_ROOT/direct-inbox.json"
  while ((SECONDS < deadline)); do
    if grep -q 'INSTRUMENTATION_CODE' "$instrument" 2>/dev/null; then
      echo "phone instrumentation ended before its Direct request was accepted" >&2
      return 1
    fi
    remaining=$((deadline - SECONDS))
    ((remaining <= 10)) || remaining=10
    if SHARE_CLIENT_TIMEOUT="${remaining}s" share_client share request --json \
        >"$inbox" 2>>"$SHARE_ROOT/direct-inbox.err"; then
      jq -e '.count == (.requests | length) + (.legacy_requests | length) and
        (.count | type) == "number" and (.acceptable_count | type) == "number"' "$inbox" >/dev/null
      if jq -e '.count > 1 or (.legacy_requests | length) > 0 or
          any(.requests[]; .identity_conflict == true or .direction != "incoming" or
            .peer.device_name != "RV1-Android-Share")' "$inbox" >/dev/null; then
        echo "fixture inbox is not the sole current conflict-free RV1 phone request" >&2
        return 1
      fi
      if jq -e '.count == 1 and .acceptable_count == 1 and .blocked_count == 0 and
          .next_command == "se share request accept" and
          (.requests[0] | .direction == "incoming" and .identity_conflict == false and
            .authorization.active == false and .peer.role == "requester" and
            .peer.device_name == "RV1-Android-Share" and
            ([.request_id, .peer.device_id, .peer.node_id, .peer.public_key, .peer.fingerprint] |
              all(.[]; type == "string" and length > 0)))' "$inbox" >/dev/null; then
        request="$(jq -er '.requests[0].request_id' "$inbox")"
        fingerprint="$(jq -er '.requests[0].peer.fingerprint' "$inbox")"
        remaining=$((deadline - SECONDS))
        ((remaining > 0)) || break
        ((remaining <= 10)) || remaining=10
        # Exact signed identity, never an eval of next_command or a stale history entry.
        SHARE_CLIENT_TIMEOUT="${remaining}s" share_client share request accept "$request" \
          --fingerprint "$fingerprint" --json \
          >"$SHARE_ROOT/direct-decision.json" 2>"$SHARE_ROOT/direct-accept.err"
        jq -e --slurpfile pending "$inbox" --arg id "$request" \
          '.action == "accepted" and .request.request_id == $id and
           .request.direction == "incoming" and .request.identity_conflict == false and
           .request.decision.state == "accepted" and .request.authorization.active == true and
           .request.peer == $pending[0].requests[0].peer' "$SHARE_ROOT/direct-decision.json" >/dev/null
        cp -- "$SHARE_ROOT/direct-decision.json" "$SHARE_ROOT/direct-accepted.json"
        cat "$SHARE_ROOT/direct-accepted.json"
        return 0
      fi
    fi
    sleep 0.5
  done
  echo "no current acceptable RV1 phone request within ${seconds}s" >&2
  return 1
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

# A Room member's file listing through the desktop CLI: proves an incoming session reaches the phone.
share_reach_check() {
  local phone=$1 seconds=$2 deadline=$((SECONDS + $2)) code=0 out=""
  [[ "$phone" =~ ^[^/[:space:]]+$ ]] || {
    echo "unexpected phone device id '$phone'" >&2
    return 1
  }
  local target="share://room/$SHARE_ROOM_RELATION/$phone"
  while ((SECONDS < deadline)); do
    code=0
    out="$(SHARE_CLIENT_TIMEOUT=60s share_client ls "$target" 2>"$SHARE_ROOT/reach.err")" || code=$?
    [[ "$code" -eq 0 ]] && break
    sleep 3
  done
  echo "reach $target: exit $code after $((seconds - (deadline - SECONDS)))s"
  printf '%s\n' "$out" | head -n 20
  [[ "$code" -eq 0 ]] || {
    echo "the desktop could not open a session to the phone" >&2
    cat "$SHARE_ROOT/reach.err" >&2
    return 1
  }
}

STATE_VARS=(SHARE_STATE_VERSION SHARE_ROOT SHARE_CLIENT SHARE_SERVER_PID SHARE_SERVER_STAMP SHARE_READY
  SHARE_SERVER SHARE_RELAY SHARE_ROOM_CODE SHARE_ROOM_RELATION SHARE_DESKTOP_DEVICE SHARE_DESKTOP_DIRECT_CODE
  SHARE_ROOM_FOLDER SHARE_ROOM_FILE SHARE_ROOM_FILE_SHA256 SE_BIN SE_SHARE_SERVER_BIN)

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
  SHARE_STATE_VERSION=0
  source "$root/state.env"
  [[ "$SHARE_STATE_VERSION" == 2 ]] || { echo "unsupported fixture state; start a fresh root" >&2; return 1; }
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
  local logs=$1 failed=0 pid stamp path
  mkdir -p "$logs" || failed=1
  for path in "$SHARE_ROOT"/helper-owner-*/process; do
    [[ -r "$path" ]] || continue
    if read -r pid stamp <"$path"; then
      share_stop_process "$pid" "$stamp" "desktop fixture helper" || failed=1
    else
      echo "incomplete helper ownership record: $path" >&2
      failed=1
    fi
  done
  if [[ -n "$SHARE_CLIENT" && -d "$SHARE_CLIENT" ]]; then
    if [[ "$SHARE_READY" == 1 ]]; then
      SHARE_CLIENT_TIMEOUT=10s share_client share status --json >"$logs/desktop-share-status.json" 2>&1 || true
    fi
    share_stop_daemon || failed=1
  fi
  if [[ -n "$SHARE_SERVER_PID" ]]; then
    share_stop_process "$SHARE_SERVER_PID" "$SHARE_SERVER_STAMP" "TLS Share server" || failed=1
  fi
  for path in "$SHARE_ROOT/share-server.log" "$SHARE_ROOT/tls-generation.log" "$SHARE_ROOT/room-policy.txt" \
      "$SHARE_ROOT/direct-initial.json" "$SHARE_ROOT/direct-inbox.json" "$SHARE_ROOT/direct-inbox.err" \
      "$SHARE_ROOT/direct-decision.json" "$SHARE_ROOT/direct-accepted.json" "$SHARE_ROOT/direct-accept.err" \
      "$SHARE_ROOT"/exec-*.out "$SHARE_ROOT"/exec-*.err "$SHARE_ROOT/reach.err"; do
    [[ -f "$path" ]] || continue
    [[ "$path" -ef "$logs/${path##*/}" ]] && continue
    cp -- "$path" "$logs/" || failed=1
  done
  return "$failed"
}

case "${1:-}" in
  up)
    [[ "$#" -eq 4 ]] || { sed -n '8,18p' "$0" >&2; exit 2; }
    mkdir -p "$2"
    share_desktop_up "$2" "$3" "$4"
    ;;
  args)
    [[ "$#" -eq 2 ]] || exit 2
    share_load_state "$2"
    share_instrumentation_args
    ;;
  accept)
    [[ "$#" -eq 4 ]] || exit 2
    share_load_state "$2"
    share_register_helper
    share_accept_phone_request "$3" "$4"
    ;;
  members)
    [[ "$#" -eq 3 ]] || exit 2
    share_load_state "$2"
    share_register_helper
    share_wait_members "$3"
    ;;
  exec)
    [[ "$#" -eq 2 || "$#" -eq 3 ]] || exit 2
    share_load_state "$2"
    share_register_helper
    SHARE_EXEC_INSTRUMENT="${3:-}"
    share_exec_check
    ;;
  reach)
    [[ "$#" -eq 4 ]] || exit 2
    share_load_state "$2"
    share_register_helper
    share_reach_check "$3" "$4"
    ;;
  down)
    [[ "$#" -eq 3 ]] || exit 2
    share_load_state "$2" || exit 0
    share_desktop_down "$3"
    ;;
  *)
    sed -n '8,18p' "$0" >&2
    exit 2
    ;;
esac
