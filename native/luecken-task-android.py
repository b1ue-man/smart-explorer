"""Android upgrade stage (job `android-device`): the published APK 0.5.180 -> the candidate.

Called only by native/test-luecken-task.py (`lib` is that module); no entry
point of its own. Pattern: native/sync-reliability-android.py (published APK
from the update feed, same-certificate development signing, instrumentation
marker on the status stream), plan E11/SP1/SP9.

build   (job step 1) the candidate debug APK and the instrumentation APK, both
        signed with the release certificate (`sync-reliability-build` of
        android/test-android-task.sh passes `-PsyncReliabilitySigning=true`; the
        keystore comes from the repository secrets exactly as in
        sync-reliability-task.yml), plus the published 0.5.180 APK from the feed
        of the base commit, checked against its published SHA-256. All three must
        carry one signing certificate: Android refuses an update otherwise.
device  (job step 2, inside the emulator) installs the published APK, runs
        `LueckenUpgradeTaskTest` in the published app (identity, saved SFTP
        connection with password, plaintext records), updates in place with
        `adb install -r`, and checks in the candidate process and after a
        force-stop that every record is sealed under the Android Keystore
        protector, no old secret byte is left in the app's files, the identity
        is the same and the saved connection connects with the stored password
        and the old pin.

Limit: the signing keystore secrets exist only in the repository's workflow
context; without them the build step fails and says so (no fallback to a
debug-signed update, which Android would refuse and which would prove nothing).
"""
import hashlib
import json
import os
import re
import signal
import subprocess
import tempfile
import shutil
from pathlib import Path

OLD_REVISION = "01546a6307887f14d6e0b9a700d86a4c476bdb67"
OLD_VERSION = "0.5.180"
FEED = "release-native/update-feed"
CLASS = "app.smartexplorer.android.task.LueckenUpgradeTaskTest#publishedUpgrade"
SIGNING_ENV = ("ANDROID_KEYSTORE_FILE", "ANDROID_KEYSTORE_PASSWORD", "ANDROID_KEY_ALIAS", "ANDROID_KEY_PASSWORD")
MARKER = re.compile(r"LUECKEN_UPGRADE_MARKER (\{[^\r\n]+\})")


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command(root, args, env, timeout=120, log=None, binary=False):
    """Runs a command in its own process group and closes all its descendants (Gradle, adb)."""
    process = subprocess.Popen(list(map(str, args)), cwd=root, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=not binary, start_new_session=True)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    finally:
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
        raise RuntimeError(f"command failed ({process.returncode}): {args[0]} {args[1] if len(args) > 1 else ''}; "
                           f"evidence={log}")
    return stdout


def signer(env):
    sdk = Path(env.get("ANDROID_SDK_ROOT") or env.get("ANDROID_HOME") or "")
    tools = sorted(sdk.glob("build-tools/*/apksigner"))
    if not tools:
        raise RuntimeError("an installed Android SDK with apksigner is required")
    return tools[-1]


def certificates(root, apks, env, logs):
    """One signing certificate for the published, the candidate and the instrumentation APK."""
    result = {}
    for name in ("old-release.apk", "app-debug.apk", "app-debug-androidTest.apk"):
        output = command(root, [signer(env), "verify", "--verbose", "--print-certs", apks / name], env,
                         log=logs / (name + "-certificate.log"))
        hashes = sorted({digest.lower() for line in output.splitlines()
                         if " in lineage certificate " not in line
                         for digest in re.findall(
                             r"^(?:V[0-9.]+ )?Signer[^:]*:? certificate SHA-256 digest: *([0-9a-fA-F]{64})\s*$",
                             line)})
        if len(hashes) != 1:
            raise RuntimeError("expected one verified APK signing certificate: " + name)
        result[name] = hashes
    if len({tuple(value) for value in result.values()}) != 1:
        raise RuntimeError("the published, candidate and instrumentation APK signatures differ; "
                           "Android would refuse the update")
    return result


def feed_blob(root, name, env):
    return command(root, ["git", "show", f"{OLD_REVISION}:{FEED}/{name}"], env, timeout=300, binary=True)


def remote(env):
    if env.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("the Android upgrade stage runs exclusively on the configured remote runner")


def candidate_check(root, env, candidate):
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True, timeout=30).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", candidate) or head != candidate or env.get("GITHUB_SHA") != head:
        raise RuntimeError("checkout, workflow and requested candidate must match")


