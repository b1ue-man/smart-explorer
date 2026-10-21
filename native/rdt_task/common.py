"""Shared infrastructure of the one batch suite ``rechte-desktop-tresore-task`` (plan D3).

The entrypoint ``native/test-rechte-desktop-tresore-task.py`` runs exactly one
GitHub job per call (``--job``). Everything a stage needs lives here:

``Signals``
    The expected acceptance signals of ``rdt_task/expected_signals.txt`` (format
    in that file's header). Stages select tests only through these exact name
    lists, never through prefix filters: every library test runs by its full
    path with ``--exact`` (D3). W14: a ``vault_task_*`` entry matches only a test
    whose last path segment equals the name, so ``mount_vault_task_*`` is never
    selected through a vault entry, and it is no batch name for the inventory.
``Report``
    ``report.json`` = ``{"job", "candidate_sha", "tests": [{"name", "stage",
    "status", "reason", "duration_s"}], "stages": [{"name", "status", "reason",
    "duration_s"}]}`` plus the job summary (``summary.md`` and
    ``$GITHUB_STEP_SUMMARY``). Statuses: ``passed`` (bestanden), ``failed``
    (gescheitert), ``not_run`` (nicht ausgefuehrt, always with a reason; it never
    counts as passed). The file is rewritten after every record, so a job that
    hits its time limit still leaves every finished result behind.
``Cargo``
    Development profile only (D3 Bauregel): the library tests of
    ``smart_explorer`` once per job, ``se`` and ``smart_explorer`` on demand, the
    share server only for stages with a device pair. No ``--workspace``,
    ``--all-targets``, ``--release``, installer or feed build. One exception:
    the throughput stages build ``speed_task`` and their ``se`` in the profile
    ``speed`` (``SPEED_PROFILE``, release optimization with debug assertions),
    since speed measured on opt-level 1 is not the speed that ships.
``StageFixtures``
    Runner-side fixtures that expected entries request with ``fixture=``. Each
    has a fixed environment contract (see ``FIXTURE_CONTRACT``); a fixture the
    runner cannot provide turns the entries that need it into ``not_run`` with
    the exact reason.
``JobContext.run_default``
    The generic stage: library tests by exact full path (``--exact``), harness
    modules, suite checks (``CHECKS`` registers of the rdt_task modules), JUnit
    results of the Android build and instrumentation classes on the emulator.
``inventory_job`` / ``inventory_sources``
    ``rdt_suite_inventory_complete``.

Dock points for later packages (U41: ``rdt_task/screen.py``, ``desktop.py``,
``launcher.py``): a module may define ``STAGES`` (stage name -> ``fn(ctx,
stage)``), ``CHECKS`` (check name -> ``fn(ctx) -> (status, reason)``),
``FIXTURES`` (fixture name -> ``fn(fixtures, name) -> (ok, reason)``) and
``DEVICE_SETUP`` (``fn(ctx, device)``, called once the emulator is ready).
"""
from __future__ import annotations

import collections
import hashlib
import json
import os
import platform
import re
import secrets
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import traceback
import xml.etree.ElementTree as ElementTree
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable

TASK_DIR = Path(__file__).resolve().parent
NATIVE = TASK_DIR.parent
REPO = NATIVE.parent
EXPECTED_FILE = TASK_DIR / "expected_signals.txt"
WINDOWS = os.name == "nt"
LINUX = sys.platform.startswith("linux")

PASSED, FAILED, NOT_RUN = "passed", "failed", "not_run"
STATUSES = (PASSED, FAILED, NOT_RUN)
WORDS = {PASSED: "bestanden", FAILED: "gescheitert", NOT_RUN: "nicht ausgeführt"}
BATCH_PREFIXES = ("perm_task_", "drive_task_", "vault_task_", "screen_task_", "desktop_task_",
                  "launcher_task_", "licenses_task_", "rdt_suite_", "speed_task_")
INVENTORY = "rdt_suite_inventory_complete"
KINDS = ("lib", "modul", "check", "skript", "junit", "instr")
SECTION_DONE, SECTION_PENDING = "[umgesetzt]", "[noch nicht umgesetzt]"
LOCK_FILE = "native/Cargo.lock"
LICENSE_FILE = "third-party/rust-crates/THIRD-PARTY-LICENSES.txt"
PREPARE_MANIFEST = "rdt-prepare.json"
EMULATOR_HOST = "10.0.2.2"  # the emulator's alias of the runner's loopback interface
FUSE_REQUEST_TIMEOUT_S = 15
LIB_TIMEOUT_S = 90 * 60
BUILD_TIMEOUT_S = 150 * 60
# Cargo profile of the throughput stages (``[profile.speed]`` in native/Cargo.toml): release optimization with
# debug assertions. Speed is measured on the optimization that ships, not on the development profile (D3 exception).
SPEED_PROFILE = "speed"
MAX_ARGS_CHARS = 12000  # far below the 32 767 characters of a Windows command line
SFTP_IMAGE = "lscr.io/linuxserver/openssh-server:latest"
ANDROID_TEST_PACKAGE_ROOTS = ("android/app/src/androidTest",)
ANDROID_UNIT_PACKAGE_ROOTS = ("android/app/src/test",)
APP_PACKAGE = "app.smartexplorer.android"
# The test servers (SFTP, SSH, FTP, SMB as Docker containers) are defined only here; android/test-android-task.sh
# sources the same file, so the suite and the existing Android script run the same servers.
ANDROID_SERVERS_SCRIPT = "android/test-servers/servers.sh"
# Settings of the app's first start. android/test-android-task.sh (emulator-run) writes them into the app's data before
# the first launch: the onboarding counts as done and no automatic update check runs.
APP_PREFS_FILE = "shared_prefs/app_prefs.xml"
APP_PREFS_XML = """<?xml version='1.0' encoding='utf-8' standalone='yes' ?>
<map>
    <boolean name="onboarding_done" value="true" />
    <boolean name="auto_update_check" value="false" />
</map>
"""

# The environment each built-in fixture provides (documented contract for test authors).
FIXTURE_CONTRACT = {
    "share": "SE_TASK_SHARE_SIGNAL=127.0.0.1:<port>, SE_TASK_SHARE_RELAY=http://127.0.0.1:<port+1>, "
             "SE_TASK_SHARE_SERVER_BIN; instrumentation: rdt_share_signal, rdt_share_relay (10.0.2.2)",
    "sftp": "SE_TASK_SFTP_HOST, SE_TASK_SFTP_PORT, SE_TASK_SFTP_USER, SE_TASK_SFTP_PASSWORD, SE_TASK_SFTP_ROOT, "
            "SE_TASK_SFTP_KNOWN_HOSTS; instrumentation: rdt_sftp_host (10.0.2.2), rdt_sftp_port, rdt_sftp_user, "
            "rdt_sftp_password, rdt_sftp_root",
    "android-servers": "SFTP (atmoz/sftp), SSH with exec (linuxserver/openssh-server), FTP and SMB servers as Docker "
                       "containers, started by servers_up and stopped by servers_down of android/test-servers/servers.sh "
                       "(images, ports and users are defined only there); instrumentation: exactly the arguments of "
                       "servers_instrumentation_args (seServerHost=10.0.2.2, seSftp*, seSsh*, seFtp*, seSmb*); a server "
                       "that does not start makes the entry not run, with the output of servers_up as the reason",
    "android-servers-optional": "android-servers for entries that read no server argument (UiTaskTest, IntentsTaskTest): "
                                "the entry gets the same arguments, and runs without them when a server does not start "
                                "(the reason is logged)",
    "bus": "DBUS_SYSTEM_BUS_ADDRESS of a private system bus anyone may own names on, SE_TASK_PRIVATE_SYSTEM_BUS=1",
    "xvfb": "DISPLAY of a fresh Xvfb (1920x1080x24), WAYLAND_DISPLAY removed",
    "nodisplay": "DISPLAY and WAYLAND_DISPLAY removed",
    "home": "HOME, XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME, XDG_STATE_HOME (Windows also USERPROFILE, "
            "APPDATA, LOCALAPPDATA) in a fresh folder; CARGO_HOME and RUSTUP_HOME stay pinned",
    "runtime-tmpfs": "XDG_RUNTIME_DIR on a fresh tmpfs (mode 0700, owned by the runner user)",
    "fuse": "requires fusermount3 and /dev/fuse (Linux)",
    "fuse-timeout": f"fs.fuse.default_request_timeout={FUSE_REQUEST_TIMEOUT_S} via sudo, "
                    "SE_TASK_FUSE_REQUEST_TIMEOUT_S",
    "dokany": "requires an installed Dokany driver (Windows); installs only from SE_TASK_DOKANY_MSI with a "
              "matching SE_TASK_DOKANY_MSI_SHA256; starts the dokan2 service",
    "own-suite": "never ready: the entry needs the environment of its own task script (named in the reason)",
    "sudo": "requires sudo -n and python3 (Linux)",
}


# --------------------------------------------------------------------------- small helpers

def is_batch_name(name: str) -> bool:
    """A test name with one of the batch prefixes (D3). ``mount_vault_task_*`` is not one (W14)."""
    return name.startswith(BATCH_PREFIXES)


def leaf(path: str) -> str:
    return path.rsplit("::", 1)[-1]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def excerpt(text: str, limit: int = 3000) -> str:
    text = text or ""
    return text if len(text) <= limit else "…\n" + text[-limit:]


def first_line(text: str) -> str:
    for line in (text or "").splitlines():
        if line.strip():
            return line.strip()
    return ""


def json_lines(text: str) -> Iterable[dict]:
    for line in (text or "").splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if isinstance(value, dict):
                yield value


def cargo_home() -> Path:
    return Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")


def rustup_home() -> Path:
    return Path(os.environ.get("RUSTUP_HOME") or Path.home() / ".rustup")


# Resolved once, before any fixture changes HOME.
CARGO_HOME = cargo_home()
RUSTUP_HOME = rustup_home()


def free_port_pair() -> int:
    """A loopback port whose successor is free as well (share server: signaling and relay port)."""
    for _ in range(200):
        with socket.socket() as first:
            first.bind(("127.0.0.1", 0))
            port = first.getsockname()[1]
        if port >= 65000:
            continue
        try:
            with socket.socket() as second:
                second.bind(("127.0.0.1", port + 1))
        except OSError:
            continue
        return port
    raise RuntimeError("no free pair of loopback ports")


def wait_tcp(port: int, timeout: float, process: subprocess.Popen | None = None) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process is not None and process.poll() is not None:
            return False
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return True
        except OSError:
            time.sleep(0.3)
    return False


def wait_path(path: Path, timeout: float, process: subprocess.Popen | None = None) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return True
        if process is not None and process.poll() is not None:
            return False
        time.sleep(0.2)
    return path.exists()


# --------------------------------------------------------------------------- processes

@dataclass
class Completed:
    code: int | None
    output: str
    timed_out: bool
    duration_s: float

    @property
    def ok(self) -> bool:
        return self.code == 0 and not self.timed_out

    def describe(self) -> str:
        return "Zeitlimit überschritten" if self.timed_out else f"Exitcode {self.code}"


def kill_tree(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    if WINDOWS:
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(process.pid)], capture_output=True, check=False)
        return
    for signum in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(process.pid, signum)
        except ProcessLookupError:
            return
        except PermissionError:
            # A child under sudo runs as root; sudo itself relays signals, `kill` as root ends the rest.
            subprocess.run(["sudo", "-n", "kill", f"-{signum.name[3:]}", "--", f"-{process.pid}"],
                           capture_output=True, check=False)
        try:
            process.wait(timeout=15)
            return
        except subprocess.TimeoutExpired:
            continue


