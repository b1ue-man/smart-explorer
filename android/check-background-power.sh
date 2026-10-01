#!/usr/bin/env bash
# Manual battery check of the Android background mode ("Share im Hintergrund erreichbar",
# "Dauerbetrieb") on a real phone. Never run by CI or the task suite: a battery measurement needs
# an unplugged phone, so connect adb over Wi-Fi first (adb pair / adb connect). Builds and
# installs nothing; it reads system dumps and, with --doze, forces Doze for the window.
set -Eeuo pipefail
shopt -s inherit_errexit

usage() {
  cat <<'EOF'
Usage:
  android/check-background-power.sh [--serial SERIAL] [--package PKG] [--minutes N]
                                    [--doze] [--keep-screen] [--allow-charging] [--out DIR]

  --serial SERIAL    adb device (default: the only connected one)
  --package PKG      app id (default: app.smartexplorer.android)
  --minutes N        measuring window in minutes (default: 60)
  --doze             force deep Doze for the window (dumpsys deviceidle force-idle)
  --keep-screen      do not switch the screen off at the start
  --allow-charging   measure although the phone is powered (wake-up counts only)
  --out DIR          output folder, must be absent or empty
                     (default: ./background-power-<date>-<time>)

Prepare: open the app once, set up Share, then leave it with Home. Over USB the phone charges
and the battery numbers are meaningless: use Wi-Fi adb.

Reports the battery drop (level, charge counter), the app's alarm wake-ups, wake locks and
network use from batterystats, the top kernel wake-up reasons, the app's thread wake-ups
(voluntary context switches per thread, e.g. daemon-ipc, share-signal, background-work), the
Doze state and the app's keep-alive log lines. Raw dumps stay in the output folder.
EOF
}

die() {
  echo "check-background-power: $*" >&2
  exit 1
}

note() {
  echo "check-background-power: $*"
}

serial=""
package="app.smartexplorer.android"
minutes=60
doze=0
keep_screen=0
allow_charging=0
out_dir=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --serial)
      [ "$#" -ge 2 ] || die "--serial needs a device"
      serial="$2"
      shift
      ;;
    --package)
      [ "$#" -ge 2 ] || die "--package needs an app id"
      package="$2"
      shift
      ;;
    --minutes)
      [ "$#" -ge 2 ] || die "--minutes needs a number"
      minutes="$2"
      shift
      ;;
    --doze) doze=1 ;;
    --keep-screen) keep_screen=1 ;;
    --allow-charging) allow_charging=1 ;;
    --out)
      [ "$#" -ge 2 ] || die "--out needs a directory"
      out_dir="$2"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown argument '$1' (see --help)" ;;
  esac
  shift
done

[[ "$minutes" =~ ^[1-9][0-9]*$ ]] || die "--minutes needs a positive whole number"
[[ "$package" =~ ^[A-Za-z0-9_.]+$ ]] || die "--package '$package' is not an app id"
command -v adb >/dev/null 2>&1 || die "adb not found in PATH"

adb_cmd=(adb)
[ -z "$serial" ] || adb_cmd+=(-s "$serial")

# One device shell command line; carriage returns of old adb versions removed.
dev() {
  "${adb_cmd[@]}" shell "$*" | tr -d '\r'
}

# Runs the script on stdin with the device shell (as the app with --as-app, debuggable builds).
dev_script() {
  if [ "${1:-}" = "--as-app" ]; then
    "${adb_cmd[@]}" shell "run-as $package sh" | tr -d '\r'
  else
    "${adb_cmd[@]}" shell sh | tr -d '\r'
  fi
}

# Waits up to $1 attempts (10 s apart) for the device; Wi-Fi adb may drop during Doze.
ensure_connected() {
  local state attempt
  for ((attempt = 1; attempt <= $1; attempt++)); do
    state="$("${adb_cmd[@]}" get-state 2>/dev/null | tr -d '\r' || true)"
    [ "$state" != "device" ] || return 0
    [ "$attempt" -lt "$1" ] || break
    note "waiting for the adb connection (reconnect with adb connect <phone>:<port> if it dropped)"
    sleep 10
  done
  die "adb device not reachable (several devices? pass --serial)"
}

# Value of "  key: value" in a dumpsys battery file.
battery_field() {
  sed -n "s/^ *$2: *//p" "$1" | head -n 1
}

# Thread table of process $1: tid, name, voluntary and involuntary context switches (tab-separated).
thread_script() {
  cat <<EOF
for t in /proc/$1/task/*; do
  n=\$(cat "\$t/comm" 2>/dev/null) || continue
  v=\$(sed -n 's/^voluntary_ctxt_switches:[[:space:]]*//p' "\$t/status" 2>/dev/null)
  w=\$(sed -n 's/^nonvoluntary_ctxt_switches:[[:space:]]*//p' "\$t/status" 2>/dev/null)
  printf '%s\t%s\t%s\t%s\n' "\${t##*/}" "\$n" "\$v" "\$w"
done
EOF
}

has_counts() {
  awk -F'\t' '$3 ~ /^[0-9]+$/ { ok = 1 } END { exit !ok }' "$1"
}

