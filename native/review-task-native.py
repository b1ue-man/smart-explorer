#!/usr/bin/env python3
"""Native stages of the one B1–B8/P31/P32 remote suite; no local execution."""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
from review_task_quality import quality_evidence

ROOT = Path(__file__).resolve().parent.parent
BASE = "bea312ea79731d913a7dd3e79de2f5404401208a"
MANIFEST = ROOT / "native/review-task-manifest.json"


def manifest():
    value = json.loads(MANIFEST.read_text())
    if value["schema"] != 1 or value["baseline"] != BASE:
        raise RuntimeError("Task manifest and reviewed baseline disagree.")
    return value


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def openh264_fixture(directory, logs, env):
    """Supply the existing codec fixture from the exact checked-in Cisco pin."""
    if platform.machine().lower() not in {"x86_64", "amd64"}:
        return
    source = (ROOT / "native/src/screen/os/native_ffi/openh264_pins.rs").read_text()
    target = 'windows' if os.name == "nt" else 'target_os = "linux"'
    marker = '#[cfg(all(' + target + ', target_arch = "x86_64"))]'
    if source.count(marker) != 1:
        raise RuntimeError("OpenH264 fixture has no unique platform pin.")
    block = source.split(marker, 1)[1].split("});", 1)[0]
    url = re.search(r'url: "([^"]+)"', block).group(1)
    expected = re.search(r'compressed_sha256: "([0-9a-f]{64})"', block).group(1)
    size = int(re.search(r'compressed_size: ([0-9_]+)', block).group(1).replace("_", ""))
    if not url.startswith("https://ciscobinary.openh264.org/") or size > 16 * 1024 * 1024:
        raise RuntimeError("OpenH264 fixture pin exceeds its download boundary.")
    path = Path(env["SE_SCREEN_TASK_OPENH264"]) if env.get("SE_SCREEN_TASK_OPENH264") else directory / "openh264.bz2"
    if not env.get("SE_SCREEN_TASK_OPENH264"):
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, request, response, code, message, headers, new_url):
                return None
        try:
            with urllib.request.build_opener(NoRedirect()).open(url, timeout=30) as response:
                data = response.read(size + 1)
        except (urllib.error.URLError, TimeoutError, OSError) as error:
            (logs / "openh264-prerequisite.json").write_text(json.dumps({
                "status": "unavailable", "url": url, "reason": str(error)}, indent=2))
            return
        if len(data) != size or hashlib.sha256(data).hexdigest() != expected:
            raise RuntimeError("OpenH264 download disagrees with its checked-in size or SHA-256.")
        path.write_bytes(data)
    if path.stat().st_size != size or sha(path) != expected:
        raise RuntimeError("OpenH264 fixture disagrees with its checked-in size or SHA-256.")
    env["SE_SCREEN_TASK_OPENH264"] = str(path.resolve())
    (logs / "openh264-prerequisite.json").write_text(json.dumps({
        "status": "available", "url": url, "compressed_sha256": expected}, indent=2))


def interrupted(signum, frame):
    raise KeyboardInterrupt("Remote invocation interrupted; stopping owned children.")


def output(command, cwd=ROOT):
    return subprocess.check_output(command, cwd=cwd, text=True, timeout=120).strip()