def build(lib, log_root, apks, env, candidate):
    root = lib.ai.ROOT
    remote(env)
    candidate_check(root, env, candidate)
    missing = [name for name in SIGNING_ENV if not env.get(name)]
    if missing:
        raise RuntimeError("same-certificate development signing is unavailable: "
                           + ", ".join(missing) + " not set (repository secrets ANDROID_KEYSTORE_*)")
    log_root.mkdir(parents=True, exist_ok=True)
    apks.mkdir(parents=True, exist_ok=True)
    task_env = dict(env, CANDIDATE_SHA=candidate, SMART_EXPLORER_TASK_LOG_ROOT=str(log_root / "android-logs"))
    command(root, ["bash", root / "android" / "test-android-task.sh", "sync-reliability-build", "--out", apks],
            task_env, timeout=7800, log=log_root / "android-build.log")
    version = feed_blob(root, "version.txt", env).decode().strip()
    if version != OLD_VERSION:
        raise RuntimeError(f"the feed of the base commit says {version}, expected {OLD_VERSION}")
    published = feed_blob(root, "smart-explorer-android.apk", env)
    expected = feed_blob(root, "smart-explorer-android.apk.sha256", env).decode().split()[0]
    (apks / "old-release.apk").write_bytes(published)
    if sha(apks / "old-release.apk") != expected:
        raise RuntimeError("the published APK differs from its published SHA-256")
    record = {"candidate": candidate, "old_revision": OLD_REVISION, "old_version": OLD_VERSION,
              "certificates": certificates(root, apks, env, log_root),
              "artifacts": {path.name: sha(path) for path in apks.glob("*.apk")}}
    (apks / "provenance.json").write_text(json.dumps(record, indent=2))
    return record


