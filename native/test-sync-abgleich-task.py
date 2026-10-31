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
             commits 1f004fd9 (`se sync compare`) and bc18587b (`se ls -R`)
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
  measure    (Linux; Windows local) wall time of `se sync compare` and
             `se ls -R` for v0.5.189 (plus the two commits) and the candidate
             on one generated tree (100 000 files in 10 000 folders, an
             assumption), per side combination, cold (page cache dropped where
             the runner allows it) and warm, `--repetitions` times, median;
             references on the same data: `find`, `du`, `se search`
             (Windows: robocopy /L)

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
CLI_COMMITS = ("1f004fd9", "bc18587b")
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


def gated_elsewhere(path, number):
    """True when the test is cfg-gated to the platform this runner is not."""
    lines = Path(path).read_text(encoding="utf-8", errors="replace").splitlines()
    gates = " ".join(line for line in lines[max(0, number - 12):number + 1] if "cfg(" in line)
    head = " ".join(line for line in lines[:40] if line.startswith("#![cfg("))
    hint = str(path).lower()
    if WINDOWS:
        return bool(re.search(r"unix|target_os\s*=\s*\"(linux|android|macos)\"", gates + head)) \
            or "linux" in hint or "unix" in hint
    return bool(re.search(r"windows", gates + head)) or "windows" in hint


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
        self.data.mkdir(parents=True)

    def start(self):
        log = self.log_root / "ssh-box.log"
        run(["docker", "rm", "-f", self.name], timeout=120)
        started = run(["docker", "run", "-d", "--name", self.name, "-p", "127.0.0.1::2222",
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
        ensure(mapped, "docker port gave no mapping")
        port = int(mapped[0].rsplit(":", 1)[1])
        ensure(speed.wait_tcp(port, b"SSH-", 120), "the OpenSSH container did not answer")
        time.sleep(2)
        return port

    def close(self):
        logs = run(["docker", "logs", self.name], timeout=120)
        with open(self.log_root / "ssh-box.log", "a", encoding="utf-8") as handle:
            handle.write(speed.redact(f"--- docker logs\n{logs.output[-20000:]}\n"))
        run(["docker", "rm", "-f", self.name], timeout=120)


def add_saved(binary, env, kind, port, user, password, root, label, log, agent=False, trust_client=None):
    """A saved connection of one `se`; None when it works, else the reason."""
    args = ["connections", "add", kind, "--host", "127.0.0.1", "--port", str(port), "--user", user, "--root", root,
            "--label", label, "--password-stdin", *(["--agent"] if agent else [])]
    added = run([binary, *args], env=env, input_text=password + "\n", timeout=300, log=log)
    if not added.ok:
        return f"connections add: {short(added, 400)}"
    if trust_client is not None:
        try:
            ai.trust_ssh_host_key(trust_client, binary, "127.0.0.1", port, log)
        except Exception as error:  # noqa: BLE001
            return f"host key: {error}"
    listed = run([binary, "ls", f"@{label}:/"], env=env, timeout=600, log=log)
    return None if listed.ok else f"ls: {short(listed, 400)}"


def share_net(work, tag, server, main_binary, host_binary, log_root, hosts=2):
    """A Share server, one main client and `hosts` host clients paired Direct, each host exporting a
    writable folder `export<i>`. The main and the host clients may be different versions."""
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
    net = types.SimpleNamespace(process=process, root=root, clients=[], endpoints=[], exports=[], main=None)

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
        for index, host in enumerate(hosts_list):
            identity = json.loads(host.call("share", "identity", "--json").stdout)
            direct = json.loads(main.call("connections", "add-peer", "--code", identity["direct_code"], "--name",
                                          f"AbgleichPeer{index}", "--json").stdout)
            host.wait(lambda h=host: json.loads(h.call("share", "request", "--json").stdout).get("count", 0) == 1,
                      "incoming request")
            host.call("share", "request", "accept", "--json")
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
        net.main = main
        return net
    except Exception:
        close()
        raise


def build_share_server(ctx):
    server_dir = ROOT / "share-server"
    built = run(["cargo", "build", "--locked", "--bin", "se-share-server"], cwd=server_dir,
                env=dict(os.environ, CARGO_TARGET_DIR=str(server_dir / "target")), timeout=3600,
                log=ctx.log_root / "build-share-server.log")
    server = server_dir / "target" / "debug" / f"se-share-server{EXE}"
    ctx.results.record("build", "development share server", built.ok and server.exists(),
                       "" if built.ok else short(built, 2000))
    return server if built.ok and server.exists() else None


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
    for root in sorted(roots, key=len, reverse=True):
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
            if not equal and kind == "cmp" and way == "LA" and shape == "nohidden":
                # Known, intended change (plan C3, V1): v0.5.189's agent listing never marks dot names as
                # hidden, so include_hidden = false did nothing on an agent side. The candidate's agent
                # must give what SFTP gives for the same data.
                sftp_new = matrix.get(("cmp", "LS", "nohidden", "new/new"))
                sftp_old = matrix.get(("cmp", "LS", "nohidden", "old/old"))
                if sftp_new is not None and value == sftp_new and sftp_old == sftp_new and reference != sftp_old:
                    results.record("equality", f"{label}: {variant} differs from v0.5.189 only by the fixed "
                                   "hidden-file filter of the agent side (V1, equals the SFTP result)", True, detail)
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
                problem = problem or add_saved(binary, client.env, "sftp", box.port, box.user, box.password,
                                               "/config/data", label, log, agent=agent,
                                               trust_client=client if label == "sftp" else None)
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
            roots = [local_a, net.root, *net.endpoints]
            endpoint0, endpoint1 = net.endpoints
            binary, env = str(net.main.cli), net.main.env
            collect(ctx, variant, "LH", binary, env, lambda sub: str(local_a / sub),
                    lambda sub: f"{endpoint0}/{sub}", lambda sub: f"{endpoint0}/{sub}", roots)
            collect(ctx, variant, "HH", binary, env, lambda sub: f"{endpoint1}/{sub}",
                    lambda sub: f"{endpoint0}/{sub}", lambda sub: f"{endpoint1}/{sub}", roots)
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
    except Exception as error:  # noqa: BLE001
        results.record(stage, f"{name}", False, f"{type(error).__name__}: {error}")
    finally:
        remove(work)


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
                evaluate_equality(ctx)
                sanity(ctx)
            else:
                results.record("equality", "binaries of v0.5.189 and of the candidate are built", False,
                               "see the build checks")
            if ctx.bins.get("new"):
                local_dst = work / "merge-local-dst"
                local_dst.mkdir(parents=True, exist_ok=True)
                merge_remote(ctx, "local-local", ctx.bins["new"], client_of("merge").env,
                             HostSide(local_dst, "", local=True), work / "merge-local")
                stage_merge_ftp_smb(ctx, client_of)
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
    lines += ["", "| Stage | Check | Result |", "|---|---|---|"]
    for item in ctx.results.items:
        lines.append(f"| {item['stage']} | {item['check']} | {'pass' if item['ok'] else 'FAIL'} |")
    if ctx.notes:
        lines += ["", "### Not run / notes", ""] + [f"- {text}" for text in ctx.notes]
    (ctx.log_root / "summary.md").write_text(speed.redact("\n".join(lines)) + "\n", encoding="utf-8")


# --------------------------------------------------------------------------- job measure

FILES, FOLDERS = 100_000, 10_000
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
            row = {"way": way, "op": op, "mode": mode, "tool": tool, "samples": [], "internal": [], "errors": []}
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
    if op == "cmp":
        return data.get("source_files", 0) + data.get("target_files", 0), data.get("seconds")
    return len(data), None


def measure(meter, way, op, mode, jobs):
    """`jobs` = {tool: (command, env)}. Each tool runs `reps` times, the order rotating per repetition. A
    warm mode first runs every tool once untimed; a cold mode drops the page cache before every run."""
    tools = list(jobs)
    if mode == "warm":
        for tool in tools:
            command, env = jobs[tool]
            run(command, env=env, timeout=1800)
    for rep in range(meter.reps):
        shift = rep % len(tools)
        for tool in tools[shift:] + tools[:shift]:
            row = meter.row(way, op, mode, tool)
            if meter.left() < 300:
                row["errors"].append("not run: time budget of the job exhausted")
                continue
            command, env = jobs[tool]
            if mode == "cold":
                speed.drop_caches()
            ran = run(command, env=env, timeout=1800)
            if not ran.ok:
                row["errors"].append(f"exit {ran.code}: {speed.redact(ran.stderr[-300:])}")
                continue
            row["samples"].append(ran.seconds)
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
    lines.append(f"- tree: {FILES} files in {FOLDERS} folders per side (an assumption: no load was named); "
                 f"repetitions: {meter.reps} (median shown); runner: {platform.platform()}, {os.cpu_count()} CPUs")
    lines += ["", "Verhaeltnis = Kandidat / v0.5.189 (below 1 = the candidate is faster); Faktor = "
              "v0.5.189 / Kandidat. Times are the wall time of the whole `se` process.", "",
              "### Weg | v0.5.189 | Kandidat | Verhältnis", "",
              "| Weg | v0.5.189 | Kandidat | Verhältnis | Faktor | n (old/new) |", "|---|---|---|---|---|---|"]
    ways = []
    for row in meter.rows:
        key = (row["way"], row["op"], row["mode"])
        if key not in ways:
            ways.append(key)
    names = {"cmp": "sync compare", "ls": "ls -R"}
    empty = {"samples": [], "internal": []}
    for way, op, mode in ways:
        old = meter.find(way, op, mode, OLD_LABEL) or empty
        new = meter.find(way, op, mode, NEW_LABEL) or empty
        old_t = statistics.median(old["samples"]) if old["samples"] else None
        new_t = statistics.median(new["samples"]) if new["samples"] else None
        ratio = f"{new_t / old_t:.2f}" if old_t and new_t else "n/a"
        factor = f"{old_t / new_t:.2f}x" if old_t and new_t else "n/a"
        lines.append(f"| {way} {names[op]} {mode} | {median_text(old['samples'])} | {median_text(new['samples'])} | "
                     f"{ratio} | {factor} | {len(old['samples'])}/{len(new['samples'])} |")
    if any(row["internal"] for row in meter.rows):
        lines += ["", "### `seconds` reported by `se sync compare` itself (median of the first run per mode)", "",
                  "| Weg | v0.5.189 | Kandidat |", "|---|---|---|"]
        for way, op, mode in ways:
            if op == "cmp":
                old = meter.find(way, op, mode, OLD_LABEL) or empty
                new = meter.find(way, op, mode, NEW_LABEL) or empty
                lines.append(f"| {way} {mode} | {median_text(old['internal'])} | {median_text(new['internal'])} |")
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

        def way_runs(way, loc_a, loc_b, ls_target, folder, binaries, environments):
            names = {label: slug(f"m-{way}-{label}")[:60] for label in binaries}
            saved = True
            for label, binary in binaries.items():
                added = run([binary, "sync", "add", "--name", names[label], "--source", loc_a, "--target", loc_b,
                             "--trigger", "manual", "--enabled", "false"], env=environments[label], timeout=300,
                            log=log_root / "measure-jobs.log")
                results.record("measure", f"{way} {label}: sync setup saved", added.ok, short(added, 300))
                saved = saved and added.ok
            for mode in modes:
                if saved:
                    measure(meter, way, "cmp", mode, {label: ([binary, "sync", "compare", names[label], "--json"],
                                                              environments[label])
                                                     for label, binary in binaries.items()})
                measure(meter, way, "ls", mode, {label: ([binary, "ls", ls_target, "-R", "--json"],
                                                         environments[label]) for label, binary in binaries.items()})
            if not WINDOWS:
                for label, binary in binaries.items():
                    reference(meter, way, f"se search '*' ({label})", [binary, "search", ls_target, "*", "--glob"],
                              environments[label])
            if folder is not None:
                folder_references(way, folder)
            meter.write()

        way_runs("LL", str(a_tree), str(b_tree), str(b_tree), b_tree, tools, envs)
        if WINDOWS:
            return
        # LS and LA: the SSH account (agent and SFTP share its data).
        box = SshBox(work, log_root)
        copy_tree(a_tree, box.data / "tree")
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
                    problem = problem or add_saved(binary, envs[label], "sftp", box.port, box.user, box.password,
                                                   "/config/data", conn, log_root / "measure-connections.log",
                                                   agent=agent, trust_client=clients[label] if conn == "sftp" else None)
                results.record("measure", f"{label}: saved SSH connections work", problem is None, problem or "")
                connected = connected and problem is None
            if connected:
                way_runs("LS", str(a_tree), "@sftp:/tree", "@sftp:/tree", box.data / "tree", tools, envs)
                remove(box.home / ".cache")
                way_runs("LA", str(a_tree), "@agent:/tree", "@agent:/tree", box.data / "tree", tools, envs)
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
                one, main_env = {label: str(net.main.cli)}, {label: net.main.env}
                way_runs("LH", str(a_tree), f"{net.endpoints[0]}/tree", f"{net.endpoints[0]}/tree",
                         net.exports[0] / "tree", one, main_env)
                way_runs("HH", f"{net.endpoints[1]}/tree", f"{net.endpoints[0]}/tree", f"{net.endpoints[1]}/tree",
                         None, one, main_env)
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
        results.record(args.job, "job", False, f"{type(error).__name__}: {error}")
    finally:
        if args.job in ("fmt", "functional"):
            speed.write_functional_summary(results, log_root, f"sync-abgleich: {title[args.job]}")
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