def run(command, log, env, seconds=3600, cwd=ROOT, stderr=None):
    print("Running:", " ".join(map(str, command)), flush=True)
    with log.open("w", encoding="utf-8") as out:
        err = stderr.open("w", encoding="utf-8") if stderr else out
        try:
            proc = subprocess.Popen(list(map(str, command)), cwd=cwd, env=env, stdout=out,
                stderr=err, start_new_session=os.name != "nt",
                creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0)
            try:
                code = proc.wait(timeout=seconds)
            except (subprocess.TimeoutExpired, KeyboardInterrupt) as interruption:
                if os.name == "nt":
                    subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], timeout=45, check=False)
                else:
                    try:
                        os.killpg(proc.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    try:
                        proc.wait(timeout=180)
                    except subprocess.TimeoutExpired:
                        try:
                            os.killpg(proc.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                proc.wait(timeout=45)
                if isinstance(interruption, KeyboardInterrupt):
                    raise
                raise RuntimeError("Deadline reached; owned child tree stopped. Inspect logs before retrying.")
        finally:
            if stderr:
                err.close()
    if code:
        print(log.read_text(errors="replace")[-24000:])
        if stderr:
            print(stderr.read_text(errors="replace")[-24000:])
        raise RuntimeError(f"Command failed ({code}): {log}")


def fingerprint(crate, target, env):
    inputs = [f"{crate}/src", f"{crate}/Cargo.toml", f"{crate}/Cargo.lock",
        f"{crate}/build.rs", f"{crate}/assets", f"{crate}/vendor", f"{crate}/.cargo", ".cargo"]
    if crate == "share-server":
        inputs.append("vendor/iroh-relay-1.0.0")
    if crate == "native":
        inputs += ["native/build_support", "native/dokany-private", "native/prepare-dokany-private.ps1",
            "native/app.rc", "native/app.manifest", "vendor/iroh-1.3.0",
            "vendor/noq-1.3.0", "vendor/noq-proto-1.3.0", "vendor/egui_glow-0.29.1"]
    digest = hashlib.sha256()
    for name in sorted(filter(None, output(["git", "ls-files", "-z", "--", *inputs]).split("\0"))):
        digest.update(name.encode())
        digest.update(bytes.fromhex(sha(ROOT / name)))
    digest.update(output(["rustc", "-vV"]).encode())
    digest.update(json.dumps({"target": target, "host": sys.platform,
        "RUSTFLAGS": env.get("RUSTFLAGS", ""), "test_debug": 0, "dev_debug": 0,
        "incremental": 1}, sort_keys=True).encode())
    return digest.hexdigest()


def artifact(crate, name, kind, cache, logs, env, package=None):
    stamp = fingerprint(crate, (name, kind, package), env)
    dest = cache / crate / (name + ("-test" if kind == "test" else "") + (".exe" if os.name == "nt" else ""))
    meta = dest.with_suffix(dest.suffix + ".json")
    try:
        data = json.loads(meta.read_text())
        if data["inputs"] == stamp and data["sha256"] == sha(dest):
            print("Reusing validated development output:", dest, flush=True)
            return dest
    except (OSError, ValueError, KeyError):
        pass
    args = ["cargo", "test" if kind == "test" else "build", "--locked"]
    if crate == "native":
        args += ["-p", package or "smart_explorer"]
    args += ["--lib"] if name in {"smart_explorer", "egui_glow"} else ["--bin", name]
    if kind == "test":
        args += ["--no-run"]
    args += ["--message-format=json"]
    label = f"{crate}-{name}-{kind}"
    log = logs / (label + ".jsonl")
    run(args, log, env, seconds=10800, cwd=ROOT / crate, stderr=logs / (label + ".stderr.log"))
    binary = None
    for line in log.read_text().splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == name:
            if bool(record.get("profile", {}).get("test")) == (kind == "test") and record.get("executable"):
                binary = Path(record["executable"])
    if binary is None or not binary.is_file():
        raise RuntimeError("Cargo did not discover the requested development executable: " + label)
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, dest)
    meta.write_text(json.dumps({"inputs": stamp, "sha256": sha(dest)}))
    return dest


def format_patch(logs, env, candidate):
    if os.name == "nt":
        return
    output(["git", "cat-file", "-e", BASE + "^{commit}"])
    paths = output(["git", "diff", "--name-only", "--diff-filter=AM", BASE, "HEAD", "--",
        "native/src", "native/android-bridge/src", "share-server/src"]).splitlines()
    patch, problems, lengths = [], [], {}
    for name in (name for name in paths if name.endswith(".rs")):
        original = (ROOT / name).read_text()
        formatted = subprocess.run(["rustfmt", "--edition", "2021", "--emit", "stdout"],
            input=original, text=True, capture_output=True, timeout=60, cwd=ROOT, env=env)
        if formatted.returncode:
            problems.append(name + ": " + formatted.stderr[-2000:])
            continue
        fixed = formatted.stdout
        lengths[name] = len(fixed.splitlines())
        if original != fixed:
            patch.extend(difflib.unified_diff(original.splitlines(keepends=True), fixed.splitlines(keepends=True),
                fromfile="a/" + name, tofile="b/" + name))
        old = subprocess.run(["git", "show", BASE + ":" + name], cwd=ROOT, capture_output=True, text=True, timeout=30)
        if (old.returncode or len(old.stdout.splitlines()) <= 500) and (lengths[name] >= 500 or len(fixed.encode()) >= 50 * 1024):
            problems.append(f"{name}: formatted responsibility exceeds repository size limit ({lengths[name]} lines)")
    patch_file = logs / "format.patch"
    patch_file.write_text("".join(patch))
    (logs / "format.json").write_text(json.dumps({"candidate": candidate, "patch_sha256": sha(patch_file),
        "formatted_lines": lengths, "problems": problems}, indent=2))
    if patch or problems:
        raise RuntimeError("Changed Rust needs the candidate-bound remote format.patch or scoped size correction: " + "; ".join(problems))


def milestone_evidence(stage, selected, unavailable=(), platform=None):
    rows = []
    for point in manifest()["milestones"]:
        key = {"native": "native_markers", "unit": "android_units", "device": "android_device"}[stage]
        markers = point[key]
        observed = sorted(name for name in selected if any(marker in name for marker in markers))
        if markers and not observed:
            raise RuntimeError("No selected fixture for " + point["point"] + " in " + stage)
        if observed:
            rows.append({"point": point["point"], "milestones": point["milestones"],
                "expected": point["expected"], "executed_fixtures": observed,
                "scope": "listed fixtures only; unverified boundaries remain unverified"})
    # Resource ownership milestones need every named boundary. A generic batch
    # prefix cannot prove that JNI, native import or renderer retirement ran.
    platform = platform or ("windows" if os.name == "nt" else "linux")
    skipped = {item["fixture"]: item["reason"] for item in unavailable}
    for point in manifest().get("resource_milestones", []):
        if stage == "native" and platform not in point.get("native_platforms", ["linux", "windows"]):
            continue
        cases = point.get(stage + "_cases", [])
        if not cases:
            continue
        observed, omitted = [], []
        for case in cases:
            matches = [name for name in [*selected, *skipped]
                if (name.endswith("::" + case) if stage == "native" else name == case)]
            if len(matches) != 1:
                raise RuntimeError("Missing or ambiguous resource boundary: " + point["id"] + ": " + case)
            name = matches[0]
            if name in skipped:
                omitted.append({"fixture": name, "reason": skipped[name]})
            else:
                observed.append(name)
        rows.append({"point": point["id"], "milestones": point["milestones"],
            "expected": point["expected"], "executed_fixtures": observed,
            "not_executed": omitted, "scope": point["scope"],
            "result": "partially_verified" if omitted and observed else "unverified" if omitted else "passed"})
    return rows


def android_unit_evidence(logs, candidate):
    """Map the already evaluated Gradle XML to this candidate's milestones."""
    plan = manifest()
    classes = set(plan["android"]["unit_classes"])
    observed = set()
    results = sorted((logs / "unit-test-results").glob("TEST-*.xml"))
    if not results:
        raise RuntimeError("No evaluated Android JVM XML was retained.")
    for path in results:
        for case in ET.parse(path).getroot().iter("testcase"):
            name = f"{case.get('classname')}#{case.get('name')}"
            if case.get("classname") not in classes or name in observed:
                raise RuntimeError("Unexpected or duplicate Android JVM result: " + name)
            if any(child.tag in {"failure", "error", "skipped"} for child in case):
                raise RuntimeError("Unsuccessful Android JVM result: " + name)
            observed.add(name)
    if {name.split("#", 1)[0] for name in observed} != classes:
        raise RuntimeError("Android JVM results omit a selected class.")
    (logs / "unit-acceptance.json").write_text(json.dumps({
        "candidate": candidate, "manifest_sha256": sha(MANIFEST),
        "passed_fixtures": sorted(observed),
        "milestone_evidence": milestone_evidence("unit", observed),
        "result": "passed", "scope": "Selected JVM cases only; no Android codec or device execution."}, indent=2))


def native_tests(binary, logs, env):
    plan = manifest()["native"]
    listing = logs / "native-available.txt"
    run([binary, "--list", "--format", "terse"], listing, env, seconds=120)
    available = [line.removesuffix(": test") for line in listing.read_text().splitlines() if line.endswith(": test")]
    filters = list(plan["prefixes"])
    platform = "windows" if os.name == "nt" else "linux"
    for suffix in plan["integrations"] + plan.get(platform + "_integrations", []):
        found = [name for name in available if name.endswith("::" + suffix)]
        if len(found) != 1:
            raise RuntimeError("Missing or ambiguous directly affected integration: " + suffix)
        filters.append(suffix)
    selected = sorted(name for name in available if any(value in name for value in filters)
        and not any(name.endswith("::" + skip) for skip in plan["exclude"]))
    for marker in plan["required_prefixes"] + plan.get(platform + "_required_prefixes", []):
        if not any(marker in name for name in selected):
            raise RuntimeError("Missing required batch behavior: " + marker)
    (logs / "native-selection.json").write_text(json.dumps(selected, indent=2))
    args = [binary, "--include-ignored", "--show-output", "--test-threads=1", *filters]
    for suffix in plan["exclude"]:
        args += ["--skip", suffix]
    log = logs / "native-suite.log"
    fixture_env = env.copy()
    if os.name != "nt":
        # One isolated display for the existing single library invocation.
        # xvfb-run owns its authority file/server and returns the binary's status.
        fixture_env.pop("WAYLAND_DISPLAY", None)
        fixture_env.update(SE_SCREEN_TASK_X11="1", XDG_SESSION_TYPE="x11")
        args = ["xvfb-run", "--auto-servernum", "--error-file=" + str(logs / "xvfb.log"),
            "--server-args=-screen 0 1280x720x24 -nolisten tcp", *args]
    run(args, log, fixture_env, seconds=7200)
    result = log.read_text(errors="replace")
    for name in selected:
        if f"test {name} ... ok" not in result:
            raise RuntimeError("Missing successful acceptance: " + name)
    unavailable = []
    for line in result.splitlines():
        if not line.startswith("SE_TASK_UNVERIFIED_JSON="):
            continue
        item = json.loads(line.split("=", 1)[1])
        suffix, reason = item.get("fixture"), item.get("reason")
        if suffix not in plan.get("environment_dependent", []) or not isinstance(reason, str) or not reason:
            raise RuntimeError("Unrecognized environment-dependent fixture result: " + line)
        matches = [name for name in selected if name.endswith("::" + suffix)]
        if len(matches) != 1 or any(row["fixture"] == matches[0] for row in unavailable):
            raise RuntimeError("Ambiguous or repeated unverified fixture: " + str(suffix))
        unavailable.append({"fixture": matches[0], "reason": reason})
    not_executed = {row["fixture"] for row in unavailable}
    completed = [name for name in selected if name not in not_executed]
    evidence = milestone_evidence("native", completed, unavailable, platform)
    quality_evidence(logs, env["CANDIDATE_SHA"], completed, unavailable,
        plan["quality"], sha(MANIFEST), sha(binary))
    (logs / "native-acceptance.json").write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
        "manifest_sha256": sha(MANIFEST), "binary_sha256": sha(binary), "platform": platform,
        "selected": selected, "passed_fixtures": completed, "not_executed": unavailable,
        "milestone_evidence": evidence, "result": "passed_with_unverified_environment" if unavailable else "passed",
        "unverified": manifest()["unverified"]}, indent=2))
    print(result[-16000:])


