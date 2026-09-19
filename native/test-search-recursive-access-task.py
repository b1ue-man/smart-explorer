#!/usr/bin/env python3
"""One remote-only task suite; incremental native library fixture, no release build."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / "native"
PREFIX = "search_recursive_access_task_"
INTEGRATIONS = [
    "recursive_filter_task_restart_only_for_broader_or_truncated_listings",
    "recursive_filter_task_filter_retention_keeps_matches_and_descends_within_view",
    "recursive_filter_task_structured_constraints_narrow_by_containment",
    "recursive_filter_task_filtered_scan_emits_matches_with_their_ancestors_only",
    "recursive_filter_task_depth_limit_still_bounds_a_filtered_scan",
    "recursive_filter_task_remote_retention_emits_matches_with_their_ancestors_only",
    "recursive_collection_honors_preexisting_cancellation",
    "analytics_access_task_first_entry_error_preserves_readable_sibling",
    "analytics_access_task_unrepresentable_and_erroring_entries_never_end_the_directory",
    "analytics_access_task_missing_local_root_is_failed_not_empty_success",
]
WINDOWS = [
    "analytics_access_task_automatic_query_fallback_preserves_access_denial",
    "analytics_access_task_midway_query_failure_finishes_through_ordinary_listing",
    "analytics_access_task_full_record_fallback_and_reparse_classification",
    "analytics_access_task_real_denied_directory_locked_files_and_unchanged_acl",
    "analytics_access_task_restricted_identity_never_falls_back_to_process_authority",
    "analytics_access_task_redirect_children_are_traversal_boundaries",
]


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


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


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" and os.environ.get("SMART_EXPLORER_REMOTE_RUNNER") != "1":
        raise RuntimeError("This suite runs only on the configured remote CI/automation runner.")
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", type=Path, required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--source-sha")
    parser.add_argument("--binary-sha256")
    args = parser.parse_args()
    logs = args.log_root.resolve()
    logs.mkdir(parents=True, exist_ok=True)
    candidate = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if candidate != os.environ.get("CANDIDATE_SHA"):
        raise RuntimeError("The requested full candidate SHA does not match the checked-out source.")
    env = dict(os.environ)
    env.update(CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="1", CARGO_PROFILE_TEST_DEBUG="0",
        CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never", RUST_BACKTRACE="1",
        SMART_EXPLORER_ANALYTICS_TASK="1", SMART_EXPLORER_COPY_PASTE_TASK="1")
    if os.name == "nt":
        if not ctypes.windll.shell32.IsUserAnAdmin():
            raise RuntimeError("Real ACL fixtures require the remote runner's administrator token.")
        privileges = subprocess.check_output(["whoami", "/priv", "/fo", "csv"], text=True)
        (logs / "privileges.txt").write_text(privileges, encoding="utf-8")
        if "SeBackupPrivilege" not in privileges:
            raise RuntimeError("Runner backup-read privilege is missing; fix setup before building.")
    binary = args.binary
    if binary:
        binary = binary.resolve()
        if args.source_sha != candidate or args.binary_sha256 != sha256(binary):
            raise RuntimeError("Supplied development binary needs this exact source SHA and SHA-256.")
    else:
        # Cargo reuses its validated dependency/incremental cache on this host.
        # Discover the executable from compiler-artifact records, not stale paths.
        build_log = logs / "build.jsonl"
        run(["cargo", "test", "--locked", "--lib", "--no-run", "--message-format=json"],
            build_log, 9000, env, logs / "build.stderr.log")
        for line in build_log.read_text(encoding="utf-8").splitlines():
            try:
                record = json.loads(line)
            except ValueError:
                continue
            if record.get("reason") == "compiler-artifact" and record.get("profile", {}).get("test") and record.get("executable"):
                if record.get("target", {}).get("name") == "smart_explorer":
                    binary = Path(record["executable"])
        if binary is None:
            raise RuntimeError("Cargo did not report the expected library fixture executable.")
    run([str(binary), "--list", "--format", "terse"], logs / "available-tests.txt", 60, env)
    available = [line.removesuffix(": test") for line in (logs / "available-tests.txt").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
    selected = [name for name in available if PREFIX in name]
    if not selected or not any("wide_scan_folded_copy" in name for name in selected):
        raise RuntimeError("This binary lacks the task's required search/tree/copy acceptance.")
    integrations = INTEGRATIONS + (WINDOWS if os.name == "nt" else ["recursive_collection_does_not_follow_directory_symlinks"])
    for suffix in integrations:
        found = [name for name in available if name.endswith("::" + suffix)]
        if len(found) != 1:
            raise RuntimeError(f"Required directly affected integration is absent or ambiguous: {suffix}")
        selected.extend(found)
    selected = sorted(set(selected))
    (logs / "selection.json").write_text(json.dumps(selected, indent=2), encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="search-recursive-access-profile-") as profile:
        # App construction is background-disabled, with an isolated test profile.
        env.update(APPDATA=str(Path(profile) / "roaming"), LOCALAPPDATA=str(Path(profile) / "local"),
            XDG_CONFIG_HOME=str(Path(profile) / "config"), XDG_DATA_HOME=str(Path(profile) / "data"))
        for name in ["APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME"]:
            Path(env[name]).mkdir(parents=True, exist_ok=True)
        run([str(binary), "--include-ignored", "--test-threads=1", "--exact", *selected],
            logs / "suite.log", 1800, env)
    result = (logs / "suite.log").read_text(encoding="utf-8", errors="replace")
    for name in selected:
        if f"test {name} ... ok" not in result:
            raise RuntimeError(f"Missing successful acceptance result: {name}")
    evidence = {"candidate": candidate, "binary_sha256": sha256(binary),
        "platform": sys.platform, "selected": selected, "result": "passed"}
    (logs / "acceptance.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(result)
    print("Focused search/recursive/access acceptance passed; candidate", candidate, flush=True)


if __name__ == "__main__":
    main()
