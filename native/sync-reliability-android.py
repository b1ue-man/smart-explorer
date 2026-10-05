#!/usr/bin/env python3
"""C09 helpers called only by the candidate-bound remote task entrypoint."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import sys
import subprocess

ROOT = Path(__file__).resolve().parent.parent
CLASS = "app.smartexplorer.android.task.SyncReliabilityTaskTest#publishedJobUpdateAndRestart"
_SPEC = importlib.util.spec_from_file_location("sync_reliability_android_runtime", ROOT / "native/review-task-native.py")
_RUNTIME = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_RUNTIME)


def remote(env):
    if env.get("GITHUB_ACTIONS") != "true" and env.get("SMART_EXPLORER_REMOTE_RUNNER") != "1":
        raise RuntimeError("C09 runs exclusively on the configured remote runner")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(args, env, timeout=120, log=None, binary=False):
    process = subprocess.Popen(list(map(str, args)), cwd=ROOT, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=not binary, start_new_session=True)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    finally:
        # Own and close descendants, including timed-out Gradle/Cargo/adb children.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        if process.poll() is None:
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
    if log:
        log.parent.mkdir(parents=True, exist_ok=True)
        log.write_bytes(stdout + stderr) if binary else log.write_text(stdout + stderr)
    if process.returncode:
        raise RuntimeError(f"C09 command failed ({process.returncode}): {args[0]}; evidence={log}")
    return stdout


def signer(env):
    sdk = Path(env.get("ANDROID_SDK_ROOT") or env.get("ANDROID_HOME") or "")
    tools = sorted(sdk.glob("build-tools/*/apksigner"))
    if not tools:
        raise RuntimeError("Installed Android SDK apksigner required")
    return tools[-1]


def certificates(apks, env, logs):
    result = {}
    for name in ("old-release.apk", "app-debug.apk", "app-debug-androidTest.apk"):
        output = command([signer(env), "verify", "--verbose", "--print-certs", apks / name], env,
                         log=logs / (name + "-certificate.log"))
        # Keep the same signer forms as android/build-release-apk.sh: numbered,
        # SDK-range and per-scheme signers; exclude source stamps and lineage.
        hashes = sorted({digest.lower() for line in output.splitlines()
                         if " in lineage certificate " not in line
                         for digest in re.findall(
                             r"^(?:V[0-9.]+ )?Signer[^:]*:? certificate SHA-256 digest: *([0-9a-fA-F]{64})\s*$",
                             line)})
        if len(hashes) != 1:
            raise RuntimeError("Expected one verified APK signing certificate: " + name)
        result[name] = hashes
    if len({tuple(value) for value in result.values()}) != 1:
        raise RuntimeError("Published/development/instrumentation APK signatures differ")
    return result


def build(logs: Path, out: Path, env: dict, candidate_sha: str) -> dict:
    remote(env); logs.mkdir(parents=True, exist_ok=True); out.mkdir(parents=True, exist_ok=True)
    task_env = dict(env, CANDIDATE_SHA=candidate_sha, SMART_EXPLORER_TASK_LOG_ROOT=str(logs))
    _RUNTIME.run(["bash", ROOT / "android/test-android-task.sh", "sync-reliability-build", "--out", out],
                 logs / "android-build.log", task_env, seconds=7200)
    old = command(["git", "show", "v0.5.169:release-native/update-feed/smart-explorer-android.apk"], env, binary=True)
    expected = command(["git", "show", "v0.5.169:release-native/update-feed/smart-explorer-android.apk.sha256"], env).split()[0]
    (out / "old-release.apk").write_bytes(old)
    if sha(out / "old-release.apk") != expected:
        raise RuntimeError("Published v0.5.169 APK differs from its immutable tag sidecar")
    record = {"candidate": candidate_sha, "old_tag": "v0.5.169", "certificates": certificates(out, env, logs),
              "artifacts": {p.name: sha(p) for p in out.glob("*.apk")}}
    (out / "provenance.json").write_text(json.dumps(record, indent=2))
    return record


def device(logs: Path, handoff: Path, apks: Path, env: dict, candidate_sha: str) -> dict:
    remote(env); logs.mkdir(parents=True, exist_ok=True)
    record = json.loads((apks / "provenance.json").read_text())
    if record["candidate"] != candidate_sha:
        raise RuntimeError("C09 APK handoff belongs to another candidate")
    for name, digest in record["artifacts"].items():
        if Path(name).name != name or sha(apks / name) != digest:
            raise RuntimeError("C09 APK handoff hash mismatch: " + name)
    certificates(apks, env, logs)
    app = None; markers = []
    def adb(*args, timeout=120, log=None):
        return command(["adb", *args], env, timeout, log)
    def stop(label):
        adb("shell", "am", "force-stop", app, log=logs / (label + ".log"))
        state = subprocess.run(["adb", "shell", "pidof", app], env=env, capture_output=True, text=True, timeout=30)
        (logs / (label + "-pid.txt")).write_text(state.stdout)
        if state.stdout.strip():
            raise RuntimeError("C09 target process survived force-stop")
    def instrument(phase):
        output = adb("shell", "am", "instrument", "-w", "-r", "-e", "class", CLASS,
                     "-e", "syncReliabilityPhase", phase, runner, timeout=1200, log=logs / (phase + ".log"))
        if "FAILURES!!!" in output or "OK (1 test)" not in output:
            raise RuntimeError("C09 instrumentation did not execute its complete phase: " + phase)
        found = re.findall(r"SYNC_RELIABILITY_MARKER (\{[^\r\n]+\})", output)
        if len(found) != 1:
            # AndroidJUnitRunner captures test stdout as status stream; still exact JSON only.
            raise RuntimeError("C09 missing device-authored phase marker: " + phase)
        marker = json.loads(found[0])
        if marker["phase"] != phase:
            raise RuntimeError("C09 wrong device phase marker")
        markers.append(marker)
    try:
        adb("wait-for-device", timeout=300)
        # Fresh installation is only allowed before the published app creates its real state.
        adb("install", "-t", "-g", apks / "old-release.apk", timeout=300, log=logs / "install-published.log")
        app = CLASS.split(".task.")[0]
        runner = app + ".test/androidx.test.runner.AndroidJUnitRunner"
        adb("install", "-t", "-g", apks / "app-debug-androidTest.apk", timeout=300, log=logs / "install-instrumentation.log")
        installed = adb("shell", "pm", "list", "instrumentation")
        matches = [(r, a) for r, a in re.findall(r"instrumentation:(\S+) \(target=([^\)]+)\)", installed)
                   if r.endswith("/androidx.test.runner.AndroidJUnitRunner") and CLASS.startswith(a + ".")]
        if len(matches) != 1:
            raise RuntimeError("C09 cannot discover unique actual target/runner")
        runner, app = matches[0]
        adb("shell", "appops", "set", "--uid", app, "MANAGE_EXTERNAL_STORAGE", "allow")
        instrument("old-prepare"); stop("published-stop")
        adb("install", "-r", "-t", "-g", apks / "app-debug.apk", timeout=300, log=logs / "update-development.log")
        instrument("update-prepare"); stop("prepare-stop"); instrument("retry")
        ids = {m["job"]["id"] for m in markers}
        if len(ids) != 1 or markers[-1]["sourceSha256"] != markers[-1]["targetSha256"]:
            raise RuntimeError("C09 end-to-end job identity/byte oracle failed")
        expected = hashlib.sha256(b"restarted reverse change").hexdigest()
        if markers[-1]["sourceSha256"] != expected:
            raise RuntimeError("C09 wrong final device bytes")
        result = {"case": "C09", "candidate": candidate_sha, "markers": markers, "result": "passed"}
        (logs / "android-c09.json").write_text(json.dumps(result, indent=2))
        return result
    finally:
        if app:
            cleanup_errors = []
            for args, name in [(("shell", "am", "force-stop", app), "final-stop"),
                               (("uninstall", runner.split("/")[0]), "remove-instrumentation"),
                               (("uninstall", app), "remove-fixture-app")]:
                try:
                    adb(*args, log=logs / (name + ".log"))
                except Exception as error:
                    cleanup_errors.append(str(error))
            if cleanup_errors:
                (logs / "cleanup-errors.json").write_text(json.dumps(cleanup_errors))
                if sys.exc_info()[0] is None:
                    raise RuntimeError("C09 cleanup failed: " + "; ".join(cleanup_errors))