def glow_scope_tests(binary, logs, env):
    """Only the modified vendor error ledger, within this same task suite."""
    plan = manifest()["native"]["glow_scope"]
    listing = logs / "glow-scope-available.txt"
    run([binary, "--list", "--format", "terse"], listing, env, seconds=120)
    available = [line.removesuffix(": test") for line in listing.read_text().splitlines() if line.endswith(": test")]
    selected = sorted(name for name in available if plan["prefix"] in name)
    for case in plan["cases"]:
        if sum(name.endswith("::" + case) for name in selected) != 1:
            raise RuntimeError("Missing or ambiguous vendor GL error boundary: " + case)
    if len(selected) != len(plan["cases"]):
        raise RuntimeError("Vendor GL error selection differs from the task manifest.")
    log = logs / "glow-scope-suite.log"
    run([binary, plan["prefix"], "--show-output", "--test-threads=1"], log, env, seconds=1800)
    result = log.read_text(errors="replace")
    for name in selected:
        if f"test {name} ... ok" not in result:
            raise RuntimeError("Missing successful vendor GL error boundary: " + name)
    (logs / "glow-scope-acceptance.json").write_text(json.dumps({
        "candidate": env["CANDIDATE_SHA"], "manifest_sha256": sha(MANIFEST),
        "binary_sha256": sha(binary), "package": "egui_glow", "version": "0.29.1",
        "selected": selected, "passed_fixtures": selected, "result": "passed",
        "scope": "Real vendor error ledger with controlled GL error reads; no physical GL context or driver failure."}, indent=2))