def run(cmd: list, *, cwd: Path | None = None, env: dict | None = None, log: Path | None = None,
        timeout: float | None = None, input_text: str | None = None,
        on_line: Callable[[str, float], None] | None = None, redact: Iterable[str] = ()) -> Completed:
    """Runs ``cmd`` with merged stdout/stderr, appends everything to ``log`` and kills the
    whole process tree on timeout. Never raises for a failing command."""
    cmd = [str(part) for part in cmd]
    shown = " ".join(cmd)
    for secret in redact:
        if secret:
            shown = shown.replace(secret, "***")
    handle = None
    if log is not None:
        log.parent.mkdir(parents=True, exist_ok=True)
        handle = open(log, "a", encoding="utf-8", errors="replace")
        handle.write(f"\n$ {shown}\n")
        handle.flush()
    kwargs: dict = {"cwd": str(cwd) if cwd else None, "env": env, "stdout": subprocess.PIPE,
                    "stderr": subprocess.STDOUT,
                    "stdin": subprocess.PIPE if input_text is not None else subprocess.DEVNULL}
    if WINDOWS:
        kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        kwargs["start_new_session"] = True
    started = time.monotonic()
    try:
        process = subprocess.Popen(cmd, **kwargs)
    except OSError as error:
        message = f"{type(error).__name__}: {error}"
        if handle:
            handle.write(message + "\n")
            handle.close()
        return Completed(None, message, False, 0.0)
    lines: list[str] = []
    closed = threading.Event()

    def reader() -> None:
        assert process.stdout is not None
        for raw in iter(process.stdout.readline, b""):
            if closed.is_set():
                # A background child (for example a daemon a test started) still holds the pipe.
                continue
            line = raw.decode("utf-8", "replace").rstrip("\r\n")
            for secret in redact:
                if secret:
                    line = line.replace(secret, "***")
            lines.append(line)
            if handle:
                try:
                    handle.write(line + "\n")
                except ValueError:  # the log was closed in the meantime
                    pass
            if on_line:
                on_line(line, time.monotonic())

    thread = threading.Thread(target=reader, daemon=True)
    thread.start()
    if input_text is not None and process.stdin is not None:
        try:
            process.stdin.write(input_text.encode("utf-8"))
            process.stdin.close()
        except OSError:
            pass
    timed_out = False
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        kill_tree(process)
        try:
            process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            pass
    thread.join(timeout=60)
    closed.set()
    duration = time.monotonic() - started
    if handle:
        handle.write(f"[exit {process.returncode}{' timeout' if timed_out else ''} after {duration:.1f}s]\n")
        handle.close()
    return Completed(None if timed_out else process.returncode, "\n".join(lines), timed_out, duration)


# --------------------------------------------------------------------------- expected signals

@dataclass
class Entry:
    name: str
    locations: tuple
    pending: bool
    line: int
    kind: str = "lib"
    ignored: bool = False
    threads: int | None = None
    env: tuple = ()
    root: bool = False
    path: str | None = None
    module: str | None = None
    fixtures: tuple = ()
    script: str | None = None
    package: str | None = None
    source: str | None = None

    def stage_in(self, job: str) -> str | None:
        for location_job, stage in self.locations:
            if location_job == job:
                return stage
        return None

    @property
    def is_class(self) -> bool:
        return self.name[:1].isupper()

    def pending_reason(self) -> str:
        return (f"nicht umgesetzt: der Test fehlt im Code (Paket {self.package}; "
                f"Abschnitt '{SECTION_PENDING}' in rdt_task/expected_signals.txt, Zeile {self.line})")


NAME_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def parse_entry(text: str, number: int, pending: bool) -> tuple[Entry | None, str | None]:
    fields = text.split()
    if len(fields) < 2:
        return None, f"Zeile {number}: Name und Ort fehlen"
    name, where, options = fields[0], fields[1], fields[2:]
    if not NAME_RE.fullmatch(name):
        return None, f"Zeile {number}: ungültiger Name {name!r}"
    locations = []
    for part in where.split(","):
        job, separator, stage = part.partition(":")
        if not separator or not job or not stage:
            return None, f"Zeile {number}: ungültiger Ort {part!r} (erwartet <job>:<stufe>)"
        locations.append((job, stage))
    entry = Entry(name=name, locations=tuple(locations), pending=pending, line=number)
    env, fixtures = [], []
    for option in options:
        if option == "ignored":
            entry.ignored = True
            continue
        if option == "root":
            entry.root = True
            continue
        key, separator, value = option.partition("=")
        if not separator or not value:
            return None, f"Zeile {number}: Option {option!r} ohne Wert"
        if key == "art":
            if value not in KINDS:
                return None, f"Zeile {number}: unbekannte Art {value!r}"
            entry.kind = value
        elif key == "threads":
            if not value.isdigit() or int(value) < 1:
                return None, f"Zeile {number}: threads={value!r} ist keine positive Zahl"
            entry.threads = int(value)
        elif key == "env":
            variable, separator2, content = value.partition("=")
            if not separator2 or not NAME_RE.fullmatch(variable):
                return None, f"Zeile {number}: env={value!r} (erwartet env=NAME=WERT)"
            env.append((variable, content))
        elif key == "pfad":
            entry.path = value
        elif key == "modul":
            entry.module = value
        elif key == "fixture":
            fixtures.extend(item for item in value.split(",") if item)
        elif key == "skript":
            entry.script = value
        elif key == "paket":
            entry.package = value
        elif key == "quelle":
            entry.source = value
        else:
            return None, f"Zeile {number}: unbekannte Option {key!r}"
    entry.env = tuple(env)
    entry.fixtures = tuple(dict.fromkeys(fixtures))
    if entry.kind == "modul" and not entry.module:
        return None, f"Zeile {number}: art=modul ohne modul="
    if entry.kind == "skript" and not entry.script:
        return None, f"Zeile {number}: art=skript ohne skript="
    if pending and not entry.package:
        return None, f"Zeile {number}: Eintrag in '{SECTION_PENDING}' ohne paket="
    if entry.path and leaf(entry.path) != name:
        return None, f"Zeile {number}: pfad={entry.path} endet nicht auf {name}"
    return entry, None


class Signals:
    def __init__(self, entries: list[Entry], errors: list[str]):
        self.entries = entries
        self.errors = errors

    @classmethod
    def load(cls, jobs: dict[str, list[str]], path: Path = EXPECTED_FILE) -> "Signals":
        if not path.is_file():
            return cls([], [f"{path} fehlt"])
        entries: list[Entry] = []
        errors: list[str] = []
        section = None
        for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            text = raw.strip()
            if not text or text.startswith("#"):
                continue
            if text in (SECTION_DONE, SECTION_PENDING):
                section = text
                continue
            if section is None:
                errors.append(f"Zeile {number}: Eintrag vor dem ersten Abschnitt")
                continue
            entry, problem = parse_entry(text, number, section == SECTION_PENDING)
            if problem:
                errors.append(problem)
            elif entry is not None:
                entries.append(entry)
        seen: dict[tuple[str, str], int] = {}
        for entry in entries:
            for job, stage in entry.locations:
                if job not in jobs:
                    errors.append(f"Zeile {entry.line}: unbekannter Job {job!r}")
                elif stage not in jobs[job]:
                    errors.append(f"Zeile {entry.line}: Stufe {stage!r} gibt es im Job {job} nicht")
                key = (entry.name, job)
                if key in seen:
                    errors.append(f"Zeile {entry.line}: {entry.name} steht im Job {job} mehr als einmal "
                                  f"(auch Zeile {seen[key]}); erwartet ist genau ein Ort je Job")
                else:
                    seen[key] = entry.line
        return cls(entries, errors)

    def for_job(self, job: str) -> list[tuple[Entry, str]]:
        return [(entry, stage) for entry in self.entries for location_job, stage in entry.locations
                if location_job == job]

    def for_stage(self, job: str, stage: str) -> list[Entry]:
        return [entry for entry in self.entries
                if any(location == (job, stage) for location in entry.locations)]

    def names(self) -> set[str]:
        return {entry.name for entry in self.entries}

    def by_name(self, name: str) -> list[Entry]:
        return [entry for entry in self.entries if entry.name == name]


# --------------------------------------------------------------------------- report

def md(text: str, limit: int = 240) -> str:
    text = " ".join((text or "").split())
    if len(text) > limit:
        text = text[: limit - 1] + "…"
    return text.replace("|", "\\|")


class Report:
    def __init__(self, job: str, candidate_sha: str, log_root: Path, resume: bool = False):
        self.job = job
        self.candidate_sha = candidate_sha
        self.path = log_root / "report.json"
        self.summary_path = log_root / "summary.md"
        self.tests: list[dict] = []
        self.stages: list[dict] = []
        if resume and self.path.is_file():
            try:
                data = json.loads(self.path.read_text(encoding="utf-8"))
            except ValueError:
                data = {}
            if data.get("job") == job:
                self.tests = list(data.get("tests", []))
                self.stages = list(data.get("stages", []))

    def record(self, name: str, stage: str, status: str, reason: str = "", duration_s: float = 0.0) -> None:
        if status not in STATUSES:
            raise ValueError(f"unknown status {status!r}")
        reason = str(reason or "")
        if len(reason) > 4000:
            reason = reason[:1500] + "\n…\n" + reason[-2400:]
        self.tests.append({"name": name, "stage": stage, "status": status, "reason": reason,
                           "duration_s": round(max(float(duration_s or 0.0), 0.0), 3)})
        line = first_line(reason)
        print(f"[{WORDS[status]}] {stage} · {name}" + (f": {line[:300]}" if line else ""), flush=True)
        self.write()

    def recorded(self, name: str) -> int:
        return sum(1 for test in self.tests if test["name"] == name)

    def stage_done(self, name: str, status: str, reason: str, duration_s: float) -> None:
        self.stages = [stage for stage in self.stages if stage["name"] != name]
        self.stages.append({"name": name, "status": status, "reason": reason,
                            "duration_s": round(duration_s, 3)})
        print(f"== Stufe {name}: {WORDS[status]}" + (f" ({first_line(reason)[:300]})" if reason else ""),
              flush=True)
        self.write()

    def counts(self, stage: str | None = None) -> collections.Counter:
        return collections.Counter(test["status"] for test in self.tests
                                   if stage is None or test["stage"] == stage)

    def failed(self) -> bool:
        return any(test["status"] == FAILED for test in self.tests) or any(
            stage["status"] == FAILED for stage in self.stages)

    def write(self) -> None:
        data = {"job": self.job, "candidate_sha": self.candidate_sha, "tests": self.tests,
                "stages": self.stages}
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self.path.with_name(self.path.name + ".tmp")
        temporary.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        os.replace(temporary, self.path)

    def summary(self) -> str:
        counts = self.counts()
        lines = [f"## rechte-desktop-tresore-task · {self.job}", "",
                 f"Kandidat `{self.candidate_sha}` · {counts[PASSED]} bestanden · {counts[FAILED]} gescheitert · "
                 f"{counts[NOT_RUN]} nicht ausgeführt", "",
                 "| Stufe | Status | bestanden | gescheitert | nicht ausgeführt | Grund |",
                 "|---|---|---:|---:|---:|---|"]
        for stage in self.stages:
            stage_counts = self.counts(stage["name"])
            lines.append(f"| {stage['name']} | {WORDS[stage['status']]} | {stage_counts[PASSED]} | "
                         f"{stage_counts[FAILED]} | {stage_counts[NOT_RUN]} | {md(stage['reason'])} |")
        failed = [test for test in self.tests if test["status"] == FAILED]
        if failed:
            lines += ["", "### Gescheitert", ""]
            lines += [f"- `{test['name']}` ({test['stage']}): {md(first_line(test['reason']), 400)}"
                      for test in failed]
        not_run = [test for test in self.tests if test["status"] == NOT_RUN]
        if not_run:
            lines += ["", "### Nicht ausgeführt (zählt nie als bestanden)", ""]
            lines += [f"- `{test['name']}` ({test['stage']}): {md(test['reason'], 400)}" for test in not_run]
        return "\n".join(lines) + "\n"

    def publish_summary(self) -> None:
        text = self.summary()
        self.summary_path.write_text(text, encoding="utf-8")
        target = os.environ.get("GITHUB_STEP_SUMMARY")
        if target:
            with open(target, "a", encoding="utf-8") as handle:
                handle.write(text + "\n")


# --------------------------------------------------------------------------- libtest

RESULT_LINE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored(?:, (.*))?)\s*$")
OUTCOME = re.compile(r"^(ok|FAILED|ignored(?:, (.*))?)$")
START_TOKEN = re.compile(r"(?:^|\s)test (\S+) \.\.\. ")
END_TOKEN = re.compile(r"(?:^|\s)(ok|FAILED)\s*$")
FAILURE_BLOCK = re.compile(r"^---- (\S+) stdout ----\n(.*?)(?=^---- \S+ stdout ----$|^failures:$|\Z)",
                           re.S | re.M)
LIST_LINE = re.compile(r"^(\S+): test\s*$", re.M)
# A test that ends itself as "not run" panics with this line (the `not_run` helpers of the Rust tests).
NOT_RUN_SAID = re.compile(r"^nicht ausgeführt: (.+)$", re.M)


class LibtestParser:
    """Per-test results from libtest's pretty output. A duration is the time between two result
    lines (exact with one test thread, an approximation with several)."""

    def __init__(self, started: float):
        self.results: dict[str, list] = {}
        self.last = started
        self.pending: str | None = None

    def feed(self, line: str, at: float) -> None:
        match = RESULT_LINE.match(line)
        if match:
            self._done(match.group(1), match.group(2), match.group(3), at)
            self.pending = None
            return
        start = START_TOKEN.search(line)
        if start:
            outcome = OUTCOME.match(line[start.end():].strip())
            if outcome:
                self._done(start.group(1), outcome.group(1).split(",", 1)[0], outcome.group(2), at)
                self.pending = None
            else:
                self.pending = start.group(1)
            return
        if self.pending:
            end = END_TOKEN.search(line)
            if end:
                self._done(self.pending, end.group(1), None, at)
                self.pending = None

    def _done(self, path: str, outcome: str, ignore_reason: str | None, at: float) -> None:
        duration = at - self.last
        self.last = at
        if outcome == "ok":
            self.results[path] = [PASSED, "", duration]
        elif outcome == "FAILED":
            self.results[path] = [FAILED, "", duration]
        else:
            self.results[path] = [NOT_RUN, "als #[ignore] markiert" + (f" ({ignore_reason})" if ignore_reason else "")
                                  + "; ohne die Option 'ignored' in expected_signals.txt nicht ausgeführt", duration]


