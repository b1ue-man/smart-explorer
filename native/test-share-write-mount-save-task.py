#!/usr/bin/env python3
"""One remote-only task suite for the Direct-share write and mount-save batch (2026-10-09).

Plan and expected results: docs/plaene/2026-10-09-direkt-schreiben-laufwerk/plan.md (D).
Builds only the incremental native library test fixture of this host (no
release build), runs the batch's `share_rights_task_` / `mount_save_task_`
acceptance and the directly affected modules once, and gates the formatting of
the batch files. Tests the test binary marks as ignored are not selected from
the module lists (they need other runners); the workflow's `android` job builds
the changed export dialog.
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

# Milestone -> acceptance tests (full function names; module paths are discovered).
REQUIRED = {
    "S1 new and re-admitted Direct devices may write": [
        "review_task_one_way_pairing_and_repair_never_create_share_back_grant",
    ],
    "S2 Direct exports read-write, room exports read-only": [
        "share_rights_task_new_direct_exports_write_and_room_exports_read",
    ],
    "S3 FC1 Home restriction lifted for Direct only": [
        "share_rights_task_restricted_direct_home_writes_again_once",
        "review_task_fc1_implicit_home_and_connections_migrate_once_without_new_grants",
        "review_task_fc1_failed_migration_is_retryable_and_returns_no_runtime_profile",
    ],
    "S4 mounted saves never leave the stage spelling": [
        "mount_save_task_facts_known_on_one_side_only_are_not_a_change",
        "mount_save_task_every_known_difference_is_a_change",
        "mount_save_task_conflict_copies_keep_the_file_type",
        "mount_save_task_failed_upload_leaves_no_stage_and_retries_cleanly",
        "mount_save_task_remote_change_during_save_becomes_a_typed_conflict_copy",
        "mount_save_task_lost_promotion_reply_never_leaves_the_stage_name",
        "mount_save_task_stage_ledger_survives_restarts_and_skips_running_saves",
        "mount_save_task_orphaned_stage_is_removed_on_the_next_mount",
    ],
}

# Directly affected existing behavior: Direct grant creation/decisions, profile
# persistence and policy edits, export configuration, and the whole mount engine
# (conflict checks, replace/rename, recovery, caches).
INTEGRATION_PREFIXES = ["mount::"]
INTEGRATION_FRAGMENTS = [
    "profile_persistence",
    "profile_policy",
    "relation_rights::",
    "direct_ledger_tests::",
    "legacy_direct_request_tests::",
    "direct_request_tombstone::",
    "export_config::",
    "profile_edits::",
]

# Batch files: formatting gate (each is rustfmt-clean).
FORMAT_FILES = """
app/core/share_exports_ui.rs cli/share/exports.rs mobile/os/shared/domains/share_peers.rs
mount/core/baseline_match.rs mount/core/commit.rs mount/core/file_commit.rs
mount/core/file_commit_stage.rs mount/core/mount_save_task_tests.rs share/api_exports.rs
share/core/direct_ledger_projection.rs share/core/direct_ledger_tests.rs
share/core/direct_reciprocal.rs share/core/direct_relation.rs share/core/export_config.rs
share/core/legacy_direct_request_decision.rs share/core/profile_migration.rs
share/core/profile_persistence_tests.rs share/core/profiles.rs
share/core/relation_rights_task_tests.rs mount/core/stage_ledger.rs mount/core/startup.rs
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


def listed(binary, logs, name, env, extra):
    output = logs / name
    run([str(binary), "--list", "--format", "terse", *extra], output, 120, env)
    return [line.removesuffix(": test") for line in
        output.read_text(encoding="utf-8").splitlines() if line.endswith(": test")]


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
        CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never", RUST_BACKTRACE="1",
        SMART_EXPLORER_E2E_TEST_NAMESPACE="share_write_" + candidate[:12])
    for name in ["SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"]:
        env.pop(name, None)
    format_gate(logs, env)
    binary = fixture(args, logs, env, candidate)
    available = listed(binary, logs, "available-tests.txt", env, [])
    ignored = set(listed(binary, logs, "ignored-tests.txt", env, ["--ignored"]))
    mapping = {}
    for milestone, names in REQUIRED.items():
        mapping[milestone] = []
        for suffix in names:
            found = [name for name in available if name.endswith("::" + suffix)]
            if len(found) != 1 or found[0] in ignored:
                raise RuntimeError(f"{milestone}: acceptance test absent, ignored or ambiguous: {suffix}")
            mapping[milestone].extend(found)
    selected = {name for names in mapping.values() for name in names}
    for prefix in INTEGRATION_PREFIXES:
        found = [name for name in available if name.startswith(prefix) and name not in ignored]
        if not found:
            raise RuntimeError(f"Directly affected module has no tests in this binary: {prefix}")
        selected.update(found)
    for fragment in INTEGRATION_FRAGMENTS:
        found = [name for name in available if fragment in name and name not in ignored]
        if not found:
            raise RuntimeError(f"Directly affected module has no tests in this binary: {fragment}")
        selected.update(found)
    selected = sorted(selected)
    (logs / "selection.json").write_text(json.dumps({"milestones": mapping, "selected": selected,
        "ignored_not_selected": sorted(ignored & set(available))}, indent=2), encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="share-write-profile-") as profile:
        env.update(APPDATA=str(Path(profile) / "roaming"), LOCALAPPDATA=str(Path(profile) / "local"),
            XDG_CONFIG_HOME=str(Path(profile) / "config"), XDG_DATA_HOME=str(Path(profile) / "data"),
            XDG_CACHE_HOME=str(Path(profile) / "cache"))
        for name in ["APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"]:
            Path(env[name]).mkdir(parents=True, exist_ok=True)
        run([str(binary), "--test-threads=1", "--exact", *selected], logs / "suite.log", 5400, env)
    result = (logs / "suite.log").read_text(encoding="utf-8", errors="replace")
    missing = [name for name in selected if f"test {name} ... ok" not in result]
    if missing:
        raise RuntimeError("Missing successful results: " + ", ".join(missing[:40]))
    evidence = {"candidate": candidate, "binary_sha256": sha256(binary), "platform": sys.platform,
        "milestones": mapping, "integration_tests": len(selected), "result": "passed"}
    (logs / "acceptance.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(result[-20000:])
    print("Share write and mount save acceptance passed; candidate", candidate, flush=True)


if __name__ == "__main__":
    main()