# Writes the thread table of $1 to $2; the shell user can usually read other apps' /proc
# entries, otherwise run-as works for debuggable builds. Returns 1 when neither works.
thread_snapshot() {
  thread_script "$1" | dev_script >"$2" 2>/dev/null || true
  has_counts "$2" && return 0
  thread_script "$1" | dev_script --as-app >"$2" 2>/dev/null || true
  has_counts "$2"
}

# First pid of the app, empty when it does not run (pidof then fails).
app_pid() {
  local pids
  pids="$(dev "pidof $package" || true)"
  echo "${pids%% *}"
}

service_lines() {
  dev "dumpsys activity services $package" | grep -E 'ServiceRecord|isForeground=' | sed 's/^ *//' || true
}

# --- preflight ---------------------------------------------------------------

ensure_connected 1
installed="$(dev "pm path $package" || true)"
[[ "$installed" == *package:* ]] || die "$package is not installed on the device"
pid="$(app_pid)"
[ -n "$pid" ] || die "$package does not run: open the app once, then leave it with Home"
uid="$(dev "pm list packages -U $package" | sed -n "s/^package:$package uid:\([0-9,]*\).*/\1/p" | cut -d, -f1)"
[[ "$uid" =~ ^[0-9]+$ ]] || die "could not read the uid of $package"

if [ -z "$out_dir" ]; then
  out_dir="./background-power-$(date +%Y%m%d-%H%M%S)"
fi
if [ -e "$out_dir" ] && [ -n "$(ls -A "$out_dir" 2>/dev/null)" ]; then
  die "$out_dir is not empty"
fi
mkdir -p "$out_dir"

dev "dumpsys battery" >"$out_dir/battery-before.txt"
if grep -Eq '^ *(AC|USB|Wireless|Dock) powered: true' "$out_dir/battery-before.txt"; then
  [ "$allow_charging" = 1 ] || die "the phone is powered: unplug it and use Wi-Fi adb (or pass --allow-charging for wake-up counts only)"
  note "phone is powered: battery numbers are meaningless, wake-up counts still apply"
fi

exempt="no"
allowlist="$(dev "dumpsys deviceidle whitelist" || true)"
[[ "$allowlist" != *",$package,"* ]] || exempt="yes"
bucket="$(dev "am get-standby-bucket $package" || true)"
service_lines >"$out_dir/services-before.txt"

note "app $package (pid $pid, uid $uid), battery exemption: $exempt, standby bucket: ${bucket:-?}"
grep -q 'BackgroundService' "$out_dir/services-before.txt" \
  || note "warning: the background service does not run – Share is then not reachable in the background"

threads_ok=1
thread_snapshot "$pid" "$out_dir/threads-before.tsv" || {
  threads_ok=0
  note "thread counters not readable (neither as shell nor through run-as); thread wake-ups are skipped"
}

# --- measuring window --------------------------------------------------------

