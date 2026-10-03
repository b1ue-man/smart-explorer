#!/usr/bin/env python3
"""Native stages of the single RV1 remote suite; no local execution."""
import argparse
import ast
import difflib
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
BASE = "7fc17ecf355b6473a506d756ad4c1fb2d2c0d173"
PREFIXES = ["review_task_", "rv1_remote_provider_task_", "sync_paths_task_", "sync_links_task_", "engine_provider_"]
EXTRA = ["walks_searches_and_hashes_report_listing_failures",
    "desktop_merge_preserves_crlf_final_separator_and_empty_line",
    "desktop_merge_excluding_all_lines_does_not_create_newline",
    "desktop_merge_rejects_lossy_and_mixed_text_without_changing_bytes",
    "desktop_job_state_failed_attempt_does_not_hide_prior_success",
    "desktop_job_state_block_takes_precedence_over_old_success_result",
    "apply_one_removes_action_only_after_success",
    "rename_swap_copies_both_final_paths_without_deleting_them",
    "rename_swap_applies_and_persists_both_final_paths",
    "ignored_remove_feed_never_becomes_a_delete_action",
    "canceled_and_over_budget_feeds_fail_closed"]
LINUX = ["link_like_destination_root_never_reaches_external_victim",
    "link_like_destination_child_never_receives_copied_content",
    "android_shared_storage_revoke_selection_and_weak_lifetime",
    "android_shared_storage_registration_covers_both_revoke_orderings",
    "android_sync_confirmation_consumed_once_preserves_later_change",
    "android_sync_start_errors_keep_the_shared_failure_kind",
    "android_sync_state_keeps_attempt_success_and_live_runner_distinct",
    "android_host_platform_unknown_totals_are_not_zero"]


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


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
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
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
            "native/app.rc", "native/app.manifest"]
    digest = hashlib.sha256()
    for name in sorted(filter(None, output(["git", "ls-files", "-z", "--", *inputs]).split("\0"))):
        digest.update(name.encode())
        digest.update(bytes.fromhex(sha(ROOT / name)))
    digest.update(output(["rustc", "-vV"]).encode())
    digest.update(json.dumps({"target": target, "host": sys.platform,
        "RUSTFLAGS": env.get("RUSTFLAGS", ""), "test_debug": 0, "dev_debug": 0,
        "incremental": 1}, sort_keys=True).encode())
    return digest.hexdigest()


def artifact(crate, name, kind, cache, logs, env):
    stamp = fingerprint(crate, (name, kind), env)
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
        args += ["-p", "smart_explorer"]
    args += ["--lib"] if name == "smart_explorer" else ["--bin", name]
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


def lan_facts(logs, env):
    if os.name == "nt":
        values = json.loads(output(["powershell", "-NoProfile", "-Command",
            "@((Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.AddressState -eq 'Preferred' }) | ForEach-Object { "
            "[pscustomobject]@{ip=$_.IPAddress;index=$_.InterfaceIndex;name=$_.InterfaceAlias} }) | ConvertTo-Json -Compress"]))
        if isinstance(values, dict):
            values = [values]
    else:
        values = [{"ip": address["local"], "index": interface["ifindex"], "name": interface["ifname"]}
            for interface in json.loads(output(["ip", "-j", "-4", "addr", "show", "up"]))
            for address in interface.get("addr_info", []) if address.get("family") == "inet"]
    choices = [value for value in values if any(ipaddress.ip_address(value["ip"]) in ipaddress.ip_network(net)
        for net in ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"])
        and not any(word in value["name"].lower() for word in ["docker", "veth", "loopback", "br-"])]
    if not choices:
        raise RuntimeError("The runner lacks a real private IPv4 interface for S09; no loopback fallback.")
    facts = choices[0]
    (logs / "lan-interface.json").write_text(json.dumps(facts, indent=2))
    env.update(SE_REVIEW_LAN_IP=facts["ip"], SE_REVIEW_LAN_IFINDEX=str(facts["index"]), SE_REVIEW_LAN_IFNAME=facts["name"])


def integrations():
    # Reuse the established compatibility manifest without executing its suite.
    tree = ast.parse((ROOT / "native/test-sync-paths-task.py").read_text())
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(name, ast.Name) and name.id == "INTEGRATIONS" for name in node.targets):
            return ast.literal_eval(node.value) + EXTRA + ([] if os.name == "nt" else LINUX)
    raise RuntimeError("Established compatibility manifest is absent.")


def native_tests(binary, logs, env, server=False):
    label = "server" if server else "native"
    listing = logs / (label + "-available.txt")
    run([binary, "--list", "--format", "terse"], listing, env, seconds=120)
    available = [line.removesuffix(": test") for line in listing.read_text().splitlines() if line.endswith(": test")]
    filters = ["review_task_"] if server else list(PREFIXES)
    if not server:
        for suffix in integrations():
            found = [name for name in available if name.endswith("::" + suffix)]
            if len(found) != 1:
                raise RuntimeError("Missing or ambiguous directly affected integration: " + suffix)
            filters.append(suffix)
    selected = [name for name in available if any(value in name for value in filters)]
    if not selected:
        raise RuntimeError("The fixture lacks RV1 acceptance cases.")
    if not server:
        for marker in ["review_task_y156_", "review_task_s09_transport_", "real_share_cross_peer",
            "engine_provider_publication_", "engine_provider_account_identity_"]:
            if not any(marker in name for name in selected):
                raise RuntimeError("Missing mandatory RV1 signal: " + marker)
    (logs / (label + "-selection.json")).write_text(json.dumps(selected, indent=2))
    args = [binary, "--include-ignored", "--test-threads=1", *filters]
    run_env = env
    if os.name != "nt" and not server:
        watch_limits = [name for name in selected if name.endswith("::review_task_watch_limit_falls_back_to_polling")]
        if len(watch_limits) != 1:
            raise RuntimeError("Missing or ambiguous real watch-limit acceptance.")
        run_env = dict(env, SE_REVIEW_WATCH_LIMIT_CASE=watch_limits[0])
        args = ["bash", ROOT / "native/review-task-runtime.sh", *args]
    log = logs / (label + "-suite.log")
    run(args, log, run_env, seconds=7200)
    result = log.read_text(errors="replace")
    for name in selected:
        if f"test {name} ... ok" not in result:
            raise RuntimeError("Missing successful acceptance: " + name)
    (logs / (label + "-acceptance.json")).write_text(json.dumps({"candidate": env["CANDIDATE_SHA"],
        "binary_sha256": sha(binary), "platform": sys.platform, "selected": selected, "result": "passed"}, indent=2))
    print(result[-16000:])


