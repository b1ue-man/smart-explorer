#!/usr/bin/env python3
"""One candidate-bound remote sync acceptance entrypoint; never a release build."""
import argparse
from collections import Counter
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
BASE = "3d70c5df48d26408fc168167b00586d519174d53"
CASES = {
    "C01": {"meaning": "Notebook exact names and complete tree", "prefix": "sync_reliability_task_notebook_"},
    "C02": {"meaning": "duplicate and literal names with exact identities", "prefix": "sync_reliability_task_names_"},
    "C03": {"meaning": "paging, persistent identities and concurrent restart", "prefix": "sync_reliability_task_identity_"},
    "C04": {"meaning": "actual providers through saved location resolution", "prefix": "sync_reliability_task_provider_"},
    "C05": {"meaning": "effective saved sync options and modes", "prefix": "sync_reliability_task_options_"},
    "C06": {"meaning": "interrupt and successfully resume the same sync", "prefix": "sync_reliability_task_resume_"},
    "C07": {"meaning": "protected children, backups, state and owners", "prefix": "sync_reliability_task_protection_"},
    "C08": {"meaning": "old jobs, triggers and actual published worker update", "prefix": "sync_reliability_task_old_jobs_"},
    "C09": {"meaning": "actual published Android app data across update and restart"},
    "C10": {"meaning": "real Google OAuth and Drive sync", "prefix": "sync_reliability_task_live_drive_"},
}
# Existing flows are selected by their full names for the directly affected
# collection creation, metadata redirects, publication, cancellation,
# protected-tree and startup contracts.
INTEGRATIONS = {"C04": [
    "webdav::core_impl::connection_tests::mutation_redirect_is_not_followed_or_reported_as_success",
    "webdav::core_impl::connection_tests::put_redirect_is_terminal_and_never_followed",
    "webdav::transfer_engine_task_tests::transfer_engine_task_webdav_folders_take_one_mkcol",
    "webdav::transfer_engine_task_tests::transfer_engine_task_webdav_mutations_reuse_pooled_connections",
    "webdav::transfer_engine_task_tests::transfer_engine_task_webdav_overload_is_congestion_with_retry_after",
], "C05": [
    "bisync::tests::safety::remote_absolute_path_never_uses_local_recycle_bin",
    "bisync::tests::safety::recycle_failure_does_not_fall_back_to_permanent_delete",
], "C06": [
    "gdrive::sync_conflict_task_safety_tests::sync_conflict_task_permissions_and_uncertain_trash_keep_exact_identity",
    "gdrive::sync_conflict_task_safety_tests::sync_conflict_task_partial_commit_retries_from_fresh_observation",
    "bisync::engine_provider_task_tests::identity_tests::engine_provider_recorded_lost_ack_preserves_old_baseline_until_retry",
    "bisync::engine_provider_task_tests::identity_tests::engine_provider_publication_and_lost_ack_use_exactly_one_contract",
    "bisync::merge_recorded::task_tests::review_task_merge_partial_publication_keeps_conflict_basis_and_retries",
    "bisync::tests::move_retry::failed_source_delete_retries_as_verified_finalize_without_recopy",
    "bisync::apply_retry::tests::cancel_interrupts_retry_wait",
], "C07": [
    "bisync::tests::links::sync_links_task_nested_link_preserves_counterparts_baseline_and_incremental_recovery",
    "bisync::tests::links::sync_links_task_target_link_reverse_mirror_and_exclusions_are_protected",
    "bisync::tests::links::sync_links_task_incremental_target_junction_returns_to_full_protected_scan",
    "bisync::tests::links::sync_links_task_cycles_and_dangling_links_do_not_abort_regular_files",
    "bisync::tests::links_remote::sync_links_task_agent_and_daemon_streams_fall_back_without_losing_protection",
    "bisync::tests::links_remote::sync_links_task_regular_agent_tree_keeps_fast_hash_path_and_filters",
    "bisync::tests::links_remote::sync_links_task_legacy_peer_uses_metadata_instead_of_silently_incomplete_hashes",
    "bisync::engine_provider_task_tests::identity_tests::engine_provider_account_identity_preserves_state_locks_inputs_and_versions",
    "bisync::tests::safety::backup_failure_blocks_overwrite_and_delete",
    "bisync::tests::safety::stat_failure_blocks_reversible_overwrite",
    "bisync::pair_lock::tests::review_task_pair_lock_excludes_a_second_holder_until_dropped",
    "share::exec_grant_runtime::tests::exact_direct_target_enables_then_disable_cancels_and_denies",
    "share::exec_grant_runtime::tests::exact_room_member_policy_is_independent",
    "share::exec_grant_runtime::tests::review_task_online_extension_preserves_policy_after_offline_barrier",
    "share::exec_registry::tests::revoke_and_launch_commit_have_two_atomic_orderings",
    "share::exec_registry::tests::review_task_restriction_keeps_other_principal_launch_and_blocks_old_token",
], "C08": [
    "cloud::core_impl::startup_regression_task_tests::startup_regression_task_refresh_sends_preserved_client_id_and_token",
    "cloud::core_impl::startup_regression_task_tests::startup_regression_task_bad_config_stops_before_any_http_request",
    "local_access::directory_handle_tests::review_task_directory_handles_create_private_children_without_replacement",
    "local_access::directory_handle_tests::review_task_private_handle_hardening_refuses_hardlinked_records",
]}
WINDOWS_INTEGRATIONS = {
    "C08": ["local_access::platform::directory_handle::create::private_security::startup_regression_task_tests::" + name
            for name in ("startup_regression_task_repairs_existing_jobs_cloud_and_control_writes",
                         "startup_regression_task_new_ordinary_children_retain_owner_access",
                         "startup_regression_task_only_effective_user_or_default_owner_is_accepted")],
}


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "native" / filename)
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


