#!/usr/bin/env python3
"""Android stage of the one B1–B8/P31/P32 entrypoint; actual JNI and owned fixtures."""
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parent.parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


native = module("batch_native", ROOT / "native/review-task-native.py")
evaluator = module("batch_android_evaluator", ROOT / "android/test-servers/task_eval.py")


def handoff(directory):
    record = json.loads((directory / "provenance.json").read_text())
    if record["candidate"] != os.environ["CANDIDATE_SHA"]:
        raise RuntimeError("APK handoff belongs to a different source candidate.")
    expected_names = {"app-debug.apk", "app-debug-androidTest.apk"}
    if set(record["artifacts"]) != expected_names:
        raise RuntimeError("APK handoff must contain the app and its instrumentation APK.")
    for name, expected in record["artifacts"].items():
        if native.sha(directory / name) != expected:
            raise RuntimeError("APK handoff failed SHA-256: " + name)


def stop_owned(process, stream):
    try:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
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
    handoff(artifacts / "apk")
    plan = native.manifest()
    device_plan = plan["android"]
    spec = ",".join(device_plan["device_classes"])
    expected = evaluator.expected_for(
        evaluator.source_tests(ROOT / "android/app/src/androidTest/java"), spec)
    if not expected:
        raise RuntimeError("Empty Android acceptance selection.")
    evidence = native.milestone_evidence("device", expected)
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

    app = None
    pushed_prefs = "/data/local/tmp/se-batches-task-prefs.xml"
    pushed_video = "/data/local/tmp/se-batches-task-video.mp4"
    owned_files = "files/batches-task-fixtures"
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
        matches = [(runner, target) for runner, target in re.findall(
            r"instrumentation:(\S+) \(target=([^\)]+)\)", instrumentation)
            if runner.endswith("/androidx.test.runner.AndroidJUnitRunner")
            and all(name.startswith(target + ".") for name in expected)]
        if len(matches) != 1:
            raise RuntimeError("Actual installed instrumentation target is ambiguous: " + instrumentation)
        runner, app = matches[0]
        app_dir = adb("shell", "run-as", app, "pwd").strip()
        if not app_dir.startswith("/data/") or any(c.isspace() for c in app_dir):
            raise RuntimeError("Unexpected app-private directory returned by run-as.")
        (logs / "android-runtime.json").write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
            "runner": runner, "target": app, "api": adb("shell", "getprop", "ro.build.version.sdk").strip(),
            "manifest_sha256": native.sha(native.MANIFEST)}, indent=2))
        run(["adb", "shell", "appops", "set", "--uid", app, "MANAGE_EXTERNAL_STORAGE", "allow"], "allow-files")
        run(["adb", "shell", "pm", "grant", app, "android.permission.POST_NOTIFICATIONS"], "allow-notification")
        run(["adb", "shell", "am", "force-stop", app], "initial-stop")
        prefs = logs / "task-prefs.xml"
        prefs.write_text('<?xml version="1.0" encoding="utf-8"?><map><boolean name="onboarding_done" value="true" />'
            '<boolean name="auto_update_check" value="false" /></map>')
        run(["adb", "push", prefs, pushed_prefs], "push-preferences")
        run(["adb", "shell", "run-as", app, "mkdir", "-p", "shared_prefs", owned_files], "fixture-dirs")
        run(["adb", "shell", "run-as", app, "cp", pushed_prefs, "shared_prefs/app_prefs.xml"], "preferences-copy")
        video = logs / "task-video.mp4"
        duration = int(device_plan["video_duration_seconds"])
        if duration < 8:
            raise RuntimeError("Media acceptance requires at least eight seconds of playable video.")
        run(["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-y",
            "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15",
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100",
            "-t", str(duration), "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            "-movflags", "+faststart", video], "make-video", 180)
        run(["adb", "push", video, pushed_video], "push-video")
        video_path = app_dir + "/" + owned_files + "/video.mp4"
        run(["adb", "shell", "run-as", app, "cp", pushed_video, video_path], "copy-video")
        (logs / "media-fixture.json").write_text(json.dumps({"duration_seconds": duration,
            "sha256": native.sha(video), "argument": device_plan["video_argument"]}, indent=2))
        start(["adb", "logcat", "-v", "threadtime"], logs / "logcat.txt")

        def instrument():
            raw = logs / "android-batches.log"
            native.run(["adb", "shell", "am", "instrument", "-w", "-r", "-e", "class", spec,
                "-e", device_plan["video_argument"], video_path, runner], raw, env, seconds=7200)
            run(["python3", ROOT / "android/test-servers/task_eval.py", "instrumentation", "--name", "android-batches",
                "--output", raw, "--sources", ROOT / "android/app/src/androidTest/java", "--classes", spec,
                "--summary", logs / "android-batches.tsv"], "android-evaluate")
            (logs / "device-acceptance.json").write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
                "manifest_sha256": native.sha(native.MANIFEST), "expected": sorted(expected),
                "milestone_evidence": evidence, "result": "passed", "unverified": plan["unverified"]}, indent=2))

        stage("android-batches", instrument)
    except Exception as error:
        failures.append("device-setup: " + str(error))
        print("FAILED:", failures[-1], flush=True)
    finally:
        if app:
            def collect_report():
                probe = subprocess.run(["adb", "shell", "run-as", app, "test", "-d", "files/task-report"],
                    capture_output=True, timeout=30)
                if probe.returncode != 0:
                    return
                archive = logs / "app-report.tar"
                native.run(["adb", "exec-out", "run-as", app,
                    "tar", "-cf", "-", "files/task-report"], archive, env, seconds=120)
                if archive.is_file() and tarfile.is_tarfile(archive):
                    with tarfile.open(archive) as tar:
                        for member in tar.getmembers():
                            if member.isfile() and member.name.endswith("/calls.tsv"):
                                (logs / "calls.tsv").write_bytes(tar.extractfile(member).read())
            stage("device-report", collect_report)
            stage("target-stop", lambda: run(["adb", "shell", "am", "force-stop", app], "final-stop"))
            stage("fixture-cleanup", lambda: run(["adb", "shell", "run-as", app, "rm", "-rf", owned_files], "fixture-cleanup"))
        stage("push-cleanup", lambda: run(["adb", "shell", "rm", "-f", pushed_prefs, pushed_video], "push-cleanup"))
        for pair in processes:
            stage("owned-process-cleanup", lambda pair=pair: stop_owned(*pair))
        (logs / "device-summary.json").write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
            "completed": completed, "failures": failures}, indent=2))
    if failures:
        raise SystemExit("\n".join(failures))
    print("Android presentation acceptance passed for candidate", env["CANDIDATE_SHA"], flush=True)


if __name__ == "__main__":
    main()
