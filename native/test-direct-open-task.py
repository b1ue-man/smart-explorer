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
PREFIXES = ("direct_open_task_", "sync_conflict_task_")
INTEGRATIONS = [
    "remote_drive_task_empty_complete_recovery_markers_are_cleanup_only",
    "remote_drive_task_invalid_or_declared_recovery_fails_closed",
    "remote_drive_task_empty_marker_with_real_payload_is_recovery",
    "remote_drive_task_only_idempotent_control_reads_are_replayable",
    "remote_drive_task_live_endpoint_refreshes_routes_but_not_identity",
    "windows_remote_task_live_peer_reconnects_using_fresh_lan_evidence",
    "remote_drive_task_iroh_mount_reconnects_without_losing_lease",
    "safe_read_reconnects_once_after_closed_generation",
    "stream_read_reconnects_only_before_bytes_are_returned",
    "stream_read_never_restarts_after_returning_bytes",
    "committed_mutation_is_not_replayed_across_keepalive_reconnect",
    "lost_write_ack_stays_failed_on_every_flush_without_replay",
    "transfer_engine_task_agent_errors_recover_permanent_target_kinds",
    "transfer_engine_task_busy_replies_become_congestion",
    "checked_resolution_copies_offered_winner_and_reports_phases",
    "cancellation_before_resolution_never_mutates_either_side",
    "cancellation_before_publish_leaves_destination_unchanged",
    "signature_drift_is_rejected_without_overwriting_destination",
    "choosing_deleted_side_backs_up_then_deletes_other_side",
    "destination_drift_after_backup_blocks_promotion",
    "source_link_swap_after_open_blocks_promotion",
    "one_side_change_propagates_then_stable",
    "nodelete_never_removes_dest_files",
    "transfer_engine_task_bisync_read_pair_failure_stops_the_other_side",
    "transfer_engine_task_bisync_read_pair_cancel_reaches_both_sides",
]

