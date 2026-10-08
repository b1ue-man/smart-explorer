#!/usr/bin/env python3
"""One remote-only task suite for the sync transparency batch (2026-10-08).

Plan and expected results: docs/plaene/2026-10-08-sync-transparenz/plan.md.
Builds only the incremental native library test fixture of this host (no
release build), runs the batch's `sync_transparency_task_` acceptance and the
directly affected modules once, and gates the formatting of the batch files.
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
PREFIX = "sync_transparency_task_"

# Milestone → acceptance tests (function names; module paths are discovered).
REQUIRED = {
    "M1 job log file, reader, rotation, verbose switch": [
        "log_reader_returns_complete_lines_from_an_offset",
        "log_rejects_unsafe_ids_and_keeps_line_breaks_inside_one_line",
        "log_rotates_at_its_bound_and_switches_verbose_mode",
    ],
    "M2 a run logs listings, comparisons and actions": [
        "run_logs_every_listing_comparison_and_action",
        "ad_hoc_runs_write_no_job_log",
        "recorded_attempt_and_saved_edit_reach_the_job_log",
    ],
    "M3 log activity is progress": ["log_lines_count_as_progress"],
    "M4 evidence-based retry, interrupted runs, dated job lines": [
        "recheck_is_used_once_and_survives_cancellation",
        "success_clears_error_recheck_and_interruption",
        "result_line_names_outcome_runner_and_series",
        "old_state_files_without_new_fields_still_load",
        "changed_credentials_allow_one_retry",
        "later_success_on_the_same_drive_account_is_evidence",
        "only_login_failures_wait_for_evidence",
        "due_and_admission_follow_a_pending_recheck",
        "admission_allows_a_retry_only_with_pending_evidence",
        "dead_run_marks_become_interrupted_with_a_verification_run",
        "stored_credentials_advance_the_revision",
        "job_line_dates_errors_and_names_the_way_out",
    ],
    "M5 Drive feed scoped to the sync root": [
        "drive_feed_ignores_changes_outside_the_root",
        "drive_feed_reports_changes_below_the_root",
        "drive_feed_reports_moves_out_and_removals_of_known_objects",
        "drive_feed_counts_unresolvable_parents_as_relevant",
        "drive_feed_relearns_ancestry_after_a_folder_move",
    ],
    "M6 encrypted Share-server suggestion": [
        "plaintext_addresses_map_to_wss_on_the_same_server",
        "encrypted_or_empty_configs_need_no_switch",
    ],
}
LINUX_ONLY = {"M7 Android facade state": ["android_state_carries_interruption_and_recheck"]}

# Directly affected existing behavior: job state store/policy/editor, daemon
# scheduling and admission, Drive feed parsing, Share address parsing, the
# desktop job line, complete engine runs and credential storage.
INTEGRATION_MODULES = [
    "syncjobs::",
    "daemon::due::",
    "daemon::job_supervisor::",
    "daemon::job_triggers::",
    "gdrive::changes::",
    "share::server_address::",
    "app::sync_job_state_ui::",
    "bisync::sync_reliability_task_",
    "creds::",
]
INTEGRATION_LINUX = ["sync_state_json::"]

# Batch files: formatting gate (all were rustfmt-clean before the batch).
FORMAT_FILES = """
app/core/frame_layout.rs app/core/menus_settings.rs app/core/menus_sync_jobs.rs
app/core/sync_job_log_ui.rs app/core/sync_job_state_ui.rs app/mod.rs
bisync/mod.rs bisync/os/shared/incremental.rs bisync/os/shared/orchestration.rs
bisync/os/shared/orchestration_full.rs bisync/os/shared/run_log.rs
bisync/os/shared/run_log_lines.rs bisync/os/shared/snapshot.rs
bisync/os/shared/snapshot_dir.rs bisync/os/shared/snapshot_walk.rs
bisync/os/shared/sync_transparency_task_log_tests.rs cli/share.rs creds/os/shared.rs
daemon/mod.rs daemon/os/shared/due.rs daemon/os/shared/job.rs
daemon/os/shared/job_recheck.rs daemon/os/shared/job_supervisor.rs
daemon/os/shared/realtime.rs daemon/os/shared/remote_watch.rs daemon/os/shared/run_loop.rs
daemon/os/shared/sync_transparency_task_recheck_tests.rs
daemon/os/shared/sync_transparency_task_supervisor_tests.rs
gdrive/core/change_scope.rs gdrive/core/extensions.rs
gdrive/core/sync_transparency_task_scope_tests.rs gdrive/mod.rs
mobile/os/shared/domains/mod.rs mobile/os/shared/domains/share_settings.rs
mobile/os/shared/domains/sync_log.rs mobile/os/shared/domains/sync_state_json.rs
share/core/signal_connection_config.rs share/core/sync_transparency_task_server_tests.rs
syncjobs/mod.rs syncjobs/os/shared/job_state.rs syncjobs/os/shared/job_state_policy.rs
syncjobs/os/shared/job_state_store.rs syncjobs/os/shared/persistence.rs
syncjobs/os/shared/sync_transparency_task_state_tests.rs
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
    for name in ["SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"]:
        env.pop(name, None)
    format_gate(logs, env)
    binary = fixture(args, logs, env, candidate)
    run([str(binary), "--list", "--format", "terse"], logs / "available-tests.txt", 120, env)
    available = [line.removesuffix(": test") for line in
        (logs / "available-tests.txt").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
    required = dict(REQUIRED)
    if os.name != "nt":
        required.update(LINUX_ONLY)
    mapping = {}
    for milestone, names in required.items():
        mapping[milestone] = []
        for suffix in names:
            found = [name for name in available if name.endswith("::" + PREFIX + suffix)]
            if len(found) != 1:
                raise RuntimeError(f"{milestone}: acceptance test absent or ambiguous: {suffix}")
            mapping[milestone].extend(found)
    selected = {name for names in mapping.values() for name in names}
    modules = INTEGRATION_MODULES + ([] if os.name == "nt" else INTEGRATION_LINUX)
    for module in modules:
        found = [name for name in available if module in name]
        if not found:
            raise RuntimeError(f"Directly affected module has no tests in this binary: {module}")
        selected.update(found)
    selected = sorted(selected)
    (logs / "selection.json").write_text(json.dumps({"milestones": mapping, "selected": selected},
        indent=2), encoding="utf-8")
    namespace = "transp-" + candidate[:12] + "-" + os.environ.get("GITHUB_RUN_ID", "local")[-12:]
    with tempfile.TemporaryDirectory(prefix="sync-transparency-profile-") as profile:
        env.update(APPDATA=str(Path(profile) / "roaming"), LOCALAPPDATA=str(Path(profile) / "local"),
            XDG_CONFIG_HOME=str(Path(profile) / "config"), XDG_DATA_HOME=str(Path(profile) / "data"),
            XDG_CACHE_HOME=str(Path(profile) / "cache"), SMART_EXPLORER_E2E_TEST_NAMESPACE=namespace)
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
    print("Sync transparency acceptance passed; candidate", candidate, flush=True)


if __name__ == "__main__":
    main()
