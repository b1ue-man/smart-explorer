#!/usr/bin/env python3
"""Device stage owned by the single RV1 entrypoint. Real JNI, bounded cleanup."""
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


native = module("rv1_native", ROOT / "native/review-task-native.py")
evaluator = module("rv1_android_evaluator", ROOT / "android/test-servers/task_eval.py")


def handoff(directory):
    record = json.loads((directory / "provenance.json").read_text())
    if record["candidate"] != os.environ["CANDIDATE_SHA"]:
        raise RuntimeError("Device handoff belongs to a different source candidate.")
    for name, expected in record["artifacts"].items():
        if Path(name).name != name or native.sha(directory / name) != expected:
            raise RuntimeError("Device handoff failed SHA-256: " + name)


def stop_owned(process, stream):
    try:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=30)
    finally:
        stream.close()


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" and os.environ.get("SMART_EXPLORER_REMOTE_RUNNER") != "1":
        raise RuntimeError("Remote runner only.")
    signal.signal(signal.SIGTERM, native.interrupted)
    env = dict(os.environ)
    logs = Path(env["SMART_EXPLORER_TASK_LOG_ROOT"]).resolve()
    logs.mkdir(parents=True, exist_ok=True)
    artifacts = Path(env["SE_TASK_ARTIFACTS"]).resolve()
    for name in ["apk", "bin"]:
        handoff(artifacts / name)
    for name in ["se", "se-share-server"]:
        (artifacts / "bin" / name).chmod(0o700)
    failures, completed, processes = [], [], []
    def stage(name, operation):
        try:
            value = operation()
            completed.append(name)
            return value
        except Exception as error:
            failures.append(name + ": " + str(error))
            print("FAILED:", failures[-1], flush=True)
            return None
    def run(args, name, seconds=120):
        return native.run(args, logs / (name + ".log"), env, seconds=seconds)
    def adb(*args):
        return native.output(["adb", *args]).replace("\r", "")
    def start(args, path):
        stream = path.open("w")
        try:
            process = subprocess.Popen(list(map(str, args)), cwd=ROOT, env=env,
                stdout=stream, stderr=stream, start_new_session=True)
        except Exception:
            stream.close()
            raise
        processes.append((process, stream))
        return process, stream
    def finish_owned(pair):
        stop_owned(*pair)
        processes.remove(pair)
    tests = evaluator.source_tests(ROOT / "android/app/src/androidTest/java")
    def cls(short):
        found = [name for name in tests if name.endswith(".task." + short)]
        if len(found) != 1:
            raise RuntimeError("Missing or ambiguous RV1 instrumented class: " + short)
        return found[0]
    sync_class, share_class = cls("ReviewSyncTaskTest"), cls("ReviewShareTaskTest")
    app = None
    share_root = Path(tempfile.mkdtemp(prefix="rv1-desktop-share-", dir=env.get("RUNNER_TEMP")))
    helper = ROOT / "android/test-servers/share-desktop.sh"
    share_started = False
    try:
        run(["adb", "wait-for-device"], "wait-device", 300)
        deadline = time.monotonic() + 600
        while adb("shell", "getprop", "sys.boot_completed").strip() != "1":
            if time.monotonic() >= deadline:
                raise RuntimeError("Emulator boot deadline.")
            time.sleep(2)
        for filename in ["app-debug.apk", "app-debug-androidTest.apk"]:
            run(["adb", "install", "-r", "-t", "-g", artifacts / "apk" / filename], "install-" + filename, 300)
        instrumentation = adb("shell", "pm", "list", "instrumentation")
        matches = [(runner, target) for runner, target in re.findall(r"instrumentation:(\S+) \(target=([^\)]+)\)", instrumentation)
            if runner.endswith("/androidx.test.runner.AndroidJUnitRunner") and sync_class.startswith(target + ".")]
        if len(matches) != 1:
            raise RuntimeError("Actual installed instrumentation target was not uniquely discovered: " + instrumentation)
        runner, app = matches[0]
        (logs / "android-runtime.json").write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
            "runner": runner, "target": app, "api": adb("shell", "getprop", "ro.build.version.sdk").strip()}, indent=2))
        run(["adb", "shell", "appops", "set", "--uid", app, "MANAGE_EXTERNAL_STORAGE", "allow"], "allow-files")
        run(["adb", "shell", "appops", "set", app, "GET_USAGE_STATS", "allow"], "allow-usage")
        run(["adb", "shell", "pm", "grant", app, "android.permission.POST_NOTIFICATIONS"], "allow-notification")
        run(["adb", "shell", "am", "force-stop", app], "initial-stop")
        # Disable unrelated update network work when ServicesTaskTest opens the app.
        prefs = logs / "task-prefs.xml"
        prefs.write_text('<?xml version="1.0" encoding="utf-8"?><map><boolean name="onboarding_done" value="true" />'
            '<boolean name="auto_update_check" value="false" /></map>')
        pushed = "/data/local/tmp/rv1-task-prefs.xml"
        run(["adb", "push", prefs, pushed], "push-preferences")
        run(["adb", "shell", "run-as", app, "mkdir", "-p", "shared_prefs"], "preferences-dir")
        run(["adb", "shell", "run-as", app, "cp", pushed, "shared_prefs/app_prefs.xml"], "preferences-copy")
        start(["adb", "logcat", "-v", "threadtime"], logs / "logcat.txt")

        def instrument(name, spec, extras=(), accept=False, seconds=1200):
            # Derive and validate actual expected names from checked-in source.
            expected = evaluator.expected_for(tests, spec)
            raw = logs / (name + ".log")
            accept_pair = None
            if accept:
                accept_pair = start(["bash", helper, "accept", share_root, "600", raw], logs / "desktop-accept.log")
            try:
                native.run(["adb", "shell", "am", "instrument", "-w", "-r", "-e", "class", spec,
                    *extras, runner], raw, env, seconds=seconds)
                run(["python3", ROOT / "android/test-servers/task_eval.py", "instrumentation", "--name", name,
                    "--output", raw, "--sources", ROOT / "android/app/src/androidTest/java", "--classes", spec,
                    "--summary", logs / (name + ".tsv")], name + "-evaluate")
                if accept_pair:
                    if accept_pair[0].wait(timeout=30) != 0:
                        raise RuntimeError("Desktop did not accept the actual fresh phone request.")
                (logs / (name + "-acceptance.json")).write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
                    "expected": sorted(expected), "result": "passed"}, indent=2))
            finally:
                if accept_pair:
                    finish_owned(accept_pair)

        merge = "recordedMergeRetrySurvivesProcessRestart"
        if merge not in tests[sync_class]:
            raise RuntimeError("Missing saved Merge restart acceptance.")
        for method in sorted(set(tests[sync_class]) - {merge}):
            stage("sync-" + method, lambda method=method: instrument("sync-" + method, sync_class + "#" + method))
        before = len(failures)
        stage("merge-prepare", lambda: instrument("merge-prepare", sync_class + "#" + merge, ["-e", "reviewMergePhase", "prepare"]))
        if len(failures) == before:
            run(["adb", "shell", "am", "force-stop", app], "merge-process-stop")
            process_state = subprocess.run(["adb", "shell", "pidof", app], capture_output=True, text=True, timeout=30)
            (logs / "merge-after-force-stop.txt").write_text(process_state.stdout or "no target process\n")
            if process_state.stdout.strip():
                raise RuntimeError("Merge prepare process survived the required force-stop.")
            stage("merge-retry", lambda: instrument("merge-retry", sync_class + "#" + merge, ["-e", "reviewMergePhase", "retry"]))

        for short, method in [("SyncTaskTest", "localConflictsResolveMergeKeepBothAndSkip"),
            ("ScanAnalyzeTaskTest", "storageAnalysisAndDuplicates"),
            ("AnalysisProtectedTaskTest", "volumeAnalysisTreatsOtherAppsFoldersAsProtected"),
            ("ServicesTaskTest", "persistentServiceNotificationPauseActionAndHomeKey")]:
            stage("integration-" + method, lambda short=short, method=method:
                instrument("integration-" + method, cls(short) + "#" + method, seconds=1800))
        stage("background-worker", lambda: instrument("background-worker", cls("BackgroundTaskTest"), seconds=1800))

        share_started = True
        ready = stage("share-host", lambda: (run(["bash", helper, "up", share_root,
            artifacts / "bin/se", artifacts / "bin/se-share-server"], "desktop-share-up", 600), True)[1])
        if ready:
            extras = native.output(["bash", helper, "args", str(share_root)]).splitlines()
            if len(extras) % 3 or any(extras[index] != "-e" for index in range(0, len(extras), 3)):
                raise RuntimeError("Desktop helper returned malformed instrumentation arguments.")
            for method in sorted(tests[share_class]):
                stage("share-" + method, lambda method=method: instrument("share-" + method,
                    share_class + "#" + method, extras, accept=method == "contactAndRoomAdmissionRequireCurrentFullPins"))
            stage("room-file-read", lambda: instrument("room-file-read",
                cls("ShareRoomTaskTest") + "#joinTheDesktopRoomAndDownloadItsFile", extras, seconds=1800))
    except Exception as error:
        failures.append("device-setup: " + str(error))
        print("FAILED:", failures[-1], flush=True)
    finally:
        if app:
            archive = logs / "app-report.tar"
            stage("device-report", lambda: native.run(["adb", "exec-out", "run-as", app, "tar", "-cf", "-", "files/task-report"], archive, env, seconds=120))
            if archive.is_file() and tarfile.is_tarfile(archive):
                with tarfile.open(archive) as tar:
                    for member in tar.getmembers():
                        if member.isfile() and member.name.endswith("/calls.tsv"):
                            (logs / "calls.tsv").write_bytes(tar.extractfile(member).read())
            stage("target-stop", lambda: run(["adb", "shell", "am", "force-stop", app], "final-stop"))
        if share_started:
            cleaned = stage("share-cleanup", lambda: (run(["bash", helper, "down", share_root,
                logs / "desktop-share-logs"], "desktop-share-down", 180), True)[1])
            if cleaned:
                shutil.rmtree(share_root)
        else:
            shutil.rmtree(share_root)
        for pair in list(processes):
            stage("owned-process-cleanup", lambda pair=pair: finish_owned(pair))
        (logs / "device-summary.json").write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
            "completed": completed, "failures": failures}, indent=2))
    if failures:
        raise SystemExit("\n".join(failures))
    print("RV1 device behavior and process-restart acceptance passed for", env["CANDIDATE_SHA"], flush=True)


if __name__ == "__main__":
    main()
