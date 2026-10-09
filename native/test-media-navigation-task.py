#!/usr/bin/env python3
"""One remote-only task suite for the media navigation batch (2026-10-09).

Plan and expected results: docs/plaene/2026-10-09-medien-weiterschalten/plan.md (D).
Builds only the incremental native library test fixture of this host (no
release build), runs the batch's `media_navigation_task_` acceptance and the
directly affected modules once, and gates the formatting of the batch files.
On Windows the fixture build also compiles the WinRT launch adapter
(`app/os/windows/media_launch.rs`). The Android viewer is checked by the
workflow's `android` job (Kotlin build and JVM tests).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / "native"
PREFIX = "media_navigation_task_"

# Milestone → acceptance tests (function names; module paths are discovered).
REQUIRED = {
    "M1 shared media classification": ["media_kinds_match_the_android_entry_kinds"],
    "M2 Windows launch decision": [
        "current_photos_get_the_viewer_uri",
        "older_photos_and_store_apps_get_a_neighbor_query",
        "classic_programs_folders_and_documents_stay_plain",
        "viewer_uri_encodes_every_reserved_byte",
        "package_versions_pick_the_newest_generation",
    ],
}

# Module path prefixes of the directly affected existing behavior: the Android entry kinds/MIME types that now
# use the shared classification, the Win32 name rules of the touched `types`
# module, and the desktop open/edit path around `open_local_path`.
INTEGRATION_MODULES = [
    "mobile::core_tests::",
    "types::",
    "app::remote_open::",
]

# Batch files: formatting gate (all were rustfmt-clean before the batch).
FORMAT_FILES = """
app/core/media_launch_plan.rs app/core/media_navigation_task_tests.rs app/mod.rs
app/os/windows.rs app/os/windows/media_launch.rs app/os/windows/platform.rs
mobile/core/entry.rs types/core/media_kind.rs types/mod.rs
""".split()


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def build_fingerprint():
    digest = hashlib.sha256()
    paths = subprocess.check_output(["git", "ls-files", "-z", "--", "native/src",
        "native/Cargo.toml", "native/Cargo.lock", "native/build.rs", "native/assets",
        "native/.cargo", ".cargo"], cwd=ROOT).decode().split("\0")
    for name in sorted(filter(None, paths)):
        digest.update(name.encode())
        digest.update(bytes.fromhex(sha256(ROOT / name)))
    digest.update(subprocess.check_output(["rustc", "-vV"], cwd=NATIVE))
    digest.update(b"native-lib-test;debug=0;incremental=1")
    return digest.hexdigest()


def run(command, log, seconds, env, stderr=None):
    print("Running:", " ".join(map(str, command)), flush=True)
    with log.open("w", encoding="utf-8") as output:
        error = stderr.open("w", encoding="utf-8") if stderr else output
        try:
            process = subprocess.Popen(command, cwd=NATIVE, env=env, stdout=output,
                stderr=error, start_new_session=os.name != "nt",
                creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0)
            try:
                code = process.wait(timeout=seconds)
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                if os.name == "nt":
                    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], check=False)
                else:
                    os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=30)
                raise RuntimeError("Task deadline reached; child tree stopped. Inspect diagnostics before retrying.")
        finally:
            if stderr:
                error.close()
    if code:
        print(log.read_text(encoding="utf-8", errors="replace")[-24000:])
        if stderr:
            print(stderr.read_text(encoding="utf-8", errors="replace")[-12000:])
        raise RuntimeError(f"Command failed with {code}; diagnostics: {log}")


def format_gate(logs, env):
    rustfmt = shutil.which("rustfmt")
    if not rustfmt:
        raise RuntimeError("rustfmt is required for the batch formatting gate.")
    report = []
    for name in FORMAT_FILES:
        path = NATIVE / "src" / name
        with path.open("rb") as source:
            result = subprocess.run([rustfmt, "--check", "--edition", "2021", "--color", "never"],
                stdin=source, capture_output=True, env=env)
        if result.returncode not in (0, 1) or result.stdout.strip():
            report.append(f"{name}\n{result.stdout.decode(errors='replace')}{result.stderr.decode(errors='replace')}")
    (logs / "format.log").write_text("\n".join(report) or "clean\n", encoding="utf-8")
    if report:
        raise RuntimeError("Batch files are not rustfmt-clean:\n" + "\n".join(report)[:8000])


def fixture(args, logs, env, candidate):
    cache = args.binary_cache.resolve() if args.binary_cache else None
    fingerprint = build_fingerprint() if cache else None
    cached = cache / ("fixture.exe" if os.name == "nt" else "fixture") if cache else None
    if cache:
        try:
            metadata = json.loads((cache / "provenance.json").read_text())
            if metadata["build_inputs_sha256"] == fingerprint and metadata["binary_sha256"] == sha256(cached):
                print("Reusing the source- and hash-bound development fixture.", flush=True)
                return cached
        except (OSError, ValueError, KeyError):
            pass
    build_log = logs / "build.jsonl"
    run(["cargo", "test", "--locked", "--lib", "--no-run", "--message-format=json"],
        build_log, 9000, env, logs / "build.stderr.log")
    binary = None
    for line in build_log.read_text(encoding="utf-8").splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if record.get("reason") == "compiler-artifact" and record.get("profile", {}).get("test") \
                and record.get("executable") and record.get("target", {}).get("name") == "smart_explorer":
            binary = Path(record["executable"])
    if binary is None:
        raise RuntimeError("Cargo did not report the expected library fixture executable.")
    if cache:
        cache.mkdir(parents=True, exist_ok=True)
        shutil.copy2(binary, cached)
        (cache / "provenance.json").write_text(json.dumps(
            {"build_inputs_sha256": fingerprint, "binary_sha256": sha256(cached), "source": candidate}),
            encoding="utf-8")
        return cached
    return binary


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" and os.environ.get("SMART_EXPLORER_REMOTE_RUNNER") != "1":
        raise RuntimeError("This suite runs only on the configured remote CI/automation runner.")
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", type=Path, required=True)
    parser.add_argument("--binary-cache", type=Path)
    args = parser.parse_args()
    logs = args.log_root.resolve()
    logs.mkdir(parents=True, exist_ok=True)
    candidate = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if candidate != os.environ.get("CANDIDATE_SHA"):
        raise RuntimeError("The requested full candidate SHA does not match the checked-out source.")
    env = dict(os.environ)
    env.update(CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="1", CARGO_PROFILE_TEST_DEBUG="0",
        CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never", RUST_BACKTRACE="1")
    format_gate(logs, env)
    binary = fixture(args, logs, env, candidate)
    run([str(binary), "--list", "--format", "terse"], logs / "available-tests.txt", 120, env)
    available = [line.removesuffix(": test") for line in
        (logs / "available-tests.txt").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
    mapping = {}
    for milestone, names in REQUIRED.items():
        mapping[milestone] = []
        for suffix in names:
            found = [name for name in available if name.endswith("::" + PREFIX + suffix)]
            if len(found) != 1:
                raise RuntimeError(f"{milestone}: acceptance test absent or ambiguous: {suffix}")
            mapping[milestone].extend(found)
    selected = {name for names in mapping.values() for name in names}
    for module in INTEGRATION_MODULES:
        found = [name for name in available if name.startswith(module)]
        if not found:
            raise RuntimeError(f"Directly affected module has no tests in this binary: {module}")
        selected.update(found)
    selected = sorted(selected)
    (logs / "selection.json").write_text(json.dumps({"milestones": mapping, "selected": selected},
        indent=2), encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="media-navigation-profile-") as profile:
        env.update(APPDATA=str(Path(profile) / "roaming"), LOCALAPPDATA=str(Path(profile) / "local"),
            XDG_CONFIG_HOME=str(Path(profile) / "config"), XDG_DATA_HOME=str(Path(profile) / "data"),
            XDG_CACHE_HOME=str(Path(profile) / "cache"))
        for name in ["APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"]:
            Path(env[name]).mkdir(parents=True, exist_ok=True)
        run([str(binary), "--test-threads=1", "--exact", *selected], logs / "suite.log", 3600, env)
    result = (logs / "suite.log").read_text(encoding="utf-8", errors="replace")
    missing = [name for name in selected if f"test {name} ... ok" not in result]
    if missing:
        raise RuntimeError("Missing successful results: " + ", ".join(missing[:40]))
    evidence = {"candidate": candidate, "binary_sha256": sha256(binary), "platform": sys.platform,
        "milestones": mapping, "integration_tests": len(selected), "result": "passed"}
    (logs / "acceptance.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(result[-20000:])
    print("Media navigation acceptance passed; candidate", candidate, flush=True)


if __name__ == "__main__":
    main()
