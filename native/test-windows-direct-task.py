#!/usr/bin/env python3
"""One remote-only task suite for the Windows Direct connection batch (0.5.184).

Builds the native library test fixture incrementally (no release build) and
runs the batch's acceptances with the directly affected integrations: opening
a peer through the background worker, relay host names through the system
resolver, the unreachable-relay log line, clock skew of tracked Direct
envelopes, and the identity whose key the secure store lost.
"""
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
PREFIX = "windows_direct_task_"
REQUIRED = [
    # M1: the worker's own answer, not the client's socket timeout.
    "open_waits_for_the_worker_answer",
    "open_names_a_silent_worker",
    "open_retries_a_broken_connection",
    # M2: relay host names through the operating system.
    "relay_names_resolve_through_the_system",
    "first_answer_prefers_any_addresses",
    "endpoint_resolves_relays_through_the_system",
    # M3: a relay that stays unreachable reaches the log.
    "unreachable_relays_are_named_except_loopback",
    "unreachable_relay_is_named_in_the_log",
    # M4: receipts between clocks within the tolerated skew.
    "receipts_accept_clocks_within_the_skew",
    # M5: an identity file whose key the secure store lost.
    "unpaired_identity_without_key_is_replaced",
    "paired_identity_without_key_names_the_repair",
]
INTEGRATIONS = [
    "maximum_lifetime_and_clock_skew_are_accepted_for_every_envelope",
    "overlong_and_i64_max_envelopes_fail_before_authentication",
    "timestamps_beyond_clock_skew_fail_for_every_envelope",
    "review_task_decision_delivery_accepts_clock_skew_in_both_directions",
    "request_receipt_decision_and_decision_receipt_verify_end_to_end",
    "stale_generation_never_runs_an_acceptance_mutation",
    "a_device_rename_does_not_change_the_security_generation",
    "cleanup_marker_survives_cleanup_and_marker_write_failures",
    "worker_ping_requires_current_version",
    "direct_open_task_real_direct_and_room_reconnect_through_daemon_without_replay",
    "direct_open_task_open_failure_retries_but_terminal_failures_and_other_backends_do_not",
    "direct_open_task_stream_retry_restarts_bytes_through_real_daemon_ipc",
    "direct_open_task_reconnect_cannot_bypass_revoked_direct_access",
    "remote_drive_task_live_endpoint_refreshes_routes_but_not_identity",
    "windows_remote_task_live_peer_reconnects_using_fresh_lan_evidence",
]


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


def fixture_binary(args, logs, env):
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
    # Cargo reuses its validated dependency/incremental cache on this host.
    # Discover the executable from compiler-artifact records, not stale paths.
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
        metadata = {"build_inputs_sha256": fingerprint, "binary_sha256": sha256(cached)}
        (cache / "provenance.json").write_text(json.dumps(metadata), encoding="utf-8")
    return binary


def select(available):
    selected = []
    for suffix in REQUIRED:
        found = [name for name in available if name.endswith("::" + PREFIX + suffix)]
        if len(found) != 1:
            raise RuntimeError(f"Required batch acceptance is absent or ambiguous: {PREFIX}{suffix}")
        selected.extend(found)
    unlisted = [name for name in available if ("::" + PREFIX) in name and name not in selected]
    if unlisted:
        raise RuntimeError(f"Batch tests without an acceptance entry: {unlisted}")
    for suffix in INTEGRATIONS:
        found = [name for name in available if name.endswith("::" + suffix)]
        if len(found) != 1:
            raise RuntimeError(f"Required directly affected integration is absent or ambiguous: {suffix}")
        selected.extend(found)
    return sorted(set(selected))


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
        SMART_EXPLORER_COPY_PASTE_TASK="1",
        SMART_EXPLORER_E2E_TEST_NAMESPACE="windows_direct_" + candidate[:12])
    for name in ["SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"]:
        env.pop(name, None)
    binary = fixture_binary(args, logs, env).resolve()
    run([str(binary), "--list", "--format", "terse"], logs / "available-tests.txt", 60, env)
    available = [line.removesuffix(": test") for line in
                 (logs / "available-tests.txt").read_text(encoding="utf-8").splitlines()
                 if line.endswith(": test")]
    selected = select(available)
    (logs / "selection.json").write_text(json.dumps(selected, indent=2), encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="windows-direct-profile-") as profile:
        # Isolated profile: the tests never touch the runner user's data.
        env.update(APPDATA=str(Path(profile) / "roaming"), LOCALAPPDATA=str(Path(profile) / "local"),
            XDG_CONFIG_HOME=str(Path(profile) / "config"), XDG_DATA_HOME=str(Path(profile) / "data"))
        for name in ["APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME"]:
            Path(env[name]).mkdir(parents=True, exist_ok=True)
        run([str(binary), "--include-ignored", "--test-threads=1", "--exact", *selected],
            logs / "suite.log", 2400, env)
    result = (logs / "suite.log").read_text(encoding="utf-8", errors="replace")
    for name in selected:
        if f"test {name} ... ok" not in result:
            raise RuntimeError(f"Missing successful acceptance result: {name}")
    evidence = {"candidate": candidate, "binary_sha256": sha256(binary),
        "platform": sys.platform, "selected": selected, "result": "passed"}
    (logs / "acceptance.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(result)
    print("Windows Direct connection acceptance passed; candidate", candidate, flush=True)


if __name__ == "__main__":
    main()