def chunks(paths: list[str], limit: int = MAX_ARGS_CHARS) -> list[list[str]]:
    result, current, size = [], [], 0
    for path in paths:
        if current and size + len(path) + 1 > limit:
            result.append(current)
            current, size = [], 0
        current.append(path)
        size += len(path) + 1
    if current:
        result.append(current)
    return result


# --------------------------------------------------------------------------- cargo

class StageError(RuntimeError):
    """A precondition of a stage group failed; the message becomes the report reason."""


class Cargo:
    """Development-profile builds of exactly what a stage needs (D3 Bauregel)."""

    def __init__(self, ctx: "JobContext"):
        self.ctx = ctx
        self.lib_ok: bool | None = None
        self.lib_error = ""
        self.test_exe: Path | None = None
        self.paths: list[str] | None = None
        self.ignored: set[str] = set()
        self.by_leaf: dict[str, list[str]] = {}
        self.bins: dict[str, Path | None] = {}
        self.bin_errors: dict[str, str] = {}
        self.server: Path | None = None
        self.server_error: str | None = None
        self._server_tried = False

    @staticmethod
    def manifest() -> Path:
        return NATIVE / "Cargo.toml"

    def build_lib(self) -> bool:
        """``cargo test --lib --no-run`` once per job, then the test list of the binary."""
        if self.lib_ok is not None:
            return self.lib_ok
        started = time.monotonic()
        completed = self.ctx.run(["cargo", "test", "--locked", "--lib", "--no-run", "--manifest-path",
                                  self.manifest(), "--message-format=json-render-diagnostics"],
                                 stage="build", cwd=REPO, env=self.ctx.base_env(), timeout=BUILD_TIMEOUT_S)
        executable = None
        for message in json_lines(completed.output):
            target = message.get("target") or {}
            if (message.get("reason") == "compiler-artifact" and (message.get("profile") or {}).get("test")
                    and target.get("name") == "smart_explorer" and "bin" not in (target.get("kind") or [])
                    and message.get("executable")):
                executable = Path(message["executable"])
        if completed.ok and executable and executable.is_file():
            listing = self.ctx.run([executable, "--list"], stage="build", cwd=NATIVE,
                                   env=self.ctx.test_env(), timeout=600)
            ignored = self.ctx.run([executable, "--list", "--ignored"], stage="build", cwd=NATIVE,
                                   env=self.ctx.test_env(), timeout=600)
            if listing.ok and ignored.ok:
                self.test_exe = executable
                self.paths = LIST_LINE.findall(listing.output)
                self.ignored = set(LIST_LINE.findall(ignored.output))
                self.by_leaf = collections.defaultdict(list)
                for path in self.paths:
                    self.by_leaf[leaf(path)].append(path)
                self.lib_ok = True
            else:
                self.lib_ok = False
                self.lib_error = "Testliste des Testprogramms nicht lesbar: " + excerpt(listing.output, 1500)
        else:
            self.lib_ok = False
            self.lib_error = f"cargo test --lib --no-run: {completed.describe()}\n" + excerpt(
                "\n".join(line for line in completed.output.splitlines() if not line.startswith("{")), 2500)
        reason = (f"{len(self.paths or [])} Tests im Testprogramm {self.test_exe}" if self.lib_ok
                  else self.lib_error)
        self.ctx.report.stage_done("build", PASSED if self.lib_ok else FAILED, reason,
                                   time.monotonic() - started)
        return self.lib_ok

    def bin_path(self, name: str, profile: str | None = None) -> Path | None:
        """``cargo build --bin <name>`` on first use: the development profile, or ``profile`` (the speed stages).
        Errors are kept under the same key: ``name`` or ``name@profile``."""
        key = name if profile is None else f"{name}@{profile}"
        if key in self.bins:
            return self.bins[key]
        chosen = [] if profile is None else ["--profile", profile]
        completed = self.ctx.run(["cargo", "build", "--locked", *chosen, "--bin", name, "--manifest-path",
                                  self.manifest(), "--message-format=json-render-diagnostics"],
                                 stage="build-bins", cwd=REPO, env=self.ctx.base_env(),
                                 timeout=BUILD_TIMEOUT_S)
        path = None
        for message in json_lines(completed.output):
            target = message.get("target") or {}
            if (message.get("reason") == "compiler-artifact" and "bin" in (target.get("kind") or [])
                    and target.get("name") == name and message.get("executable")):
                path = Path(message["executable"])
        if completed.ok and path and path.is_file():
            self.bins[key] = path.resolve()
        else:
            self.bins[key] = None
            self.bin_errors[key] = f"cargo build {' '.join(chosen)} --bin {name}: {completed.describe()}".replace("  ", " ")
        return self.bins[key]

    def share_server(self) -> Path | None:
        """The candidate share server: from the desktop-bins artifact when present, else built
        in the development profile (only stages with a device pair ask for it)."""
        if self._server_tried:
            return self.server
        self._server_tried = True
        handed = self.ctx.state.get("desktop_bins_dir")
        if handed:
            for candidate in (Path(handed) / "se-share-server", Path(handed) / "se-share-server.exe"):
                if candidate.is_file():
                    self.server = candidate
                    return self.server
            # Device jobs never build Rust (D3 Bauregel): the binary comes from desktop-bins only.
            self.server_error = "se-share-server fehlt im Artefakt rdt-desktop-bins (Job desktop-bins)"
            return None
        manifest = REPO / "share-server" / "Cargo.toml"
        if not manifest.is_file():
            self.server_error = "share-server/Cargo.toml fehlt"
            return None
        if not shutil.which("cargo"):
            self.server_error = "cargo fehlt in diesem Job"
            return None
        completed = self.ctx.run(["cargo", "build", "--locked", "--manifest-path", manifest,
                                  "--message-format=json-render-diagnostics"], stage="build-share-server",
                                 cwd=REPO, env=self.ctx.base_env(), timeout=BUILD_TIMEOUT_S)
        executables = [Path(message["executable"]) for message in json_lines(completed.output)
                       if message.get("reason") == "compiler-artifact" and message.get("executable")
                       and "bin" in ((message.get("target") or {}).get("kind") or [])]
        executables.sort(key=lambda path: (0 if "share" in path.name else 1, path.name))
        if completed.ok and executables:
            self.server = executables[0].resolve()
        else:
            self.server_error = f"cargo build share-server: {completed.describe()}"
        return self.server


# --------------------------------------------------------------------------- fixtures

BUS_CONFIG = """<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path={socket}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <allow own="*"/>
    <allow send_destination="*" eavesdrop="true"/>
    <allow receive_sender="*"/>
    <allow eavesdrop="true"/>
  </policy>
</busconfig>
"""


class StageFixtures:
    """Fixtures of one stage, created on first use and closed when the stage ends."""

    def __init__(self, ctx: "JobContext", stage: str):
        self.ctx = ctx
        self.stage = stage
        self.root = ctx.work / "fixtures" / stage
        self.state: dict[str, tuple[bool, str]] = {}
        self.envs: dict[str, dict] = {}
        self.removed: dict[str, set] = {}
        self.args: dict[str, list] = {}
        self.cleanups: list[Callable[[], object]] = []
        self.processes: list[subprocess.Popen] = []
        self._counter = 0

    def make_dir(self, name: str) -> Path:
        self._counter += 1
        path = self.root / f"{name}-{self._counter}"
        path.mkdir(parents=True, exist_ok=True)
        return path

    def provide(self, name: str, env: dict | None = None, remove: Iterable[str] = (),
                args: Iterable[tuple[str, str]] = ()) -> None:
        self.envs[name] = {key: str(value) for key, value in (env or {}).items()}
        self.removed[name] = set(remove)
        self.args[name] = list(args)

    def add_cleanup(self, action: Callable[[], object]) -> None:
        self.cleanups.append(action)

    def add_process(self, process: subprocess.Popen) -> None:
        self.processes.append(process)

    def ensure(self, names: Iterable[str]) -> tuple[bool, str]:
        reasons = []
        for name in names:
            if name not in self.state:
                provider = self.ctx.fixture_provider(name)
                if provider is None:
                    self.state[name] = (False, "unbekannte Fixture (weder in common.py noch in einem FIXTURES-Register)")
                else:
                    try:
                        self.state[name] = provider(self, name)
                    except Exception as error:  # noqa: BLE001 - a broken fixture is reported, not raised
                        self.state[name] = (False, f"{type(error).__name__}: {error}")
                status = "bereit" if self.state[name][0] else "nicht verfügbar"
                self.ctx.log_line(self.stage, f"fixture {name}: {status}: {self.state[name][1]}")
            ok, reason = self.state[name]
            if not ok:
                reasons.append(f"Fixture {name}: {reason}")
        return not reasons, "; ".join(reasons)

    def env_for(self, names: Iterable[str]) -> tuple[dict, set]:
        env, removed = {}, set()
        for name in names:
            env.update(self.envs.get(name, {}))
            removed |= self.removed.get(name, set())
        return env, removed

    def args_for(self, names: Iterable[str]) -> list[tuple[str, str]]:
        result = []
        for name in names:
            result.extend(self.args.get(name, []))
        return result

    def close(self) -> None:
        for process in reversed(self.processes):
            kill_tree(process)
        for action in reversed(self.cleanups):
            try:
                action()
            except Exception as error:  # noqa: BLE001
                self.ctx.log_line(self.stage, f"fixture cleanup failed: {error}")
        self.processes.clear()
        self.cleanups.clear()