def candidate(env):
    if env.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("This task runs only on the configured remote GitHub Actions runner.")
    requested = env.get("CANDIDATE_SHA", "")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=30).strip()
    if not re.fullmatch("[0-9a-f]{40}", requested) or requested != head or requested != env.get("GITHUB_SHA"):
        raise RuntimeError("Candidate input, event SHA and checked-out source must match exactly.")
    return requested


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, default=os.fspath), encoding="utf-8")


def handoff(shared, out, source, binaries):
    out.mkdir(parents=True, exist_ok=True)
    record = {"candidate": source, "artifacts": {}}
    for name, path in binaries.items():
        target = out / name
        shutil.copy2(path, target)
        record["artifacts"][name] = shared.sha(target)
    write_json(out / "provenance.json", record)
    return record


def discover(shared, binary, env, report):
    raw = subprocess.check_output([binary, "--list", "--format", "terse"], cwd=ROOT,
                                  env=env, text=True, timeout=120)
    available = {line.removesuffix(": test") for line in raw.splitlines() if line.endswith(": test")}
    required = [name for name in CASES if name not in ("C09", "C10")]
    if os.name != "nt":
        required.append("C10")
    mapping = {}
    for case in required:
        names = {name for name in available if CASES[case]["prefix"] in name}
        existing = INTEGRATIONS.get(case, []) + (WINDOWS_INTEGRATIONS.get(case, []) if os.name == "nt" else [])
        for name in existing:
            if name not in available:
                raise RuntimeError("Required existing whole flow is absent: " + name)
            names.add(name)
        if not names:
            raise RuntimeError("No compiled whole-flow scenario implements " + case)
        mapping[case] = sorted(names)
    write_json(report, {"candidate": env["CANDIDATE_SHA"], "cases": mapping})
    return mapping


def host_results(text, selected):
    # Serial pretty output prints the name before execution, then flushes the
    # result after any uncaptured diagnostics. An aborted case has no result.
    starts = list(re.finditer(r"^test (\S+) \.\.\. ", text, re.MULTILINE))
    footers = list(re.finditer(r"^(?:failures(?: \(time limit exceeded\))?|successes):[ \t]*$|^test result:",
                               text, re.MULTILINE))
    observed = {}
    for index, start in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(text)
        end = next((footer.start() for footer in footers if start.end() <= footer.start() < end), end)
        result = re.search(r"(ok|FAILED|ignored)(?:, [^\r\n]*)?\Z", text[start.end():end].rstrip())
        if result:
            observed[start.group(1)] = result.group(1)
    names = Counter(start.group(1) for start in starts)
    summaries = re.findall(r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured;",
                           text, re.MULTILINE)
    running = re.findall(r"^running (\d+) tests?$", text, re.MULTILINE)
    counters = tuple(map(int, summaries[0][1:])) if len(summaries) == 1 else None
    actual = tuple(Counter(observed.values())[status] for status in ("ok", "FAILED", "ignored"))
    exact = (set(names) == set(selected) == set(observed) and all(count == 1 for count in names.values())
             and running == [str(len(selected))] and counters is not None
             and counters == (*actual, 0) and sum(counters) == len(selected)
             and summaries[0][0] == ("FAILED" if actual[1] else "ok"))
    evidence = {"missing": sorted(set(selected) - observed.keys()),
                "unexpected": sorted(names.keys() - set(selected)),
                "duplicates": sorted(name for name, count in names.items() if count != 1),
                "reported_counters": counters, "completed_counters": actual}
    return observed, exact, evidence