forced=0
cleanup() {
  if [ "$forced" = 1 ]; then
    dev "dumpsys deviceidle unforce" >/dev/null 2>&1 || true
    dev "dumpsys battery reset" >/dev/null 2>&1 || true
    forced=0
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

start_epoch="$(dev "date +%s")"
dev "dumpsys batterystats --reset" >/dev/null
dev "dumpsys batterystats --enable full-wake-history" >/dev/null || true
[ "$keep_screen" = 1 ] || dev "input keyevent KEYCODE_SLEEP" >/dev/null
if [ "$doze" = 1 ]; then
  forced=1
  # force-idle needs the screen off and an unplugged phone (virtually unplugged here).
  sleep 2
  dev "dumpsys battery unplug" >/dev/null
  dev "dumpsys deviceidle force-idle" | sed 's/^/  deviceidle: /' || true
fi

note "measuring for $minutes min – keep the phone untouched"
for ((minute = 1; minute <= minutes; minute++)); do
  sleep 60
  if [ $((minute % 10)) -eq 0 ] || [ "$minute" -eq "$minutes" ]; then
    note "$minute/$minutes min"
  fi
done

# --- collect -----------------------------------------------------------------

ensure_connected 30
{
  echo "deep: $(dev "dumpsys deviceidle get deep" || true)"
  echo "light: $(dev "dumpsys deviceidle get light" || true)"
} >"$out_dir/deviceidle.txt"
cleanup
dev "dumpsys battery" >"$out_dir/battery-after.txt"
pid_after="$(app_pid)"
service_lines >"$out_dir/services-after.txt"
if [ "$threads_ok" = 1 ] && [ "$pid_after" = "$pid" ]; then
  thread_snapshot "$pid" "$out_dir/threads-after.tsv" || threads_ok=0
fi
dev "dumpsys batterystats $package" >"$out_dir/batterystats-app.txt"
dev "dumpsys batterystats" >"$out_dir/batterystats-all.txt"
dev "dumpsys alarm" >"$out_dir/alarm.txt"
dev "dumpsys power" >"$out_dir/power.txt"
"${adb_cmd[@]}" logcat -d -T "$start_epoch.000" -v time -s \
  SmartExplorerKeepAlive:V SmartExplorerWake:V SmartExplorerBg:V SmartExplorerHost:V \
  | tr -d '\r' >"$out_dir/logcat-app.txt" || true

# --- report ------------------------------------------------------------------

echo
echo "== Window: $minutes min, forced Doze: $([ "$doze" = 1 ] && echo yes || echo no), battery exemption: $exempt"
sed 's/^/   Doze at the end, /' "$out_dir/deviceidle.txt"
if [ "$pid_after" != "$pid" ]; then
  echo "   app process changed: pid $pid -> ${pid_after:-none} (killed or restarted)"
fi
grep -q 'BackgroundService' "$out_dir/services-after.txt" \
  && echo "   background service: running at the end" \
  || echo "   background service: NOT running at the end"

echo
echo "== Battery"
level_before="$(battery_field "$out_dir/battery-before.txt" level)"
level_after="$(battery_field "$out_dir/battery-after.txt" level)"
echo "   level: ${level_before:-?} % -> ${level_after:-?} %"
counter_before="$(battery_field "$out_dir/battery-before.txt" 'Charge counter')"
counter_after="$(battery_field "$out_dir/battery-after.txt" 'Charge counter')"
if [[ "$counter_before" =~ ^[0-9]+$ && "$counter_after" =~ ^[0-9]+$ && "$counter_before" -gt 0 ]]; then
  awk -v b="$counter_before" -v a="$counter_after" -v m="$minutes" \
    'BEGIN { used = (b - a) / 1000; printf "   used: %.1f mAh, average %.1f mA (whole phone)\n", used, used * 60 / m }'
else
  echo "   charge counter not reported by this device"
fi

echo
echo "== App alarms (dumpsys alarm)"
grep -F "$package" "$out_dir/alarm.txt" | grep -F 'wakeups' | sed 's/^ */   /' | head -n 5 || true
echo "   pending keep-alive alarms: $(grep -c 'KEEP_ALIVE' "$out_dir/alarm.txt" || true) line(s) mention KEEP_ALIVE"

echo
echo "== App in batterystats (wake locks, alarms, network, CPU)"
grep -Ei 'wake ?lock|wakeup alarm|foreground service|mobile radio|network|cpu times|total cpu|estimated power' \
  "$out_dir/batterystats-app.txt" | sed 's/^ */   /' | head -n 40 || true

echo
echo "== Top kernel wake-up reasons (whole phone)"
grep -E '^ *Wakeup reason ' "$out_dir/batterystats-all.txt" | sed 's/^ *//' \
  | awk 'match($0, /\(([0-9]+) times\)/) { print substr($0, RSTART + 1, RLENGTH - 8) "\t" $0 }' \
  | sort -t "$(printf '\t')" -k1,1nr | head -n 10 | cut -f2- | sed 's/^/   /' || true

echo
echo "== App thread wake-ups (voluntary context switches in the window)"
if [ "$threads_ok" = 1 ] && [ -f "$out_dir/threads-after.tsv" ]; then
  awk -F'\t' '
    NR == FNR { if ($3 ~ /^[0-9]+$/) before[$1] = $3; next }
    $3 !~ /^[0-9]+$/ { next }
    ($1 in before) { sum[$2] += $3 - before[$1]; next }
    { sum[$2] += $3; fresh[$2] = 1 }
    END { for (n in sum) printf "%d\t%s%s\n", sum[n], n, ((n in fresh) ? " (started in the window)" : "") }
  ' "$out_dir/threads-before.tsv" "$out_dir/threads-after.tsv" | sort -t "$(printf '\t')" -k1,1nr >"$out_dir/thread-wakeups.tsv"
  awk -F'\t' -v m="$minutes" '{ printf "   %8d  (%7.1f/h)  %s\n", $1, $1 * 60 / m, $2 }' "$out_dir/thread-wakeups.tsv" | head -n 15
  for watched in daemon-ipc share-signal background-work core-events; do
    awk -F'\t' -v w="$watched" -v m="$minutes" \
      '$2 == w { printf "   watched %-16s %8d  (%7.1f/h)\n", w, $1, $1 * 60 / m; found = 1 } END { if (!found) printf "   watched %-16s not present\n", w }' \
      "$out_dir/thread-wakeups.tsv"
  done
else
  echo "   skipped (counters not readable or the process changed)"
fi

echo
echo "== Keep-alive log (logcat since the start)"
echo "   alarm probes: $(grep -c 'probe (networkChanged=false)' "$out_dir/logcat-app.txt" || true)," \
  "network probes: $(grep -c 'probe (networkChanged=true)' "$out_dir/logcat-app.txt" || true)," \
  "failed probes: $(grep -cE 'share\.wake failed|did not finish' "$out_dir/logcat-app.txt" || true)," \
  "refused service starts: $(grep -cE 'background service not started|could not enter the foreground' "$out_dir/logcat-app.txt" || true)"
tail -n 10 "$out_dir/logcat-app.txt" | sed 's/^/   /' || true

echo
note "raw dumps in $out_dir"
