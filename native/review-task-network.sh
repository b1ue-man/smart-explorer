#!/usr/bin/env bash
# A helper of test-review-task.sh, not a second suite or a build entrypoint.
set -Eeuo pipefail
[[ ${GITHUB_ACTIONS:-} == true || ${SMART_EXPLORER_REMOTE_RUNNER:-} == 1 ]] || {
  echo 'P27 transport acceptance is remote-runner only.' >&2
  exit 2
}
self=$(realpath -- "${BASH_SOURCE[0]}")
mode=${1:-}
shift || true
case "$mode" in
  exec)
    [[ $# -gt 0 ]] || { echo 'Missing exact fixture invocation.' >&2; exit 2; }
    for tool in unshare ip tc ethtool python3; do command -v "$tool" >/dev/null; done
    [[ $EUID -eq 0 ]] || sudo -n true
    # The runner owns this unprivileged process and the pipe's only write end.
    # Even SIGKILL closes it. Root cleanup never relies on the runner being
    # allowed to signal Root descendants through sudo's process groups.
    exec python3 -c '
import os, signal, subprocess, sys
command = ["bash", sys.argv[1], "supervise", os.readlink("/proc/self/ns/net"), *sys.argv[2:]]
if os.geteuid() != 0:
    command = ["sudo", "-n", "--preserve-env", *command]
worker = subprocess.Popen(command, stdin=subprocess.PIPE, close_fds=True)
def stop(signum, frame):
    worker.stdin.close()
    try:
        worker.wait(timeout=20)
    except subprocess.TimeoutExpired:
        pass
    raise SystemExit(128 + signum)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
try:
    raise SystemExit(worker.wait())
finally:
    worker.stdin.close()
' "$self" "$@"
    ;;
  supervise)
    [[ $EUID -eq 0 ]]
    # stdin is the lifetime pipe, so the supervisor code uses -c, not stdin.
    exec python3 -c '
import ctypes, os, selectors, signal, subprocess, sys
assert os.geteuid() == 0
signal.signal(signal.SIGTERM, signal.SIG_IGN)
signal.signal(signal.SIGINT, signal.SIG_IGN)
owner = os.getpid()
def bound_child():
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
    libc.prctl.restype = ctypes.c_int
    if libc.prctl(1, signal.SIGKILL, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "PR_SET_PDEATHSIG")
    if os.getppid() != owner:
        os.kill(os.getpid(), signal.SIGKILL)
command = ["unshare", "--net", "--pid", "--fork", "--kill-child=KILL", "--mount-proc", "--",
    "bash", sys.argv[1], "inside", *sys.argv[2:]]
child = subprocess.Popen(command, close_fds=True, preexec_fn=bound_child)
watch = selectors.DefaultSelector()
watch.register(0, selectors.EVENT_READ)
try:
    while child.poll() is None:
        if watch.select(timeout=0.1):
            # No protocol data is sent: readability means EOF or a violation.
            os.read(0, 1)
            child.kill()
            child.wait(timeout=15)
            raise SystemExit(143)
    raise SystemExit(child.returncode)
finally:
    watch.close()
    if child.poll() is None:
        child.kill()
        child.wait(timeout=15)
' "$self" "$@"
    ;;
  inside)
    parent=${1:?Missing parent namespace}
    shift
    current=$(readlink /proc/self/ns/net)
    [[ "$current" != "$parent" ]] || { echo 'Namespace isolation failed.' >&2; exit 2; }
    [[ $$ -eq 1 ]] || { echo 'Owned PID namespace is missing.' >&2; exit 2; }
    # New namespaces contain only lo. Never alter an interface from the host.
    [[ $(ip -o link show | wc -l) -eq 1 ]]
    ip link set dev lo mtu 1500 up
    ethtool -K lo tso off gso off gro off
    export SE_P27_PARENT_NAMESPACE="$parent" SE_P27_NAMESPACE="$current"
    export SE_P27_PID_NAMESPACE
    SE_P27_PID_NAMESPACE=$(readlink /proc/self/ns/pid)
    export SE_P27_NETWORK_HELPER="$self"
    trap 'tc qdisc del dev lo root 2>/dev/null || true' EXIT
    "$@"
    ;;
  profile|snapshot)
    [[ -n ${SE_P27_NAMESPACE:-} && -n ${SE_P27_PARENT_NAMESPACE:-} ]]
    [[ $(readlink /proc/self/ns/net) == "$SE_P27_NAMESPACE" ]]
    [[ "$SE_P27_NAMESPACE" != "$SE_P27_PARENT_NAMESPACE" ]]
    [[ $(ip -o link show | wc -l) -eq 1 ]]
    [[ $# -eq 2 && $1 =~ ^[0-9]+$ && $2 =~ ^(10000000|1000000)$ ]]
    port=$1
    rate=$2
    (( port > 0 && port <= 65535 ))
    if [[ $mode == profile ]]; then
      tc qdisc replace dev lo root handle 1: prio bands 3 priomap 2 2 2 2 2 2 2 2 2 2 2 2 2 2 2 2
      # Shape only traffic entering the relay. Shaping the forwarded copy too
      # would turn a 10-Mbit/s profile into a 5-Mbit/s payload path.
      tc qdisc add dev lo parent 1:1 handle 10: netem delay 5ms rate "${rate}bit" limit 700
      tc qdisc add dev lo parent 1:2 handle 20: netem delay 5ms limit 700
      tc filter add dev lo parent 1: protocol ip pref 1 u32 match ip protocol 6 0xff match ip dport "$port" 0xffff flowid 1:1
      tc filter add dev lo parent 1: protocol ip pref 2 u32 match ip protocol 6 0xff match ip sport "$port" 0xffff flowid 1:2
    fi
    python3 - "$port" "$rate" <<'PY'
import json
import os
import re
import subprocess
import sys

port, rate = map(int, sys.argv[1:])
def read(*args):
    return json.loads(subprocess.check_output(args, text=True, timeout=20))

qdiscs = read("tc", "-j", "-s", "qdisc", "show", "dev", "lo")
assert sum(row.get("kind") == "netem" for row in qdiscs) == 2
assert {row.get("handle") for row in qdiscs} >= {"1:", "10:", "20:"}
queues = {row["handle"]: row for row in qdiscs}
assert queues["10:"]["options"]["rate"]["rate"] * 8 == rate
assert not queues["20:"]["options"].get("rate")
for handle in ("10:", "20:"):
    assert abs(queues[handle]["options"]["delay"]["delay"] - 0.005) < 0.000001
    assert queues[handle]["options"]["limit"] == 700
filters = read("tc", "-j", "-s", "filter", "show", "dev", "lo", "parent", "1:")
assert {row.get("options", {}).get("flowid") for row in filters} >= {"1:1", "1:2"}
features = subprocess.check_output(["ethtool", "-k", "lo"], text=True, timeout=20)
for feature in ("tcp-segmentation-offload", "generic-segmentation-offload", "generic-receive-offload"):
    assert re.search(r"^" + feature + r": off(?:\s|$)", features, re.MULTILINE)
links = read("ip", "-j", "link", "show")
assert len(links) == 1 and links[0]["ifname"] == "lo" and links[0]["mtu"] == 1500
assert os.readlink("/proc/self/ns/pid") == os.environ["SE_P27_PID_NAMESPACE"]
assert os.readlink("/proc/1/ns/pid") == os.environ["SE_P27_PID_NAMESPACE"]
print(json.dumps({"namespace": os.readlink("/proc/self/ns/net"),
    "pid_namespace": os.readlink("/proc/self/ns/pid"),
    "parent_namespace": os.environ["SE_P27_PARENT_NAMESPACE"],
    "relay_port": port, "rate_bps": rate, "delay_per_leg_us": 5000,
    "qdiscs": qdiscs,
    "filters": filters, "offloads": features, "links": links,
    "tc_version": subprocess.check_output(["tc", "-V"], text=True, timeout=20).strip(),
    "routes": read("ip", "-j", "route", "show", "table", "all")}, sort_keys=True))
PY
    ;;
  *) echo 'Usage: review-task-network.sh exec COMMAND... | profile PORT RATE_BPS | snapshot PORT RATE_BPS' >&2; exit 2 ;;
esac