def native_stage(args, env, source, logs, shared):
    records, failures = {}, []
    if os.name != "nt":
        required_auth = ["SE_DRIVE_TEST_CLIENT_ID", "SE_DRIVE_TEST_REFRESH_TOKEN"]
        missing_auth = [name for name in required_auth if not env.get(name, "").strip()]
        records["live-drive-authorization-inputs"] = {"required": required_auth, "missing": missing_auth}
        if missing_auth:
            failures.append("C10:authorization-inputs")
    def stage(name, operation):
        try:
            value = operation()
            records[name] = {"result": "passed", "evidence": value}
            return value
        except Exception as error:
            failures.append(name)
            records[name] = {"result": "failed", "error": str(error)}
            print(name + " failed: " + str(error), flush=True)
            return None
    shared.BASE = BASE
    stage("source-format", lambda: shared.format_patch(logs, env, source))
    cache = ROOT / ".review-task-cache"
    binary = stage("native-host", lambda: shared.artifact("native", "smart_explorer", "test", cache, logs, env))
    cli = stage("development-cli", lambda: shared.artifact("native", "se", "bin", cache, logs, env))
    server = stage("development-share-server", lambda: shared.artifact("share-server", "se-share-server", "bin", cache, logs, env))
    if args.out and cli and server:
        binaries = {"se.exe" if os.name == "nt" else "se": cli}
        if server:
            binaries["se-share-server"] = server
        stage("device-handoff", lambda: handoff(shared, args.out, source, binaries))
    mapping = stage("case-discovery", lambda: discover(shared, binary, env, logs / "cases.json")) if binary else None
    if mapping:
        selected = sorted({name for names in mapping.values() for name in names})
        profile = logs / "native-profile"
        profile.mkdir(parents=True, exist_ok=True)
        namespace = "sync-" + source[:12] + "-" + env["GITHUB_RUN_ID"] + "-" + env["GITHUB_RUN_ATTEMPT"]
        if len(namespace) > 48 or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*", namespace) is None:
            raise RuntimeError("Remote runtime cannot identify a valid isolated Windows credential/IPC namespace.")
        fixture_env = dict(env, APPDATA=str(profile), LOCALAPPDATA=str(profile / "local"),
                           XDG_CONFIG_HOME=str(profile / "config"), XDG_DATA_HOME=str(profile / "data"),
                           XDG_CACHE_HOME=str(profile / "cache"), RUST_BACKTRACE="1",
                           SMART_EXPLORER_E2E_TEST_NAMESPACE=namespace)
        provider = module("sync_reliability_providers", "sync-reliability-providers.py")
        with contextlib.ExitStack() as owned:
            try:
                additions = owned.enter_context(provider.fixtures(logs / "providers", fixture_env, cli, server))
                fixture_env.update(additions)
                records["provider-runtime"] = {"result": "passed"}
            except Exception as error:
                records["provider-runtime"] = {"result": "failed", "error": str(error)}
                failures.append("provider-runtime")
                # Other cases still run and C04 itself must fail for missing
                # runtime fixtures. Missing providers never become a skip.
            stage("whole-flow-host", lambda: shared.run(
                [binary, "--exact", *selected, "--include-ignored", "--test-threads=1",
                 "--no-capture", "--format=pretty", "--color=never"],
                logs / "whole-flow.log", fixture_env, seconds=10800))
            text = (logs / "whole-flow.log").read_text(errors="replace") if (logs / "whole-flow.log").exists() else ""
            observed, exact, evidence = host_results(text, selected)
            records["exact-execution"] = {"result": "passed" if exact else "failed", "evidence": evidence}
            if not exact:
                failures.append("exact-execution")
                records["exact-execution"]["error"] = "Actual test host did not report every selected case exactly once with matching counters."
            for case, names in mapping.items():
                records[case] = {"result": "passed" if all(observed.get(name) == "ok" for name in names) else "failed",
                                 "scenarios": {name: observed.get(name, "missing") for name in names}}
                if records[case]["result"] != "passed":
                    failures.append(case)
            stage("provider-cleanup", owned.close)
        if cli:
            old = module("sync_reliability_legacy_worker", "sync-reliability-legacy-worker.py")
            result = stage("published-worker-update", lambda: old.run(cli, logs / "legacy-worker", env, source))
            if result is None:
                records["C08"]["result"] = "failed"
                failures.append("C08")
            else:
                records["C08"]["published_worker"] = result
    return {"stage": "windows" if os.name == "nt" else "linux", "candidate": source,
            "contracts": CASES, "results": records, "failures": sorted(set(failures))}