def fixture_nodisplay(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    fixtures.provide(name, remove=("DISPLAY", "WAYLAND_DISPLAY"))
    return True, "DISPLAY und WAYLAND_DISPLAY entfernt"


def fixture_home(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    root = fixtures.make_dir("home")
    env = {"HOME": root / "home", "XDG_CONFIG_HOME": root / "config", "XDG_DATA_HOME": root / "data",
           "XDG_CACHE_HOME": root / "cache", "XDG_STATE_HOME": root / "state"}
    if WINDOWS:
        env.update(USERPROFILE=root / "home", APPDATA=root / "appdata", LOCALAPPDATA=root / "localappdata")
    for path in env.values():
        Path(path).mkdir(parents=True, exist_ok=True)
    fixtures.provide(name, env=env)
    return True, f"HOME={root / 'home'}"


def fixture_sudo(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "nur unter Linux"
    if not fixtures.ctx.sudo_ok():
        return False, "sudo -n ist nicht ohne Passwort verfügbar"
    if not shutil.which("python3"):
        return False, "python3 fehlt"
    fixtures.provide(name)
    return True, "sudo -n und python3 vorhanden"


def fixture_fuse(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "FUSE nur unter Linux"
    fusermount = shutil.which("fusermount3")
    if not fusermount:
        return False, "fusermount3 fehlt (Paket fuse3 ist nicht installiert)"
    if not os.access("/dev/fuse", os.R_OK | os.W_OK):
        return False, "/dev/fuse ist für den Runner-Nutzer nicht les- und schreibbar"
    fixtures.provide(name)
    return True, f"fusermount3={fusermount}, /dev/fuse"


def fixture_fuse_timeout(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    knob = Path("/proc/sys/fs/fuse/default_request_timeout")
    if not LINUX or not knob.exists():
        return False, (f"Kernel {platform.release()} ohne fs.fuse.default_request_timeout "
                       "(den Schalter gibt es erst ab Linux 6.14)")
    old = knob.read_text(encoding="utf-8").strip()
    setting = f"fs.fuse.default_request_timeout={FUSE_REQUEST_TIMEOUT_S}"
    if not fixtures.ctx.run(["sudo", "-n", "sysctl", "-w", setting], stage=fixtures.stage, timeout=60).ok:
        return False, "sysctl über sudo -n gescheitert"
    fixtures.add_cleanup(lambda: fixtures.ctx.run(["sudo", "-n", "sysctl", "-w",
                                                   f"fs.fuse.default_request_timeout={old}"],
                                                  stage=fixtures.stage, timeout=60))
    fixtures.provide(name, env={"SE_TASK_FUSE_REQUEST_TIMEOUT_S": FUSE_REQUEST_TIMEOUT_S})
    return True, f"{setting} (vorher {old})"


def fixture_runtime_tmpfs(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "nur unter Linux"
    target = fixtures.make_dir("runtime")
    mounted = fixtures.ctx.run(["sudo", "-n", "mount", "-t", "tmpfs", "-o",
                                f"size=256m,mode=0700,uid={os.getuid()},gid={os.getgid()}", "tmpfs", target],
                               stage=fixtures.stage, timeout=60)
    if not mounted.ok:
        return False, f"tmpfs über sudo -n nicht einhängbar ({mounted.describe()})"
    fixtures.add_cleanup(lambda: fixtures.ctx.run(["sudo", "-n", "umount", "-l", target],
                                                  stage=fixtures.stage, timeout=60))
    fixtures.provide(name, env={"XDG_RUNTIME_DIR": target})
    return True, f"XDG_RUNTIME_DIR={target} (tmpfs)"


def fixture_xvfb(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "Xvfb nur unter Linux"
    binary = shutil.which("Xvfb")
    if not binary:
        return False, "Xvfb fehlt (Paket xvfb)"
    number = next((candidate for candidate in range(90, 200)
                   if not Path(f"/tmp/.X11-unix/X{candidate}").exists()
                   and not Path(f"/tmp/.X{candidate}-lock").exists()), None)
    if number is None:
        return False, "keine freie Anzeigenummer"
    log = open(fixtures.make_dir("xvfb") / "xvfb.log", "ab")
    process = subprocess.Popen([binary, f":{number}", "-screen", "0", "1920x1080x24", "-nolisten", "tcp"],
                               stdout=log, stderr=subprocess.STDOUT)
    fixtures.add_process(process)
    if not wait_path(Path(f"/tmp/.X11-unix/X{number}"), 30, process):
        return False, f"Xvfb :{number} startete nicht (Exitcode {process.poll()})"
    fixtures.provide(name, env={"DISPLAY": f":{number}"}, remove=("WAYLAND_DISPLAY",))
    return True, f"DISPLAY=:{number}"


def fixture_bus(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "nur unter Linux"
    daemon = shutil.which("dbus-daemon")
    if not daemon:
        return False, "dbus-daemon fehlt"
    root = fixtures.make_dir("bus")
    socket_path = root / "system_bus_socket"
    config = root / "system-bus.conf"
    config.write_text(BUS_CONFIG.replace("{socket}", str(socket_path)), encoding="utf-8")
    log = open(root / "dbus-daemon.log", "ab")
    process = subprocess.Popen([daemon, f"--config-file={config}", "--nofork", "--nopidfile"],
                               stdout=log, stderr=subprocess.STDOUT)
    fixtures.add_process(process)
    if not wait_path(socket_path, 20, process):
        return False, f"privater Systembus startete nicht (Exitcode {process.poll()})"
    address = f"unix:path={socket_path}"
    fixtures.provide(name, env={"DBUS_SYSTEM_BUS_ADDRESS": address, "SE_TASK_PRIVATE_SYSTEM_BUS": "1"})
    return True, address


def fixture_share(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    binary = fixtures.ctx.cargo.share_server()
    if binary is None:
        return False, f"Share-Server nicht verfügbar: {fixtures.ctx.cargo.server_error}"
    port = free_port_pair()
    log = open(fixtures.make_dir("share") / "share-server.log", "ab")
    process = subprocess.Popen([str(binary), f"127.0.0.1:{port}"], stdout=log, stderr=subprocess.STDOUT)
    fixtures.add_process(process)
    if not wait_tcp(port, 30, process):
        return False, f"Share-Server lauschte nicht auf 127.0.0.1:{port} (Exitcode {process.poll()})"
    fixtures.provide(name, env={"SE_TASK_SHARE_SIGNAL": f"127.0.0.1:{port}",
                                "SE_TASK_SHARE_RELAY": f"http://127.0.0.1:{port + 1}",
                                "SE_TASK_SHARE_SERVER_BIN": binary},
                     args=[("rdt_share_signal", f"{EMULATOR_HOST}:{port}"),
                           ("rdt_share_relay", f"http://{EMULATOR_HOST}:{port + 1}")])
    return True, f"Kandidaten-Share-Server {binary.name} auf 127.0.0.1:{port}"


def _openssh_port(ctx: "JobContext", stage: str, container: str) -> int:
    mapped = ctx.run(["docker", "port", container, "2222/tcp"], stage=stage, timeout=60)
    port = int(mapped.output.strip().splitlines()[0].rsplit(":", 1)[1])
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
                if connection.recv(4).startswith(b"SSH-"):
                    time.sleep(2)
                    return port
        except OSError:
            pass
        time.sleep(1)
    raise RuntimeError("OpenSSH-Container antwortete nicht")


def _relax_source_penalties(ctx: "JobContext", stage: str, container: str) -> bool:
    """OpenSSH 9.8+ penalises aborted handshakes per source address (PerSourcePenalties) and
    would then drop the runner; the fixture server turns that off when its sshd knows the
    option. The client side under test is unchanged (pattern of native/test-luecken-task.py)."""
    config = 'f=/config/sshd/sshd_config; [ -f "$f" ] || f=/etc/ssh/sshd_config; '
    check = (config + 'printf "\\nPerSourcePenalties no\\n" >> "$f"; '
             'for s in /usr/sbin/sshd.pam /usr/sbin/sshd; do [ -x "$s" ] && exec "$s" -t -f "$f"; done; exit 1')
    if ctx.run(["docker", "exec", container, "sh", "-c", check], stage=stage, timeout=60).ok:
        return True
    ctx.run(["docker", "exec", container, "sh", "-c", config + 'sed -i "/^PerSourcePenalties no$/d" "$f"'],
            stage=stage, timeout=60)
    return False


def fixture_sftp(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    ctx = fixtures.ctx
    if not LINUX or not shutil.which("docker"):
        return False, "Docker fehlt (die SFTP-Fixture läuft nur auf Linux-Runnern)"
    user, password = "rdtsftp", secrets.token_hex(16)
    container = f"rdt-sftp-{os.getpid()}-{secrets.token_hex(3)}"
    started = ctx.run(["docker", "run", "-d", "--name", container, "-p", "127.0.0.1::2222",
                       "-e", f"PUID={os.getuid()}", "-e", f"PGID={os.getgid()}", "-e", "PASSWORD_ACCESS=true",
                       "-e", f"USER_NAME={user}", "-e", f"USER_PASSWORD={password}", SFTP_IMAGE],
                      stage=fixtures.stage, timeout=900, redact=(password,))
    if not started.ok:
        return False, f"docker run {SFTP_IMAGE}: {started.describe()}"
    fixtures.add_cleanup(lambda: ctx.run(["docker", "rm", "-f", container], stage=fixtures.stage, timeout=120))
    port = _openssh_port(ctx, fixtures.stage, container)
    if _relax_source_penalties(ctx, fixtures.stage, container):
        ctx.run(["docker", "restart", container], stage=fixtures.stage, timeout=180)
        port = _openssh_port(ctx, fixtures.stage, container)
    known_hosts = fixtures.make_dir("sftp") / "known_hosts"
    scanned = ctx.run(["ssh-keyscan", "-p", str(port), "127.0.0.1"], stage=fixtures.stage, timeout=60)
    known_hosts.write_text("\n".join(line for line in scanned.output.splitlines()
                                     if line.strip() and not line.startswith("#")) + "\n", encoding="utf-8")
    fixtures.provide(name, env={"SE_TASK_SFTP_HOST": "127.0.0.1", "SE_TASK_SFTP_PORT": port,
                                "SE_TASK_SFTP_USER": user, "SE_TASK_SFTP_PASSWORD": password,
                                "SE_TASK_SFTP_ROOT": "/config", "SE_TASK_SFTP_KNOWN_HOSTS": known_hosts},
                     args=[("rdt_sftp_host", EMULATOR_HOST), ("rdt_sftp_port", str(port)),
                           ("rdt_sftp_user", user), ("rdt_sftp_password", password), ("rdt_sftp_root", "/config")])
    return True, f"OpenSSH-Container {SFTP_IMAGE} auf 127.0.0.1:{port}"


def dokany_installed() -> bool:
    root = Path(os.environ.get("SystemRoot", r"C:\Windows"))
    return (root / "System32" / "drivers" / "dokan2.sys").is_file()


def provide_dokany(fixtures: StageFixtures, name: str, reason: str) -> tuple[bool, str]:
    """The driver service runs: a fresh install may leave it stopped. No build variable is set here: the
    private Dokany pair SMART_EXPLORER_DOKANY_DLL_DIR/_SHA256 is read by build.rs and must come together."""
    started = fixtures.ctx.run(["sc", "start", "dokan2"], stage=fixtures.stage, timeout=120)
    running = started.ok or "1056" in started.output  # 1056: the service is already running
    fixtures.provide(name)
    return True, reason + ("" if running else f"; sc start dokan2: {started.describe()}")


def fixture_dokany(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    if not WINDOWS:
        return False, "Dokany nur unter Windows"
    if dokany_installed():
        return provide_dokany(fixtures, name, "Dokany-Treiber dokan2.sys vorhanden")
    msi, pin = os.environ.get("SE_TASK_DOKANY_MSI"), (os.environ.get("SE_TASK_DOKANY_MSI_SHA256") or "").lower()
    if not msi or not pin:
        return False, ("Dokany ist nicht installiert; die Suite installiert es nur aus SE_TASK_DOKANY_MSI mit "
                       "passender SE_TASK_DOKANY_MSI_SHA256, und beide sind nicht gesetzt")
    path = Path(msi)
    if not path.is_file() or sha256_file(path) != pin:
        return False, f"Dokany-MSI {path} fehlt oder ihre SHA-256 weicht vom Pin ab"
    installed = fixtures.ctx.run(["msiexec", "/i", path, "/qn", "/norestart"], stage=fixtures.stage, timeout=900)
    if dokany_installed():
        return provide_dokany(fixtures, name, f"Dokany aus der geprüften MSI {path.name} installiert")
    return False, f"msiexec: {installed.describe()}; dokan2.sys fehlt danach"


def fixture_own_suite(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    """Entries that only run inside their own task script: the reason names it, the entry is not run."""
    return False, ("braucht die Umgebung des eigenen Prüfskripts (native/test-mount-batching-task.ps1, "
                   "test-mount-optimization-task.ps1, test-windows-remote-task.ps1: privates Dokany-Artefakt mit "
                   "SMART_EXPLORER_DOKANY_DLL_DIR/_SHA256 für build.rs, node.exe, Prüfskript); in dieser Suite "
                   "nicht nachgebildet")


# The functions of servers.sh expect the shell of android/test-android-task.sh (set -Eeuo pipefail, inherit_errexit).
ANDROID_SERVERS_UP = ('set -Eeuo pipefail; shopt -s inherit_errexit; source "$1"; servers_up; '
                      'servers_instrumentation_args >"$2"')
ANDROID_SERVERS_DOWN = 'set +e; source "$1"; servers_down "$2"'


def read_instrumentation_args(text: str) -> list[tuple[str, str]]:
    """The output of ``servers_instrumentation_args``: ``-e``, name and value, one line each, repeated."""
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()  # the line break after the last value (an empty last value stays as an empty line)
    if not lines or len(lines) % 3:
        raise ValueError(f"{len(lines)} Zeilen sind kein Vielfaches von drei (-e, Name, Wert)")
    pairs = []
    for index in range(0, len(lines), 3):
        flag, key, value = lines[index:index + 3]
        if flag != "-e" or not NAME_RE.fullmatch(key):
            raise ValueError(f"Zeile {index + 1}: erwartet '-e <Name>', gefunden {flag!r} {key!r}")
        pairs.append((key, value))
    return pairs


def fixture_android_servers(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    """The test servers of the Android task suite through the functions of android/test-servers/servers.sh (the
    containers, ports and users of android/test-android-task.sh; nothing of that is repeated here): servers_up
    starts them, servers_instrumentation_args gives the ``-e`` arguments the instrumented tests read (``seSftp*``,
    ``seSsh*``, ``seFtp*``, ``seSmb*``, ``seServerHost``), servers_down stops them when the stage ends and writes the
    container logs below logs/android-servers-<stage>."""
    ctx = fixtures.ctx
    script = REPO / ANDROID_SERVERS_SCRIPT
    if not LINUX:
        return False, "die Android-Testserver laufen als Docker-Container nur auf Linux-Runnern"
    missing = [tool for tool in ("bash", "docker", "ssh-keyscan", "ssh-keygen") if not shutil.which(tool)]
    if missing:
        return False, f"{', '.join(missing)} fehlt (servers.sh verlangt docker, ssh-keyscan und ssh-keygen)"
    if not script.is_file():
        return False, f"{ANDROID_SERVERS_SCRIPT} fehlt im Checkout"
    logs = ctx.logs / f"android-servers-{fixtures.stage}"
    args_file = fixtures.make_dir("android-servers") / "instrumentation-args.txt"
    # Registered before the start: a servers_up that fails half way has started containers already.
    fixtures.add_cleanup(lambda: ctx.run(["bash", "-c", ANDROID_SERVERS_DOWN, "servers-down", script, logs],
                                         stage=fixtures.stage, timeout=600))
    started = ctx.run(["bash", "-c", ANDROID_SERVERS_UP, "servers-up", script, args_file], stage=fixtures.stage,
                      timeout=30 * 60)
    if not started.ok:
        return False, f"servers_up ({ANDROID_SERVERS_SCRIPT}): {started.describe()}: {excerpt(started.output, 900)}"
    try:
        pairs = read_instrumentation_args(args_file.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return False, f"die Instrumentierungsargumente von servers.sh sind nicht lesbar: {error}"
    blank = [key for key, value in pairs if not value.strip()]
    if blank:
        return False, "servers.sh liefert leere Instrumentierungsargumente: " + ", ".join(blank)
    fixtures.provide(name, args=pairs)
    return True, (f"{len(pairs)} Instrumentierungsargumente; SFTP, SSH, FTP und SMB als Container, "
                  f"vom Emulator erreichbar als {EMULATOR_HOST}")


def fixture_android_servers_optional(fixtures: StageFixtures, name: str) -> tuple[bool, str]:
    """For entries that read no server argument: they get the arguments of ``android-servers`` (the same call as in
    android/test-android-task.sh), and run without them when a server does not start. The servers are started once
    per stage, also when both fixtures are named in it."""
    ready, reason = fixtures.ensure(("android-servers",))
    if ready:
        fixtures.provide(name, args=fixtures.args.get("android-servers", []))
        return True, "Argumente von android-servers übernommen"
    fixtures.provide(name)
    return True, "läuft ohne Serverargumente weiter (die Einträge lesen keine): " + reason


BUILTIN_FIXTURES: dict[str, Callable[[StageFixtures, str], tuple[bool, str]]] = {
    "share": fixture_share, "sftp": fixture_sftp, "bus": fixture_bus, "xvfb": fixture_xvfb,
    "nodisplay": fixture_nodisplay, "home": fixture_home, "runtime-tmpfs": fixture_runtime_tmpfs,
    "fuse": fixture_fuse, "fuse-timeout": fixture_fuse_timeout, "dokany": fixture_dokany, "sudo": fixture_sudo,
    "own-suite": fixture_own_suite, "android-servers": fixture_android_servers,
    "android-servers-optional": fixture_android_servers_optional,
}


# --------------------------------------------------------------------------- Android sources, JUnit, emulator

@dataclass
class KotlinIndex:
    classes: dict  # simple class name -> fully qualified name
    methods: dict  # method name -> list of fully qualified class names


def kotlin_index(roots: Iterable[str]) -> KotlinIndex:
    classes: dict[str, str] = {}
    methods: dict[str, list[str]] = collections.defaultdict(list)
    # Only top-level classes (no indentation): a nested helper class must not capture the test methods.
    class_re = re.compile(r"^(?:(?:public|internal|open|abstract|final|data|private)\s+)*class\s+(\w+)")
    fun_re = re.compile(r"\bfun\s+`?(\w+)`?\s*\(")
    for root in roots:
        base = REPO / root
        if not base.is_dir():
            continue
        for path in sorted(list(base.rglob("*.kt")) + list(base.rglob("*.java"))):
            text = path.read_text(encoding="utf-8", errors="replace")
            package_match = re.search(r"^\s*package\s+([\w.]+)", text, re.M)
            package = package_match.group(1) if package_match else ""
            current = None
            for line in text.splitlines():
                declared = class_re.match(line)
                if declared:
                    current = f"{package}.{declared.group(1)}" if package else declared.group(1)
                    classes.setdefault(declared.group(1), current)
                    continue
                for function in fun_re.findall(line):
                    if current:
                        methods[function].append(current)
    return KotlinIndex(classes, methods)


def collect_junit(android_root: Path) -> dict[tuple[str, str], tuple[str, str, float]]:
    """JUnit XML of every Gradle test task under ``android/*/build/test-results``."""
    results: dict[tuple[str, str], tuple[str, str, float]] = {}
    if not android_root.is_dir():
        return results
    for build in android_root.glob("*/build/test-results"):
        for xml_path in build.rglob("TEST-*.xml"):
            try:
                tree = ElementTree.parse(xml_path)
            except ElementTree.ParseError:
                continue
            for case in tree.iter("testcase"):
                key = (case.get("classname") or "", case.get("name") or "")
                duration = float(case.get("time") or 0.0)
                problem = case.find("failure") if case.find("failure") is not None else case.find("error")
                if problem is not None:
                    message = (problem.get("message") or "") + "\n" + (problem.text or "")
                    results[key] = (FAILED, message.strip(), duration)
                elif case.find("skipped") is not None:
                    results[key] = (NOT_RUN, "von JUnit übersprungen: " + (case.find("skipped").get("message") or ""),
                                    duration)
                else:
                    results[key] = (PASSED, f"{key[0]}#{key[1]}", duration)
    return results


INSTRUMENTATION_CODES = {0: PASSED, -1: FAILED, -2: FAILED, -3: NOT_RUN, -4: NOT_RUN}


def parse_instrumentation(text: str) -> dict[tuple[str, str], tuple[str, str]]:
    """``am instrument -r`` status blocks: (class, method) -> (status, detail)."""
    results: dict[tuple[str, str], tuple[str, str]] = {}
    current: dict[str, str] = {}
    key = None
    for line in (text or "").splitlines():
        if line.startswith("INSTRUMENTATION_STATUS: "):
            name, _, value = line[len("INSTRUMENTATION_STATUS: "):].partition("=")
            current[name] = value
            key = name
        elif line.startswith("INSTRUMENTATION_STATUS_CODE: "):
            try:
                code = int(line.split(":", 1)[1].strip())
            except ValueError:
                code = -1
            pair = (current.get("class", ""), current.get("test", ""))
            if pair[0] and pair[1]:
                if code == 1:
                    results.setdefault(pair, (FAILED, "gestartet, aber ohne Ergebnis (Prozessabsturz?)"))
                else:
                    status = INSTRUMENTATION_CODES.get(code, FAILED)
                    detail = current.get("stack", "")
                    if code == -3:
                        detail = "mit @Ignore markiert"
                    elif code == -4:
                        detail = "Annahme nicht erfüllt (assumeTrue): " + first_line(detail)
                    results[pair] = (status, detail)
            current, key = {}, None
        elif line.startswith("INSTRUMENTATION_RESULT: ") or line.startswith("INSTRUMENTATION_CODE: "):
            key = None
        elif key:
            current[key] = current.get(key, "") + "\n" + line
    return results


class Device:
    """The emulator of an Android device job: APK install, the app's preparation (``prepare_app``) and
    instrumentation runs."""

    def __init__(self, ctx: "JobContext", apks_dir: Path | None, expected_api: int):
        self.ctx = ctx
        self.apks_dir = apks_dir
        self.expected_api = expected_api
        sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or ""
        found = shutil.which("adb")
        self.adb = Path(found) if found else Path(sdk) / "platform-tools" / "adb"
        self.api: int | None = None
        self.component: str | None = None
        self.missing_inputs = False  # the APKs of android-build are absent: not run here, failed there
        self.index = kotlin_index(ANDROID_TEST_PACKAGE_ROOTS)

    def adb_run(self, *args, timeout: float = 600, input_text: str | None = None) -> Completed:
        return self.ctx.run([self.adb, *args], stage="device", timeout=timeout, input_text=input_text)

    def shell(self, *args, timeout: float = 600, input_text: str | None = None) -> Completed:
        """``adb shell`` with every argument quoted for the device's shell. adb joins the arguments with blanks and
        the device's ``/system/bin/sh`` parses that string again: a value with blanks or parentheses (for example
        ``-e speed_dd_note "Seitencache nicht geleert (kein root)"``) would split or end in ``syntax error:
        unexpected '('``, and ``sh -c "echo 3 > file"`` would redirect on the device's outer shell."""
        return self.adb_run("shell", *(shlex.quote(str(argument)) for argument in args), timeout=timeout,
                            input_text=input_text)

    def setup(self) -> tuple[bool, str]:
        if not self.adb.exists():
            return False, "adb fehlt"
        if not self.adb_run("wait-for-device", timeout=900).ok:
            return False, "adb wait-for-device gescheitert"
        level = self.shell("getprop", "ro.build.version.sdk", timeout=120).output.strip()
        self.api = int(level) if level.isdigit() else None
        apks = sorted(self.apks_dir.rglob("*.apk")) if self.apks_dir and self.apks_dir.is_dir() else []
        tests = [apk for apk in apks if "androidtest" in apk.name.lower()]
        apps = [apk for apk in apks if apk not in tests]
        apps.sort(key=lambda apk: (0 if "debug" in apk.name.lower() else 1,
                                   0 if ("x86_64" in apk.name or "universal" in apk.name) else 1, apk.name))
        if not apps or not tests:
            self.missing_inputs = True
            return False, (f"APKs fehlen im Artefakt rdt-apks ({', '.join(apk.name for apk in apks) or 'leer'}); "
                           "Ursache steht im Bericht des Jobs android-build")
        for apk in (apps[0], tests[0]):
            installed = self.adb_run("install", "-r", "-t", "-g", apk, timeout=900)
            if not installed.ok or "Success" not in installed.output:
                return False, f"adb install {apk.name}: {installed.describe()}: {excerpt(installed.output, 600)}"
        listed = self.shell("pm", "list", "instrumentation", timeout=120)
        components = re.findall(r"^instrumentation:(\S+) \(target=(\S+)\)", listed.output, re.M)
        candidates = [component for component, target in components
                      if component.split("/", 1)[0].startswith(target)] or [c for c, _ in components]
        if not candidates:
            return False, "keine Instrumentierung installiert"
        self.component = candidates[0]
        reason = f"API {self.api}; Instrumentierung {self.component}; APKs {apps[0].name}, {tests[0].name}"
        if self.api != self.expected_api:
            reason += f"; Rückfall: API {self.api} statt {self.expected_api} (Annahme D3)"
        return True, reason

    def app_write(self, target: str, content: str) -> bool:
        """``app_write`` of android/test-android-task.sh: ``content`` into the app's own data through ``run-as`` (the
        debug build is debuggable), then read back; ``target`` is relative to the app's data folder."""
        script = f"mkdir -p $(dirname {target}) && cat > {target}"
        written = self.shell("run-as", APP_PACKAGE, "sh", "-c", script, timeout=120, input_text=content)
        read = self.shell("run-as", APP_PACKAGE, "cat", target, timeout=120)
        return written.ok and read.ok and read.output.strip() == content.strip()

    def prepare_app(self) -> tuple[bool, str]:
        """The state of the installed app that ``emulator-run`` of android/test-android-task.sh creates before its
        instrumentation runs: access to all files (the tests create their folders below
        /storage/*/SmartExplorerTask), usage access, the notification permission, the app stopped and its first-start
        settings written into its data. A fresh install lacks all of it: every test that creates a fixture folder ends
        in "Ordner nicht angelegt: /storage/emulated/0/SmartExplorerTask/...", and the first start shows the
        onboarding. Returns (all steps done, one line per step)."""
        level = self.api or self.expected_api
        # (label, arguments of adb shell, first API level on which the operation or permission exists)
        steps = (
            ("Zugriff auf alle Dateien",
             ("appops", "set", "--uid", APP_PACKAGE, "MANAGE_EXTERNAL_STORAGE", "allow"), 30),
            ("Nutzungszugriff", ("appops", "set", APP_PACKAGE, "GET_USAGE_STATS", "allow"), 0),
            ("Benachrichtigungen", ("pm", "grant", APP_PACKAGE, "android.permission.POST_NOTIFICATIONS"), 33),
            ("App beendet", ("am", "force-stop", APP_PACKAGE), 0),
        )
        parts: list[str] = []
        ok = True
        for label, command, since in steps:
            if level < since:
                parts.append(f"{label}: entfällt, gibt es erst ab API {since}")
                continue
            done = self.shell(*command, timeout=120)
            # appops and pm name an unknown operation or permission in their output.
            if done.ok and not re.search(r"\b(?:Exception|Error)\b", done.output):
                parts.append(f"{label}: {done.describe()}")
            else:
                ok = False
                parts.append(f"{label} GESCHEITERT: {done.describe()}: {excerpt(done.output, 200)}")
        if self.app_write(APP_PREFS_FILE, APP_PREFS_XML):
            parts.append("Erststart-Einstellungen: geschrieben (onboarding_done, auto_update_check=false)")
        else:
            ok = False
            parts.append(f"Erststart-Einstellungen GESCHEITERT: {APP_PREFS_FILE} ließ sich über run-as nicht schreiben "
                         "oder nicht zurücklesen")
        summary = "; ".join(parts)
        self.ctx.log_line("device-setup", "app: " + summary)
        return ok, ("App vorbereitet wie in android/test-android-task.sh: " if ok else "Vorbereitung der App: ") + summary

    def resolve(self, name: str) -> tuple[str, str | None] | None:
        if name[:1].isupper():
            fqn = self.index.classes.get(name)
            return (fqn, None) if fqn else None
        owners = self.index.methods.get(name)
        return (owners[0], name) if owners else None

    def run_class(self, fqn: str, args: list[tuple[str, str]], timeout: float = 45 * 60):
        """``fqn`` is a class (``a.B``, all its methods) or one method (``a.B#m``). Every argument is quoted for the
        device's shell (``shell``): the values of the ``-e`` arguments may hold blanks and parentheses."""
        command = ["am", "instrument", "-w", "-r"]
        for key, value in args:
            command += ["-e", key, value]
        command += ["-e", "class", fqn, self.component]
        started = time.monotonic()
        completed = self.shell(*command, timeout=timeout)
        return parse_instrumentation(completed.output), completed, time.monotonic() - started


# --------------------------------------------------------------------------- job context

class JobContext:
    def __init__(self, job: str, plan: list[tuple[str, tuple[str, ...]]], log_root: Path, work: Path,
                 candidate_sha: str, args, signals: Signals, modules: dict, resume: bool = False):
        self.job = job
        self.plan = plan
        self.log_root = log_root
        self.logs = log_root / "logs"
        self.logs.mkdir(parents=True, exist_ok=True)
        self.work = work
        self.work.mkdir(parents=True, exist_ok=True)
        self.candidate_sha = candidate_sha
        self.args = args
        self.signals = signals
        self.modules = modules  # module name -> module, or None when the file does not exist
        self.report = Report(job, candidate_sha, log_root, resume=resume)
        self.cargo = Cargo(self)
        self.state: dict = {}
        self.device: Device | None = None
        self.fixtures: StageFixtures | None = None
        self.current_stage: str | None = None
        self._sudo: bool | None = None

    # ---- logging, environment, processes
    def log_path(self, stage: str | None) -> Path:
        return self.logs / f"{stage or 'job'}.log"

    def log_line(self, stage: str | None, text: str) -> None:
        with open(self.log_path(stage), "a", encoding="utf-8") as handle:
            handle.write(text + "\n")

    def run(self, cmd: list, stage: str | None = None, **kwargs) -> Completed:
        return run(cmd, log=self.log_path(stage or self.current_stage), **kwargs)

    def base_env(self) -> dict:
        env = dict(os.environ)
        env["CARGO_HOME"] = str(CARGO_HOME)
        env["RUSTUP_HOME"] = str(RUSTUP_HOME)
        env.setdefault("RUST_BACKTRACE", "1")
        env["CARGO_TERM_COLOR"] = "never"
        return env

    def test_env(self) -> dict:
        """What ``cargo test`` sets for a test process, for runs of the test program itself."""
        env = self.base_env()
        env["CARGO_MANIFEST_DIR"] = str(NATIVE)
        env["CARGO_PKG_NAME"] = "smart_explorer"
        env["CARGO_CRATE_NAME"] = "smart_explorer"
        return env

    def sudo_ok(self) -> bool:
        if self._sudo is None:
            self._sudo = LINUX and bool(shutil.which("sudo")) and run(["sudo", "-n", "true"], timeout=60).ok
        return self._sudo

    # ---- registries of the rdt_task modules
    def present_modules(self) -> list:
        return [module for module in self.modules.values() if module is not None]

    def checks(self) -> dict[str, Callable]:
        merged: dict[str, Callable] = {}
        for module in self.present_modules():
            for name, function in getattr(module, "CHECKS", {}).items():
                merged.setdefault(name, function)
        return merged

    def fixture_provider(self, name: str):
        if name in BUILTIN_FIXTURES:
            return BUILTIN_FIXTURES[name]
        for module in self.present_modules():
            provider = getattr(module, "FIXTURES", {}).get(name)
            if provider:
                return provider
        return None

    # ---- entries and results
    def entries(self, stage: str) -> list[Entry]:
        return self.signals.for_stage(self.job, stage)

    def record(self, name: str, stage: str, status: str, reason: str = "", duration_s: float = 0.0) -> None:
        self.report.record(name, stage, status, reason, duration_s)

    def recorded(self, name: str) -> bool:
        return self.report.recorded(name) > 0

    def not_run_all(self, stage: str, reason: str, entries: Iterable[Entry] | None = None) -> None:
        for entry in (entries if entries is not None else self.entries(stage)):
            if not self.recorded(entry.name):
                self.record(entry.name, stage, NOT_RUN, reason + (f"; {entry.pending_reason()}" if entry.pending else ""))

    def stage_not_implemented(self, stage: str, owners: Iterable[str]) -> None:
        files = ", ".join(f"rdt_task/{owner}.py" for owner in owners)
        reason = f"Stufe nicht umgesetzt: {files} fehlt (Suite Teil B, Paket U41)"
        self.state.setdefault("stage_notes", {})[stage] = reason
        self.not_run_all(stage, reason)

    def expand(self, value: str, tmp: Path) -> str:
        if "{se}" in value:
            path = self.cargo.bin_path("se")
            if path is None:
                raise StageError(self.cargo.bin_errors.get("se", "Bau von se gescheitert"))
            value = value.replace("{se}", str(path))
        if "{gui}" in value:
            path = self.cargo.bin_path("smart_explorer")
            if path is None:
                raise StageError(self.cargo.bin_errors.get("smart_explorer", "Bau von smart_explorer gescheitert"))
            value = value.replace("{gui}", str(path))
        for key, replacement in (("{tmp}", tmp), ("{native}", NATIVE), ("{repo}", REPO), ("{work}", self.work)):
            value = value.replace(key, str(replacement))
        if value.startswith(str(tmp)):
            Path(value).mkdir(parents=True, exist_ok=True)
        return value

    # ---- the generic stage
    def run_default(self, stage: str, extra_env: dict | None = None) -> None:
        entries = [entry for entry in self.entries(stage) if not self.recorded(entry.name)]
        libs = [entry for entry in entries if entry.kind in ("lib", "modul")]
        if libs:
            self.run_lib(stage, libs, extra_env or {})
        for entry in entries:
            if entry.kind == "check" and entry.name != INVENTORY and not self.recorded(entry.name):
                self.run_check(stage, entry)
        junit = [entry for entry in entries if entry.kind == "junit"]
        if junit:
            self.run_junit(stage, junit)
        instr = [entry for entry in entries if entry.kind == "instr"]
        if instr:
            self.run_instr(stage, instr)

    def run_check(self, stage: str, entry: Entry) -> None:
        function = self.checks().get(entry.name)
        if function is None:
            if entry.pending:
                self.record(entry.name, stage, NOT_RUN, entry.pending_reason())
            else:
                self.record(entry.name, stage, FAILED, "keine Umsetzung im Register CHECKS eines rdt_task-Moduls")
            return
        started = time.monotonic()
        try:
            status, reason = function(self)
        except Exception as error:  # noqa: BLE001 - an unexpected error fails exactly this check
            status, reason = FAILED, f"{type(error).__name__}: {error}\n{traceback.format_exc(limit=6)}"
        self.record(entry.name, stage, status, reason, time.monotonic() - started)

    def run_lib(self, stage: str, entries: list[Entry], extra_env: dict) -> None:
        if not self.cargo.build_lib():
            for entry in entries:
                self.record(entry.name, stage, FAILED, "Bau der Bibliothekstests gescheitert: " + self.cargo.lib_error)
            return
        assert self.cargo.paths is not None
        planned: list[tuple[Entry, list[str]]] = []
        for entry in entries:
            if entry.kind == "lib":
                found = self.cargo.by_leaf.get(entry.name, [])
                if len(found) == 1:
                    planned.append((entry, found))
                elif not found:
                    if entry.pending:
                        self.record(entry.name, stage, NOT_RUN, entry.pending_reason())
                    else:
                        self.record(entry.name, stage, FAILED, "fehlt im Testprogramm dieser Plattform "
                                    "(nicht kompiliert, Modul nicht eingebunden oder umbenannt)")
                else:
                    self.record(entry.name, stage, FAILED, "Name mehrdeutig im Testprogramm: " + ", ".join(found))
            else:
                fragment = entry.module or ""
                found = [path for path in self.cargo.paths
                         if fragment in "::" + path and not is_batch_name(leaf(path))]
                if found:
                    planned.append((entry, found))
                elif entry.pending:
                    self.record(entry.name, stage, NOT_RUN, entry.pending_reason())
                else:
                    self.record(entry.name, stage, FAILED, f"keine Tests mit dem Pfadbruchstück {fragment}")
        groups: dict[tuple, list[tuple[Entry, list[str]]]] = collections.defaultdict(list)
        for entry, paths in planned:
            key = (tuple(sorted(entry.env)), entry.threads, entry.root, tuple(sorted(entry.fixtures)))
            groups[key].append((entry, paths))
        for (env_items, threads, root, fixture_names), items in groups.items():
            self._run_group(stage, items, dict(env_items), threads, root, fixture_names, extra_env)

    def _run_group(self, stage: str, items: list[tuple[Entry, list[str]]], env_items: dict, threads: int | None,
                   root: bool, fixture_names: tuple, extra_env: dict) -> None:
        assert self.fixtures is not None
        ready, why = self.fixtures.ensure(fixture_names)
        runs: dict[bool, list[str]] = {True: [], False: []}
        modul_skips: dict[str, tuple[int, str]] = {}
        active: list[tuple[Entry, list[str]]] = []
        for entry, paths in items:
            if entry.kind == "lib":
                if not ready:
                    self.record(entry.name, stage, NOT_RUN, why)
                    continue
                runs[entry.ignored].extend(paths)
                active.append((entry, paths))
            else:
                if ready:
                    runs[True].extend(paths)
                    active.append((entry, paths))
                else:
                    normal = [path for path in paths if path not in self.cargo.ignored]
                    skipped = len(paths) - len(normal)
                    runs[False].extend(normal)
                    modul_skips[entry.name] = (skipped, why)
                    active.append((entry, normal))
        if not active:
            return
        if root and not self.sudo_ok():
            for entry, _ in active:
                self.record(entry.name, stage, NOT_RUN, "läuft als root, aber sudo -n ist nicht verfügbar")
            return
        tmp = self.work / "groups" / f"{stage}-{secrets.token_hex(4)}"
        tmp.mkdir(parents=True, exist_ok=True)
        try:
            env = self.test_env() if root else self.base_env()
            fixture_env, removed = self.fixtures.env_for(fixture_names)
            env.update(fixture_env)
            env.update(extra_env)
            for key in removed:
                env.pop(key, None)
            for key, value in env_items.items():
                env[key] = self.expand(value, tmp)
            if root and "HOME" not in env_items and "HOME" not in fixture_env:
                home = tmp / "root-home"
                home.mkdir(parents=True, exist_ok=True)
                env["HOME"] = str(home)
        except StageError as error:
            for entry, _ in active:
                self.record(entry.name, stage, FAILED, f"Umgebung nicht herstellbar: {error}")
            return
        results: dict[str, list] = {}
        for include_ignored, paths in runs.items():
            if paths:
                results.update(self.run_libtest(stage, paths, env, include_ignored, threads, root))
        if root:
            self.run(["sudo", "-n", "rm", "-rf", "--", tmp], stage=stage, timeout=300)
        for entry, paths in active:
            if entry.kind == "lib":
                status, reason, duration = results.get(paths[0], [FAILED, "kein Ergebnis", 0.0])
                self.record(entry.name, stage, status, reason or paths[0], duration)
                continue
            outcomes = [results.get(path, [FAILED, "kein Ergebnis", 0.0]) for path in paths]
            failed = [path for path, outcome in zip(paths, outcomes) if outcome[0] == FAILED]
            passed = sum(1 for outcome in outcomes if outcome[0] == PASSED)
            skipped, skip_reason = modul_skips.get(entry.name, (0, ""))
            not_run = sum(1 for outcome in outcomes if outcome[0] == NOT_RUN) + skipped
            duration = sum(outcome[2] for outcome in outcomes)
            reason = f"{passed} bestanden, {len(failed)} gescheitert, {not_run} nicht ausgeführt"
            if skipped:
                reason += f" ({skipped} #[ignore]-Tests ohne {skip_reason})"
            if failed:
                details = [f"{path}: {first_line(results.get(path, ['', '', 0])[1])}" for path in failed[:20]]
                self.record(entry.name, stage, FAILED, reason + "\n" + "\n".join(details), duration)
            elif passed:
                self.record(entry.name, stage, PASSED, reason, duration)
            else:
                self.record(entry.name, stage, NOT_RUN, reason, duration)

    def run_libtest(self, stage: str, paths: list[str], env: dict, include_ignored: bool,
                    threads: int | None, root: bool) -> dict[str, list]:
        results: dict[str, list] = {}
        for part in chunks(paths):
            arguments = ["--exact", *part]
            if include_ignored:
                arguments.append("--include-ignored")
            if threads:
                arguments.append(f"--test-threads={threads}")
            if root:
                command, cwd = ["sudo", "-n", "-E", self.cargo.test_exe, *arguments], NATIVE
            else:
                command = ["cargo", "test", "--locked", "--lib", "--manifest-path", Cargo.manifest(), "--", *arguments]
                cwd = REPO
            parser = LibtestParser(time.monotonic())
            completed = self.run(command, stage=stage, cwd=cwd, env=env, timeout=LIB_TIMEOUT_S, on_line=parser.feed)
            blocks = {match.group(1): match.group(2).strip() for match in FAILURE_BLOCK.finditer(completed.output)}
            for path in part:
                if path not in parser.results:
                    parser.results[path] = [FAILED, f"kein Ergebnis ({completed.describe()}): "
                                            + excerpt(completed.output, 1200), 0.0]
                elif parser.results[path][0] == FAILED:
                    block = blocks.get(path) or excerpt(completed.output, 1200)
                    said = NOT_RUN_SAID.search(block)
                    if said:
                        parser.results[path] = [NOT_RUN, said.group(1).strip(), parser.results[path][2]]
                    else:
                        parser.results[path][1] = block
            results.update(parser.results)
        return results

    def run_junit(self, stage: str, entries: list[Entry]) -> None:
        results = self.state.get("junit")
        for entry in entries:
            if results is None:
                self.record(entry.name, stage, NOT_RUN, "keine Gradle-Testberichte: der Android-Bau lief nicht"
                            + (f"; {entry.pending_reason()}" if entry.pending else ""))
                continue
            if entry.is_class:
                matches = [(key, value) for key, value in results.items()
                           if key[0] == entry.name or key[0].endswith("." + entry.name)]
            else:
                matches = [(key, value) for key, value in results.items() if key[1] == entry.name]
            if not matches:
                if entry.pending:
                    self.record(entry.name, stage, NOT_RUN, entry.pending_reason())
                else:
                    self.record(entry.name, stage, FAILED, "nicht in den Gradle-Testberichten")
                continue
            self.record(entry.name, stage, *aggregate([(f"{key[0]}#{key[1]}", value[0], value[1], value[2])
                                                       for key, value in matches]))

    def run_instr(self, stage: str, entries: list[Entry]) -> None:
        index = self.device.index if self.device else kotlin_index(ANDROID_TEST_PACKAGE_ROOTS)
        runs: dict[tuple, tuple] = {}
        for entry in entries:
            target = (index.classes.get(entry.name), None) if entry.is_class else (
                (index.methods[entry.name][0], entry.name) if index.methods.get(entry.name) else (None, None))
            if not target[0]:
                if entry.pending:
                    self.record(entry.name, stage, NOT_RUN, entry.pending_reason())
                else:
                    self.record(entry.name, stage, FAILED, "fehlt in android/app/src/androidTest")
                continue
            if self.device is None:
                reason = self.state.get("device_reason", "Emulator nicht bereit")
                self.record(entry.name, stage, NOT_RUN, f"Emulator nicht bereit: {reason}")
                continue
            assert self.fixtures is not None
            ready, why = self.fixtures.ensure(entry.fixtures)
            if not ready:
                self.record(entry.name, stage, NOT_RUN, why)
                continue
            args = self.fixtures.args_for(entry.fixtures)
            # A class entry starts the whole class; a method entry starts only that method (a.B#m).
            spec = target[0] if entry.is_class else f"{target[0]}#{entry.name}"
            key = (spec, tuple(args))
            if key not in runs:
                runs[key] = self.device.run_class(spec, args)
            parsed, completed, duration = runs[key]
            if entry.is_class:
                items = [(f"{cls}#{method}", status, detail, 0.0) for (cls, method), (status, detail)
                         in parsed.items() if cls == target[0]]
                if not items:
                    self.record(entry.name, stage, FAILED, f"keine Ergebnisse: {completed.describe()}: "
                                + excerpt(completed.output, 1200), duration)
                else:
                    self.record(entry.name, stage, *aggregate(items)[:2], duration)
            else:
                outcome = parsed.get((target[0], entry.name))
                if outcome is None:
                    self.record(entry.name, stage, FAILED, f"kein Ergebnis für {target[0]}#{entry.name}: "
                                f"{completed.describe()}: " + excerpt(completed.output, 1200), duration)
                else:
                    self.record(entry.name, stage, outcome[0], outcome[1] or f"{target[0]}#{entry.name}", duration)


def aggregate(items: list[tuple[str, str, str, float]]) -> tuple[str, str, float]:
    """One status for several sub-results (class entries): failed > passed > not_run."""
    failed = [item for item in items if item[1] == FAILED]
    passed = [item for item in items if item[1] == PASSED]
    not_run = [item for item in items if item[1] == NOT_RUN]
    duration = sum(item[3] for item in items)
    reason = f"{len(passed)} bestanden, {len(failed)} gescheitert, {len(not_run)} nicht ausgeführt"
    details = [f"{label}: {first_line(detail)}" for label, _, detail, _ in failed[:20]]
    details += [f"{label}: nicht ausgeführt: {first_line(detail)}" for label, _, detail, _ in not_run[:20]]
    reason = reason + ("\n" + "\n".join(details) if details else "")
    if failed:
        return FAILED, reason, duration
    if passed:
        return PASSED, reason, duration
    return NOT_RUN, reason, duration


# --------------------------------------------------------------------------- common stages and checks

def check_prepare_files_placed(ctx: JobContext) -> tuple[str, str]:
    """L1/L2: the job builds with native/Cargo.lock and the license file of the job prepare."""
    manifest_path = REPO / PREPARE_MANIFEST
    if not manifest_path.is_file():
        return FAILED, f"{PREPARE_MANIFEST} fehlt: das Artefakt rdt-prepare liegt nicht im Checkout"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    problems, notes = [], []
    if (manifest.get("candidate_sha") or "").lower() != ctx.candidate_sha.lower():
        problems.append(f"Artefakt gehört zu {manifest.get('candidate_sha')}, Kandidat ist {ctx.candidate_sha}")
    files = manifest.get("files") or {}
    for relative, digest in files.items():
        path = REPO / relative
        if not path.is_file():
            problems.append(f"{relative} fehlt im Checkout")
        elif sha256_file(path) != digest:
            problems.append(f"{relative} weicht vom Artefakt ab")
    if LOCK_FILE not in files:
        problems.append(f"{LOCK_FILE} fehlt im Artefakt")
    if LICENSE_FILE not in files:
        if (REPO / LICENSE_FILE).is_file():
            notes.append(f"{LICENSE_FILE} nicht im Artefakt, die Datei aus dem Commit gilt")
        else:
            problems.append(f"{LICENSE_FILE} fehlt im Artefakt und im Commit (eingebettete Lizenztexte fehlen)")
    if problems:
        return FAILED, "; ".join(problems + notes)
    return PASSED, "; ".join([f"{', '.join(sorted(files))} aus rdt-prepare übernommen"] + notes)


def check_desktop_bins_built(ctx: JobContext) -> tuple[str, str]:
    return ctx.state.get("desktop_bins", (FAILED, "Stufe desktop-bins lief nicht"))


def check_android_apks_built(ctx: JobContext) -> tuple[str, str]:
    return ctx.state.get("apk_result", (FAILED, "Stufe apk lief nicht"))


def check_device_ready(ctx: JobContext) -> tuple[str, str]:
    ready = ctx.state.get("device_ready")
    if ready is None:
        return FAILED, "Stufe device-setup lief nicht"
    if ready[0]:
        return PASSED, ready[1]
    return (NOT_RUN if ctx.state.get("device_missing_inputs") else FAILED), ready[1]


def stage_desktop_bins(ctx: JobContext, stage: str) -> None:
    """Linux ``se`` and the share server for the device pairs of the Android jobs (D3)."""
    out = Path(ctx.args.out) if ctx.args.out else ctx.work / "desktop-bins"
    out.mkdir(parents=True, exist_ok=True)
    se = ctx.cargo.bin_path("se")
    server = ctx.cargo.share_server()
    problems = []
    if se is None:
        problems.append(ctx.cargo.bin_errors.get("se", "se fehlt"))
    else:
        shutil.copy2(se, out / se.name)
    if server is None:
        problems.append(ctx.cargo.server_error or "Share-Server fehlt")
    else:
        shutil.copy2(server, out / ("se-share-server.exe" if WINDOWS else "se-share-server"))
    ctx.state["desktop_bins"] = (FAILED, "; ".join(problems)) if problems else (
        PASSED, f"se und se-share-server im Entwicklungsprofil nach {out}")
    ctx.run_default(stage)


def stage_apk(ctx: JobContext, stage: str) -> None:
    """Debug and test APK through the existing entry android/test-android-task.sh (black box),
    then the JUnit/Robolectric reports for the stages drive-android and launcher-jvm."""
    out = Path(ctx.args.out) if ctx.args.out else ctx.work / "apks"
    out.mkdir(parents=True, exist_ok=True)
    script = REPO / "android" / "test-android-task.sh"
    if not script.is_file():
        ctx.state["apk_result"] = (FAILED, "android/test-android-task.sh fehlt")
    else:
        env = dict(ctx.base_env(), SMART_EXPLORER_TASK_LOG_ROOT=str(ctx.log_root / "android-logs"))
        completed = ctx.run(["bash", script, "android-build", "--out", out], stage=stage, env=env, timeout=160 * 60)
        apks = sorted(out.rglob("*.apk"))
        tests = [apk for apk in apks if "androidtest" in apk.name.lower()]
        if completed.ok and tests and len(apks) > len(tests):
            ctx.state["apk_result"] = (PASSED, "APKs: " + ", ".join(apk.name for apk in apks))
        else:
            hint = ""
            if "aarch64" in completed.output or "armv7" in completed.output:
                hint = ("; Hinweis: der Einstieg verlangt weitere Android-Ziele, die Suite installiert nach "
                        "D3-Bauregel nur x86_64-linux-android")
            ctx.state["apk_result"] = (FAILED, f"android-build: {completed.describe()}; APKs: "
                                       f"{', '.join(apk.name for apk in apks) or 'keine'}{hint}\n"
                                       + excerpt(completed.output, 2000))
    ctx.state["junit"] = collect_junit(REPO / "android")
    ctx.run_default(stage)


def stage_device_setup(ctx: JobContext, stage: str) -> None:
    apks = Path(ctx.args.apks) if ctx.args.apks else None
    bins = Path(ctx.args.bins) if ctx.args.bins else ctx.work / "no-desktop-bins"
    if bins.is_dir():
        for path in bins.iterdir():
            if path.is_file() and not WINDOWS:
                path.chmod(0o755)  # artifacts do not keep the executable bit
    # Always set: the device jobs take the share server from rdt-desktop-bins and never build it.
    ctx.state["desktop_bins_dir"] = str(bins)
    device = Device(ctx, apks, ctx.args.api_level or 36)
    try:
        ready, reason = device.setup()
    except Exception as error:  # noqa: BLE001
        ready, reason = False, f"{type(error).__name__}: {error}"
    usable = ready
    if usable:
        # The app's state of android/test-android-task.sh (storage access, notifications, first-start settings). A
        # failed step does not stop the stages: rdt_suite_device_ready fails with the step named, and every test
        # that needs it fails with its own message.
        ctx.device = device
        try:
            prepared, note = device.prepare_app()
        except Exception as error:  # noqa: BLE001
            prepared, note = False, f"Vorbereitung der App: {type(error).__name__}: {error}"
        ready, reason = prepared, f"{reason}; {note}"
    ctx.state["device_ready"] = (ready, reason)
    ctx.state["device_reason"] = reason
    ctx.state["device_missing_inputs"] = device.missing_inputs
    if usable:
        for module in ctx.present_modules():
            hook = getattr(module, "DEVICE_SETUP", None)
            if hook:
                hook(ctx, device)
    ctx.run_default(stage)


STAGES = {"desktop-bins": stage_desktop_bins, "apk": stage_apk, "device-setup": stage_device_setup}
CHECKS = {
    "rdt_suite_prepare_files_placed": check_prepare_files_placed,
    "rdt_suite_desktop_bins_built": check_desktop_bins_built,
    "rdt_suite_android_apks_built": check_android_apks_built,
    "rdt_suite_device_ready": check_device_ready,
}


# --------------------------------------------------------------------------- documentation helpers

CITATION_EXTENSIONS = ("rs", "kt", "kts", "java", "py", "ps1", "sh", "nsi", "md", "toml", "yml", "yaml", "json",
                       "xml", "txt", "gradle", "hbs", "conf", "c", "h", "i")
CITATION = re.compile(r"(?<![\w/.-])((?:[\w.-]+/)*[\w.-]+\.(?:" + "|".join(CITATION_EXTENSIONS)
                      + r")):(\d+)(?:\s*[-–]\s*(\d+))?")
CITATION_ALIASES = (("N/", "native/src/"), ("A/", "android/app/src/main/java/app/smartexplorer/android/"),
                    ("AT/", "android/app/src/androidTest/java/app/smartexplorer/android/"))
CITATION_BASES = ("", "native/src/", "native/", "android/app/src/main/java/app/smartexplorer/android/")


class Citations:
    """``Datei:Zeile`` references of the surface inventories (C6) resolved against the checkout."""

    def __init__(self) -> None:
        listed = run(["git", "ls-files"], cwd=REPO, timeout=120)
        self.by_name: dict[str, list[Path]] = collections.defaultdict(list)
        for line in listed.output.splitlines():
            self.by_name[Path(line).name].append(REPO / line)
        self._lines: dict[Path, int] = {}

    def line_count(self, path: Path) -> int:
        if path not in self._lines:
            try:
                self._lines[path] = len(path.read_text(encoding="utf-8", errors="replace").splitlines())
            except OSError:
                self._lines[path] = -1
        return self._lines[path]

    def resolve(self, text: str, line: int) -> str | None:
        """None when the reference resolves, else the problem."""
        relative = text.replace("\\", "/")
        for alias, target in CITATION_ALIASES:
            if relative.startswith(alias):
                relative = target + relative[len(alias):]
        candidates = [REPO / (base + relative) for base in CITATION_BASES if (REPO / (base + relative)).is_file()]
        if not candidates and "/" not in relative:
            candidates = self.by_name.get(relative, [])
        if not candidates:
            return f"{text}:{line}: Datei fehlt"
        if any(line <= self.line_count(candidate) for candidate in candidates):
            return None
        return f"{text}:{line}: Zeile jenseits des Dateiendes"

    def check_text(self, text: str) -> tuple[int, list[str]]:
        problems, count = [], 0
        for match in CITATION.finditer(text):
            count += 1
            problem = self.resolve(match.group(1), int(match.group(2)))
            if problem:
                problems.append(problem)
        return count, problems


INSTALLER = REPO / "native" / "installer.nsi"


def nsis_uninstall_lines(text: str) -> list[tuple[int, str]]:
    """Lines of the NSIS uninstall section (``Section "Uninstall"`` ... ``SectionEnd``) and of
    every ``un.`` function; the whole file when neither exists."""
    lines = text.splitlines()
    selected: list[tuple[int, str]] = []
    inside = False
    for number, line in enumerate(lines, 1):
        stripped = line.strip()
        lowered = stripped.lower()
        if re.match(r'^section\s+(/o\s+)?"?-?un(install)?\b', lowered) or re.match(r"^function\s+un\.", lowered):
            inside = True
        if inside:
            selected.append((number, stripped))
        if lowered.startswith("sectionend") or lowered.startswith("functionend"):
            inside = False
    return selected or [(number, line.strip()) for number, line in enumerate(lines, 1)]


def nsis_normal(line: str) -> str:
    """Case, quotes and path separators removed, for comparisons of NSIS lines."""
    return line.replace('"', "").replace("'", "").replace("/", "\\").lower()


def table_rows(text: str) -> list[tuple[int, list[str]]]:
    rows = []
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not stripped.startswith("|") or re.fullmatch(r"\|[\s:|-]+\|?", stripped):
            continue
        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        rows.append((number, cells))
    return rows


# --------------------------------------------------------------------------- inventory

def inventory_job(ctx: JobContext) -> tuple[list[str], list[str]]:
    """Job part of rdt_suite_inventory_complete: every expected name of the job has exactly one
    result, no unexpected batch name appears, and the compiled tests match the list."""
    problems = [f"expected_signals.txt: {error}" for error in ctx.signals.errors]
    notes: list[str] = []
    expected = ctx.signals.for_job(ctx.job)
    expected_names = {entry.name for entry, _ in expected}
    counts = collections.Counter(test["name"] for test in ctx.report.tests)
    for entry, stage in expected:
        if entry.name == INVENTORY:
            continue
        number = counts.get(entry.name, 0)
        if number != 1:
            problems.append(f"{entry.name} ({stage}): {number} Ergebnisse statt genau einem")
        else:
            recorded = next(test for test in ctx.report.tests if test["name"] == entry.name)
            if recorded["stage"] != stage:
                problems.append(f"{entry.name}: Ergebnis in Stufe {recorded['stage']}, erwartet {stage}")
    for name, number in counts.items():
        if is_batch_name(name) and name not in expected_names:
            problems.append(f"{name}: Ergebnis ohne Eintrag für den Job {ctx.job}")
    for entry, stage in expected:
        for fixture in entry.fixtures:
            if ctx.fixture_provider(fixture) is None:
                problems.append(f"{entry.name}: unbekannte Fixture {fixture}")
    if ctx.cargo.lib_ok and ctx.cargo.paths is not None:
        all_names = ctx.signals.names()
        for name, paths in sorted(ctx.cargo.by_leaf.items()):
            if not is_batch_name(name):
                continue
            if name not in all_names:
                problems.append(f"{name}: im Testprogramm ({paths[0]}), aber ohne Eintrag in expected_signals.txt")
            if len(paths) > 1:
                problems.append(f"{name}: mehrfach im Testprogramm ({', '.join(paths)})")
        for entry, stage in expected:
            if entry.kind != "lib":
                continue
            paths = ctx.cargo.by_leaf.get(entry.name, [])
            if not paths:
                if not entry.pending:
                    problems.append(f"{entry.name} ({stage}): fehlt im Testprogramm dieser Plattform")
                continue
            if entry.pending:
                notes.append(f"{entry.name}: im Code, steht aber noch in '{SECTION_PENDING}' (Paket {entry.package})")
            if entry.path and paths != [entry.path]:
                problems.append(f"{entry.name}: Pfad {', '.join(paths)} statt {entry.path}")
            actually_ignored = paths[0] in ctx.cargo.ignored
            if actually_ignored != entry.ignored:
                problems.append(f"{entry.name}: #[ignore] im Code = {actually_ignored}, Option ignored = {entry.ignored}")
    return problems, notes


def rust_functions() -> set[str]:
    """Every ``fn`` name under native/src (for entries without a batch prefix, such as mcp_task_*)."""
    names: set[str] = set()
    for path in (NATIVE / "src").rglob("*.rs"):
        names.update(re.findall(r"\bfn\s+(\w+)", path.read_text(encoding="utf-8", errors="replace")))
    return names


def source_names() -> dict[str, set[str]]:
    """Batch names in the sources: Rust functions, Kotlin tests (methods and classes)."""
    prefix = "|".join(re.escape(item) for item in BATCH_PREFIXES)
    rust_re = re.compile(r"\bfn\s+((?:" + prefix + r")\w*)")
    kotlin_re = re.compile(r"\bfun\s+`?((?:" + prefix + r")\w*)`?\s*\(")
    found: dict[str, set[str]] = {"rust": set(), "junit": set(), "instr": set()}
    for path in (NATIVE / "src").rglob("*.rs"):
        found["rust"].update(rust_re.findall(path.read_text(encoding="utf-8", errors="replace")))
    for key, roots in (("junit", ANDROID_UNIT_PACKAGE_ROOTS), ("instr", ANDROID_TEST_PACKAGE_ROOTS)):
        for root in roots:
            base = REPO / root
            if base.is_dir():
                for path in list(base.rglob("*.kt")) + list(base.rglob("*.java")):
                    found[key].update(kotlin_re.findall(path.read_text(encoding="utf-8", errors="replace")))
    return found


def inventory_sources(ctx: JobContext) -> tuple[list[str], list[str]]:
    """Source part (job prepare): every batch name in the code has an entry, and every entry of
    the section [umgesetzt] exists in the code."""
    problems: list[str] = []
    notes: list[str] = []
    found = source_names()
    unit_index = kotlin_index(ANDROID_UNIT_PACKAGE_ROOTS)
    test_index = kotlin_index(ANDROID_TEST_PACKAGE_ROOTS)
    checks = set(ctx.checks()) | {INVENTORY}
    scripts: dict[str, str] = {}
    listed = ctx.signals.names()
    other_functions: set[str] | None = None
    for kind, names in found.items():
        for name in sorted(names - listed):
            problems.append(f"{name}: im Code ({kind}), aber ohne Eintrag in expected_signals.txt")
    for name in sorted(name for name in checks if is_batch_name(name) and name not in listed):
        problems.append(f"{name}: Prüfung im Register CHECKS ohne Eintrag in expected_signals.txt")
    for entry in ctx.signals.entries:
        if entry.kind == "lib":
            if is_batch_name(entry.name):
                exists = entry.name in found["rust"]
            else:
                if other_functions is None:
                    other_functions = rust_functions()
                exists = entry.name in other_functions
        elif entry.kind == "modul":
            exists = True  # resolved against the compiled tests of the Windows job
        elif entry.kind == "check":
            exists = entry.name in checks
        elif entry.kind == "skript":
            if entry.script not in scripts:
                script = REPO / entry.script
                scripts[entry.script] = script.read_text(encoding="utf-8", errors="replace") if script.is_file() else ""
            exists = f'"{entry.name}"' in scripts[entry.script] or f"'{entry.name}'" in scripts[entry.script]
        elif entry.kind == "junit":
            exists = (entry.name in unit_index.classes) if entry.is_class else (entry.name in unit_index.methods)
        else:
            exists = (entry.name in test_index.classes) if entry.is_class else (entry.name in test_index.methods)
        if entry.pending and exists:
            # A name may be implemented on one platform and pending on another (two lines).
            if not any(not other.pending for other in ctx.signals.by_name(entry.name)):
                notes.append(f"{entry.name}: im Code, steht aber noch in '{SECTION_PENDING}' (Paket {entry.package})")
        elif not entry.pending and not exists:
            problems.append(f"{entry.name} (Zeile {entry.line}, art={entry.kind}): steht in '{SECTION_DONE}', "
                            "fehlt aber im Code")
    return problems, notes


# --------------------------------------------------------------------------- report job

def aggregate_reports(ctx: JobContext, reports_dir: Path, jobs: list[str], needs: dict) -> None:
    """Job report: one overview over every job; a job without report.json becomes not_run
    for each of its expected entries (D3: linux-2204 when the image is not offered)."""
    overview = {"candidate_sha": ctx.candidate_sha, "jobs": {}}
    lines = ["## rechte-desktop-tresore-task · Gesamtübersicht", "",
             "| Job | Ergebnis des Jobs | bestanden | gescheitert | nicht ausgeführt |", "|---|---|---:|---:|---:|"]
    failing = False
    for job in jobs:
        result = (needs.get(job) or {}).get("result", "unbekannt")
        # Exact folder only: rdt-report-android-device must never pick up rdt-report-android-device-api30.
        candidates = [reports_dir / f"rdt-report-{job}" / "report.json"]
        data = None
        for candidate in candidates:
            try:
                data = json.loads(candidate.read_text(encoding="utf-8"))
                break
            except (OSError, ValueError):
                continue
        if data is None:
            reason = (f"kein Bericht: Job {job} endete mit '{result}' vor dem Einstieg"
                      + ("; laut D3 und C2 (Annahme zu A10): Abbild ubuntu-22.04 nicht verfügbar"
                         if job == "linux-2204" else ""))
            tests = [{"name": entry.name, "stage": stage, "status": NOT_RUN, "reason": reason, "duration_s": 0.0}
                     for entry, stage in ctx.signals.for_job(job)]
            data = {"job": job, "candidate_sha": ctx.candidate_sha, "tests": tests, "stages": [],
                    "synthesized": True}
            if job != "linux-2204" and result in ("failure", "cancelled"):
                failing = True
        counts = collections.Counter(test["status"] for test in data.get("tests", []))
        if counts[FAILED]:
            failing = True
        overview["jobs"][job] = {"result": result, "report": data}
        lines.append(f"| {job} | {result} | {counts[PASSED]} | {counts[FAILED]} | {counts[NOT_RUN]} |")
        for test in data.get("tests", []):
            ctx.report.tests.append(dict(test, stage=f"{job}:{test['stage']}"))
    (ctx.log_root / "overall.json").write_text(json.dumps(overview, indent=2, ensure_ascii=False) + "\n",
                                                encoding="utf-8")
    ctx.report.write()
    text = "\n".join(lines) + "\n"
    (ctx.log_root / "overall.md").write_text(text, encoding="utf-8")
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as handle:
            handle.write(text + "\n")
    ctx.state["overall_failing"] = failing
