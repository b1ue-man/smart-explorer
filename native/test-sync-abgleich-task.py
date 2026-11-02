#!/usr/bin/env python3
"""One remote-only task suite for the batch of 2026-10-31 "Sync-Abgleich so
schnell wie die Speicheranalyse" (plan docs/plaene/2026-10-31-sync-abgleich-schnell/,
reading aid suite.md in the same folder).

Run only on GitHub Actions (.github/workflows/sync-abgleich-task.yml); never on
the workstation. Development builds for the functional and behaviour jobs,
release builds only for the measurement (`--release`, never the
release-artifact workflow). The suite loads the helpers of
native/test-speed-setup-task.py (process runner, test selection, baseline
comparison, Share fixture pieces, servers) and re-points its constants to this
batch (`BASELINE_TAG`, `PREFIXES`, `AFFECTED`).

Jobs (--job):
  fmt        rustfmt (stdin mode) of every Rust file changed since v0.5.189; a
             deviation fails only when it touches a changed line
  functional (Linux, Windows) every test whose name contains `abgleich_task_`
             must be in the test list (a source test missing from the list is
             an error unless it is gated to the other platform) and pass; the
             ignored and integration-target ones run too; then the directly
             affected modules (bisync, rscan, scanner, share, agent,
             agent_proto, transfer, copy, vfs, cli, mobile, daemon, app,
             analytics reclaim, syncjobs); a failure there counts only when
             v0.5.189 passes the same test. Linux first builds the embedded
             se-agent bundles from se-agent/ (native/build-agent-bundles.sh)
  behaviour  (Linux) equality and backward compatibility of the sync
             comparison and the recursive listing: v0.5.189 plus the two CLI
             commits 55fd7661 (`se sync compare`) and 9aea6715 (`se ls -R`)
             next to the candidate, on generated trees (files, folders, hidden
             dot files, links, a FIFO, an unreadable folder, a mounted
             sub-tree) for local-local, local-SFTP (no agent), local-SSH with
             se-agent over sshd, local-Direct Share and Share-Share, with
             job presets, include_hidden = false, cross_mounts on and off and
             filter patterns; plus candidate client against a v0.5.189 far
             side and the reverse. Then the merge rule (`se cp --conflict
             update`) local-local, SFTP, SSH agent, Direct Share, FTP, SMB
             (WebDAV: see the report) and the reverse copy
             (Windows: merge local-local)
             Plan E (folder keys, keys without file transfer), Linux only,
             counted in bytes that cross the loopback and container interfaces while one
             `se` command runs (/proc/net/dev, idle traffic subtracted; the
             job log does not serve: `se sync compare` writes none):
             (1) local-SSH agent, local-Direct Share and Share-Share: first
             comparison without stored keys, second comparison without a
             change, comparison after one file changed deep in the tree, each
             equal to v0.5.189 and far below its bytes; (2) checksum mode
             against SFTP without agent, SSH agent, Direct Share and FTP
             (a pyftpdlib with HASH/XMD5) moves no file content while
             v0.5.189 moves all of it (WebDAV: not reachable by the command
             line, named library tests instead); (3) `se cp --conflict
             update` of an unchanged folder onto an equal target moves no
             content and reports every file equal, and on a FAT volume files
             of equal size and equal content with times exactly one hour
             apart count as equal (other content: the times decide)
             Connection flow (plan 2026-10-31 section F), Linux only, five
             candidate instances (stage "rights") and two v0.5.189/candidate
             instances on one development Share server, checked by the other
             device's real commands (ls, cat, cp, exec, screen probe):
             (1) a Direct request accepted without rights: the connection
             exists, `grants pending` names it, read, write, commands and
             screen are refused; `grants confirm --preset read-only` reads
             but does not write, `--preset read-write` writes, `--preset all`
             runs commands; `grants cancel` on a second new connection keeps
             it without rights; (2) a Direct code with `identity --preset
             files-exec`, a PIN offer with `discoverable --preset read-write`
             and a room with `room create --preset read-only` give a new
             device or member exactly those rights without a confirmation;
             (3) a confirmation with `--folder` shows the device only that
             folder while another device still sees all; (4) a share made by
             v0.5.189 keeps its rights when the candidate is started on its
             profile
  measure    (Linux; Windows local) wall time of `se sync compare` (first
             comparison, second comparison without a change, checksum mode)
             and `se ls -R` for v0.5.189 (plus the two commits) and the
             candidate on one generated tree (100 000 files in 10 000 folders,
             an assumption; checksum mode on 20 000 files in 2 000 folders),
             per side combination, cold (page cache dropped where the runner
             allows it) and warm, `--repetitions` times, median, with the
             wire bytes of each run; references on the same data: `find`,
             `du`, `se search` (Windows: robocopy /L)

Outputs in --log-root: summary.md (appended to the step summary),
summary.json (checks), measurements.json (every run), logs per stage.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import random
import re
import secrets
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import types

NATIVE = Path(__file__).resolve().parent
ROOT = NATIVE.parent
WINDOWS = os.name == "nt"
EXE = ".exe" if WINDOWS else ""
BASELINE_TAG = "v0.5.189"
CLI_COMMITS = ("55fd7661e8ac745dcb710b6174a14d653df264db",  # `se sync compare`
               "9aea6715b4135911074d4387ef4db7653a43b3fe")  # `se ls -R`
TEST_PREFIX = "abgleich_task_"
BUNDLES = ("se-agent-x86_64-linux-musl", "se-agent-aarch64-linux-musl")
AFFECTED = ("bisync::", "rscan::", "scanner::", "share::", "agent::", "agent_proto::", "transfer::", "copy::",
            "vfs::", "cli::", "mobile::", "daemon::", "app::", "reclaim::", "syncjobs::")


def _load_speed():
    spec = importlib.util.spec_from_file_location("speed_setup_task", NATIVE / "test-speed-setup-task.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


speed = _load_speed()
ai = speed.ai
ensure = ai.ensure
# The helpers of the earlier suite read these module constants.
speed.BASELINE_TAG = BASELINE_TAG
speed.PREFIXES = (TEST_PREFIX,)
speed.MOVED = ()
speed.AFFECTED = AFFECTED

run = speed.run
leaf = speed.leaf
remove = speed.remove


# --------------------------------------------------------------------------- small helpers

def slug(text):
    return re.sub(r"[^A-Za-z0-9]+", "-", text).strip("-").lower()


def note(notes, text):
    notes.append(speed.redact(text))
    print(f"[note] {speed.redact(text)}", flush=True)


def short(ran, size=600):
    return speed.redact((ran.stdout + "\n" + ran.stderr)[-size:])


_SUDO = []


def sudo_ok():
    if not _SUDO:
        _SUDO.append(speed.sudo_ok())
    return _SUDO[0]


# --------------------------------------------------------------------------- bytes over the connection (Linux)

def wire_bytes():
    """Bytes that crossed the runner's connection interfaces so far (Linux, /proc/net/dev): the bytes
    received on `lo` (every packet between two processes of this runner, such as SSH to a container's
    published port, FTP, the Share's QUIC datagrams and their relay, is received there once, so the
    difference over one command is its traffic in both directions) plus the bytes received and sent on
    the host ends of container networks (`veth*`), which carry the same connections when Docker routes
    a published port without its user-space proxy. The sum is a measure, not a protocol trace: a
    connection that passes both is counted twice, in the same way for v0.5.189 and the candidate. None
    where the counters do not exist (Windows)."""
    try:
        total, seen = 0, False
        for line in Path("/proc/net/dev").read_text(encoding="utf-8").splitlines():
            name, _, rest = line.partition(":")
            name, fields = name.strip(), rest.split()
            if name == "lo":
                total, seen = total + int(fields[0]), True
            elif name.startswith("veth"):
                total += int(fields[0]) + int(fields[8])
        return total if seen else None
    except (OSError, ValueError, IndexError):
        return None


_IDLE_RATE = [0.0]


def calibrate_idle(seconds=3.0):
    """Bytes per second that cross the counted interfaces while nothing of the test runs (keepalives of the
    Share server and the hosts, containers); every metered run subtracts this rate times its duration."""
    before = wire_bytes()
    if before is None:
        return 0.0
    time.sleep(seconds)
    after = wire_bytes()
    _IDLE_RATE[0] = max(0.0, (after - before) / seconds) if after is not None else 0.0
    return _IDLE_RATE[0]


def metered(command, env=None, timeout=1800, log=None):
    """(result of the command, bytes that crossed the counted interfaces while it ran minus the idle
    traffic; None where the counters do not exist)."""
    before = wire_bytes()
    ran = run(command, env=env, timeout=timeout, log=log)
    after = wire_bytes()
    if before is None or after is None:
        return ran, None
    return ran, max(0, after - before - int(_IDLE_RATE[0] * ran.seconds))


def human(size):
    if size is None:
        return "n/a"
    value, unit = float(size), "B"
    for unit in ("B", "KiB", "MiB", "GiB"):
        if value < 1024 or unit == "GiB":
            break
        value /= 1024
    return f"{value:.0f} {unit}" if unit == "B" else f"{value:.1f} {unit}"


def keys_dir(env):
    """Where this `se` stores the folder keys of its sync comparisons (`sync_data_dir()/folder-keys`)."""
    base = env.get("APPDATA") if WINDOWS else (env.get("XDG_DATA_HOME") or str(Path(env["HOME"]) / ".local" / "share"))
    return Path(base) / "smart_explorer" / "sync" / "folder-keys"


def forget_keys(env):
    remove(keys_dir(env))


def job_work(name):
    work = Path(os.environ.get("RUNNER_TEMP") or tempfile.gettempdir()) / f"se-abgleich-{name}"
    remove(work)
    work.mkdir(parents=True)
    return work


class Ctx:
    """State of one job: results, work folder, logs, binaries, collected runs, notes."""

    def __init__(self, results, work, log_root):
        self.results, self.work, self.log_root = results, work, Path(log_root)
        self.bins, self.matrix, self.notes, self.trees, self.facts = {}, {}, [], [], {}
        self.bundle_sha = {}
        # Plan E: {(way, kind, variant): runs}, rows of the byte table, generated trees by kind and role.
        self.series, self.byte_rows, self.templates = {}, [], {}
        # The development share server, built once per job (build_share_server).
        self.server, self.server_tried = None, False


# --------------------------------------------------------------------------- job functional

def declared_tests(folders):
    """{name: (path, line)} of every `#[test]` function whose name contains the prefix."""
    found = {}
    for folder in folders:
        for path in sorted(Path(folder).rglob("*.rs")):
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
            for number, line in enumerate(lines):
                match = re.search(r"\bfn (\w*" + TEST_PREFIX + r"\w*)\s*[(<]", line)
                if match and any("#[test" in before or "::test]" in before or "#[tokio::test" in before
                                 for before in lines[max(0, number - 6):number]):
                    found[match.group(1)] = (path, number)
    return found


def attribute_block(lines, number):
    """The attribute and comment lines directly above line `number` (the declaration they belong to)."""
    block, index = [], number - 1
    while index >= 0 and lines[index].strip().startswith(("#", "//")):
        block.append(lines[index])
        index -= 1
    return block


def enclosing_modules(lines, number):
    """The attribute blocks of the inline modules (`mod name {`) that enclose line `number`."""
    blocks, indent = [], len(lines[number]) - len(lines[number].lstrip())
    for index in range(number - 1, -1, -1):
        line = lines[index]
        own = len(line) - len(line.lstrip())
        if own < indent and re.match(r"\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{", line):
            blocks.extend(attribute_block(lines, index))
            indent = own
    return blocks


_INCLUDERS = None


def includers():
    """{resolved source file: [(including file, line of its declaration)]} over the native sources: a
    `#[path = "..."]` declaration (relative to the including file's folder), or a plain `mod name;`
    (`name.rs` or `name/mod.rs` beside a `mod.rs`/`lib.rs`/`main.rs`, else in the folder of the file's stem)."""
    global _INCLUDERS
    if _INCLUDERS is None:
        _INCLUDERS = {}
        for source in sorted((NATIVE / "src").rglob("*.rs")):
            lines = source.read_text(encoding="utf-8", errors="replace").splitlines()
            for number, line in enumerate(lines):
                match = re.match(r'\s*#\[path\s*=\s*"([^"]+)"\]', line)
                if match:
                    target = (source.parent / match.group(1)).resolve()
                    _INCLUDERS.setdefault(target, []).append((source, number))
                    continue
                plain = re.match(r"\s*(pub(\([^)]*\))?\s+)?mod\s+(\w+)\s*;", line)
                if not plain or any("#[path" in above for above in attribute_block(lines, number)):
                    continue
                base = source.parent if source.name in ("mod.rs", "lib.rs", "main.rs") \
                    else source.parent / source.stem
                for candidate in (base / f"{plain.group(3)}.rs", base / plain.group(3) / "mod.rs"):
                    if candidate.is_file():
                        _INCLUDERS.setdefault(candidate.resolve(), []).append((source, number))
    return _INCLUDERS


def excluded_here(texts):
    """True when cfg texts keep code off this runner's platform; `not(x)` counts for the other platform."""
    text = " ".join(line for line in texts if "cfg(" in line)
    negated = re.findall(r"not\(\s*([^()]*?)\s*\)", text)
    positive = re.sub(r"not\(\s*[^()]*?\s*\)", " ", text)
    unix_word = r'\bunix\b|target_os\s*=\s*"(linux|android|macos)"'
    windows_word = r'\bwindows\b|target_os\s*=\s*"windows"'
    if WINDOWS:
        return bool(re.search(unix_word, positive)) or any(re.search(windows_word, n) for n in negated)
    return bool(re.search(windows_word, positive)) or any(re.search(r"\bunix\b", n) for n in negated)


def file_excluded_here(path, depth=0):
    """True when file `path` is not compiled on this runner's platform: its `#![cfg(...)]` head keeps it
    off, or it is included with `#[path]` and every one of those declarations is kept off (by its own
    attributes, an inline module around it, or the including file)."""
    lines = Path(path).read_text(encoding="utf-8", errors="replace").splitlines()
    if excluded_here([line for line in lines[:40] if line.startswith("#![cfg(")]):
        return True
    including = includers().get(Path(path).resolve(), [])
    if not including or depth >= 8:
        return False
    for source, number in including:
        outer = source.read_text(encoding="utf-8", errors="replace").splitlines()
        declaration = number
        while declaration + 1 < len(outer) and outer[declaration].strip().startswith("#"):
            declaration += 1
        texts = attribute_block(outer, declaration) + enclosing_modules(outer, number)
        if not excluded_here(texts) and not file_excluded_here(source, depth + 1):
            return False
    return True


def gated_elsewhere(path, number):
    """True when the test is cfg-gated to the platform this runner is not: by the attributes of the test,
    of the inline modules around it, of its file, or of the `#[path]` declarations that include the file."""
    lines = Path(path).read_text(encoding="utf-8", errors="replace").splitlines()
    hint = str(path).lower()
    if any(word in hint for word in (("linux", "unix") if WINDOWS else ("windows",))):
        return True
    return excluded_here(attribute_block(lines, number) + enclosing_modules(lines, number)) \
        or file_excluded_here(path)


# Library tests that carry the claims the command line cannot reach (WebDAV is connected over HTTPS with the
# built-in web roots only, so no fixture server serves `se`; Google Drive needs an account) or that no fixture
# server can show (an FTP server with XMD5, the shape of the key phases). Each must exist and, being named
# `abgleich_task_`, passes with the rest of the prefix.
EVIDENCE_TESTS = (
    "abgleich_task_webdav_md5_comes_from_the_checksum_property_without_a_download",
    "abgleich_task_webdav_digest_comes_from_the_checksum_property_without_a_download",
    "abgleich_task_ftp_md5_by_hash_command_without_a_data_connection",
    "abgleich_task_ftp_md5_by_xmd5_command_takes_251_250_and_213",
    "abgleich_task_sftp_md5_flow_asks_the_server_and_checks_size_and_time",
    "abgleich_task_drive_digest_is_the_checksum_google_reports_not_a_download",
    "abgleich_task_keyed_walk_sends_only_what_changed_and_equals_the_full_walk",
    "abgleich_task_first_comparison_sends_keys_then_only_unequal_folders",
    "abgleich_task_local_side_keys_spare_the_remote_side_equal_folders",
    "abgleich_task_fat_hour_with_content_keys_follows_the_keys",
    # The filter on the side with the files leaves out no more than the client's globset (deletion safety).
    "abgleich_task_host_patterns_mean_what_globset_means",
    "abgleich_task_host_patterns_of_a_whole_job_mean_what_its_set_means",
    "abgleich_task_agent_walk_hashes_and_sends_nothing_the_filter_leaves_out",
    "abgleich_task_agent_fresh_walk_never_answers_from_the_digest_cache",
    # A local side forms its folder keys in its one snapshot walk, equal to the agent's (review 3 G6).
    "abgleich_task_local_side_keys_come_from_its_one_walk_and_equal_the_agents",
)


def stage_bundles(results, log_root):
    ran = run(["bash", "native/build-agent-bundles.sh"], cwd=ROOT, timeout=3600, log=log_root / "agent-bundles.log")
    results.record("agent", "se-agent bundles built from se-agent/ (native/build-agent-bundles.sh)", ran.ok,
                   "" if ran.ok else short(ran, 2500))
    if ran.ok:
        # The complete release rebuilds the bundles (native/publish-release-local.ps1
        # runs build-agent-bundles.sh) and commits them with the release, so a
        # batch that changes se-agent/ or agent_proto is expected to differ here.
        # The tests of this run embed the fresh bundles built above.
        same = run(["git", "diff", "--quiet", "--exit-code", "--", "native/agent-bin"], cwd=ROOT, timeout=300)
        print("[NOTE] agent: committed native/agent-bin "
              + ("equals the fresh build" if same.ok else
                 "differs from the fresh build; this run uses the fresh bundles, the release commits them"),
              flush=True)
    return ran.ok


def stage_listing(results, listed, ignored):
    stage = "listing"
    leafs = {leaf(name) for name in listed}
    declared = declared_tests([NATIVE / "src"])
    results.record(stage, f"tests named `{TEST_PREFIX}` exist in native/src", bool(declared),
                   "no test function with that name found")
    missing = sorted(name for name in declared if name not in leafs)
    excused = [name for name in missing if gated_elsewhere(*declared[name])]
    failed = [name for name in missing if name not in excused]
    results.record(stage, f"every `{TEST_PREFIX}` test of the sources is in the test list "
                   f"({len(declared) - len(missing)} listed, {len(excused)} gated to the other platform)",
                   not failed, "missing from the list (not compiled or misspelled): " + ", ".join(failed))
    absent = [name for name in EVIDENCE_TESTS if name not in leafs
              and (name not in declared or not gated_elsewhere(*declared[name]))]
    results.record(stage, f"the {len(EVIDENCE_TESTS)} library tests for WebDAV, FTP, SFTP, Drive, the key phases "
                   "the FAT hour shift, the host filter and the fresh reads are in the test list", not absent, "missing: " + ", ".join(absent))
    plain = sorted(name for name in listed if TEST_PREFIX in leaf(name) and name not in ignored)
    results.record(stage, f"{len(plain)} plain `{TEST_PREFIX}` tests are listed", bool(plain), "")
    return declared


def run_ignored(results, stage, executable, names, env, log, cwd=NATIVE):
    """Ignored tests by exact full name; each one must pass."""
    if not names:
        return
    ran = run([executable, "--ignored", "--test-threads=1", "--exact", *names], cwd=cwd, env=env, timeout=3 * 3600,
              log=log)
    outcomes = speed.test_outcomes(ran.output)
    for name in names:
        results.record(stage, f"{leaf(name)} (ignored, run explicitly)", outcomes.get(name) == "ok",
                       "" if outcomes.get(name) == "ok" else speed.failure_detail(ran.output))


def integration_targets():
    """Names of the integration test targets in native/tests whose sources hold a prefixed test."""
    targets = set()
    base = NATIVE / "tests"
    for path in declared_tests([base]).values():
        relative = path[0].relative_to(base)
        targets.add(relative.parts[0].removesuffix(".rs"))
    return sorted(targets)


def stage_integration(results, log_root, work, env):
    targets = integration_targets()
    results.record("integration", f"integration targets with `{TEST_PREFIX}` tests: {', '.join(targets) or 'none'}",
                   True)
    for target in targets:
        executable, built = speed.cargo_executable(["test", "--locked", "--test", target], target,
                                                   log_root / f"integration-{target}-build.log", env=env)
        results.record("integration", f"{target} builds", bool(executable), "" if executable else built.tail(3000))
        if not executable:
            continue
        ignored = set(speed.list_tests(executable, ignored=True, env=env))
        every = [name for name in speed.list_tests(executable, env=env) if TEST_PREFIX in leaf(name)]
        plain = [name for name in every if name not in ignored]
        if plain:
            ran = run([executable, "--test-threads=1", TEST_PREFIX], cwd=NATIVE, env=env, timeout=3 * 3600,
                      log=log_root / f"integration-{target}.log")
            outcomes = speed.test_outcomes(ran.output)
            for name in plain:
                results.record("integration", f"{target}: {leaf(name)}", outcomes.get(name) == "ok",
                               "" if outcomes.get(name) == "ok" else speed.failure_detail(ran.output))
        heavy = [name for name in every if name in ignored]
        if not heavy:
            continue
        servers, environment = None, dict(env)
        try:
            if not WINDOWS:
                servers = speed.Servers(work, log_root).start(["sftp", "ftp", "smb", "webdav"])
                for kind, reason in servers.failed.items():
                    results.record("integration", f"{kind} server for the ignored cases", False, reason)
                environment.update(servers.env())
            run_ignored(results, "integration", executable, heavy, environment,
                        log_root / f"integration-{target}-ignored.log")
        finally:
            if servers is not None:
                servers.close()


def job_functional(results, log_root):
    work = Path(tempfile.mkdtemp(prefix="se-abgleich-functional-"))
    env = dict(os.environ, SMART_EXPLORER_COPY_PASTE_TASK="1", SMART_EXPLORER_GUI_TASK="1", RUST_BACKTRACE="1")
    profile = work / "app-profile"
    for variable in (("APPDATA",) if WINDOWS else ("XDG_DATA_HOME", "XDG_CONFIG_HOME")):
        (profile / variable.lower()).mkdir(parents=True, exist_ok=True)
        env[variable] = str(profile / variable.lower())
    try:
        if not WINDOWS:
            stage_bundles(results, log_root)
        executable, built = speed.cargo_executable(["test", "--locked", "--lib"], "smart_explorer",
                                                   log_root / "build.log", env=env)
        results.record("build", "library test binary (development profile)", bool(executable),
                       "" if executable else built.tail(4000))
        if not executable:
            return
        try:
            speed.stage_unit(results, log_root, executable, env)
        except Exception as error:  # noqa: BLE001 - every stage runs
            results.record("unit", "stage", False, f"{type(error).__name__}: {error}")
        try:
            listed = speed.list_tests(executable, env=env)
            ignored = set(speed.list_tests(executable, ignored=True, env=env))
            stage_listing(results, listed, ignored)
            heavy = [name for name in listed if TEST_PREFIX in leaf(name) and name in ignored]
            servers, environment = None, dict(env)
            try:
                if heavy and not WINDOWS:
                    servers = speed.Servers(work, log_root).start(["sftp", "ftp", "smb", "webdav"])
                    environment.update(servers.env())
                run_ignored(results, "unit", executable, heavy, environment, log_root / "unit-ignored.log")
            finally:
                if servers is not None:
                    servers.close()
        except Exception as error:  # noqa: BLE001
            results.record("listing", "stage", False, f"{type(error).__name__}: {error}")
        try:
            stage_integration(results, log_root, work, env)
        except Exception as error:  # noqa: BLE001
            results.record("integration", "stage", False, f"{type(error).__name__}: {error}")
        try:
            speed.stage_baseline(results, log_root, env, getattr(speed.stage_unit, "affected_failures", []),
                                 getattr(speed.stage_unit, "selection", []))
        except Exception as error:  # noqa: BLE001
            results.record("unit", "baseline comparison", False, f"{type(error).__name__}: {error}")
    finally:
        shutil.rmtree(work, ignore_errors=True)


# --------------------------------------------------------------------------- builds (behaviour, measure)

def git_bytes(ref_path):
    done = subprocess.run(["git", "show", ref_path], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    ensure(done.returncode == 0, f"git show {ref_path}: {done.stderr.decode(errors='replace')[:300]}")
    return done.stdout


def add_worktree(ctx, name, ref, *, cli_commits, bundles):
    """A detached worktree of `ref`; `cli_commits` adds the native part of the two CLI commits;
    `bundles` is 'old' (the tag's committed bundles), 'new' (the freshly built ones) or None (as checked out)."""
    tree = ctx.work / name
    log = ctx.log_root / "worktrees.log"
    added = run(["git", "worktree", "add", "--detach", tree, ref], cwd=ROOT, timeout=600, log=log)
    if not added.ok:
        ctx.results.record("build", f"worktree {name} ({ref})", False, short(added))
        return None
    ctx.trees.append(tree)
    if cli_commits:
        for commit in CLI_COMMITS:
            patch = run(["git", "diff", "--binary", f"{commit}^", commit, "--", "native"], cwd=ROOT, timeout=300)
            applied = run(["git", "apply", "--whitespace=nowarn", "-"], cwd=tree, input_text=patch.stdout,
                          timeout=300, log=log)
            if not patch.ok or not patch.stdout.strip() or not applied.ok:
                ctx.results.record("build", f"{name}: native part of {commit} applies to {ref}", False,
                                   short(applied))
                return None
        for needed in ("native/src/cli/sync_jobs/compare.rs", "native/src/cli/ls_recursive.rs"):
            if not (tree / needed).is_file():
                ctx.results.record("build", f"{name}: {needed} present after the two CLI commits", False, "")
                return None
    if bundles in ("old", "new"):
        for bundle in BUNDLES:
            target = tree / "native" / "agent-bin" / bundle
            if bundles == "old":
                target.write_bytes(git_bytes(f"{BASELINE_TAG}:native/agent-bin/{bundle}"))
            else:
                shutil.copy2(NATIVE / "agent-bin" / bundle, target)
            target.chmod(0o755)
    return tree


def build_se(ctx, label, native_dir, release):
    profile = "release" if release else "debug"
    env = dict(os.environ, CARGO_TARGET_DIR=str(NATIVE / "target"))
    args = ["cargo", "build", "--locked", "--bin", "se", *(["--release"] if release else [])]
    ran = run(args, cwd=native_dir, env=env, timeout=4 * 3600, log=ctx.log_root / f"build-{label}.log")
    ctx.results.record("build", f"se {label} ({profile})", ran.ok, "" if ran.ok else short(ran, 3000))
    if not ran.ok:
        return None
    destination = ctx.work / "bins" / label / f"se{EXE}"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(NATIVE / "target" / profile / f"se{EXE}", destination)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    ctx.facts[f"se {label}"] = f"sha256 {digest[:16]}"
    bundle = native_dir / "agent-bin" / BUNDLES[0]
    if bundle.is_file():
        ctx.bundle_sha[label] = hashlib.sha256(bundle.read_bytes()).hexdigest()
    return destination


def build_variants(ctx, release, hybrids):
    """new (the candidate), old (v0.5.189 + the two CLI commits), and with `hybrids` the candidate
    client with v0.5.189's agent (new_oldagent) and v0.5.189's client with the candidate's agent."""
    ctx.bins["new"] = build_se(ctx, "new", NATIVE, release)
    old = add_worktree(ctx, "old-src", BASELINE_TAG, cli_commits=True, bundles=None)
    ctx.bins["old"] = build_se(ctx, "old", old / "native", release) if old else None
    if hybrids:
        mixed = add_worktree(ctx, "new-oldagent-src", "HEAD", cli_commits=False, bundles="old")
        ctx.bins["new_oldagent"] = build_se(ctx, "new_oldagent", mixed / "native", release) if mixed else None
        mixed = add_worktree(ctx, "old-newagent-src", BASELINE_TAG, cli_commits=True, bundles="new")
        ctx.bins["old_newagent"] = build_se(ctx, "old_newagent", mixed / "native", release) if mixed else None
    for label in ("old", "new"):
        ctx.facts[f"agent bundle of {label}"] = ctx.bundle_sha.get(label, "unknown")[:16]


# GitHub refuses a git blob above 100 MiB (GH001). Until 2026-11-02 the complete release committed every feed
# payload; the sizes used here are those of the v0.5.189 release assets with the same names (assumption: an
# asset carries the same bytes as its former payload). The growth of the release `se` from v0.5.189 (with the
# two CLI commits) to the candidate, built here with the same toolchain, estimates the growth of the release
# payloads before the three-hour release build would find it.
GITHUB_BLOB_LIMIT = 100 * 1024 * 1024
PAYLOADS = ("smart_explorer.exe", "se.exe") if WINDOWS else ("smart_explorer", "se")


def release_sizes(tag):
    """Name -> size in bytes of every asset of the GitHub release `tag` (gh api; GH_TOKEN is set in the job)."""
    repo = os.environ.get("GITHUB_REPOSITORY", "b1ue-man/smart-explorer")
    ran = run(["gh", "api", f"repos/{repo}/releases/tags/{tag}"], cwd=ROOT, timeout=300)
    ensure(ran.ok, f"gh api release {tag}: {ran.tail(400)}")
    return {asset["name"]: int(asset["size"]) for asset in json.loads(ran.stdout)["assets"]}


def stage_size(ctx):
    new, old = ctx.bins.get("new"), ctx.bins.get("old")
    if not (new and old):
        return
    growth = new.stat().st_size - old.stat().st_size
    ctx.facts["release se growth over v0.5.189"] = f"{growth / 2**20:+.2f} MiB ({growth:+d} bytes)"
    sizes = release_sizes(BASELINE_TAG)
    for payload in PAYLOADS:
        ensure(payload in sizes, f"release {BASELINE_TAG} has no asset {payload}")
        released = sizes[payload]
        projected = released + growth
        margin = GITHUB_BLOB_LIMIT - projected
        ctx.facts[f"{payload} projected"] = (f"{projected / 2**20:.2f} MiB (v0.5.189 release asset "
                                             f"{released / 2**20:.2f} MiB, {margin / 2**20:.2f} MiB below the limit)")
        ctx.results.record("size", f"{payload}: v0.5.189 release asset size plus the growth of the release se stays "
                           "below GitHub's 100 MiB blob limit (estimate)", margin > 0,
                           f"projected {projected} bytes, limit {GITHUB_BLOB_LIMIT}")
        if 0 < margin < 2 * 2**20:
            note(ctx.notes, f"{payload}: less than 2 MiB below the 100 MiB limit by this estimate; a size "
                 "measure is due before the release")


def drop_worktrees(ctx):
    for tree in ctx.trees:
        run(["git", "worktree", "remove", "--force", str(tree)], cwd=ROOT, timeout=300)
    run(["git", "worktree", "prune"], cwd=ROOT, timeout=300)


# --------------------------------------------------------------------------- generated trees

BASE = int(time.time()) // 3600 * 3600 - 30 * 86400  # whole hour, 30 days ago (FTP lists the time of recent files)
MOUNTS = []


def put(path, data, mtime):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    os.utime(path, (mtime, mtime))


def mount_tmpfs(path):
    """A small tmpfs on `path` (owned by the runner user); False when this runner cannot mount."""
    path.mkdir(parents=True, exist_ok=True)
    if WINDOWS or not sudo_ok():
        return False
    mounted = run(["sudo", "-n", "mount", "-t", "tmpfs", "-o", f"uid={os.getuid()},gid={os.getgid()},mode=0755,"
                   "size=64m", "se-abgleich", str(path)], timeout=60)
    if mounted.ok:
        MOUNTS.append(path)
    return mounted.ok


def unmount_all(prefix=None):
    for path in list(reversed(MOUNTS)):
        if prefix is None or str(path).startswith(str(prefix)):
            run(["sudo", "-n", "umount", "-l", str(path)], timeout=60)
            MOUNTS.remove(path)


def normalize_dirs(root):
    for folder, _dirs, _names in os.walk(root, topdown=False):
        os.utime(folder, (BASE, BASE))


def make_tree(root, role, notes=None):
    """The rich equality tree of one side (`role` a or b): shared files with equal time and size, files
    that differ on both sides (conflicts), files of one side only, hidden dot files and folders, names
    with a space and non-ASCII letters, an empty folder, filter targets, links, a FIFO and a mounted
    sub-tree. Every folder carries the same time so two copies of a side list alike."""
    root, a = Path(root), role == "a"
    for folder in range(12):
        for number in range(8):
            put(root / f"d{folder:02d}" / f"file{number}.txt", f"common {folder} {number}\n".encode() * (number + 1),
                BASE)
    put(root / "d00" / "diff.txt", b"side a content\n" if a else b"side b content, longer\n", BASE + (0 if a else 7200))
    put(root / f"only_{role}" / "x.txt", f"only {role}\n".encode(), BASE)
    put(root / "d01" / f"only_{role}.txt", f"only {role} in d01\n".encode(), BASE)
    put(root / "deep" / "a" / "b" / "c" / "leaf.bin", bytes(range(256)), BASE)
    put(root / ".hidden_file", b"hidden\n", BASE)
    put(root / ".hidden_dir" / "inside.txt", b"inside hidden\n", BASE)
    put(root / "d03" / ".dot_in_folder", b"dot\n", BASE)
    put(root / "ünï cödé.txt", b"unicode name\n", BASE)
    put(root / "with space.txt", b"space\n", BASE)
    put(root / "dir with space" / "inner.txt", b"inner\n", BASE)
    put(root / "zero.bin", b"", BASE)
    (root / "empty_dir").mkdir(exist_ok=True)
    put(root / "skip.tmp", b"tmp\n", BASE)
    put(root / "d02" / "nested.tmp", b"tmp\n", BASE)
    put(root / "node_modules" / "pkg" / "index.js", b"js\n", BASE)
    if not WINDOWS:
        os.symlink("d00/file0.txt", root / "link_file")
        os.symlink("d00", root / "link_dir")
        os.symlink("missing_target", root / "link_dangling")
        os.mkfifo(root / "fifo")
        for special in ("link_file", "link_dir", "link_dangling", "fifo"):
            os.utime(root / special, (BASE, BASE), follow_symlinks=False)
    mounted = mount_tmpfs(root / "mnt")
    if not mounted and notes is not None and not WINDOWS:
        note(notes, "mounted sub-tree: sudo -n mount is not available, `mnt` is a plain folder here")
    put(root / "mnt" / "in_mount.txt", b"in the mount\n", BASE)
    put(root / "mnt" / "sub" / "more.txt", b"more in the mount\n", BASE)
    normalize_dirs(root)


def make_lk_tree(root, role):
    """A small tree with an unreadable folder (no permissions for anybody)."""
    root = Path(root)
    put(root / "ok" / "a.txt", b"readable\n", BASE)
    put(root / "locked" / "secret.txt", b"secret\n", BASE)
    put(root / f"only_{role}.txt", f"only {role}\n".encode(), BASE)
    normalize_dirs(root)
    if not WINDOWS:
        os.chmod(root / "locked", 0)


def unlock(root):
    locked = Path(root) / "locked"
    if locked.exists() and not WINDOWS:
        os.chmod(locked, 0o700)


def snapshot(root):
    """{relative path: (size, whole-second mtime, sha256)} of every file below `root`."""
    found = {}
    for folder, _dirs, names in os.walk(root):
        for name in names:
            path = Path(folder) / name
            info = os.lstat(path)
            found[path.relative_to(root).as_posix()] = (info.st_size, int(info.st_mtime),
                                                        hashlib.sha256(path.read_bytes()).hexdigest())
    return found


def copy_tree(source, target):
    """A copy that keeps times, links and special files (cp -a / robocopy)."""
    remove(target)
    if WINDOWS:
        run(["robocopy", str(source), str(target), "/E", "/COPY:DAT", "/DCOPY:DAT", "/MT:16", "/NFL", "/NDL", "/NJH",
             "/NJS", "/NP", "/R:0", "/W:0"], timeout=3600)
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        run(["cp", "-a", str(source), str(target)], timeout=3600)


# --------------------------------------------------------------------------- plan E fixtures

# Folder keys: 30 x 20 folders of 20 files (12 000 files, 600 folders) and a few folders that differ
# between the sides, so the listing is large next to the keys and the unequal folders are not empty.
KEY_GROUPS, KEY_FOLDERS, KEY_FILES = 30, 20, 20
KEY_CHANGED = "g13/f07/x/y/z/changed.txt"
# Keys without transfer: 8 equal files, one of equal size and time but other content, one of equal
# content but another time, 4 MiB each of random bytes (no compression can hide a transfer).
SUM_BLOCK, SUM_EQUAL = 4 << 20, 8
SUM_DIFF = "s/diff_same_size.bin"
SUM_REMOTE_BYTES = (SUM_EQUAL + 2) * SUM_BLOCK
SUM_SIDES = {"LS": 1, "LA": 1, "LH": 1, "HH": 2, "LF": 1}
# Bounds on the bytes of the candidate next to v0.5.189 (a decision of the suite, plan E4):
FIRST_BOUND, FOLLOW_BOUND, CHANGED_BOUND, EXTRA_BOUND = 0.5, 0.25, 0.35, 0.25
SUM_BOUND, SUM_OLD_MIN = 0.2, 0.5


def make_keys_tree(root, role):
    root = Path(root)
    for group in range(KEY_GROUPS):
        for number in range(KEY_FOLDERS):
            folder = root / f"g{group:02d}" / f"f{number:02d}"
            folder.mkdir(parents=True, exist_ok=True)
            for entry in range(KEY_FILES):
                path = folder / f"entry_with_a_longer_name_{entry:03d}.dat"
                path.write_bytes(b"%02d%02d%03d\n" % (group, number, entry))
                os.utime(path, (BASE, BASE))
    put(root / KEY_CHANGED, b"original\n", BASE)
    put(root / "u01" / f"only_{role}.txt", f"only {role}\n".encode(), BASE)
    put(root / "u02" / "size.txt", b"size of side a\n" if role == "a" else b"side b has more content here\n", BASE)
    put(root / "u03" / "time.txt", b"same content\n", BASE + (0 if role == "a" else 7200))
    normalize_dirs(root)


def sum_block(seed):
    return random.Random(seed).randbytes(SUM_BLOCK)


def make_sum_tree(root, role):
    root = Path(root)
    for number in range(SUM_EQUAL):
        put(root / "s" / f"equal{number}.bin", sum_block(number), BASE)
    put(root / SUM_DIFF, sum_block(100 if role == "a" else 101), BASE)
    put(root / "s" / "time_only.bin", sum_block(200), BASE + (0 if role == "a" else 7200))
    normalize_dirs(root)


def place(ctx, kind, destination, role):
    """A copy of the generated tree of `kind` ("keys" or "sum") for `role` at `destination`; each tree is
    generated once per job and copied (cp -a keeps the times)."""
    template = ctx.templates.get((kind, role))
    if template is None:
        template = ctx.work / f"template-{kind}-{role}"
        (make_keys_tree if kind == "keys" else make_sum_tree)(template, role)
        ctx.templates[(kind, role)] = template
    copy_tree(template, Path(destination))


# --------------------------------------------------------------------------- remote fixtures (Linux)

class SshBox:
    """An OpenSSH container whose /config (the account's home, `.cache/smart-explorer` of the deployed
    agent) is a host folder; the account has the runner's uid, so the data under `home/data` is plain
    local data for verification and seeding."""

    def __init__(self, work, log_root):
        self.home = Path(work) / "ssh-home"
        self.data = self.home / "data"
        self.log_root = Path(log_root)
        self.user, self.password = "seuser", secrets.token_hex(12)
        speed.SECRETS.add(self.password)
        self.name = f"se-abgleich-ssh-{os.getpid()}"
        self.port = None
        # {host address: port} of the two published ports (SSH_HOSTS).
        self.ports = {}
        self.data.mkdir(parents=True)

    def endpoint(self, agent):
        """(host, port) of the plain SFTP (False) or the agent connection (True)."""
        host = SSH_HOSTS[agent]
        return host, self.ports[host]

    def start(self):
        log = self.log_root / "ssh-box.log"
        run(["docker", "rm", "-f", self.name], timeout=120)
        started = run(["docker", "run", "-d", "--name", self.name,
                       *(item for host in SSH_HOSTS.values() for item in ("-p", f"{host}::2222")),
                       "-e", f"PUID={os.getuid()}", "-e", f"PGID={os.getgid()}", "-e", "PASSWORD_ACCESS=true",
                       "-e", f"USER_NAME={self.user}", "-e", f"USER_PASSWORD={self.password}",
                       "-v", f"{self.home}:/config", "lscr.io/linuxserver/openssh-server:latest"],
                      timeout=900, log=log)
        ensure(started.ok, f"docker run openssh-server: {short(started)}")
        self.port = self.wait()
        # Many handshakes from one address: turn the per-source penalties of OpenSSH 9.8+ off (as the
        # luecken suite does); the client under test is unchanged.
        config = 'f=/config/sshd/sshd_config; [ -f "$f" ] || f=/etc/ssh/sshd_config; '
        check = (config + 'printf "\\nPerSourcePenalties no\\n" >> "$f"; '
                 'for s in /usr/sbin/sshd.pam /usr/sbin/sshd; do [ -x "$s" ] && exec "$s" -t -f "$f"; done; exit 1')
        if run(["docker", "exec", self.name, "sh", "-c", check], timeout=60, log=log).ok:
            ensure(run(["docker", "restart", self.name], timeout=180, log=log).ok, "docker restart")
            self.port = self.wait()
        else:
            run(["docker", "exec", self.name, "sh", "-c", config + 'sed -i "/^PerSourcePenalties no$/d" "$f"'],
                timeout=60, log=log)
        return self

    def wait(self):
        mapped = run(["docker", "port", self.name, "2222/tcp"], timeout=60).stdout.strip().splitlines()
        self.ports = {line.rsplit(":", 1)[0]: int(line.rsplit(":", 1)[1]) for line in mapped if ":" in line}
        ensure(set(SSH_HOSTS.values()) <= set(self.ports), f"docker port gave {mapped}")
        port = self.ports[SSH_HOSTS[False]]
        ensure(speed.wait_tcp(port, b"SSH-", 120), "the OpenSSH container did not answer")
        time.sleep(2)
        return port

    def close(self):
        logs = run(["docker", "logs", self.name], timeout=120)
        with open(self.log_root / "ssh-box.log", "a", encoding="utf-8") as handle:
            handle.write(speed.redact(f"--- docker logs\n{logs.output[-20000:]}\n"))
        run(["docker", "rm", "-f", self.name], timeout=120)


# `se connections add` keeps one saved connection per host, port, user and root (a second label for the
# same endpoint replaces the first, in v0.5.189 as in the candidate): the SSH container publishes its port
# on two loopback addresses (all of 127/8 is the loopback on Linux), the plain SFTP connection uses the
# first and the agent connection the second, and each address's host key is confirmed.
SSH_HOSTS = {False: "127.0.0.1", True: "127.0.0.2"}


def add_saved(binary, env, kind, port, user, password, root, label, log, agent=False, trust_client=None,
              host="127.0.0.1"):
    """A saved connection of one `se`; None when it works, else the reason."""
    args = ["connections", "add", kind, "--host", host, "--port", str(port), "--user", user, "--root", root,
            "--label", label, "--password-stdin", *(["--agent"] if agent else [])]
    added = run([binary, *args], env=env, input_text=password + "\n", timeout=300, log=log)
    if not added.ok:
        return f"connections add: {short(added, 400)}"
    if trust_client is not None:
        try:
            ai.trust_ssh_host_key(trust_client, binary, host, port, log)
        except Exception as error:  # noqa: BLE001
            return f"host key: {error}"
    listed = run([binary, "ls", f"@{label}:/"], env=env, timeout=600, log=log)
    return None if listed.ok else f"ls: {short(listed, 400)}"


def share_net(work, tag, server, main_binary, host_binary, log_root, hosts=2, pair_hosts=True):
    """A Share server, one main client and `hosts` host clients paired Direct, each host exporting a
    writable folder `export<i>`. The main and the host clients may be different versions. A host of the
    candidate accepts the request with the preset `read-write` (since the candidate a new connection has
    no right until it is confirmed, `grants set --write` alone would leave the main without a folder);
    a host of v0.5.189 has no preset and accepts as before. `pair_hosts=False` stops after the clients are
    connected to the server (the connection flow stage pairs them itself); `net.hosts`, `net.owner`
    and `net.pair` carry what that stage needs."""
    pair = speed._load(f"share_fixture_{slug(tag)}", NATIVE / "sync-reliability-share-fixture.py")
    root = Path(work) / f"share-{slug(tag)}"
    root.mkdir(parents=True, exist_ok=True)
    root.chmod(0o700)
    logs = Path(log_root) / f"share-{slug(tag)}"
    logs.mkdir(parents=True, exist_ok=True)
    owner = types.SimpleNamespace(root=root, logs=logs, cleanups=[])
    signal_port, relay_port = pair.pair_ports()
    cert, key = root / "server.pem", root / "server.key"
    made = run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-keyout", key, "-out", cert,
                "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1", "-addext",
                "basicConstraints=critical,CA:FALSE"], timeout=120, log=logs / "openssl.log")
    ensure(made.ok, f"openssl: {short(made)}")
    run(["openssl", "x509", "-in", cert, "-outform", "DER", "-out", root / "server.der"], timeout=60)
    pin = hashlib.sha256((root / "server.der").read_bytes()).hexdigest()
    process = subprocess.Popen(
        [str(server), f"127.0.0.1:{signal_port}", "--tls-cert", str(cert), "--tls-key", str(key), "--state-file",
         str(root / "state.json")], stdin=subprocess.DEVNULL, stdout=open(logs / "share-server.log", "ab"),
        stderr=subprocess.STDOUT,
        env=dict(os.environ, SE_SHARE_IDLE_KEEPALIVE_SECS="30", SE_SHARE_ALLOW_PLAINTEXT="0",
                 SE_SHARE_REQUIRE_KEY_LOGIN="0", SE_IROH_RELAY_DISABLE="0",
                 SE_IROH_RELAY_BIND=f"127.0.0.1:{relay_port}", SE_SHARE_FIXTURE_ROOT=str(root)))
    net = types.SimpleNamespace(process=process, root=root, clients=[], endpoints=[], exports=[], main=None,
                                hosts=[], owner=owner, pair=pair)

    def close():
        for client in reversed(net.clients):
            try:
                client.close()
            except Exception as error:  # noqa: BLE001
                print(f"[note] share net {tag}: cleanup: {error}", flush=True)
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
    net.close = close
    try:
        pair.wait_server(process, (signal_port, relay_port))
        base = dict(os.environ, SE_SHARE_RELAY_URL=f"https://127.0.0.1:{relay_port}")
        main = pair.Client(owner, Path(main_binary), pair.isolated(base, root / "main"), "main")
        net.clients.append(main)
        hosts_list = []
        for index in range(hosts):
            host = pair.Client(owner, Path(host_binary), pair.isolated(base, root / f"host{index}"), f"host{index}")
            net.clients.append(host)
            hosts_list.append(host)
        for client in (main, *hosts_list):
            client.server_process = process
            client.call("share", "configure", "--server", f"wss://127.0.0.1:{signal_port}/#sha256={pin}")
            client.wait(lambda c=client: (lambda worker: worker.get("reachable") and worker.get("running")
                                          and worker.get("connected"))(
                json.loads(c.call("share", "status", "--json").stdout).get("worker", {})), "connected worker")
        net.main, net.hosts = main, hosts_list
        if not pair_hosts:
            return net
        for index, host in enumerate(hosts_list):
            identity = json.loads(host.call("share", "identity", "--json").stdout)
            direct = json.loads(main.call("connections", "add-peer", "--code", identity["direct_code"], "--name",
                                          f"AbgleichPeer{index}", "--json").stdout)
            host.wait(lambda h=host: json.loads(h.call("share", "request", "--json").stdout).get("count", 0) == 1,
                      "incoming request")
            with_presets = host.call("share", "grants", "presets", check=False).returncode == 0
            host.call("share", "request", "accept", "--json", *(["--preset", "read-write"] if with_presets else []))
            host.call("share", "grants", "set", pair.identity_of(main), "--write", "--json")
            export = root / f"export{index}"
            export.mkdir(exist_ok=True)
            host.call("share", "export", "add", str(export), "--label", f"AbgleichExport{index}")
            host.call("share", "export", "set", f"AbgleichExport{index}", "--write")
            host.call("share", "worker", "refresh")
            endpoint = direct["endpoint"] + f"/AbgleichExport{index}"
            main.wait(lambda e=endpoint: main.call("ls", e, check=False).returncode == 0, "exported folder")
            net.endpoints.append(endpoint)
            net.exports.append(export)
        return net
    except Exception:
        close()
        raise


def build_share_server(ctx):
    if ctx.server_tried:
        return ctx.server
    server_dir = ROOT / "share-server"
    built = run(["cargo", "build", "--locked", "--bin", "se-share-server"], cwd=server_dir,
                env=dict(os.environ, CARGO_TARGET_DIR=str(server_dir / "target")), timeout=3600,
                log=ctx.log_root / "build-share-server.log")
    server = server_dir / "target" / "debug" / f"se-share-server{EXE}"
    ctx.results.record("build", "development share server", built.ok and server.exists(),
                       "" if built.ok else short(built, 2000))
    ctx.server_tried = True
    ctx.server = server if built.ok and server.exists() else None
    return ctx.server


# --------------------------------------------------------------------------- equality

SHAPES = (("default", "tree", []),
          ("nohidden", "tree", ["--include-hidden", "false"]),
          ("mounts", "tree", ["--cross-mounts", "true"]),
          ("filter", "tree", ["--ignore", "**/*.tmp", "--ignore", "node_modules/**"]),
          ("locked", "lk", []))
# Client version / far-side version -> which built binary plays which role.
AGENT_BINARY = {"old/old": "old", "new/new": "new", "new/old": "new_oldagent", "old/new": "old_newagent"}
SHARE_BINARY = {"old/old": ("old", "old"), "new/new": ("new", "new"), "new/old": ("new", "old"),
                "old/new": ("old", "new")}


def replace_roots(text, roots):
    for root in sorted((str(root) for root in roots), key=len, reverse=True):
        for form in {str(root), Path(root).as_posix()}:
            text = text.replace(form, "<R>")
    return text


def norm_compare(stdout):
    data = json.loads(stdout)
    return {"source_files": data["source_files"], "target_files": data["target_files"], "counts": data["counts"],
            "actions": sorted([item["action"], item["path"]] for item in data["actions"]),
            "conflicts": sorted(data["conflicts"]), "omissions": data["omissions"],
            "duplicate_removals": data["duplicate_removals"], "blocked": data["blocked"]}


def compare_once(binary, env, name, a, b, flags, roots, log):
    added = run([binary, "sync", "add", "--name", name, "--source", a, "--target", b, "--trigger", "manual",
                 "--enabled", "false", *flags], env=env, timeout=300, log=log)
    if not added.ok:
        return {"error": "sync add: " + replace_roots(short(added, 500), roots)}
    ran = run([binary, "sync", "compare", name, "--json"], env=env, timeout=1800, log=log)
    if not ran.ok:
        return {"rc": ran.code, "error": replace_roots(ran.stderr.strip()[-500:], roots)}
    try:
        return norm_compare(ran.stdout)
    except (ValueError, KeyError, TypeError) as error:
        return {"error": f"compare output unreadable ({error}): {ran.stdout[:300]}"}


def ls_once(binary, env, target, roots, drop, log):
    ran = run([binary, "ls", target, "-R", "--json"], env=env, timeout=1800, log=log)
    failed = sorted(replace_roots(path, roots) for path in re.findall(r"not read: (.+?): ", ran.stderr))
    try:
        rows = json.loads(ran.stdout) if ran.stdout.strip() else []
    except ValueError as error:
        return {"rc": ran.code, "error": f"listing unreadable ({error}): {ran.stdout[:200]}", "not_read": failed}
    return {"rc": ran.code, "not_read": failed,
            "rows": {row["path"]: {key: value for key, value in row.items() if key not in drop} for row in rows},
            "hidden": {row["path"]: row.get("hidden") for row in rows}}


def collect(ctx, variant, way, binary, env, loc_a, loc_b, ls_target, roots, drop=()):
    """All sync-compare shapes and both recursive listings of one way with one binary."""
    log = ctx.log_root / f"equality-{slug(way)}.log"
    for shape, sub, flags in SHAPES:
        ctx.matrix[("cmp", way, shape, variant)] = compare_once(
            binary, env, slug(f"{way}-{shape}-{variant}"), loc_a(sub), loc_b(sub), flags, roots, log)
    for sub in ("tree", "lk"):
        ctx.matrix[("ls", way, sub, variant)] = ls_once(binary, env, ls_target(sub), roots, drop, log)


def diff_text(a, b, prefix="", limit=14):
    lines = []
    if isinstance(a, dict) and isinstance(b, dict):
        for key in sorted(set(a) | set(b), key=str):
            if key not in a:
                lines.append(f"{prefix}{key}: only in the other: {json.dumps(b[key], sort_keys=True)[:160]}")
            elif key not in b:
                lines.append(f"{prefix}{key}: only in v0.5.189: {json.dumps(a[key], sort_keys=True)[:160]}")
            elif a[key] != b[key]:
                if isinstance(a[key], (dict, list)):
                    lines += diff_text(a[key], b[key], f"{prefix}{key}.", limit)
                else:
                    lines.append(f"{prefix}{key}: {a[key]!r} != {b[key]!r}")
    elif isinstance(a, list) and isinstance(b, list):
        first, second = ({json.dumps(item, sort_keys=True) for item in side} for side in (a, b))
        lines += [f"{prefix}only in v0.5.189: {item}" for item in sorted(first - second)[:limit]]
        lines += [f"{prefix}only in the other: {item}" for item in sorted(second - first)[:limit]]
    else:
        lines.append(f"{prefix}{a!r} != {b!r}")
    return lines[:limit]


# Plan C3 (V1, 7ca9b26f): the listings of an SSH agent and of a Share host carry the hidden flag; v0.5.189's
# never did (its agent listing set `hidden: false`, `agent/core/metadata.rs`), so `include_hidden = false`
# did nothing on those sides. The candidate marks dot names hidden there as a local listing and SFTP do.
HIDDEN_FIXED_WAYS = {"LA": "LS", "LH": "LL", "HH": "LL"}


def dot_name(rel):
    return str(rel).rstrip("/").rsplit("/", 1)[-1].startswith(".")


def hidden_follows_dots(reference, value):
    """True when the candidate's `ls -R` of a host side differs from v0.5.189 only in the hidden flags and
    marks exactly the dot names hidden."""
    if not (isinstance(reference, dict) and isinstance(value, dict)):
        return False

    def without_hidden(data):
        data = json.loads(json.dumps(data, default=str))
        data.pop("hidden", None)
        for row in data.get("rows", {}).values():
            if isinstance(row, dict):
                row.pop("hidden", None)
        return data
    if without_hidden(reference) != without_hidden(value):
        return False
    flags = dict(value.get("hidden", {}))
    flags.update({rel: row["hidden"] for rel, row in value.get("rows", {}).items()
                  if isinstance(row, dict) and "hidden" in row})
    return bool(flags) and all(bool(flag) == dot_name(rel) for rel, flag in flags.items())


def hidden_filter_fixed(matrix, way, value):
    """True when `include_hidden = false` takes away on the host way `way` exactly as many files as on its
    basis way (which always marked dot names hidden and is unchanged from v0.5.189), and changes nothing
    else there: the candidate's `cmp nohidden` equals its `cmp default` but for those files."""
    basis = HIDDEN_FIXED_WAYS[way]
    basis_all = matrix.get(("cmp", basis, "default", "new/new"))
    basis_visible = matrix.get(("cmp", basis, "nohidden", "new/new"))
    way_all = matrix.get(("cmp", way, "default", "new/new"))
    if None in (basis_all, basis_visible, way_all, value) \
            or matrix.get(("cmp", basis, "nohidden", "old/old")) != basis_visible:
        return False
    counted = ("source_files", "target_files")
    try:
        gone = [basis_all[key] - basis_visible[key] for key in counted]
        taken = [way_all[key] - value[key] for key in counted]
    except (KeyError, TypeError):
        return False

    def rest(result):
        return {key: item for key, item in result.items() if key not in counted}
    return gone == taken and rest(value) == rest(way_all)


def evaluate_equality(ctx):
    results, matrix = ctx.results, ctx.matrix
    groups = {}
    for (kind, way, shape, variant), value in matrix.items():
        groups.setdefault((kind, way, shape), {})[variant] = value
    for (kind, way, shape), by in sorted(groups.items()):
        reference = by.get("old/old")
        label = f"{'sync compare' if kind == 'cmp' else 'ls -R'} {way} {shape}"
        if reference is None:
            results.record("equality", f"{label}: v0.5.189 reference run exists", False, "no run")
            continue
        for variant, value in sorted(by.items()):
            if variant == "old/old":
                continue
            equal = value == reference
            detail = "; ".join(diff_text(reference, value))
            if not equal and kind == "cmp" and way in HIDDEN_FIXED_WAYS and shape == "nohidden":
                # Known, intended change (HIDDEN_FIXED_WAYS): the candidate must give what a side gives that
                # always marked dot names hidden (SFTP for the agent, a local folder for a Share host),
                # and that side must be unchanged from v0.5.189.
                basis = HIDDEN_FIXED_WAYS[way]
                basis_new = matrix.get(("cmp", basis, "nohidden", "new/new"))
                basis_old = matrix.get(("cmp", basis, "nohidden", "old/old"))
                same_as_basis = basis_new is not None and value == basis_new and basis_old == basis_new
                if same_as_basis or hidden_filter_fixed(matrix, way, value):
                    results.record("equality", f"{label}: {variant} differs from v0.5.189 only by the fixed "
                                   f"hidden-file filter of the host side (V1: it leaves out what the {basis} way "
                                   "leaves out)", True, detail)
                    continue
            if not equal and kind == "ls" and way in HIDDEN_FIXED_WAYS and hidden_follows_dots(reference, value):
                results.record("equality", f"{label}: {variant} differs from v0.5.189 only by the hidden flags, "
                               "which mark exactly the dot names (V1)", True, detail)
                continue
            results.record("equality", f"{label}: {variant} == old/old (v0.5.189)", equal, detail)
    # Hidden flags of agent listings must follow the dot-name rule SFTP applies (plan C3).
    for sub in ("tree", "lk"):
        agent, sftp = matrix.get(("ls", "LA", sub, "new/new")), matrix.get(("ls", "LS", sub, "new/new"))
        if agent and sftp and "hidden" in agent and "hidden" in sftp:
            results.record("equality", f"ls -R {sub}: the candidate's agent listing marks the same entries hidden "
                           "as SFTP", agent["hidden"] == sftp["hidden"],
                           "; ".join(diff_text(sftp["hidden"], agent["hidden"])))


def fixture_trees(ctx):
    """The local sides A (role a) and B (role b), each with the rich tree and the unreadable-folder tree."""
    trees = {}
    for name, role in (("local-a", "a"), ("local-b", "b")):
        root = ctx.work / name
        make_tree(root / "tree", role, ctx.notes)
        make_lk_tree(root / "lk", role)
        if role == "a":
            place(ctx, "keys", root / "keys", role)
            place(ctx, "sum", root / "sum", role)
        trees[name] = root
    return trees


def deployed_agent(box, expected):
    cache = box.home / ".cache" / "smart-explorer"
    return cache.is_dir() and any(path.name.endswith(expected) for path in cache.iterdir())


def stage_ssh(ctx, trees, client_of):
    """Local-local, local-SFTP and local-SSH agent for every version pair; then the merge cases
    on the same SSH account."""
    results = ctx.results
    box = SshBox(ctx.work, ctx.log_root)
    make_tree(box.data / "tree", "b", ctx.notes)
    make_lk_tree(box.data / "lk", "b")
    place(ctx, "keys", box.data / "keys", "b")
    place(ctx, "sum", box.data / "sum", "b")
    local_a, local_b = trees["local-a"], trees["local-b"]
    roots = [local_a, local_b, box.data, "/config/data"]
    try:
        box.start()
    except Exception as error:  # noqa: BLE001
        results.record("equality", "OpenSSH container", False, f"{type(error).__name__}: {error}")
        box.close()
        return
    log = ctx.log_root / "equality-connections.log"
    try:
        for variant, key in AGENT_BINARY.items():
            binary = ctx.bins.get(key)
            if binary is None:
                continue
            client = client_of(variant)
            problem = None
            for label, agent in (("sftp", False), ("agent", True)):
                host, port = box.endpoint(agent)
                problem = problem or add_saved(binary, client.env, "sftp", port, box.user, box.password,
                                               "/config/data", label, log, agent=agent, trust_client=client,
                                               host=host)
            if problem:
                results.record("equality", f"{variant}: saved SSH connections work", False, problem)
                continue
            if variant in ("old/old", "new/new"):
                collect(ctx, variant, "LL", binary, client.env, lambda sub: str(local_a / sub),
                        lambda sub: str(local_b / sub), lambda sub: str(local_b / sub), roots)
                ctx.matrix[("ls", "L", "tree", variant)] = ls_once(binary, client.env, str(local_a / "tree"), roots,
                                                                   (), ctx.log_root / "equality-l.log")
                collect(ctx, variant, "LS", binary, client.env, lambda sub: str(local_a / sub),
                        lambda sub: f"@sftp:/{sub}", lambda sub: f"@sftp:/{sub}", roots)
            remove(box.home / ".cache")
            collect(ctx, variant, "LA", binary, client.env, lambda sub: str(local_a / sub),
                    lambda sub: f"@agent:/{sub}", lambda sub: f"@agent:/{sub}", roots, drop=("hidden",))
            expected = ctx.bundle_sha.get(key, "")
            if expected:
                listing = ", ".join(path.name for path in (box.home / ".cache" / "smart-explorer").glob("*"))
                results.record("equality", f"LA {variant}: the agent bundle with sha256 {expected[:12]} was deployed "
                               "to the far side", deployed_agent(box, expected), f"deployed: {listing}")
            if variant in ("old/old", "new/new"):
                # Plan E: folder keys and keys without transfer over the SFTP and agent connections.
                keys_series(ctx, "LA", variant, binary, client.env, str(local_a / "keys"), "@agent:/keys",
                            box.data / "keys" / KEY_CHANGED)
                for way, label in (("LS", "sftp"), ("LA", "agent")):
                    sum_series(ctx, way, variant, binary, client.env, str(local_a / "sum"), f"@{label}:/sum")
        # The SSH account is also the far side of the merge cases.
        unmount_all(box.home)
        client = client_of("new/new")
        if ctx.bins.get("new"):
            for label, name in (("sftp", "SFTP"), ("agent", "SSH with se-agent")):
                merge_remote(ctx, f"local-{name}", ctx.bins["new"], client.env,
                             HostSide(box.data / "merge", f"@{label}:/merge", local=False),
                             ctx.work / f"merge-{label}")
    finally:
        box.close()
        for tree in (box.data / "lk", local_a / "lk", local_b / "lk"):
            unlock(tree)


def stage_share(ctx, trees):
    results = ctx.results
    server = build_share_server(ctx)
    if server is None:
        return
    local_a = trees["local-a"]
    for variant, (main_key, host_key) in SHARE_BINARY.items():
        main_binary, host_binary = ctx.bins.get(main_key), ctx.bins.get(host_key)
        if main_binary is None or host_binary is None:
            continue
        net = None
        try:
            net = share_net(ctx.work, f"eq-{variant}", server, main_binary, host_binary, ctx.log_root)
            for index, role in ((0, "b"), (1, "a")):
                make_tree(net.exports[index] / "tree", role, ctx.notes)
                make_lk_tree(net.exports[index] / "lk", role)
                if variant in ("old/old", "new/new"):
                    place(ctx, "keys", net.exports[index] / "keys", role)
                    place(ctx, "sum", net.exports[index] / "sum", role)
            roots = [local_a, net.root, *net.endpoints]
            endpoint0, endpoint1 = net.endpoints
            binary, env = str(net.main.cli), net.main.env
            collect(ctx, variant, "LH", binary, env, lambda sub: str(local_a / sub),
                    lambda sub: f"{endpoint0}/{sub}", lambda sub: f"{endpoint0}/{sub}", roots)
            collect(ctx, variant, "HH", binary, env, lambda sub: f"{endpoint1}/{sub}",
                    lambda sub: f"{endpoint0}/{sub}", lambda sub: f"{endpoint1}/{sub}", roots)
            if variant in ("old/old", "new/new"):
                # Plan E: folder keys and keys without transfer over the Direct Share.
                keys_series(ctx, "LH", variant, binary, env, str(local_a / "keys"), f"{endpoint0}/keys",
                            net.exports[0] / "keys" / KEY_CHANGED)
                keys_series(ctx, "HH", variant, binary, env, f"{endpoint1}/keys", f"{endpoint0}/keys",
                            net.exports[0] / "keys" / KEY_CHANGED)
                sum_series(ctx, "LH", variant, binary, env, str(local_a / "sum"), f"{endpoint0}/sum")
                sum_series(ctx, "HH", variant, binary, env, f"{endpoint1}/sum", f"{endpoint0}/sum")
            if variant == "new/new":
                unmount_all(net.root)
                merge_remote(ctx, "local-Direct Share", binary, env,
                             HostSide(net.exports[0] / "merge", f"{endpoint0}/merge", local=False),
                             ctx.work / "merge-share")
        except Exception as error:  # noqa: BLE001 - the other versions still run
            results.record("equality", f"Share pair {variant}", False, f"{type(error).__name__}: {error}")
        finally:
            if net is not None:
                net.close()
                unmount_all(net.root)
                for exported in net.exports:
                    unlock(exported / "lk")


def sanity(ctx):
    for key, value in sorted(ctx.matrix.items()):
        kind, way, shape, variant = key
        if variant != "old/old" or shape in ("locked", "lk"):
            continue
        if kind == "cmp":
            ok = "error" not in value and value.get("source_files", 0) > 50 and value.get("target_files", 0) > 50
            ctx.results.record("equality", f"fixture: sync compare {way} {shape} sees both generated trees (v0.5.189)",
                               ok, json.dumps(value, sort_keys=True)[:400] if not ok else "")
        else:
            ok = len(value.get("rows", {})) > 50
            ctx.results.record("equality", f"fixture: ls -R {way} {shape} lists the generated tree (v0.5.189)",
                               ok, json.dumps(value, sort_keys=True)[:400] if not ok else "")


# --------------------------------------------------------------------------- plan E: bytes of comparisons

def add_job(binary, env, name, a, b, extra, log):
    added = run([binary, "sync", "add", "--name", name, "--source", a, "--target", b, "--trigger", "manual",
                 "--enabled", "false", *extra], env=env, timeout=300, log=log)
    return added.ok, short(added, 400)


def timed_compare(binary, env, name, log):
    ran, wire = metered([binary, "sync", "compare", name, "--json"], env, 1800, log)
    if not ran.ok:
        return {"error": f"exit {ran.code}: {short(ran, 500)}", "bytes": wire}
    try:
        return {"norm": norm_compare(ran.stdout), "bytes": wire, "seconds": ran.seconds}
    except (ValueError, KeyError, TypeError) as error:
        return {"error": f"compare output unreadable ({error}): {ran.stdout[:300]}", "bytes": wire}


def keys_series(ctx, way, variant, binary, env, loc_a, loc_b, changed_path):
    """Three comparisons of one saved setup by one `se`: the first with no stored folder keys, the second
    without any change, the third after one file changed deep in the far side's tree. The bytes of each
    cross the counted interfaces; `changed_path` is the far side's file on the runner's disk, restored afterwards."""
    log = ctx.log_root / f"keys-{slug(way)}.log"
    name = slug(f"keys-{way}-{variant}")[:60]
    saved, why = add_job(binary, env, name, loc_a, loc_b, [], log)
    if not saved:
        ctx.results.record("keys", f"{way} {variant}: sync setup saved", False, why)
        return
    calibrate_idle()
    forget_keys(env)
    steps = {"first": timed_compare(binary, env, name, log), "follow": timed_compare(binary, env, name, log)}
    original = Path(changed_path).read_bytes()
    try:
        put(Path(changed_path), b"changed on the far side, and longer than before\n", BASE + 86400)
        steps["changed"] = timed_compare(binary, env, name, log)
    finally:
        put(Path(changed_path), original, BASE)
    ctx.series[(way, "keys", variant)] = steps


def sum_series(ctx, way, variant, binary, env, loc_a, loc_b):
    """One checksum-mode comparison with no stored keys; its bytes cross the counted interfaces."""
    log = ctx.log_root / f"sum-{slug(way)}.log"
    name = slug(f"sum-{way}-{variant}")[:60]
    saved, why = add_job(binary, env, name, loc_a, loc_b, ["--compare", "checksum"], log)
    if not saved:
        ctx.results.record("sum", f"{way} {variant}: checksum setup saved", False, why)
        return
    calibrate_idle()
    forget_keys(env)
    ctx.series[(way, "sum", variant)] = timed_compare(binary, env, name, log)


def paths_of(norm):
    return {path for _kind, path in norm["actions"]} | set(norm["conflicts"])


def usable(results, stage, label, step):
    if step is None:
        results.record(stage, f"{label}: run exists", False, "no run")
        return False
    if "error" in step:
        results.record(stage, f"{label}: the comparison ran", False, step["error"])
        return False
    return True


def evaluate_keys(ctx):
    results, stage = ctx.results, "keys"
    for way in sorted({w for (w, kind, _v) in ctx.series if kind == "keys"}):
        old, new = ctx.series.get((way, "keys", "old/old")), ctx.series.get((way, "keys", "new/new"))
        if old is None or new is None:
            results.record(stage, f"{way}: runs of v0.5.189 and of the candidate exist", False,
                           f"v0.5.189: {'yes' if old else 'no'}, candidate: {'yes' if new else 'no'}")
            continue
        labels = {"first": "first comparison, no stored keys", "follow": "second comparison, nothing changed",
                  "changed": "comparison after one file changed deep in the tree"}
        sound = True
        for step, text in labels.items():
            label = f"{way} {text}"
            both = usable(results, stage, f"{label} (v0.5.189)", old[step]) and usable(
                results, stage, f"{label} (candidate)", new[step])
            sound = sound and both
            if not both:
                continue
            results.record(stage, f"{label}: the result equals v0.5.189's",
                           old[step]["norm"] == new[step]["norm"],
                           "; ".join(diff_text(old[step]["norm"], new[step]["norm"])))
            ctx.byte_rows.append((f"{way} {text}", old[step]["bytes"], new[step]["bytes"]))
        if not sound:
            continue
        unequal = old["first"]["norm"]
        results.record(stage, f"fixture: {way} v0.5.189 sees the four unequal files of three folders",
                       len(unequal["actions"]) + len(unequal["conflicts"]) >= 4, json.dumps(unequal)[:300])
        results.record(stage, f"{way}: the changed file is reported after the change, not before",
                       KEY_CHANGED in paths_of(new["changed"]["norm"]) and KEY_CHANGED not in paths_of(
                           new["follow"]["norm"]) and KEY_CHANGED in paths_of(old["changed"]["norm"]), "")
        first_old = old["first"]["bytes"]
        if first_old is None:
            results.record(stage, f"{way}: bytes over the connection are measurable", False, "no network counter")
            continue
        results.record(stage, f"fixture: {way} v0.5.189 moves a listing over the connection "
                       f"({human(first_old)} for {KEY_GROUPS * KEY_FOLDERS * KEY_FILES} files)",
                       first_old >= 100_000, f"only {human(first_old)}")
        new_first, new_follow, new_changed = (new[s]["bytes"] for s in ("first", "follow", "changed"))
        summary = (f"bytes: v0.5.189 first {human(first_old)}; candidate first {human(new_first)}, "
                   f"second {human(new_follow)}, after the change {human(new_changed)}")
        results.record(stage, f"{way} (a) first comparison without stored keys sends keys and the unequal "
                       f"folders only: below {FIRST_BOUND:.0%} of v0.5.189's bytes",
                       new_first < FIRST_BOUND * first_old, summary)
        results.record(stage, f"{way} (b) second comparison without a change lists nothing: below "
                       f"{FOLLOW_BOUND:.0%} of v0.5.189's bytes and not above the first (8 KiB tolerance)",
                       new_follow < FOLLOW_BOUND * first_old and new_follow <= new_first + 8192, summary)
        results.record(stage, f"{way} (c) one changed file deep in the tree sends only the runs on its path: "
                       f"below {CHANGED_BOUND:.0%} of v0.5.189's bytes and at most {EXTRA_BOUND:.0%} more than "
                       "the second comparison", new_changed < CHANGED_BOUND * first_old
                       and new_changed - new_follow < EXTRA_BOUND * first_old, summary)


def evaluate_sum(ctx):
    results, stage = ctx.results, "sum"
    for way in sorted({w for (w, kind, _v) in ctx.series if kind == "sum"}):
        old, new = ctx.series.get((way, "sum", "old/old")), ctx.series.get((way, "sum", "new/new"))
        label = f"{way} checksum mode"
        if not (usable(results, stage, f"{label} (v0.5.189)", old) and usable(results, stage, f"{label} (candidate)",
                                                                                new)):
            continue
        content = SUM_REMOTE_BYTES * SUM_SIDES[way]
        results.record(stage, f"{label}: the result equals v0.5.189's", old["norm"] == new["norm"],
                       "; ".join(diff_text(old["norm"], new["norm"])))
        results.record(stage, f"{label}: the file of equal size and time but other content is reported (both "
                       "versions)", SUM_DIFF in paths_of(old["norm"]) and SUM_DIFF in paths_of(new["norm"]),
                       f"v0.5.189: {sorted(paths_of(old['norm']))[:6]}; candidate: {sorted(paths_of(new['norm']))[:6]}")
        ctx.byte_rows.append((label, old["bytes"], new["bytes"]))
        if old["bytes"] is None or new["bytes"] is None:
            results.record(stage, f"{label}: bytes over the connection are measurable", False, "no network counter")
            continue
        summary = f"bytes: v0.5.189 {human(old['bytes'])}, candidate {human(new['bytes'])}, content {human(content)}"
        results.record(stage, f"fixture: {way} v0.5.189 moves the file content over the connection (at least "
                       f"{SUM_OLD_MIN:.0%} of {human(content)})", old["bytes"] >= SUM_OLD_MIN * content, summary)
        results.record(stage, f"{way} the far side forms the keys: the candidate moves less than {SUM_BOUND:.0%} "
                       f"of the {human(content)} of file content", new["bytes"] < SUM_BOUND * content, summary)


# --------------------------------------------------------------------------- merge rule

OLD, NEW = BASE, BASE + 5 * 86400
BIG_SRC, BIG_DST = os.urandom(1 << 20), os.urandom((1 << 20) + 1)
# relative path -> (source (bytes, mtime) or None, destination (bytes, mtime) or None, expected kind)
MERGE_FILES = {
    "mix/missing.txt": ((b"inserted file\n", NEW), None, "insert"),
    "mix/equal.txt": ((b"equal file\n", OLD), (b"equal file\n", OLD), "equal"),
    "mix/older_at_dst.txt": ((b"newer source content\n", NEW), (b"older target\n", OLD), "replace"),
    "mix/newer_at_dst.txt": ((b"older source\n", OLD), (b"newer target content, longer\n", NEW), "keep"),
    "mix/conflict.txt": ((b"conflict source\n", OLD), (b"conflict target is longer\n", OLD), "both"),
    "mix/sub/deep/inserted.txt": ((b"inserted in a missing folder\n", NEW), None, "insert"),
    "mix/big_older.bin": ((BIG_SRC, NEW), (BIG_DST, OLD), "replace"),
    "pure/missing.txt": ((b"pure missing\n", NEW), None, "insert"),
    "pure/equal.txt": ((b"pure equal\n", OLD), (b"pure equal\n", OLD), "equal"),
    "pure/older.txt": ((b"pure newer source\n", NEW), (b"pure older target\n", OLD), "replace"),
    "dst_only.txt": (None, (b"only at the destination\n", OLD), "keep"),
}
EXPECTED_COUNTS = {"neuere ersetzt": 3, "gleiche übersprungen": 2, "Ziel neuer behalten": 1,
                   "als Konflikt doppelt behalten": 1}


def make_merge_trees(src_tree, dst_tree):
    for relative, (source, destination, _kind) in MERGE_FILES.items():
        if source is not None:
            put(src_tree / relative, *source)
        if destination is not None:
            put(dst_tree / relative, *destination)
    normalize_dirs(src_tree)
    normalize_dirs(dst_tree)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def check_forward(after):
    """Problems of the destination after the first `--conflict update` copy."""
    problems = []
    for relative, (source, destination, kind) in MERGE_FILES.items():
        got = after.get(relative)
        want_data, want_time = (source if kind in ("insert", "replace") else destination)
        if got is None:
            problems.append(f"{relative}: missing at the destination")
            continue
        if got[2] != sha(want_data):
            problems.append(f"{relative}: content is not the expected one ({kind})")
        if abs(got[1] - want_time) > 2:
            problems.append(f"{relative}: time {got[1] - want_time} s away from the expected {kind} time")
    extra = sorted(set(after) - set(MERGE_FILES))
    conflict_source = MERGE_FILES["mix/conflict.txt"][0][0]
    if len(extra) != 1 or not extra[0].startswith("mix/") or after[extra[0]][2] != sha(conflict_source):
        problems.append(f"expected exactly one kept-both copy of mix/conflict.txt below mix/, found {extra}")
    return problems


def update_counts(stderr):
    return {key: int(match.group(1)) for key in EXPECTED_COUNTS
            for match in [re.search(r"(\d+) " + re.escape(key), stderr)] if match}


class HostSide:
    """A far side whose files are plain local folders (local disk, SSH account home, Share export,
    pyftpdlib): the copy goes into `cli_parent`, which holds the destination's `tree` folder."""

    def __init__(self, folder, cli_parent, local):
        self.folder, self.cli_parent, self.local = Path(folder), str(cli_parent), local
        self.folder.mkdir(parents=True, exist_ok=True)

    def seed(self, tree):
        remove(self.folder / "tree")
        shutil.copytree(tree, self.folder / "tree", symlinks=True)

    def fetch(self):
        return self.folder / "tree"

    def remote(self, relative):
        if self.local:
            return str(self.folder / "tree" / relative)
        return self.cli_parent.rstrip("/") + "/tree/" + relative


class SmbSide:
    def __init__(self, servers, work):
        self.servers, self.work, self.cli_parent, self.runs = servers, Path(work), "@smb:/", 0
        self.local = False

    def seed(self, tree):
        self.servers.seed("smb", tree, "tree")

    def fetch(self):
        self.runs += 1
        out = self.work / f"smb-out-{self.runs}"
        remove(out)
        out.mkdir(parents=True)
        copied = run(["docker", "cp", f"{self.servers.info['smb']['container']}:/shared/tree", str(out)],
                     timeout=1800, log=self.servers.log)
        ensure(copied.ok, f"docker cp out of the SMB share: {short(copied)}")
        return out / "tree"

    def remote(self, relative):
        return "@smb:/tree/" + relative


def merge_remote(ctx, name, binary, env, side, work):
    """Forward `se cp -r --conflict update`, the rule's effect on the destination, the counts of the notice
    line, and the reverse copy of the folder that has only missing, equal and older files."""
    stage = "merge"
    results = ctx.results
    work = Path(work)
    remove(work)
    src_tree, dst_stage = work / "src" / "tree", work / "dst-stage" / "tree"
    log = ctx.log_root / f"merge-{slug(name)}.log"
    try:
        make_merge_trees(src_tree, dst_stage)
        side.seed(dst_stage)
        before_src = snapshot(src_tree)
        forward = run([binary, "cp", "-r", "--conflict", "update", str(src_tree), side.cli_parent if not side.local
                       else str(side.folder)], env=env, timeout=1800, log=log)
        results.record(stage, f"{name}: se cp -r --conflict update succeeds", forward.ok, short(forward, 1200))
        if not forward.ok:
            return
        after = snapshot(side.fetch())
        problems = check_forward(after)
        results.record(stage, f"{name}: missing inserted, older replaced, equal skipped, newer destination kept, "
                       "same-time other-size kept twice, bytes exact, copies carry the source time",
                       not problems, "; ".join(problems[:12]))
        counts = update_counts(forward.stderr)
        results.record(stage, f"{name}: the notice counts 3 replaced, 2 equal skipped, 1 newer destination, "
                       "1 conflict", counts == EXPECTED_COUNTS, f"counted {counts}; stderr: {forward.stderr[-400:]}")
        results.record(stage, f"{name}: the source folder is unchanged", snapshot(src_tree) == before_src, "")
        reverse = run([binary, "cp", "-r", "--conflict", "update", side.remote("pure"), str(src_tree)], env=env,
                      timeout=1800, log=log)
        results.record(stage, f"{name}: the same copy backwards succeeds", reverse.ok, short(reverse, 1200))
        if reverse.ok:
            reverse_counts = update_counts(reverse.stderr)
            results.record(stage, f"{name}: the same copy backwards finds only equal files (3 skipped, nothing "
                           "replaced or kept)", reverse_counts == {"gleiche übersprungen": 3},
                           f"counted {reverse_counts}; stderr: {reverse.stderr[-400:]}")
            results.record(stage, f"{name}: the backwards copy leaves the source bytes and times alone",
                           snapshot(src_tree) == before_src, "")
        merge_equal(ctx, name, binary, env, side, work / "equal")
    except Exception as error:  # noqa: BLE001
        results.record(stage, f"{name}", False, f"{type(error).__name__}: {error}")
    finally:
        remove(work)


def merge_equal(ctx, name, binary, env, side, work):
    """`se cp -r --conflict update` of an unchanged folder onto an equal target (plan E): nothing is
    replaced, every file is reported equal, and no file content crosses the connection."""
    stage = "merge"
    results = ctx.results
    work = Path(work)
    remove(work)
    src_tree = work / "src" / "tree"
    files = {f"f{number:02d}.bin": os.urandom(1 << 20) for number in range(16)}
    files.update({"sub/deep/inner1.bin": os.urandom(4096), "sub/deep/inner2.bin": os.urandom(4096)})
    content = sum(len(data) for data in files.values())
    for number, (relative, data) in enumerate(sorted(files.items())):
        put(src_tree / relative, data, BASE + 3600 * (number % 5))
    normalize_dirs(src_tree)
    side.seed(src_tree)
    before_src, before_dst = snapshot(src_tree), snapshot(side.fetch())
    log = ctx.log_root / f"merge-{slug(name)}.log"
    calibrate_idle()
    ran, wire = metered([binary, "cp", "-r", "--conflict", "update", str(src_tree),
                         side.cli_parent if not side.local else str(side.folder)], env, 1800, log)
    results.record(stage, f"{name}: an unchanged folder onto an equal target succeeds ({len(files)} files)",
                   ran.ok, short(ran, 1200))
    if not ran.ok:
        return
    counts = update_counts(ran.stderr)
    results.record(stage, f"{name}: every one of the {len(files)} files is reported equal, nothing else",
                   counts == {"gleiche übersprungen": len(files)}, f"counted {counts}; stderr: {ran.stderr[-400:]}")
    results.record(stage, f"{name}: target and source are unchanged (bytes and times)",
                   snapshot(side.fetch()) == before_dst and snapshot(src_tree) == before_src, "")
    if not side.local and wire is not None:
        results.record(stage, f"{name}: no file content crosses the connection (below 25% of "
                       f"{human(content)})", wire < 0.25 * content, f"bytes over the connection: {human(wire)}")


def stage_merge_ftp_smb(ctx, client_of):
    binary = ctx.bins.get("new")
    if binary is None:
        return
    client = client_of("merge")
    log = ctx.log_root / "merge-connections.log"
    pyftp = servers = None
    try:
        try:
            pyftp = speed.PyFtp(ctx.work, ctx.log_root).start()
            problem = add_saved(binary, client.env, "ftp", pyftp.port, speed.USER, pyftp.password, "/", "ftp", log)
            if problem:
                ctx.results.record("merge", "local-FTP: saved connection works", False, problem)
            else:
                merge_remote(ctx, "local-FTP", binary, client.env,
                             HostSide(pyftp.root / "merge", "@ftp:/merge", local=False), ctx.work / "merge-ftp")
        except Exception as error:  # noqa: BLE001
            ctx.results.record("merge", "local-FTP: pyftpdlib server", False, f"{type(error).__name__}: {error}")
        try:
            servers = speed.Servers(ctx.work, ctx.log_root).start(["smb"])
            if "smb" not in servers.info:
                ctx.results.record("merge", "local-SMB: Samba server", False, servers.failed.get("smb", "not started"))
            else:
                info = servers.info["smb"]
                problem = add_saved(binary, client.env, "smb", info["port"], speed.USER, servers.password,
                                    info["remote"], "smb", log)
                if problem:
                    ctx.results.record("merge", "local-SMB: saved connection works", False, problem)
                else:
                    merge_remote(ctx, "local-SMB", binary, client.env, SmbSide(servers, ctx.work),
                                 ctx.work / "merge-smb")
        except Exception as error:  # noqa: BLE001
            ctx.results.record("merge", "local-SMB", False, f"{type(error).__name__}: {error}")
    finally:
        if pyftp is not None:
            pyftp.close()
        if servers is not None:
            servers.close()
    reason = ("the se command line connects WebDAV only over HTTPS with the built-in web roots "
              "(`SE_SYNC_FIXTURE_CA_DER` is read only by test builds), so no local fixture server can be reached "
              "by `se cp`; the rule on WebDAV is covered by the library cases named `abgleich_task_` of the "
              "functional job, if any exist")
    note(ctx.notes, "local-WebDAV merge: not run: " + reason)
    ctx.results.record("merge", "NOT RUN: local-WebDAV `se cp --conflict update`", True, reason)


# A pyftpdlib that hashes files itself (draft-bryan-ftpext-hash: `FEAT` lists `HASH` with MD5 and SHA-256,
# `OPTS HASH <algorithm>` selects one, `HASH <file>` answers `213 <algorithm> 0-<last> <hex> <file>`;
# plus `XMD5` / `XSHA256`), so the FTP side of the checksum mode has a server that can form the key.
HASH_FTP_SERVER = r'''
import hashlib
import os
import sys

from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler
from pyftpdlib.servers import FTPServer

FEATURES = ("HASH SHA-256*;MD5", "XMD5", "XSHA256")


class Handler(FTPHandler):
    hash_name = "SHA-256"
    _feat = False
    _feat_has_hash = False

    def push(self, s):
        if self._feat and "HASH" in str(s):
            self._feat_has_hash = True
        return FTPHandler.push(self, s)

    def respond(self, resp, *args, **kwargs):
        if self._feat and resp == "211 End FEAT." and not self._feat_has_hash:
            self.push("".join(" %s\r\n" % feature for feature in FEATURES))
        return FTPHandler.respond(self, resp, *args, **kwargs)

    def ftp_FEAT(self, line):
        self._feat, self._feat_has_hash = True, False
        try:
            FTPHandler.ftp_FEAT(self, line)
        finally:
            self._feat = False

    def ftp_OPTS(self, line):
        command, _, argument = line.partition(" ")
        if command.upper() != "HASH":
            return FTPHandler.ftp_OPTS(self, line)
        name = argument.strip().upper()
        if name in ("MD5", "SHA-256"):
            self.hash_name = name
            self.respond("200 HASH " + name)
        else:
            self.respond("501 unsupported algorithm")

    def digest(self, line, name):
        # pyftpdlib already turned the argument of a command with `perm` into the file system path
        # (`FTPHandler.pre_process_command`); converting it again would name a path below the root.
        path = line
        if not os.path.isfile(path):
            self.respond("550 not a regular file")
            return None
        context = hashlib.md5() if name == "MD5" else hashlib.sha256()
        size = 0
        with open(path, "rb") as handle:
            while True:
                chunk = handle.read(1 << 20)
                if not chunk:
                    break
                context.update(chunk)
                size += len(chunk)
        return context.hexdigest(), size

    def ftp_HASH(self, line):
        found = self.digest(line, self.hash_name)
        if found:
            self.respond("213 %s 0-%d %s %s" % (self.hash_name, found[1] - 1, found[0], self.fs.fs2ftp(line)))

    def ftp_XMD5(self, line):
        found = self.digest(line, "MD5")
        if found:
            self.respond("251 " + found[0])

    def ftp_XSHA256(self, line):
        found = self.digest(line, "SHA-256")
        if found:
            self.respond("250 " + found[0])


for _name in ("HASH", "XMD5", "XSHA256"):
    Handler.proto_cmds = dict(Handler.proto_cmds)
    Handler.proto_cmds[_name] = dict(perm="r", auth=True, arg=True, help="Syntax: %s <file>." % _name)

port, root, user, password = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
authorizer = DummyAuthorizer()
authorizer.add_user(user, password, root, perm="elradfmwMT")
Handler.authorizer = authorizer
FTPServer(("127.0.0.1", port), Handler).serve_forever()
'''


class HashFtp(speed.PyFtp):
    """pyftpdlib with `HASH`, `XMD5` and `XSHA256`; the files live in a local folder."""

    def __init__(self, work, log_root):
        super().__init__(work, log_root)
        self.root = Path(work) / "hash-ftp-root"
        self.root.mkdir(parents=True, exist_ok=True)

    def start(self):
        installed = run([sys.executable, "-m", "pip", "install", "pyftpdlib>=1.5.9,<3"], timeout=600,
                        log=self.log_root / "pyftpdlib.log")
        ensure(installed.ok, f"pip install pyftpdlib: {installed.tail(400)}")
        script = self.root.parent / "hash-ftp-server.py"
        script.write_text(HASH_FTP_SERVER, encoding="utf-8")
        self.process = subprocess.Popen([sys.executable, str(script), str(self.port), str(self.root), speed.USER,
                                         self.password], stdout=open(self.log_root / "hash-ftp-server.log", "wb"),
                                        stderr=subprocess.STDOUT)
        ensure(speed.wait_tcp(self.port, b"220", 60), "the hashing pyftpdlib does not answer")
        return self


def stage_sum_ftp(ctx, trees, client_of):
    """Checksum mode against FTP: the server forms the MD5 (`HASH`), v0.5.189 downloads the files."""
    server = None
    try:
        server = HashFtp(ctx.work, ctx.log_root).start()
        place(ctx, "sum", server.root / "sum", "b")
        for variant, key in (("old/old", "old"), ("new/new", "new")):
            binary = ctx.bins.get(key)
            if binary is None:
                continue
            client = client_of(variant)
            problem = add_saved(binary, client.env, "ftp", server.port, speed.USER, server.password, "/", "hashftp",
                                ctx.log_root / "sum-ftp-connections.log")
            if problem:
                ctx.results.record("sum", f"LF {variant}: saved FTP connection works", False, problem)
                continue
            sum_series(ctx, "LF", variant, binary, client.env, str(trees["local-a"] / "sum"), "@hashftp:/sum")
    except Exception as error:  # noqa: BLE001
        ctx.results.record("sum", "LF: hashing FTP server", False, f"{type(error).__name__}: {error}")
    finally:
        if server is not None:
            server.close()


def stage_fat(ctx, binary, env):
    """Summer time: on a FAT volume (times in local time) files of equal size whose times differ by exactly
    one hour count as equal when both sides' digests show the same content; with other content, two hours
    or another size the times decide as before (plan C7, E: Sommerzeit, W10 of the second review)."""
    stage = "merge"
    results = ctx.results
    if WINDOWS or not sudo_ok():
        reason = "a FAT image is mounted with `sudo -n mount -o loop`, which this runner does not allow"
        note(ctx.notes, "FAT summer-time case: not run: " + reason)
        results.record(stage, "NOT RUN: FAT volume with times one hour apart", True, reason)
        return
    work = ctx.work / "fat"
    remove(work)
    work.mkdir(parents=True)
    log = ctx.log_root / "merge-fat.log"
    image, mount_point = work / "fat.img", work / "mnt"
    with open(image, "wb") as handle:
        handle.truncate(128 << 20)
    made = run(["mkfs.vfat", "-F", "32", str(image)], timeout=300, log=log)
    if not made.ok:
        results.record(stage, "FAT volume: image formatted (mkfs.vfat)", False, short(made, 600))
        return
    mount_point.mkdir()
    mounted = run(["sudo", "-n", "mount", "-t", "vfat", "-o", f"loop,uid={os.getuid()},gid={os.getgid()},umask=022",
                   str(image), str(mount_point)], timeout=120, log=log)
    results.record(stage, "FAT volume: image mounted as vfat", mounted.ok, short(mounted, 600))
    if not mounted.ok:
        return
    MOUNTS.append(mount_point)
    try:
        src_data, dst_data = b"S" * 24, b"D" * 24
        # name: (destination time, destination bytes, expected: equal / replace / keep)
        cases = {"same.txt": (BASE, dst_data, "equal"),
                 "plus_hour_same.txt": (BASE + 3600, src_data, "equal"),
                 "minus_hour_same.txt": (BASE - 3600, src_data, "equal"),
                 "plus_hour.txt": (BASE + 3600, dst_data, "keep"),
                 "minus_hour.txt": (BASE - 3600, dst_data, "replace"),
                 "two_hours.txt": (BASE - 7200, dst_data, "replace"),
                 "hour_other_size.txt": (BASE + 3600, b"D" * 40, "keep")}
        src_tree, dst_tree = work / "src" / "tree", mount_point / "tree"
        for relative, (stamp, data, _kind) in cases.items():
            put(src_tree / relative, src_data if relative != "hour_other_size.txt" else b"S" * 10, BASE)
            put(dst_tree / relative, data, stamp)
        before_src = snapshot(src_tree)
        ran = run([binary, "cp", "-r", "--conflict", "update", str(src_tree), str(mount_point)], env=env,
                  timeout=600, log=log)
        results.record(stage, "FAT volume: se cp -r --conflict update succeeds", ran.ok, short(ran, 1200))
        if not ran.ok:
            return
        after = snapshot(dst_tree)
        problems = []
        for relative, (stamp, data, kind) in cases.items():
            got = after.get(relative)
            if got is None:
                problems.append(f"{relative}: missing")
                continue
            want_data, want_time = (src_data, BASE) if kind == "replace" else (data, stamp)
            if got[2] != sha(want_data) or abs(got[1] - want_time) > 2:
                problems.append(f"{relative}: expected {kind}, found size {got[0]} and time {got[1] - want_time} s "
                                "off the expected one")
        results.record(stage, "FAT volume: one hour apart with equal size and content is equal (kept, not "
                       "replaced), one hour apart with other content follows the times, two hours older is replaced, "
                       "one hour newer with another size stays", not problems, "; ".join(problems))
        counts = update_counts(ran.stderr)
        expected = {"gleiche übersprungen": 3, "neuere ersetzt": 2, "Ziel neuer behalten": 2}
        results.record(stage, "FAT volume: the notice counts 3 equal skipped, 2 replaced, 2 newer destination",
                       counts == expected, f"counted {counts}; stderr: {ran.stderr[-400:]}")
        results.record(stage, "FAT volume: the source folder is unchanged", snapshot(src_tree) == before_src, "")
    finally:
        unmount_all(work)


# --------------------------------------------------------------------------- connection flow (Linux)
#
# Plan 2026-10-31 section F (F1 to F6): a new connection starts without rights, the rights are chosen in
# one step ("confirm") or kept at none ("cancel"), a Direct code, a PIN offer and a room carry the rights
# chosen when they were made, the folders are chosen per device. Two to five isolated `se` instances of
# the candidate (each with its own profile, daemon and Share worker) talk through one development Share
# server; every right is checked by the other device's real commands, not by reading the profile alone
# (the profile is read only to prove which state the connection is in).

RIGHTS_PIN = "739518"
RIGHTS_MARK = "rights-exec-ok"
RIGHTS_DATA, RIGHTS_OTHER = "RightsData", "RightsOther"
RIGHTS_TEXT = {RIGHTS_DATA: "rights data text", RIGHTS_OTHER: "rights other text"}
# The presets as the host must store them: (folders, write, commands, terminal, screen_view, screen_control).
PRESET_SHAPES = {
    "none": ("none", False, "off", "off", "off", "off"),
    "read-only": ("all", False, "off", "off", "off", "off"),
    "read-write": ("all", True, "off", "off", "off", "off"),
    "files-exec": ("all", True, "allow", "allow", "off", "off"),
    "all": ("all", True, "allow", "allow", "allow", "allow"),
}


def cli(client, *args, stdin=None, timeout=150):
    """One `se` command of a client in its own profile; never raises (a timeout is code None)."""
    return run([client.cli, *args], env=client.env, timeout=timeout, input_text=stdin,
               log=client.owner.logs / f"rights-{client.label}.log")


def cli_json(client, *args, stdin=None, timeout=150):
    done = cli(client, *args, stdin=stdin, timeout=timeout)
    ensure(done.ok, f"{client.label}: se {' '.join(args)} failed: {short(done)}")
    try:
        return json.loads(done.stdout)
    except ValueError as error:
        raise AssertionError(f"{client.label}: se {' '.join(args)} gave no JSON ({error}): {short(done)}") from error


def until(check, what, seconds=120):
    """The first truthy result of `check`, polled once a second; a failing poll is retried."""
    deadline, last = time.monotonic() + seconds, None
    while time.monotonic() < deadline:
        try:
            last = check()
            if last:
                return last
        except (AssertionError, ValueError, KeyError, IndexError, TypeError, OSError) as error:
            last = f"{type(error).__name__}: {error}"
        time.sleep(1)
    raise AssertionError(f"timed out after {seconds} s: {what} (last: {last})")


def flat(rights):
    """The rights of one connection as the tuple of PRESET_SHAPES."""
    return (rights["folders"]["kind"], rights["write"], rights["commands"], rights["terminal"],
            rights["screen_view"], rights["screen_control"])


def grant_of(owner, device_id):
    grants = [grant for grant in cli_json(owner, "share", "grants", "list", "--json")["grants"]
              if grant["device_id"] == device_id]
    ensure(len(grants) == 1, f"{len(grants)} grants for device {device_id[:12]} on {owner.label}")
    return grants[0]


def pending_of(owner, device_id):
    listed = cli_json(owner, "share", "grants", "pending", "--json")["pending"]
    return [item for item in listed if item["target"].get("device_id") == device_id]


class Peer:
    """One connecting device and what its commands reach on the owner through one endpoint."""

    def __init__(self, rig, client, endpoint, name):
        self.rig, self.client, self.endpoint, self.name = rig, client, endpoint, name
        self.device_id = rig.net.pair.identity_of(client)
        self.attempts = 0

    def run(self, *args, timeout=120):
        return cli(self.client, *args, timeout=timeout)

    def prefix(self):
        """`share://direct/ID`: the device itself, the target of `se exec` and `se screen`."""
        return "/".join(self.endpoint.split("/")[:4])

    def established(self):
        """The owner's answer has arrived: this device's saved contact is „Freigegeben“ (independent of rights)."""
        rows = cli_json(self.client, "connections", "list", "--json")
        return any(row.get("kind") == "direct" and row.get("endpoint") == self.endpoint
                   and row.get("access_state") == "Freigegeben" for row in rows)

    def root_names(self):
        done = self.run("ls", self.endpoint)
        return done.stdout if done.ok else ""

    def reads(self, folder=RIGHTS_DATA):
        done = self.run("cat", f"{self.endpoint}/{folder}/readme.txt")
        return done.ok and done.stdout.strip() == RIGHTS_TEXT[folder]

    def lists(self, folder=RIGHTS_DATA):
        done = self.run("ls", f"{self.endpoint}/{folder}")
        return done.ok and "readme.txt" in done.stdout

    def writes(self, endpoint=None, folder_path=None):
        """Tries one upload into the owner's RightsData folder; whether the bytes arrived there."""
        self.attempts += 1
        name = f"w-{slug(self.name)}-{self.attempts}.txt"
        source = self.rig.net.root / f"src-{slug(self.name)}-{self.attempts}.txt"
        source.write_text(f"written by {self.name} {self.attempts}", encoding="utf-8")
        target = f"{endpoint or self.endpoint + '/' + RIGHTS_DATA}/{name}"
        done = self.run("cp", str(source), target, timeout=150)
        landed = (folder_path or self.rig.data) / name
        return done.ok and landed.is_file() and landed.read_text(encoding="utf-8") == source.read_text(encoding="utf-8")

    def execs(self):
        done = self.run("exec", self.prefix(), "--", "echo", RIGHTS_MARK)
        return done.ok and RIGHTS_MARK in done.stdout

    def screen(self):
        """`se screen probe`: (supported, view, control) as the owner reports its rights, or None."""
        done = self.run("screen", "probe", self.prefix(), "--json", timeout=160)
        if not done.ok:
            return None
        report = json.loads(done.stdout)
        return report.get("supported"), report.get("view"), report.get("control")

    def screen_refused(self):
        """The owner answers a screen probe with neither view nor control (a refusal, not a failed probe)."""
        screen = self.screen()
        ensure(screen is not None and screen[1] == "off" and screen[2] == "off",
               f"{self.name}: screen probe {screen}")

    def nothing_allowed(self):
        """Every one of read, write, commands and screen is refused; returns the evidence text."""
        ensure(RIGHTS_DATA not in self.root_names(), f"{self.name}: the root listing names {RIGHTS_DATA}")
        ensure(not self.lists(), f"{self.name}: could list {RIGHTS_DATA}")
        ensure(not self.reads(), f"{self.name}: could read a file")
        ensure(not self.writes(), f"{self.name}: could write a file")
        ensure(not self.execs(), f"{self.name}: could run a command")
        self.screen_refused()
        return "read, write, commands and the screen refused"

    def reads_eventually(self, seconds=120):
        until(lambda: self.reads(), f"{self.name} reads {RIGHTS_DATA}/readme.txt", seconds)

    def writes_eventually(self, seconds=120):
        until(lambda: self.writes(), f"{self.name} writes into {RIGHTS_DATA}", seconds)

    def execs_eventually(self, seconds=120):
        until(lambda: self.execs(), f"{self.name} runs a command on the owner", seconds)


class Rig:
    """The owner (hosts[0] of a net) and the devices that connect to it."""

    def __init__(self, ctx, net):
        self.ctx, self.net, self.results = ctx, net, ctx.results
        self.owner = net.hosts[0]
        self.owner_id = net.pair.identity_of(self.owner)
        self.data, self.other = net.root / "rights-data", net.root / "rights-other"
        self.code = None
        self.peers = {}

    def check(self, name, function):
        self.results.check("rights", name, function)

    def do(self, client, *args, stdin=None):
        done = cli(client, *args, stdin=stdin)
        ensure(done.ok, f"{client.label}: se {' '.join(args)} failed: {short(done)}")
        return done

    def prepare(self):
        """Two writable Direct folders on the owner and its Direct code."""
        for folder, label in ((self.data, RIGHTS_DATA), (self.other, RIGHTS_OTHER)):
            folder.mkdir(parents=True, exist_ok=True)
            (folder / "readme.txt").write_text(RIGHTS_TEXT[label], encoding="utf-8")
            self.do(self.owner, "share", "export", "add", str(folder), "--label", label)
            self.do(self.owner, "share", "export", "set", label, "--write")
        self.do(self.owner, "share", "worker", "refresh")
        self.code = cli_json(self.owner, "share", "identity", "--json")["direct_code"]
        speed.SECRETS.add(RIGHTS_PIN)
        shapes = {item["key"]: flat(item["rights"]) for item in cli_json(self.owner, "share", "grants", "presets",
                                                                          "--json")["presets"]}
        return shapes

    def connect(self, client, name, accept=()):
        """`add-peer` with the owner's Direct code, then the owner accepts the request (with `accept` options)."""
        direct = cli_json(client, "connections", "add-peer", "--code", self.code, "--name", name, "--json")
        peer = Peer(self, client, direct["endpoint"], name)
        accepted = []
        until(lambda: accepted.append(cli(self.owner, "share", "request", "accept", peer.device_id, "--json", *accept))
              or accepted[-1].ok, f"{name}: the owner accepts the request", 120)
        until(peer.established, f"{name}: the connection is established on its own side", 120)
        self.peers[name] = peer
        return peer


def rights_flow(ctx, net):
    rig = Rig(ctx, net)
    listed = rig.prepare()
    rig.check("owner: `se share grants presets` lists exactly none, read-only, read-write, files-exec and all "
              "with the documented rights",
              lambda: ensure(listed == PRESET_SHAPES, f"listed {listed}") or "the five presets")
    ensure(rig.code, "the owner has no Direct code")
    main, device_a, device_b, device_p = net.main, net.hosts[1], net.hosts[2], net.hosts[3]
    flow_direct(rig, main)
    flow_cancel_and_folders(rig, device_a)
    flow_code(rig, device_b)
    flow_pin(rig, device_p)
    flow_room(rig, main)


def flow_direct(rig, main):
    """1. A request accepted without rights: connected, listed as waiting, everything refused; confirm sets
    exactly the chosen rights step by step."""
    peer = []
    owner = rig.owner

    def connected():
        peer.append(rig.connect(main, "RightsM"))
        return f"established; endpoint {peer[0].endpoint}"
    rig.check("request accepted without rights: the connection exists on both sides", connected)
    if not peer:
        return
    m = peer[0]

    def waiting():
        items = pending_of(owner, m.device_id)
        ensure(len(items) == 1, f"{len(items)} pending entries for the device")
        ensure(items[0]["origin"] == "request", items[0]["origin"])
        ensure(flat(items[0]["rights"]) == PRESET_SHAPES["none"], items[0]["rights"])
        text = cli(owner, "share", "grants", "pending")
        ensure(text.ok and any(line.startswith("rights_pending\tdevice\t" + m.device_id) for line in
                               text.stdout.splitlines()), f"text form: {short(text)}")
        status = cli_json(owner, "share", "status", "--json")
        ensure(any(item["target"].get("device_id") == m.device_id for item in status["rights_pending"]),
               "share status --json does not list the waiting connection")
        grant = grant_of(owner, m.device_id)
        ensure(grant["rights"]["pending"] is True and grant["authorization"]["active"], grant["rights"])
        return "listed by grants pending (JSON and text), share status and grants list"
    rig.check("grants pending names the connection that waits for its rights", waiting)
    rig.check("without rights: reading, writing, commands and the screen are refused", m.nothing_allowed)

    def confirm(preset, *options):
        return cli_json(owner, "share", "grants", "confirm", m.device_id, "--preset", preset, *options, "--json")

    def read_only():
        done = confirm("read-only")
        ensure(done["action"] == "rights_confirmed" and flat(done["rights"]) == PRESET_SHAPES["read-only"], done)
        ensure(not pending_of(owner, m.device_id), "the connection still waits after the confirmation")
        m.reads_eventually()
        ensure(m.lists(), "cannot list the folder")
        ensure(not m.writes(), "could write although only reading was confirmed")
        ensure(not m.execs(), "could run a command although only reading was confirmed")
        m.screen_refused()
        return "reads; write, commands and screen refused"
    rig.check("grants confirm --preset read-only: reading works, writing is refused", read_only)

    def read_write():
        done = confirm("read-write")
        ensure(flat(done["rights"]) == PRESET_SHAPES["read-write"], done)
        m.writes_eventually()
        ensure(m.reads(), "cannot read after the confirmation")
        ensure(not m.execs(), "could run a command with read-write")
        ensure(flat(grant_of(owner, m.device_id)["rights"]) == PRESET_SHAPES["read-write"], "stored rights differ")
        return "writes; commands refused"
    rig.check("grants confirm --preset read-write: writing works, commands stay refused", read_write)

    def everything():
        done = confirm("all")
        ensure(flat(done["rights"]) == PRESET_SHAPES["all"], done)
        m.execs_eventually()
        stored = grant_of(owner, m.device_id)["rights"]
        ensure(flat(stored) == PRESET_SHAPES["all"], stored)
        screen = m.screen()
        if screen is not None and screen[0]:
            ensure(screen[1] == "allow" and screen[2] == "allow", f"screen probe {screen}")
            seen = f"screen probe answers {screen}"
        else:
            # A runner without a display: the host may be unable to describe a screen; the stored rights
            # above are what the owner would apply to a session.
            seen = f"screen probe gave {screen}; stored screen rights are allow"
        return f"commands run; {seen}"
    rig.check("grants confirm --preset all: commands run, desktop view and control are stored as allowed", everything)


def flow_cancel_and_folders(rig, device):
    """1 (cancel) and 3 (folders per device)."""
    owner = rig.owner
    peer = []

    def connected():
        peer.append(rig.connect(device, "RightsA"))
        return "established"
    rig.check("second new connection: accepted without rights, waits", connected)
    if not peer:
        return
    a = peer[0]
    rig.check("second new connection: listed as waiting",
              lambda: ensure(len(pending_of(owner, a.device_id)) == 1, "not listed by grants pending") or "listed")

    def cancelled():
        done = cli_json(owner, "share", "grants", "cancel", a.device_id, "--json")
        ensure(done["action"] == "rights_cancelled", done)
        ensure(not pending_of(owner, a.device_id), "still listed as waiting after cancel")
        grant = grant_of(owner, a.device_id)
        ensure(grant["authorization"]["active"], "the connection was removed by cancel")
        ensure(flat(grant["rights"]) == PRESET_SHAPES["none"] and grant["rights"]["pending"] is False,
               grant["rights"])
        ensure(a.established(), "the connection is not established on its own side after cancel")
        return a.nothing_allowed()
    rig.check("grants cancel: the connection stays and has no rights", cancelled)

    def folders():
        done = cli_json(owner, "share", "grants", "confirm", a.device_id, "--preset", "read-only",
                        "--folder", RIGHTS_DATA, "--json")
        chosen = done["rights"]["folders"]
        ensure(chosen["kind"] == "selected" and [Path(path).name for path in chosen["paths"]] == ["rights-data"],
               chosen)
        a.reads_eventually()
        ensure(RIGHTS_DATA in a.root_names() and RIGHTS_OTHER not in a.root_names(),
               f"root listing: {a.root_names()!r}")
        ensure(not a.reads(RIGHTS_OTHER) and not a.lists(RIGHTS_OTHER), "reached the folder that was not chosen")
        ensure(not a.writes(), "could write although only reading was confirmed")
        stored = grant_of(owner, a.device_id)["rights"]["folders"]
        ensure(stored["kind"] == "selected", stored)
        return "sees only " + RIGHTS_DATA
    rig.check("confirm with a folder choice: the device sees only that folder, a cancelled connection can be "
              "confirmed later", folders)

    def other_device():
        m = rig.peers.get("RightsM")
        ensure(m is not None and m.reads(RIGHTS_OTHER), "the first device (all folders) cannot read the other folder")
        return "the first device still reads " + RIGHTS_OTHER
    rig.check("folders are per device: another device with all folders still reads the other folder", other_device)


def flow_code(rig, device):
    """2a. A Direct code with a preset: a new device that connects with it has exactly those rights."""
    owner = rig.owner
    peer = []

    def code_rights():
        done = cli_json(owner, "share", "identity", "--preset", "files-exec", "--json")
        ensure(flat(done["code_rights"]) == PRESET_SHAPES["files-exec"], done["code_rights"])
        shown = cli_json(owner, "share", "identity", "--json")["code_rights"]
        ensure(shown is not None and flat(shown) == PRESET_SHAPES["files-exec"], shown)
        status = cli_json(owner, "share", "status", "--json")["code_rights"]
        ensure(status is not None and flat(status) == PRESET_SHAPES["files-exec"], status)
        return "code rights files-exec"
    rig.check("se share identity --preset files-exec stores the rights of the Direct code", code_rights)

    def connected():
        peer.append(rig.connect(device, "RightsB"))
        return "established"
    rig.check("a new device connects with that code and the owner accepts without choosing rights", connected)
    if not peer:
        return
    b = peer[0]

    def exact():
        ensure(not pending_of(owner, b.device_id), "the connection waits for a confirmation")
        grant = grant_of(owner, b.device_id)
        ensure(flat(grant["rights"]) == PRESET_SHAPES["files-exec"] and grant["rights"]["pending"] is False,
               grant["rights"])
        b.reads_eventually()
        b.writes_eventually()
        b.execs_eventually()
        ensure(b.reads(RIGHTS_OTHER), "does not see every folder")
        b.screen_refused()
        return "reads, writes, runs commands; the screen is refused"
    rig.check("the device has exactly the code's rights (files and commands, no screen) without a confirmation",
              exact)


def flow_pin(rig, device):
    """2b. A PIN offer with a preset: a device that pairs with the PIN has exactly those rights."""
    owner = rig.owner
    peer = []

    def paired():
        offer = cli_json(owner, "share", "discoverable", "--preset", "read-write", "--name", "RightsPin",
                         "--minutes", "10", "--pin-stdin", "--allow-weak-pin", "--json",
                         stdin=RIGHTS_PIN + "\n")
        ensure(offer.get("action") == "discoverable", offer)

        def found():
            listed = cli_json(device, "share", "discover", "list", "--json")["offers"]
            return [item["discovery_id"] for item in listed if item["display_name"] == "RightsPin"]
        discovery = until(found, "the offer RightsPin is discoverable", 120)[0]
        # The PIN is read up to the end of the input, so it carries no line ending.
        done = cli(device, "share", "discover", "pair", discovery, "--pin-stdin", "--json", stdin=RIGHTS_PIN,
                   timeout=170)
        ensure(done.ok and "completed" in done.stdout, f"pairing: {short(done)}")
        owner_id = rig.owner_id

        def endpoint():
            rows = cli_json(device, "connections", "list", "--json")
            direct = [row for row in rows if row.get("kind") == "direct" and row.get("access_state") == "Freigegeben"]
            named = [row for row in direct if row.get("device_id") == owner_id]
            return [row["endpoint"] for row in (named or (direct if len(direct) == 1 else []))]
        peer.append(Peer(rig, device, until(endpoint, "the paired contact is listed", 120)[0], "RightsP"))
        rig.peers["RightsP"] = peer[0]
        return "paired with the PIN"
    rig.check("PIN pairing with the offer's preset: the device pairs", paired)
    if not peer:
        return
    p = peer[0]

    def exact():
        ensure(not pending_of(rig.owner, p.device_id), "the connection waits for a confirmation")
        grant = grant_of(rig.owner, p.device_id)
        ensure(flat(grant["rights"]) == PRESET_SHAPES["read-write"] and grant["rights"]["pending"] is False,
               grant["rights"])
        p.reads_eventually()
        p.writes_eventually()
        ensure(not p.execs(), "could run a command with a read-write offer")
        p.screen_refused()
        return "reads and writes; commands and the screen refused"
    rig.check("the paired device has exactly the offer's rights (read-write) without a confirmation", exact)


def flow_room(rig, member):
    """2c. A room created with --preset read-only: a member reads, does not write."""
    owner = rig.owner
    room = {}

    def created():
        done = rig.do(owner, "share", "room", "create", "--name", "RightsRoom", "--preset", "read-only")
        fields = dict(line.split("\t", 1) for line in done.stdout.splitlines() if "\t" in line)
        ensure(fields.get("room_code", "").startswith("SE-R3-"), done.stdout)
        room.update(fields)
        room["relation"] = fields["room_code"].removeprefix("SE-R3-").rsplit("-", 1)[0]
        listed = cli_json(owner, "share", "room", "list", "--json")["rooms"]
        mine = [item for item in listed if item["id"] == fields["room_id"]]
        ensure(len(mine) == 1 and mine[0]["policy"]["members_may_write"] is False, mine)
        ensure(not [item for item in cli_json(owner, "share", "grants", "pending", "--json")["pending"]
                    if item["target"].get("profile_id") == fields["room_id"]], "the room waits for its rights")
        return f"room {fields['room_id']}"
    rig.check("share room create --preset read-only: the room has its rights and does not wait", created)
    if not room:
        return

    def joined():
        rig.do(member, "connections", "add-room", "--code", room["room_code"], "--name", "RightsRoom")
        for client in (owner, member):
            rig.do(client, "share", "worker", "refresh")
        room["endpoint"] = f"share://room/{room['relation']}/{rig.owner_id}/{RIGHTS_DATA}"
        until(lambda: cli(member, "cat", f"{room['endpoint']}/readme.txt").stdout.strip() == RIGHTS_TEXT[RIGHTS_DATA],
              "the member reads through the room", 180)
        return "the member reads through the room"
    rig.check("a member joins with the room code and reads", joined)
    if "endpoint" not in room:
        return

    def read_only_member():
        m = rig.peers.get("RightsM")
        ensure(m is not None, "the first device did not connect")
        # The member holds read-write through its Direct connection; through the room only reading is allowed.
        ensure(m.writes(), "control: the member cannot write through its Direct connection")
        ensure(not m.writes(endpoint=room["endpoint"]), "the member could write through the read-only room")
        ensure(m.run("ls", room["endpoint"]).ok, "the member cannot list through the room")
        return "reads through the room, upload refused, Direct upload works"
    rig.check("the room member reads but cannot write (its Direct connection can)", read_only_member)


def stage_rights(ctx):
    """Item 1 to 3 of the connection flow on one net of five candidate instances."""
    new = ctx.bins.get("new")
    server = build_share_server(ctx)
    if new is None or server is None:
        ctx.results.record("rights", "candidate and share server are built", False, "see the build checks")
        return
    net = None
    try:
        net = share_net(ctx.work, "rights", server, new, new, ctx.log_root, hosts=4, pair_hosts=False)
        rights_flow(ctx, net)
    except Exception as error:  # noqa: BLE001 - the compatibility stage still runs
        ctx.results.record("rights", "connection flow", False, f"{type(error).__name__}: {error}")
    finally:
        if net is not None:
            net.close()


def stage_rights_compat(ctx):
    """Item 4: a share made by v0.5.189 keeps its rights when the candidate is started on its profile."""
    old, new = ctx.bins.get("old"), ctx.bins.get("new")
    server = build_share_server(ctx)
    if old is None or new is None or server is None:
        ctx.results.record("rights", "v0.5.189, candidate and share server are built", False, "see the build checks")
        return
    net = None
    try:
        net = share_net(ctx.work, "rights-old", server, old, old, ctx.log_root, hosts=1)
        host, main, export = net.hosts[0], net.main, net.exports[0]
        main_id = net.pair.identity_of(main)
        seeded = Peer(types.SimpleNamespace(net=net, data=export), main, net.endpoints[0], "OldMain")

        def before():
            until(lambda: seeded.run("ls", net.endpoints[0]).ok, "v0.5.189 main lists the export", 120)
            until(lambda: seeded.writes(endpoint=net.endpoints[0], folder_path=export), "v0.5.189 main writes", 120)
            grants = json.loads(host.call("share", "grants", "list", "--json").stdout)["grants"]
            mine = [grant for grant in grants if grant["device_id"] == main_id]
            ensure(len(mine) == 1 and mine[0]["write"] is True, mine)
            return "v0.5.189 host: the main device reads and writes"
        ctx.results.check("rights", "v0.5.189 share: the profile made by v0.5.189 grants reading and writing", before)
        host.close()
        restarted = net.pair.Client(net.owner, Path(new), host.env, "host-candidate")
        net.clients.append(restarted)
        restarted.server_process = net.process
        restarted.call("share", "worker", "refresh")
        restarted.wait(lambda: (lambda worker: worker.get("reachable") and worker.get("running")
                                and worker.get("connected"))(
            json.loads(restarted.call("share", "status", "--json").stdout).get("worker", {})), "connected worker")

        def kept():
            grant = grant_of(restarted, main_id)
            rights = grant["rights"]
            ensure(grant["write"] is True and grant["authorization"]["active"], grant)
            ensure(flat(rights) == PRESET_SHAPES["read-write"] and rights["pending"] is False, rights)
            ensure(not pending_of(restarted, main_id), "the old share waits for a confirmation")
            ensure(not [item for item in cli_json(restarted, "share", "status", "--json")["rights_pending"]
                        if item["target"].get("device_id") == main_id], "share status lists it as waiting")
            return f"stored rights {flat(rights)}"
        ctx.results.check("rights", "candidate started on the v0.5.189 profile: the grant keeps its rights and does "
                          "not wait for a confirmation", kept)

        def still_works():
            until(lambda: seeded.run("ls", net.endpoints[0]).ok, "v0.5.189 main lists the export", 180)
            until(lambda: seeded.writes(endpoint=net.endpoints[0], folder_path=export), "v0.5.189 main writes", 180)
            ensure(not seeded.execs(), "commands became allowed")
            return "the v0.5.189 device still reads and writes, commands stay refused"
        ctx.results.check("rights", "candidate host: the v0.5.189 device still reads and writes through the "
                          "candidate's file access check", still_works)
    except Exception as error:  # noqa: BLE001
        ctx.results.record("rights", "v0.5.189 profile compatibility", False, f"{type(error).__name__}: {error}")
    finally:
        if net is not None:
            net.close()


# --------------------------------------------------------------------------- jobs behaviour

def job_behaviour(results, log_root):
    work = job_work("behaviour")
    ctx = Ctx(results, work, log_root)
    clients = {}

    def client_of(variant):
        if variant not in clients:
            clients[variant] = ai.Client(work, "se-" + slug(variant))
        return clients[variant]
    try:
        if WINDOWS:
            build_variants_windows(ctx)
            if ctx.bins.get("new"):
                local = work / "merge-local"
                merge_remote(ctx, "local-local", ctx.bins["new"], os.environ.copy(),
                             HostSide(work / "merge-local-dst", "", local=True), local)
        else:
            stage_bundles(results, log_root)
            build_variants(ctx, release=False, hybrids=True)
            if ctx.bins.get("new") and ctx.bins.get("old"):
                trees = fixture_trees(ctx)
                stage_ssh(ctx, trees, client_of)
                stage_share(ctx, trees)
                stage_sum_ftp(ctx, trees, client_of)
                stage_rights(ctx)
                stage_rights_compat(ctx)
                evaluate_equality(ctx)
                sanity(ctx)
                evaluate_keys(ctx)
                evaluate_sum(ctx)
            else:
                results.record("equality", "binaries of v0.5.189 and of the candidate are built", False,
                               "see the build checks")
            if ctx.bins.get("new"):
                local_dst = work / "merge-local-dst"
                local_dst.mkdir(parents=True, exist_ok=True)
                merge_remote(ctx, "local-local", ctx.bins["new"], client_of("merge").env,
                             HostSide(local_dst, "", local=True), work / "merge-local")
                stage_merge_ftp_smb(ctx, client_of)
                stage_fat(ctx, ctx.bins["new"], client_of("merge").env)
    finally:
        ai.stop_background_daemons(list(clients.values()))
        unmount_all()
        drop_worktrees(ctx)
        (log_root / "equality-matrix.json").write_text(
            json.dumps({"/".join(key): value for key, value in sorted(ctx.matrix.items())}, indent=2, default=str),
            encoding="utf-8")
        write_behaviour_summary(ctx)
        shutil.rmtree(work, ignore_errors=True)


def build_variants_windows(ctx):
    ctx.bins["new"] = build_se(ctx, "new", NATIVE, release=False)


def write_behaviour_summary(ctx):
    failed = [item for item in ctx.results.items if not item["ok"]]
    lines = [f"## sync-abgleich: behaviour ({speed.platform_key()})", "",
             f"{len(ctx.results.items) - len(failed)} passed, {len(failed)} failed", ""]
    for key, value in ctx.facts.items():
        lines.append(f"- {key}: {value}")
    if ctx.byte_rows:
        lines += ["", "### Bytes over the connection (lo and container networks, idle traffic subtracted; plan E)", "",
                  "| Weg / Schritt | v0.5.189 | Kandidat | Verhältnis |", "|---|---|---|---|"]
        for label, old, new in ctx.byte_rows:
            ratio = f"{new / old:.3f}" if old and new is not None else "n/a"
            lines.append(f"| {label} | {human(old)} | {human(new)} | {ratio} |")
    lines += ["", "| Stage | Check | Result |", "|---|---|---|"]
    for item in ctx.results.items:
        lines.append(f"| {item['stage']} | {item['check']} | {'pass' if item['ok'] else 'FAIL'} |")
    if failed:
        lines += ["", "### Failed checks", ""]
        lines += [f"- {item['stage']}: {item['check']}: {item['detail'][:400]}" for item in failed]
    if ctx.notes:
        lines += ["", "### Not run / notes", ""] + [f"- {text}" for text in ctx.notes]
    (ctx.log_root / "series.json").write_text(
        json.dumps({"/".join(key): value for key, value in sorted(ctx.series.items())}, indent=2, default=str),
        encoding="utf-8")
    (ctx.log_root / "summary.md").write_text(speed.redact("\n".join(lines)) + "\n", encoding="utf-8")


# --------------------------------------------------------------------------- job measure

FILES, FOLDERS = 100_000, 10_000
SUM_FILES, SUM_FOLDERS = 20_000, 2_000
COMPARE_OPS = ("cmp", "cmp2", "sum")
OP_NAMES = {"cmp": "sync compare (first, no stored keys)", "cmp2": "sync compare (second, nothing changed)",
            "sum": "sync compare --compare checksum", "ls": "ls -R"}
OLD_LABEL, NEW_LABEL = BASELINE_TAG, "candidate"


def make_big(root, files, folders):
    per_folder = files // folders
    for folder in range(folders):
        directory = Path(root) / f"g{folder // 100:03d}" / f"f{folder:05d}"
        directory.mkdir(parents=True, exist_ok=True)
        for number in range(per_folder):
            path = directory / f"n{number:03d}.dat"
            path.write_bytes(b"%08d" % (folder * per_folder + number))
            os.utime(path, (BASE, BASE))
    normalize_dirs(root)


class Meter:
    def __init__(self, ctx, reps, deadline):
        self.ctx, self.reps, self.deadline = ctx, reps, deadline
        self.rows, self.refs, self.counts = [], [], {}

    def left(self):
        return self.deadline - time.monotonic()

    def find(self, way, op, mode, tool):
        for row in self.rows:
            if (row["way"], row["op"], row["mode"], row["tool"]) == (way, op, mode, tool):
                return row
        return None

    def row(self, way, op, mode, tool):
        row = self.find(way, op, mode, tool)
        if row is None:
            row = {"way": way, "op": op, "mode": mode, "tool": tool, "samples": [], "internal": [], "bytes": [],
                   "errors": []}
            self.rows.append(row)
        return row

    def write(self):
        (self.ctx.log_root / "measurements.json").write_text(
            json.dumps({"facts": self.ctx.facts, "notes": self.ctx.notes, "rows": self.rows,
                        "references": self.refs, "counts": {"/".join(key): value
                                                            for key, value in self.counts.items()}}, indent=2),
            encoding="utf-8")


def parse_count(op, ran):
    """(entries, seconds reported by the command itself) of one run's JSON output."""
    try:
        data = json.loads(ran.stdout)
    except ValueError:
        return None, None
    if op in COMPARE_OPS:
        return data.get("source_files", 0) + data.get("target_files", 0), data.get("seconds")
    return len(data), None


def measure(meter, way, op, mode, jobs, prepare=None, prime=False):
    """`jobs` = {tool: (command, env)}. Each tool runs `reps` times, the order rotating per repetition. A
    warm mode first runs every tool once untimed; `prime` does so in the cold mode as well (the second
    comparison needs the keys of a first one); a cold mode drops the page cache before every run.
    `prepare(env)` runs before every run, timed or not (the first comparison forgets its stored keys).
    Each timed run also records the bytes that crossed the counted interfaces (Linux, `wire_bytes`)."""
    tools = list(jobs)
    if mode == "warm" or prime:
        for tool in tools:
            command, env = jobs[tool]
            if prepare is not None:
                prepare(env)
            run(command, env=env, timeout=1800)
    for rep in range(meter.reps):
        shift = rep % len(tools)
        for tool in tools[shift:] + tools[:shift]:
            row = meter.row(way, op, mode, tool)
            if meter.left() < 300:
                row["errors"].append("not run: time budget of the job exhausted")
                continue
            command, env = jobs[tool]
            if prepare is not None:
                prepare(env)
            if mode == "cold":
                speed.drop_caches()
            ran, wire = metered(command, env=env, timeout=1800)
            if not ran.ok:
                row["errors"].append(f"exit {ran.code}: {speed.redact(ran.stderr[-300:])}")
                continue
            row["samples"].append(ran.seconds)
            if wire is not None:
                row["bytes"].append(wire)
            if rep == 0:
                entries, internal = parse_count(op, ran)
                if entries is not None:
                    meter.counts[(way, op, tool)] = entries
                if internal is not None:
                    row["internal"].append(internal)
    meter.write()


def reference(meter, way, tool, command, env=None):
    """A reference command on the same data, cold (Linux) as often as the repetitions."""
    ok_codes = range(8) if WINDOWS else (0,)
    seconds, error = [], None
    for _ in range(meter.reps):
        if meter.left() < 300:
            error = "not run: time budget of the job exhausted"
            break
        if not WINDOWS:
            speed.drop_caches()
        ran = run(command, env=env, timeout=1800)
        if ran.code not in ok_codes:
            error = f"exit {ran.code}: {ran.stderr[-200:]}"
            break
        seconds.append(ran.seconds)
    meter.refs.append({"way": way, "tool": tool, "samples": seconds, "error": error})


def median_text(samples):
    return f"{statistics.median(samples):.2f} s" if samples else "n/a"


def write_measure_summary(meter, results):
    ctx = meter.ctx
    lines = [f"## sync-abgleich: measure ({speed.platform_key()})", ""]
    lines += [f"- {key}: {value}" for key, value in ctx.facts.items()]
    lines.append(f"- tree: {FILES} files in {FOLDERS} folders per side ({SUM_FILES} files in {SUM_FOLDERS} folders "
                 "for the checksum mode; an assumption: no load was named); "
                 f"repetitions: {meter.reps} (median shown); runner: {platform.platform()}, {os.cpu_count()} CPUs")
    lines += ["", "Verhaeltnis = Kandidat / v0.5.189 (below 1 = the candidate is faster); Faktor = "
              "v0.5.189 / Kandidat. Times are the wall time of the whole `se` process.", "",
              "### Weg | v0.5.189 | Kandidat | Verhältnis", "",
              "| Weg | v0.5.189 | Kandidat | Verhältnis | Faktor | n (old/new) | Bytes v0.5.189 | "
              "Bytes Kandidat |", "|---|---|---|---|---|---|---|---|"]
    ways = []
    for row in meter.rows:
        key = (row["way"], row["op"], row["mode"])
        if key not in ways:
            ways.append(key)
    names = OP_NAMES
    empty = {"samples": [], "internal": [], "bytes": []}
    for way, op, mode in ways:
        old = meter.find(way, op, mode, OLD_LABEL) or empty
        new = meter.find(way, op, mode, NEW_LABEL) or empty
        old_t = statistics.median(old["samples"]) if old["samples"] else None
        new_t = statistics.median(new["samples"]) if new["samples"] else None
        ratio = f"{new_t / old_t:.2f}" if old_t and new_t else "n/a"
        factor = f"{old_t / new_t:.2f}x" if old_t and new_t else "n/a"
        wire_old = human(statistics.median(old["bytes"])) if old["bytes"] else "n/a"
        wire_new = human(statistics.median(new["bytes"])) if new["bytes"] else "n/a"
        lines.append(f"| {way} {names[op]} {mode} | {median_text(old['samples'])} | {median_text(new['samples'])} | "
                     f"{ratio} | {factor} | {len(old['samples'])}/{len(new['samples'])} | {wire_old} | {wire_new} |")
    if any(row["internal"] for row in meter.rows):
        lines += ["", "### `seconds` reported by `se sync compare` itself (median of the first run per mode)", "",
                  "| Weg | v0.5.189 | Kandidat |", "|---|---|---|"]
        for way, op, mode in ways:
            if op in COMPARE_OPS:
                old = meter.find(way, op, mode, OLD_LABEL) or empty
                new = meter.find(way, op, mode, NEW_LABEL) or empty
                lines.append(f"| {way} {op} {mode} | {median_text(old['internal'])} | "
                             f"{median_text(new['internal'])} |")
    lines += ["", "### References on the same data (no filtering, nothing compared)", "",
              "| Weg | Referenz | Median |", "|---|---|---|"]
    for ref in meter.refs:
        lines.append(f"| {ref['way']} | {ref['tool']} | {median_text(ref['samples'])}"
                     f"{' (' + ref['error'] + ')' if ref['error'] else ''} |")
    errors = [(row, error) for row in meter.rows for error in row["errors"] if not error.startswith("not run")]
    if errors:
        lines += ["", "### Run errors", ""]
        lines += [f"- {row['way']} {row['op']} {row['mode']} {row['tool']}: {error[:300]}"
                  for row, error in errors[:60]]
    failed = [item for item in results.items if not item["ok"]]
    lines += ["", f"### Checks: {len(results.items) - len(failed)} passed, {len(failed)} failed", ""]
    lines += [f"- FAILED {item['stage']}: {item['check']}: {item['detail'][:300]}" for item in failed]
    if ctx.notes:
        lines += ["", "### Not measured / notes", ""] + [f"- {text}" for text in ctx.notes]
    (ctx.log_root / "summary.md").write_text(speed.redact("\n".join(lines)) + "\n", encoding="utf-8")


def check_counts(meter):
    for way, op in sorted({(way, op) for way, op, _tool in meter.counts}):
        old, new = meter.counts.get((way, op, OLD_LABEL)), meter.counts.get((way, op, NEW_LABEL))
        if old is None or new is None:
            continue
        what = "entries" if op == "ls" else "files seen on both sides"
        meter.ctx.results.record("measure", f"{way} {op}: the candidate sees as many {what} as v0.5.189 "
                                 f"({old} / {new})", old == new, "" if old == new else f"{old} != {new}")


def job_measure(results, log_root, args):
    work = job_work("measure")
    ctx = Ctx(results, work, log_root)
    meter = Meter(ctx, max(1, args.repetitions), time.monotonic() + args.budget_minutes * 60)
    clients, box = {}, None
    try:
        ctx.facts["free disk at start"] = f"{speed.free_gib(work):.1f} GiB in {work}"
        if not WINDOWS:
            stage_bundles(results, log_root)
        build_variants(ctx, release=True, hybrids=False)
        if not (ctx.bins.get("old") and ctx.bins.get("new")):
            return
        stage_size(ctx)
        tools = {OLD_LABEL: ctx.bins["old"], NEW_LABEL: ctx.bins["new"]}
        for label in tools:
            clients[label] = ai.Client(work, "se-" + slug(label))
        envs = {label: clients[label].env for label in tools}
        modes = ["warm"] if WINDOWS or not sudo_ok() else ["cold", "warm"]
        if modes == ["warm"]:
            note(ctx.notes, "cold runs: not measured, the page cache cannot be dropped on this runner "
                 "(Windows, or sudo -n unavailable)")
        note(ctx.notes, "the storage analysis has no command in the se command line, so it is not measured here; the "
             "references are `find`, `du` (Windows: robocopy /L) on the same folder and `se search '*'`")
        big = work / "big"
        make_big(big / "a" / "tree", FILES, FOLDERS)
        copy_tree(big / "a" / "tree", big / "b" / "tree")
        a_tree, b_tree = big / "a" / "tree", big / "b" / "tree"
        # The checksum mode reads every file of both sides, so it gets its own, smaller tree.
        make_big(big / "a" / "sum", SUM_FILES, SUM_FOLDERS)
        copy_tree(big / "a" / "sum", big / "b" / "sum")
        a_sum, b_sum = big / "a" / "sum", big / "b" / "sum"

        def folder_references(way, folder):
            if WINDOWS:
                target = work / "empty-reference"
                target.mkdir(exist_ok=True)
                reference(meter, way, "robocopy /L /E (list only)", ["robocopy", str(folder), str(target), "/L",
                          "/E", "/NFL", "/NDL", "/NJH", "/NJS", "/NP"])
            else:
                reference(meter, way, "find -printf (path, size, time)", ["find", str(folder), "-printf",
                                                                         "%p %s %T@\\n"])
                reference(meter, way, "du -sb", ["du", "-sb", str(folder)])

        def way_runs(way, loc_a, loc_b, ls_target, folder, binaries, environments, sum_locs):
            names = {label: slug(f"m-{way}-{label}")[:60] for label in binaries}
            sum_names = {label: slug(f"s-{way}-{label}")[:60] for label in binaries}
            saved = sum_saved = True
            for label, binary in binaries.items():
                added = run([binary, "sync", "add", "--name", names[label], "--source", loc_a, "--target", loc_b,
                             "--trigger", "manual", "--enabled", "false"], env=environments[label], timeout=300,
                            log=log_root / "measure-jobs.log")
                results.record("measure", f"{way} {label}: sync setup saved", added.ok, short(added, 300))
                saved = saved and added.ok
                added = run([binary, "sync", "add", "--name", sum_names[label], "--source", sum_locs[0], "--target",
                             sum_locs[1], "--trigger", "manual", "--enabled", "false", "--compare", "checksum"],
                            env=environments[label], timeout=300, log=log_root / "measure-jobs.log")
                results.record("measure", f"{way} {label}: checksum setup saved", added.ok, short(added, 300))
                sum_saved = sum_saved and added.ok
            calibrate_idle()

            def compares(names_by_label):
                return {label: ([binary, "sync", "compare", names_by_label[label], "--json"], environments[label])
                        for label, binary in binaries.items()}
            for mode in modes:
                if saved:
                    # First comparison: no stored folder keys before any run; second: the keys of a first one.
                    measure(meter, way, "cmp", mode, compares(names), prepare=forget_keys)
                    measure(meter, way, "cmp2", mode, compares(names), prime=True)
                measure(meter, way, "ls", mode, {label: ([binary, "ls", ls_target, "-R", "--json"],
                                                         environments[label]) for label, binary in binaries.items()})
            if sum_saved:
                measure(meter, way, "sum", "warm", compares(sum_names), prepare=forget_keys)
            if not WINDOWS:
                for label, binary in binaries.items():
                    reference(meter, way, f"se search '*' ({label})", [binary, "search", ls_target, "*", "--glob"],
                              environments[label])
            if folder is not None:
                folder_references(way, folder)
            meter.write()

        way_runs("LL", str(a_tree), str(b_tree), str(b_tree), b_tree, tools, envs, (str(a_sum), str(b_sum)))
        if WINDOWS:
            return
        # LS and LA: the SSH account (agent and SFTP share its data).
        box = SshBox(work, log_root)
        copy_tree(a_tree, box.data / "tree")
        copy_tree(a_sum, box.data / "sum")
        try:
            box.start()
        except Exception as error:  # noqa: BLE001
            results.record("measure", "OpenSSH container", False, f"{type(error).__name__}: {error}")
            box = None
        if box is not None:
            connected = True
            for label, binary in tools.items():
                problem = None
                for conn, agent in (("sftp", False), ("agent", True)):
                    host, port = box.endpoint(agent)
                    problem = problem or add_saved(binary, envs[label], "sftp", port, box.user, box.password,
                                                   "/config/data", conn, log_root / "measure-connections.log",
                                                   agent=agent, trust_client=clients[label], host=host)
                results.record("measure", f"{label}: saved SSH connections work", problem is None, problem or "")
                connected = connected and problem is None
            if connected:
                way_runs("LS", str(a_tree), "@sftp:/tree", "@sftp:/tree", box.data / "tree", tools, envs,
                         (str(a_sum), "@sftp:/sum"))
                remove(box.home / ".cache")
                way_runs("LA", str(a_tree), "@agent:/tree", "@agent:/tree", box.data / "tree", tools, envs,
                         (str(a_sum), "@agent:/sum"))
        # LH and HH: one Share pair per version, because the host runs the version under test.
        server = build_share_server(ctx)
        for label, binary in tools.items():
            if server is None:
                break
            if meter.left() < 1800:
                note(ctx.notes, f"Share {label}: not measured, time budget of the job exhausted")
                continue
            net = None
            try:
                net = share_net(work, f"m-{label}", server, binary, binary, log_root)
                for index in (0, 1):
                    copy_tree(a_tree, net.exports[index] / "tree")
                    copy_tree(a_sum, net.exports[index] / "sum")
                one, main_env = {label: str(net.main.cli)}, {label: net.main.env}
                way_runs("LH", str(a_tree), f"{net.endpoints[0]}/tree", f"{net.endpoints[0]}/tree",
                         net.exports[0] / "tree", one, main_env, (str(a_sum), f"{net.endpoints[0]}/sum"))
                way_runs("HH", f"{net.endpoints[1]}/tree", f"{net.endpoints[0]}/tree", f"{net.endpoints[1]}/tree",
                         None, one, main_env, (f"{net.endpoints[1]}/sum", f"{net.endpoints[0]}/sum"))
            except Exception as error:  # noqa: BLE001
                results.record("measure", f"Share pair {label}", False, f"{type(error).__name__}: {error}")
            finally:
                if net is not None:
                    net.close()
    finally:
        if box is not None:
            box.close()
        check_counts(meter)
        ai.stop_background_daemons(list(clients.values()))
        unmount_all()
        drop_worktrees(ctx)
        meter.write()
        write_measure_summary(meter, results)
        shutil.rmtree(work, ignore_errors=True)


# --------------------------------------------------------------------------- main

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    parser.add_argument("--job", choices=("fmt", "functional", "behaviour", "measure"), required=True)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--budget-minutes", type=float, default=320)
    args = parser.parse_args()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = ai.Results(log_root)
    title = {"fmt": "rustfmt on changed lines", "functional": f"functional ({speed.platform_key()})"}
    try:
        if args.job == "fmt":
            speed.job_fmt(results, log_root)
        elif args.job == "functional":
            job_functional(results, log_root)
        elif args.job == "behaviour":
            job_behaviour(results, log_root)
        else:
            job_measure(results, log_root, args)
    except Exception as error:  # noqa: BLE001 - the report is written in every case
        import traceback
        trace = traceback.format_exc()
        print(trace, flush=True)
        results.record(args.job, "job", False, f"{type(error).__name__}: {error}; {trace[-1500:]}")
    finally:
        if args.job in ("fmt", "functional"):
            speed.write_functional_summary(results, log_root, f"sync-abgleich: {title[args.job]}")
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