def evaluate(args, env, source, logs):
    expected = {"linux", "windows", "android-build", "device"}
    reports = {}
    for path in args.reports.rglob("summary.json"):
        report = json.loads(path.read_text())
        if report.get("candidate") != source or report.get("stage") not in expected:
            raise RuntimeError("Unexpected candidate/stage in downloaded acceptance report: " + str(path))
        if report["stage"] in reports:
            raise RuntimeError("Duplicate stage evidence: " + report["stage"])
        reports[report["stage"]] = report
    failures = ["missing:" + name for name in sorted(expected - reports.keys())]
    failures += [name + ":" + failure for name, report in reports.items() for failure in report["failures"]]
    for case in CASES:
        owners = ["device"] if case == "C09" else ["linux"] if case == "C10" else ["linux", "windows"]
        for owner in owners:
            if reports.get(owner, {}).get("results", {}).get(case, {}).get("result") != "passed":
                failures.append(owner + ":" + case)
    return {"stage": "evaluate", "candidate": source, "contracts": CASES,
            "reports": reports, "failures": sorted(set(failures))}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--stage", choices=("native", "android-build", "device", "evaluate"), required=True)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--apks", type=Path)
    parser.add_argument("--handoff", type=Path)
    parser.add_argument("--reports", type=Path)
    args = parser.parse_args()
    if args.stage == "evaluate" and args.reports is None:
        parser.error("evaluate requires --reports")
    env = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="1", CARGO_PROFILE_TEST_DEBUG="0",
               CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never")
    source = candidate(env)
    shared = module("sync_reliability_native_shared", "review-task-native.py")
    signal.signal(signal.SIGTERM, shared.interrupted)
    stage_name = "windows" if args.stage == "native" and os.name == "nt" else "linux" if args.stage == "native" else args.stage
    logs = Path(env["RUNNER_TEMP"]) / "sync-reliability-task" / stage_name
    logs.mkdir(parents=True, exist_ok=True)
    env["SMART_EXPLORER_TASK_LOG_ROOT"] = str(logs)
    try:
        if args.stage == "native":
            report = native_stage(args, env, source, logs, shared)
        elif args.stage == "evaluate":
            report = evaluate(args, env, source, logs)
        else:
            android = module("sync_reliability_android", "sync-reliability-android.py")
            if args.stage == "android-build":
                if args.out is None:
                    raise RuntimeError("Android build stage requires its handoff directory.")
                value = android.build(logs, args.out, env, source)
                results = {"android-build": {"result": "passed", "evidence": value}}
            else:
                if args.apks is None:
                    raise RuntimeError("Device stage requires the candidate-bound APK handoff.")
                value = android.device(logs, args.handoff, args.apks, env, source)
                if value.get("case") != "C09" or value.get("candidate") != source or value.get("result") != "passed":
                    raise RuntimeError("Android did not return a successful device-authored C09 result.")
                results = {"C09": value}
            report = {"stage": stage_name, "candidate": source, "results": results, "failures": []}
    except Exception as error:
        report = {"stage": stage_name, "candidate": source, "results": {}, "failures": [str(error)]}
        print("Stage failed: " + str(error), flush=True)
    write_json(logs / "summary.json", report)
    print(json.dumps({"candidate": source, "stage": stage_name, "failures": report["failures"]}), flush=True)
    if report["failures"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
