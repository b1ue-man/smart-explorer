#!/usr/bin/env python3
"""One focused remote Windows suite for the 0.5.170 startup regressions."""
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
SELECTORS = [
    "startup_regression_task_",
    "review_task_directory_handles_create_private_children_without_replacement",
    "review_task_private_handle_hardening_refuses_hardlinked_records",
]


def worker_pids(paths):
    script = "Get-CimInstance Win32_Process | Where-Object { $_.Name -like '*se*.exe' } | Select-Object ProcessId,ExecutablePath | ConvertTo-Json -Compress"
    result = subprocess.check_output(["pwsh", "-NoProfile", "-Command", script], text=True, timeout=30).strip()
    rows = json.loads(result) if result else []
    if isinstance(rows, dict):
        rows = [rows]
    return {row["ProcessId"] for row in rows
            if row.get("ExecutablePath") and Path(row["ExecutablePath"]).resolve() in paths}


def runtime(shared, cli, logs, env):
    data = logs / "runtime-data"
    data.mkdir()
    runtime_env = dict(env, APPDATA=str(data), LOCALAPPDATA=str(data / "local"))
    previous = logs / "previous-se.exe"
    with previous.open("wb") as stream:
        subprocess.run(["git", "show", "v0.5.169:release-native/update-feed/se.exe"],
                       cwd=ROOT, stdout=stream, check=True, timeout=120)
    sidecar = shared.output(["git", "show", "v0.5.169:release-native/update-feed/se.exe.sha256"])
    if shared.sha(previous) != sidecar.split()[0]:
        raise RuntimeError("Previous published CLI does not match its committed hash.")
    paths = {cli.resolve(), previous.resolve()}
    initial = worker_pids(paths)
    sync = data / "smart_explorer" / "sync"
    try:
        shared.run([previous, "share", "status", "--json"], logs / "previous-worker.json", runtime_env,
                   seconds=120, stderr=logs / "previous-worker.stderr.log")
        before = json.loads((logs / "previous-worker.json").read_text())
        if not before["worker"]["reachable"]:
            raise RuntimeError("Previous published worker did not become reachable.")
        generation = (sync / "daemon.generation").read_text().strip()
        version = subprocess.check_output([cli, "--version"], env=runtime_env, text=True, timeout=30).strip().split()[-1]
        shared.run([cli, "update", "--complete-install", version], logs / "handoff.json", runtime_env,
                   seconds=120, stderr=logs / "handoff.stderr.log")
        handoff = json.loads((logs / "handoff.json").read_text())
        if handoff != {"version": version, "worker": "replaced", "worker_error": None}:
            raise RuntimeError("Version-bound worker handoff failed: " + json.dumps(handoff))
        shared.run([cli, "share", "status", "--json"], logs / "current-worker.json", runtime_env,
                   seconds=120, stderr=logs / "current-worker.stderr.log")
        current = json.loads((logs / "current-worker.json").read_text())
        if not current["worker"]["reachable"] or (sync / "daemon.generation").read_text().strip() == generation:
            raise RuntimeError("Replacement worker did not publish a new reachable generation.")
    finally:
        if sync.is_dir():
            (sync / "daemon.stop").write_text("stop")
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline and worker_pids(paths) - initial:
            time.sleep(1)
        remaining = worker_pids(paths) - initial
        for pid in remaining:
            subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], timeout=45, check=True)
        if worker_pids(paths) - initial:
            raise RuntimeError("Owned worker processes did not close.")


def main():
    if os.name != "nt" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("This suite runs only on the configured remote Windows CI runner.")
    candidate = os.environ.get("CANDIDATE_SHA", "")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if len(candidate) != 40 or candidate != head or candidate != os.environ.get("GITHUB_SHA"):
        raise SystemExit("Requested candidate, workflow ref and checkout must match.")
    spec = importlib.util.spec_from_file_location("startup_task_shared", ROOT / "native/review-task-native.py")
    shared = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(shared)
    logs = Path(os.environ["RUNNER_TEMP"]) / "startup-regression-task"
    logs.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="1",
               CARGO_PROFILE_TEST_DEBUG="0", CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never")
    cache = ROOT / ".review-task-cache"
    binary = shared.artifact("native", "smart_explorer", "test", cache, logs, env)
    listing = subprocess.check_output([binary, "--list", "--format", "terse"], text=True, timeout=120)
    selected = [line.removesuffix(": test") for line in listing.splitlines()
                if line.endswith(": test") and any(value in line for value in SELECTORS)]
    if not selected or any(not any(value in name for name in selected) for value in SELECTORS):
        raise RuntimeError("Required behavior cases were not discovered in the actual test host.")
    shared.run([binary, "--exact", *selected, "--test-threads=1", "--nocapture"],
               logs / "behavior.log", env, seconds=1200)
    result = (logs / "behavior.log").read_text()
    report = re.search(r"test result: ok\. (\d+) passed;", result)
    if report is None or int(report.group(1)) != len(selected):
        raise RuntimeError("The test host did not execute every selected behavior case.")
    cli = shared.artifact("native", "se", "bin", cache, logs, env)
    runtime(shared, cli, logs, env)
    (logs / "summary.json").write_text(json.dumps({"candidate": candidate,
        "selected": selected, "worker_handoff": "v0.5.169-to-current", "owned_processes_closed": True}))
    print("Windows startup, saved data, OAuth request and live handoff accepted for", candidate, flush=True)


if __name__ == "__main__":
    main()