def published_legacy(logs):
    request = urllib.request.Request("https://api.github.com/repos/b1ue-man/smart-explorer/releases/tags/v0.5.126",
        headers={"Accept": "application/vnd.github+json", "User-Agent": "rv1-remote-suite"})
    with urllib.request.urlopen(request, timeout=60) as response:
        release = json.load(response)
    assets = {asset["name"]: asset["browser_download_url"] for asset in release["assets"]}
    for name in ["se", "se.sha256"]:
        with urllib.request.urlopen(assets[name], timeout=120) as response, (logs / name).open("wb") as out:
            shutil.copyfileobj(response, out)
    if (logs / "se.sha256").read_text().split()[0].lower() != sha(logs / "se"):
        raise RuntimeError("Published legacy CLI failed its accompanying SHA-256.")
    (logs / "se").chmod(0o700)
    return logs / "se"


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" and os.environ.get("SMART_EXPLORER_REMOTE_RUNNER") != "1":
        raise RuntimeError("Remote runner only.")
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary-cache", type=Path)
    parser.add_argument("--android-build", action="store_true")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    logs = Path(os.environ["SMART_EXPLORER_TASK_LOG_ROOT"]).resolve()
    logs.mkdir(parents=True, exist_ok=True)
    candidate = output(["git", "rev-parse", "HEAD"])
    if candidate != os.environ.get("CANDIDATE_SHA"):
        raise RuntimeError("Candidate differs from checkout.")
    signal.signal(signal.SIGTERM, interrupted)
    env = dict(os.environ, RUST_BACKTRACE="1", SMART_EXPLORER_ANALYTICS_TASK="1", SMART_EXPLORER_COPY_PASTE_TASK="1")
    if args.android_build:
        if args.out is None:
            raise RuntimeError("Android handoff needs --out.")
        run(["bash", ROOT / "android/test-android-task.sh", "review-build", "--out", args.out],
            logs / "android-build.log", env, seconds=12600)
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
    stage("format", lambda: format_patch(logs, env, candidate))
    with tempfile.TemporaryDirectory(prefix="rv1-native-") as profile:
        for name, child in {"APPDATA": "roaming", "LOCALAPPDATA": "local", "XDG_CONFIG_HOME": "config",
            "XDG_DATA_HOME": "data", "XDG_CACHE_HOME": "cache", "XDG_RUNTIME_DIR": "runtime"}.items():
            path = Path(profile) / child
            path.mkdir(mode=0o700)
            env[name] = str(path)
        fixture = stage("library-build", lambda: artifact("native", "smart_explorer", "test", args.binary_cache, logs, env))
        if fixture and stage("runner-interface", lambda: (lan_facts(logs, env), True)[1]):
            stage("native-behavior", lambda: native_tests(fixture, logs, env))
        if os.name != "nt":
            server_fixture = stage("server-build", lambda: artifact("share-server", "se-share-server", "test", args.binary_cache, logs, env))
            if server_fixture:
                stage("server-behavior", lambda: native_tests(server_fixture, logs, env, server=True))
            cli = stage("cli-development", lambda: artifact("native", "se", "dev", args.binary_cache, logs, env))
            server = stage("server-development", lambda: artifact("share-server", "se-share-server", "dev", args.binary_cache, logs, env))
            if cli and server:
                legacy = stage("published-legacy", lambda: published_legacy(logs))
                if legacy:
                    mixed_env = dict(env, SMART_EXPLORER_SE_BINARY=str(cli), SMART_EXPLORER_SHARE_SERVER_BINARY=str(server),
                        SMART_EXPLORER_LEGACY_SE_BINARY=str(legacy), SMART_EXPLORER_KEEP_E2E_ROOT="1", TMPDIR=str(logs))
                    stage("mixed-version", lambda: run(["bash", ROOT / "native/test-share-mixed-version-e2e.sh"],
                        logs / "mixed-version.log", mixed_env, seconds=1800))
                if args.out:
                    args.out.mkdir(parents=True, exist_ok=True)
                    records = {}
                    for name, binary in [("se", cli), ("se-share-server", server)]:
                        dest = args.out / name
                        shutil.copy2(binary, dest)
                        records[name] = sha(dest)
                    (args.out / "provenance.json").write_text(json.dumps({"candidate": candidate, "artifacts": records}, indent=2))
    (logs / "summary.json").write_text(json.dumps({"candidate": candidate, "failures": failures}, indent=2))
    if failures:
        raise SystemExit("\n".join(failures))
    print("RV1 native behavior acceptance passed for", candidate, flush=True)


if __name__ == "__main__":
    main()