def p27_network_tests(binary, logs, env):
    """One isolated transport measurement using the existing library binary."""
    if os.name == "nt":
        return
    plan = manifest()["native"]["p27_network"]
    listing = logs / "native-available.txt"
    if not listing.is_file():
        run([binary, "--list", "--format", "terse"], listing, env, seconds=120)
    available = listing.read_text().splitlines()
    if available.count(plan["fixture"] + ": test") != 1:
        raise RuntimeError("Missing or ambiguous P27 network fixture.")
    run(["bash", ROOT / "native/review-task-network.sh", "exec", binary.resolve(),
        "--exact", plan["fixture"], "--ignored", "--nocapture", "--test-threads=1"],
        logs / "p27-network.log", env, seconds=1800)
    evidence_path = logs / "p27-network/acceptance.json"
    evidence = json.loads(evidence_path.read_text())
    if (evidence.get("schema") != 1 or evidence.get("candidate") != env["CANDIDATE_SHA"]
            or evidence.get("fixture") != plan["fixture"].rsplit("::", 1)[-1]
            or evidence.get("result") != "passed"):
        raise RuntimeError("P27 evidence is unsuccessful or belongs to a different candidate.")
    profiles = evidence.get("profiles", [])
    observed = [row.get("acceptance", {}).get("profile") for row in profiles]
    if observed != plan["profiles"]:
        raise RuntimeError("P27 evidence omits or changes a required network profile.")
    for row, profile in zip(profiles, plan["profiles"]):
        if (row["acceptance"].get("result") != "passed" or row["acceptance"].get("failures")
                or row.get("configured_profile", {}).get("rate_bps") != profile["bps"]):
            raise RuntimeError("P27 network profile did not satisfy its acceptance contract.")
        for kind in ["reference", "screen"]:
            raw = logs / "p27-network" / (profile["label"] + "-" + kind + ".json")
            if not raw.is_file() or not raw.stat().st_size:
                raise RuntimeError("Missing P27 raw observations: " + str(raw))
    (logs / "p27-acceptance.json").write_text(json.dumps({
        "candidate": env["CANDIDATE_SHA"], "manifest_sha256": sha(MANIFEST),
        "binary_sha256": sha(binary), "evidence_sha256": sha(evidence_path),
        "point": "P27", "milestones": ["P27a", "P27b"], "result": "passed",
        "fixture": plan["fixture"], "profiles": observed, "scope": evidence["scope"]}, indent=2))