SYNC_REQUIRED = [
    "sync_names_preserve_browsing_literals_and_folder_identity",
    "unique_common_content_converges_without_alias_copies",
    "a_replaces_exact_destination_then_removes_other_objects",
    "b_requires_a_variant_and_preserves_the_selected_id",
    "multiple_common_versions_and_remote_pairs_need_selection",
    "delete_policy_guard_and_filters_protect_variants",
    "failed_backups_preserve_files_and_baseline",
    "changed_variant_and_early_cancel_never_authorize_cleanup",
    "partial_commit_retries_from_fresh_observation",
    "permissions_and_uncertain_trash_keep_exact_identity",
    "ordinary_promotion_keeps_uniqueness_guard",
    "filtered_counterparts_and_links_remain_protected",
    "one_way_move_keeps_destination_after_duplicate_cleanup",
    "mobile_exposes_variants_and_disables_ambiguous_merge",
]


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def build_fingerprint():
    digest = hashlib.sha256()
    # Include all checked-in native build inputs; omit source-independent
    # documentation and orchestration so a runner-only fix can reuse the binary.
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


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" and os.environ.get("SMART_EXPLORER_REMOTE_RUNNER") != "1":
        raise RuntimeError("This suite runs only on the configured remote CI/automation runner.")
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", type=Path, required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--source-sha")
    parser.add_argument("--binary-sha256")
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
        SMART_EXPLORER_COPY_PASTE_TASK="1",
        SMART_EXPLORER_E2E_TEST_NAMESPACE="direct_open_" + candidate[:12])
    for name in ["SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"]:
        env.pop(name, None)
    binary = args.binary
    cache = args.binary_cache.resolve() if args.binary_cache else None
    fingerprint = build_fingerprint() if cache else None
    cached_binary = cache / ("fixture.exe" if os.name == "nt" else "fixture") if cache else None
    if binary is None and cache:
        try:
            metadata = json.loads((cache / "provenance.json").read_text())
            if metadata["build_inputs_sha256"] == fingerprint and metadata["binary_sha256"] == sha256(cached_binary):
                binary = cached_binary
                print("Reusing the source- and hash-bound development fixture.", flush=True)
        except (OSError, ValueError, KeyError):
            pass
    if binary:
        binary = binary.resolve()
        if args.binary and (args.source_sha != candidate or args.binary_sha256 != sha256(binary)):
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
        if cache:
            cache.mkdir(parents=True, exist_ok=True)
            shutil.copy2(binary, cached_binary)
            metadata = {"build_inputs_sha256": fingerprint, "binary_sha256": sha256(cached_binary)}
            (cache / "provenance.json").write_text(json.dumps(metadata), encoding="utf-8")
    run([str(binary), "--list", "--format", "terse"], logs / "available-tests.txt", 60, env)
    available = [line.removesuffix(": test") for line in (logs / "available-tests.txt").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
    selected = [name for name in available if any(prefix in name for prefix in PREFIXES)]
    required = [
        "pending_download_is_neither_edit_nor_recovery",
        "parallel_download_and_atomic_save_do_not_block_new_open",
        "new_missing_payload_and_failed_manifest_never_become_editing",
        "recovery_rejects_escaped_and_nonregular_editor_paths",
        "failed_and_disconnected_workers_remove_only_their_downloads",
        "atomic_save_waits_for_stability_then_saves_back",
        "manifest_failure_suppresses_upload_and_preserves_edit",
        "save_conflict_and_failed_revision_check_preserve_remote",
        "acknowledged_save_retains_revision_when_stat_is_unavailable",
        "transport_envelope_preserves_kind_and_context",
        "agent_decodes_transport_without_changing_legacy_or_busy",
        "quic_failure_keeps_underlying_disconnect_cause",
        "stream_retry_restarts_bytes_through_real_daemon_ipc",
        "open_failure_retries_but_terminal_failures_and_other_backends_do_not",
        "exhausted_retry_keeps_previous_destination_and_cleans_stages",
        "changed_truncated_and_grown_source_never_publish",
        "empty_exported_and_id_selected_files_keep_their_contract",
        "local_write_failure_is_never_a_remote_retry",
        "real_direct_and_room_reconnect_through_daemon_without_replay",
        "reconnect_cannot_bypass_revoked_direct_access",
    ]
    for suffix in required:
        found = [name for name in selected if name.endswith("::direct_open_task_" + suffix)]
        if len(found) != 1:
            raise RuntimeError(f"Required Direct opening acceptance is absent or ambiguous: {suffix}")
    for suffix in SYNC_REQUIRED:
        if suffix == "mobile_exposes_variants_and_disables_ambiguous_merge" and sys.platform != "linux":
            continue  # The mobile facade's host fixture is enabled on Unix.
        found = [name for name in selected if name.endswith("::sync_conflict_task_" + suffix)]
        if len(found) != 1:
            raise RuntimeError(f"Required sync conflict acceptance is absent or ambiguous: {suffix}")
    integrations = INTEGRATIONS
    for suffix in integrations:
        if suffix == "source_link_swap_after_open_blocks_promotion" and sys.platform != "linux":
            continue  # Windows exercises the junction omission in the task fixture.
        found = [name for name in available if name.endswith("::" + suffix)]
        if len(found) != 1:
            raise RuntimeError(f"Required directly affected integration is absent or ambiguous: {suffix}")
        selected.extend(found)
    selected = sorted(set(selected))
    (logs / "selection.json").write_text(json.dumps(selected, indent=2), encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="direct-open-profile-") as profile:
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
    if sys.platform == "linux":
        # Compile only the changed Kotlin consumer and its focused JSON contract
        # fixture. No APK, native Android build, or unrelated Android test suite.
        metadata_log = logs / "android-cargo-metadata.json"
        run(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform",
             "aarch64-linux-android"], metadata_log, 1800, env, logs / "android-metadata.stderr.log")
        packages = json.loads(metadata_log.read_text(encoding="utf-8"))["packages"]
        verifier = [Path(p["manifest_path"]).parent / "maven" for p in packages
                    if p["name"] == "rustls-platform-verifier-android"]
        if len(verifier) != 1 or not (verifier[0] / "rustls/rustls-platform-verifier").is_dir():
            raise RuntimeError("Cannot discover the locked Android verifier Maven dependency.")
        run(["sh", str(ROOT / "android/gradlew"), "-p", str(ROOT / "android"), "--no-daemon",
             "--console=plain", "--stacktrace", f"-PrustlsVerifierMaven={verifier[0]}",
             ":app:testDebugUnitTest", "--tests", "app.smartexplorer.android.api.SyncConflictVariantTest"],
            logs / "android-conflict-contract.log", 3600, env)
        reports = ROOT / "android/app/build/test-results/testDebugUnitTest"
        shutil.copytree(reports, logs / "android-conflict-results", dirs_exist_ok=True)
    evidence = {"candidate": candidate, "binary_sha256": sha256(binary),
        "platform": sys.platform, "selected": selected, "result": "passed"}
    (logs / "acceptance.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(result)
    print("Direct opening, reconnect and sync conflict acceptance passed; candidate", candidate, flush=True)


if __name__ == "__main__":
    main()
