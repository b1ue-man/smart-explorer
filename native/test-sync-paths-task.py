#!/usr/bin/env python3
"""One remote-only task suite; incremental native library fixture, no release build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / "native"
PREFIXES = ("sync_paths_task_", "sync_links_task_")
INTEGRATIONS = [
    "failed_apply_paths_stay_out_of_new_baseline_and_retry",
    "backup_failure_blocks_overwrite_and_delete",
    "keep_both_copy_failure_blocks_resolution_and_recovers",
    "remote_absolute_path_never_uses_local_recycle_bin",
    "destination_drift_after_backup_blocks_promotion",
    "source_drift_from_planned_signature_blocks_copy",
    "cancel_interrupts_retry_wait",
    "unc_saved_root_matching_respects_share_boundary",
    "copy_paste_task_provider_webdav_conditional_create_and_abort",
    "copy_paste_task_provider_webdav_conflict_kind_survives_repeat_flush",
    "gui_design_task_drive_names_are_safe_reversible_and_collision_free",
    "gui_design_task_drive_rename_and_copy_promotion_preserve_original_titles",
    "partial_agent_hash_walk_error_never_falls_back_to_listing",
    "no_op_run_skips_rewalk",
    "mirror_cancel_during_preflight_deletes_nothing",
    "source_appearing_after_preflight_blocks_all_deletes",
]


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
    parser.add_argument("--source-sha")
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
    for name in ["SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"]:
        env.pop(name, None)
    # The library tests of every workspace crate run through cargo: cargo builds and runs each test
    # executable of the crates (the Android bridge is excluded). Listing and runs use this command.
    cargo_tests = ["cargo", "test", "--locked", "--workspace", "--exclude", "smart_explorer_android", "--exclude", "smart_explorer", "--lib", "--"]
    run([*cargo_tests, "--list", "--format", "terse"], logs / "available-tests.txt", 1800, env)
    available = [line.removesuffix(": test") for line in (logs / "available-tests.txt").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
    selected = [name for name in available if any(prefix in name for prefix in PREFIXES)]
    if not selected or not any("real_share_cross_peer" in name for name in selected):
        raise RuntimeError("This binary lacks the task's required sync endpoint and real Share acceptance.")
    required = ["nested_link_preserves_counterparts_baseline_and_incremental_recovery",
        "agent_and_daemon_streams_fall_back_without_losing_protection",
        "legacy_peer_uses_metadata_instead_of_silently_incomplete_hashes",
        "quick_mirror_preserves_links_counterparts_and_parent_directories",
        "saved_job_and_gui_retain_partial_result_notice",
        "cross_remote_contract_protects_counterpart_subtree"]
    if os.name == "nt":
        required.append("windows_cloud_data_tags_are_not_redirecting_links")
    for suffix in required:
        if not any(name.endswith("::sync_links_task_" + suffix) for name in selected):
            raise RuntimeError(f"Required link/junction acceptance is absent: {suffix}")
    integrations = INTEGRATIONS + ([] if os.name == "nt" else [
        "link_like_destination_root_never_reaches_external_victim",
        "link_like_destination_child_never_receives_copied_content"])
    for suffix in integrations:
        found = [name for name in available if name.endswith("::" + suffix)]
        if len(found) != 1:
            raise RuntimeError(f"Required directly affected integration is absent or ambiguous: {suffix}")
        selected.extend(found)
    selected = sorted(set(selected))
    (logs / "selection.json").write_text(json.dumps(selected, indent=2), encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="sync-paths-profile-") as profile:
        # App construction is background-disabled, with an isolated test profile.
        env.update(APPDATA=str(Path(profile) / "roaming"), LOCALAPPDATA=str(Path(profile) / "local"),
            XDG_CONFIG_HOME=str(Path(profile) / "config"), XDG_DATA_HOME=str(Path(profile) / "data"))
        for name in ["APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME"]:
            Path(env[name]).mkdir(parents=True, exist_ok=True)
        run([*cargo_tests, "--include-ignored", "--test-threads=1", "--exact", *selected],
            logs / "suite.log", 1800, env)
    result = (logs / "suite.log").read_text(encoding="utf-8", errors="replace")
    for name in selected:
        if f"test {name} ... ok" not in result:
            raise RuntimeError(f"Missing successful acceptance result: {name}")
    evidence = {"candidate": candidate, "test_command": " ".join(cargo_tests),
        "platform": sys.platform, "selected": selected, "result": "passed"}
    (logs / "acceptance.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(result)
    print("Focused sync path/link compatibility acceptance passed; candidate", candidate, flush=True)


if __name__ == "__main__":
    main()