def dependencies(logs, env, candidate):
    """Resolve missing dev features remotely; never build uncommitted lock bytes."""
    lock = ROOT / "native/Cargo.lock"
    original = lock.read_text()
    try:
        run(["cargo", "metadata", "--format-version=1"], logs / "dependencies-metadata.json",
            env, seconds=3600, cwd=ROOT / "native", stderr=logs / "dependencies.log")
    finally:
        resolved = lock.read_text()
        patch = "".join(difflib.unified_diff(original.splitlines(keepends=True),
            resolved.splitlines(keepends=True), fromfile="a/native/Cargo.lock", tofile="b/native/Cargo.lock"))
        (logs / "dependencies.patch").write_text(patch)
        (logs / "dependencies.json").write_text(json.dumps({"candidate": candidate,
            "original_sha256": hashlib.sha256(original.encode()).hexdigest(),
            "resolved_sha256": sha(lock), "changed": bool(patch)}, indent=2))
        if patch:
            shutil.copy2(lock, logs / "Cargo.lock.resolved")
    if patch:
        raise RuntimeError("Dependency resolution produced dependencies.patch; commit it and rerun this same suite before compiling the candidate.")


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" and os.environ.get("SMART_EXPLORER_REMOTE_RUNNER") != "1":
        raise RuntimeError("Remote runner only.")
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary-cache", type=Path)
    parser.add_argument("--dependencies", action="store_true")
    parser.add_argument("--android-build", action="store_true")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    logs = Path(os.environ["SMART_EXPLORER_TASK_LOG_ROOT"]).resolve()
    logs.mkdir(parents=True, exist_ok=True)
    candidate = output(["git", "rev-parse", "HEAD"])
    if candidate != os.environ.get("CANDIDATE_SHA"):
        raise RuntimeError("Candidate differs from checkout.")
    signal.signal(signal.SIGTERM, interrupted)
    env = dict(os.environ, RUST_BACKTRACE="1", SMART_EXPLORER_ANALYTICS_TASK="1",
        SMART_EXPLORER_COPY_PASTE_TASK="1", TOKIO_WORKER_THREADS="2")
    manifest()
    if args.dependencies:
        preparation_failures = []
        for name, operation in [("dependencies", lambda: dependencies(logs, env, candidate)),
                ("format", lambda: format_patch(logs, env, candidate))]:
            try:
                operation()
            except Exception as error:
                preparation_failures.append(name + ": " + str(error))
        (logs / "preparation-summary.json").write_text(json.dumps({
            "candidate": candidate, "failures": preparation_failures}, indent=2))
        if preparation_failures:
            raise SystemExit("\n".join(preparation_failures))
        return
    if args.android_build:
        if args.out is None:
            raise RuntimeError("Android handoff needs --out.")
        run(["bash", ROOT / "android/test-android-task.sh", "review-build", "--out", args.out],
            logs / "android-build.log", env, seconds=12600)
        android_unit_evidence(logs, candidate)
        return
    if args.binary_cache is None:
        raise RuntimeError("Native stage needs --binary-cache.")
    for name in ["SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"]:
        env.pop(name, None)
    failures = []
    def stage(name, operation):
        try:
            return operation()
        except Exception as error:
            failures.append(name + ": " + str(error))
            print("FAILED:", failures[-1], flush=True)
            return None
    with tempfile.TemporaryDirectory(prefix="batches-native-") as profile:
        for name, child in {"APPDATA": "roaming", "LOCALAPPDATA": "local", "XDG_CONFIG_HOME": "config",
            "XDG_DATA_HOME": "data", "XDG_CACHE_HOME": "cache", "XDG_RUNTIME_DIR": "runtime"}.items():
            path = Path(profile) / child
            path.mkdir(mode=0o700)
            env[name] = str(path)
        stage("openh264-prerequisite", lambda: openh264_fixture(Path(profile), logs, env))
        fixture = stage("library-build", lambda: artifact("native", "smart_explorer", "test", args.binary_cache, logs, env))
        cli = stage("worker-development", lambda: artifact("native", "se", "dev", args.binary_cache, logs, env))
        if fixture and cli:
            env["SE_SCREEN_TASK_WORKER"] = str(cli.resolve())
            stage("native-behavior", lambda: native_tests(fixture, logs, env))
        if fixture:
            stage("p27-network-behavior", lambda: p27_network_tests(fixture, logs, env))
        glow = stage("glow-scope-build", lambda: artifact("native", "egui_glow", "test", args.binary_cache, logs, env, package="egui_glow"))
        if glow:
            stage("glow-scope-behavior", lambda: glow_scope_tests(glow, logs, env))
    (logs / "summary.json").write_text(json.dumps({"candidate": candidate, "failures": failures}, indent=2))
    if failures:
        raise SystemExit("\n".join(failures))
    print("B1–B8 native suite evaluated for", candidate, "— executed and unverified cases are separate in native-acceptance.json", flush=True)


if __name__ == "__main__":
    main()