def device(lib, results, log_root, apks, env, candidate):
    stage = "android-device"
    root = lib.ai.ROOT
    remote(env)
    candidate_check(root, env, candidate)
    logs = log_root
    logs.mkdir(parents=True, exist_ok=True)
    record = json.loads((apks / "provenance.json").read_text())
    if record["candidate"] != candidate:
        raise RuntimeError("the APK handoff belongs to another candidate")
    for name, digest in record["artifacts"].items():
        if Path(name).name != name or sha(apks / name) != digest:
            raise RuntimeError("APK handoff hash mismatch: " + name)
    certificates(root, apks, env, logs)
    work = Path(tempfile.mkdtemp(prefix="se-luecken-android-"))
    container = None
    app = runner = None
    markers = {}

    def adb(*args, timeout=120, log=None):
        return command(root, ["adb", *args], env, timeout, log)

    def stop(label):
        adb("shell", "am", "force-stop", app, log=logs / (label + ".log"))
        state = subprocess.run(["adb", "shell", "pidof", app], env=env, capture_output=True, text=True, timeout=30)
        (logs / (label + "-pid.txt")).write_text(state.stdout)
        if state.stdout.strip():
            raise RuntimeError("the app process survived force-stop")

    def instrument(phase):
        output = adb("shell", "am", "instrument", "-w", "-r", "-e", "class", CLASS,
                     "-e", "lueckenPhase", phase, *arguments, runner, timeout=1500, log=logs / (phase + ".log"))
        if "FAILURES!!!" in output or "OK (1 test)" not in output:
            tail = output[-1500:]
            raise RuntimeError(f"instrumentation phase {phase} did not run to the end: {tail}")
        found = MARKER.findall(output)
        if len(found) != 1:
            raise RuntimeError(f"phase {phase}: expected one device-authored marker, found {len(found)}")
        marker = json.loads(found[0])
        if marker.get("phase") != phase:
            raise RuntimeError(f"phase {phase}: wrong marker {marker.get('phase')}")
        markers[phase] = marker
        return marker

    try:
        key = work / "agent_key"
        lib.ensure(lib.ai.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "se-luecken-android", "-f", key],
                              log=logs / "fixture.log").returncode == 0, "ssh-keygen failed")
        container, port, user, password = lib.start_openssh(logs, (work / "agent_key.pub").read_text().strip())
        lines, by_type = lib.scan_host_keys(port, work, logs / "fixture.log")
        fingerprints = ",".join(sorted(set(by_type.values())))
        arguments = []
        for name, value in (("seServerHost", "10.0.2.2"), ("seSftpPort", port), ("seSftpUser", user),
                            ("seSftpRoot", "/config"), ("seSftpPass", password), ("seSftpFingerprints", fingerprints),
                            ("seSshPort", port), ("seSshFingerprints", fingerprints),
                            ("lueckenOldVersion", OLD_VERSION)):
            arguments += ["-e", name, str(value)]
    except Exception as error:  # noqa: BLE001
        results.record(stage, "fixtures (OpenSSH container reachable from the emulator)", False,
                       f"{type(error).__name__}: {error}")
        shutil.rmtree(work, ignore_errors=True)
        return
    try:
        adb("wait-for-device", timeout=300)
        adb("install", "-t", "-g", apks / "old-release.apk", timeout=300, log=logs / "install-published.log")
        adb("install", "-t", "-g", apks / "app-debug-androidTest.apk", timeout=300,
            log=logs / "install-instrumentation.log")
        listed = adb("shell", "pm", "list", "instrumentation")
        matches = [(name, target) for name, target in re.findall(r"instrumentation:(\S+) \(target=([^\)]+)\)", listed)
                   if name.endswith("/androidx.test.runner.AndroidJUnitRunner") and CLASS.startswith(target + ".")]
        if len(matches) != 1:
            raise RuntimeError(f"cannot discover the unique instrumentation runner: {matches}")
        runner, app = matches[0]
        adb("shell", "appops", "set", "--uid", app, "MANAGE_EXTERNAL_STORAGE", "allow")

        def old_phase():
            marker = instrument("old-prepare")
            lib.ensure(marker["records"] >= 2 and marker["needleCount"] >= 1 and marker["deviceId"], marker)
            stop("published-stop")
            return f"{marker['records']} plaintext records, {marker['needleCount']} searched secrets"
        results.check(stage, f"published app {OLD_VERSION}: Share identity, saved SFTP connection with password "
                      "and plaintext secret records created", old_phase)

        def updated():
            lib.ensure("old-prepare" in markers, "the published app did not prepare its state")
            install = ["install", "-r", "-t", "-g", apks / "app-debug.apk"]
            try:
                adb(*install, timeout=300, log=logs / "update-candidate.log")
            except RuntimeError:
                if "INSTALL_FAILED_VERSION_DOWNGRADE" not in (logs / "update-candidate.log").read_text():
                    raise
                # The development build may carry a lower version code than the published one.
                adb("install", "-r", "-d", "-t", "-g", apks / "app-debug.apk", timeout=300,
                    log=logs / "update-candidate-downgrade.log")
            old, new = markers["old-prepare"], instrument("update-check")
            check_upgraded(lib, old, new)
            stop("candidate-stop")
            return f"protector {new['protector']}, {new['records']} sealed records, key {new['keyId']}"
        results.check(stage, "update to the candidate APK in place: every record sealed under android-keystore, no old "
                      "secret byte left, identity, saved connection (password, pin) work", updated)

        def restarted():
            lib.ensure("update-check" in markers, "the update check did not run")
            new = instrument("restart-check")
            check_upgraded(lib, markers["old-prepare"], new)
            lib.ensure(new["keyId"] == markers["update-check"]["keyId"], "the data key changed on restart")
            return f"same key {new['keyId']} after a restart"
        results.check(stage, "restart of the updated app: records, key and connection unchanged", restarted)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "emulator run", False, f"{type(error).__name__}: {error}")
    finally:
        if app:
            for args, name in ((("shell", "am", "force-stop", app), "final-stop"),
                               (("uninstall", runner.split("/")[0]), "remove-instrumentation"),
                               (("uninstall", app), "remove-app")):
                try:
                    adb(*args, log=logs / (name + ".log"))
                except Exception:  # noqa: BLE001 - best effort cleanup
                    pass
        if container:
            lib.ai.run(["docker", "rm", "-f", container], timeout=120)
        shutil.rmtree(work, ignore_errors=True)
        (logs / "markers.json").write_text(json.dumps(markers, indent=2))


def check_upgraded(lib, old, new):
    lib.ensure(new["protector"] == "android-keystore", f"key protector {new['protector']}")
    lib.ensure(new["plaintext"] == 0, f"{new['plaintext']} plaintext records after the update")
    lib.ensure(new["stageFiles"] == 0, f"{new['stageFiles']} leftover stage files")
    lib.ensure(new["found"] == 0, f"{new['found']} app files still contain an old secret")
    lib.ensure(new["records"] >= old["records"], f"records {old['records']} -> {new['records']}")
    lib.ensure(new["deviceId"] == old["deviceId"], f"Share device id {old['deviceId']} -> {new['deviceId']}")
    lib.ensure(new["listed"] is True, "the saved connection did not list")
