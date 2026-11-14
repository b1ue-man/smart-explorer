#!/usr/bin/env python3
"""Task-level suite of the crate split (plan docs/plaene/2026-11-01-idealzustand,
D13) and the shared task setup (D12). Run only on GitHub Actions
(.github/workflows/crates-task.yml); never on a developer workstation.

Parts:
  workspace  every crate of the workspace except the Android-only bridge:
             the crates' tests (with test-support), then the programs and
             their integration tests in their own invocation (without it),
             on Linux and Windows (MSVC). Proves that the layers
             core/storage/transfer/engine, the GUI, the CLI, the mobile
             facade and the programs compile and link (every port has its
             provider, the lower crates' own tests their stand-ins) and that
             the tests that moved between crates still pass. Then the crate
             tests that are #[ignore] because they need an environment a
             plain run lacks, each group with that environment and on the OS
             its former suite used (IGNORED_GROUPS below): isolated app data
             and the suite switches (direct opening, sync paths and links;
             both OS), the egui layout harness with its visuals folder
             (Linux), real ACL, token and junction fixtures with the administrator
             token and SeBackupPrivilege (Windows). On Linux the program
             test that needs a systemd manager (se_cli, containment
             self-test) runs last. Tests that need another suite's
             resources (live servers, containers, a Google account, the
             candidate se, mounts and removable volumes, the runner's LAN
             interface, a GPU, codec files, Media Foundation, Dokany, a
             desktop session, the clipboard) are
             #[ignore = "<need>; Suite <name>"] and run in that suite
             (copy-paste-task, fixes-2026-10-28-task, mount-batching-task,
             mount-optimization-task, rechte-desktop-tresore-task, review-task,
             speed-setup-task, sync-reliability-task, transfer-engine-task,
             windows-remote-task; analytics-access-task as well for the read
             helper), not here.
  servers    (Linux) the tests of the two packages outside the workspace: the
             Share signaling/relay server (share-server, with its
             tests/relay_startup.rs) and the SSH agent (se-agent, its
             confined-process tests).
  windows-gnu  on Linux with MinGW-w64: compile and link every test binary and
             program for x86_64-pc-windows-gnu with GNU ld, whose archive
             order is the one a port provider above its accessor depends on
             (rustc's symbols.o names every #[no_mangle] symbol before the
             rlibs, rust-lang/rust#95604).
  android    the Android JNI library (native/android-bridge over the facade
             and the mobile crate) for one ABI (--abi arm64-v8a or x86_64, the
             two the release builds): compile and link the cdylib; with
             arm64-v8a also configure the Gradle app, which reads its version
             from the workspace manifest (version.workspace, D13), with the
             rustls verifier Maven repository the release build passes
             (-PrustlsVerifierMaven, android/build-release-apk.sh), then
             compile its Kotlin sources and run its JVM unit tests.
  agent      the SSH-agent bundles through native/build-agent-bundles.sh, the
             path the release's prepare job takes (se-agent includes the
             shared protocol sources from crates/storage and crates/core).
  s4         the S4 measurement (plan 2026-11-08, E30) of the ignored tests
             s4_messung_task_*: single-core AEAD throughput of the three AEAD
             methods and the loopback throughput of a Share transfer, with the
             share of encryption in a transfer (line `S4-MESSUNG`) and the
             threshold of 10 %. One job per platform, run alone on its runner
             (Linux x86_64, Linux ARM64, Windows x86_64), with the speed profile
             that ships the release optimization.
  e2e        (Linux) the code renewal across processes (native/test-share-code-renewal-e2e.sh,
             E17): share server, host, an online contact, an offline contact that
             returns, the published v0.5.195 `se` (no code_renewal_v1, downloaded
             from the GitHub Release page and checked by SHA-256) and a removed
             contact. It builds the development `se` and share server first.
  verbindungen  the measurements M1 to M4 and M6 to M9 of the plan 2026-11-14 "Verbindungen je
             Zweck" (C7, package P8): the ignored tests `verbindungen_messung_task_*` of the engine,
             built with the speed profile, alone on their runner (Linux x86_64 with netem, Linux
             ARM64, Windows x86_64), as the s4 part runs. Linux x86_64 shapes the loopback with the
             profiles `lan`, `wan` and `loss` of native/rdt_task/speed_net.py (imported unchanged) and
             `loss1` (1 % loss, 40 ms round trip, 100 Mbit/s, native/crates_task_verbindungen.py) for
             M2, M3, M7 and M8; M1, M4 and M6 run on every platform without shaping. Cisco's OpenH264
             (the pinned file of openh264_pins.rs, SE_SCREEN_TASK_OPENH264) serves the H.264 cases of
             Linux x86_64 and Windows; Windows also measures the Media Foundation software coder,
             Linux ARM64 has no pin (its H.264 lines say `nicht_verfuegbar`). Every line
             `VERBINDUNGEN-MESSUNG` goes to the step summary and to native/target/crates-task-logs/
             verbindungen-messung.txt; a measurement that did not measure (a line missing or twice,
             `status=fehler`, another link or platform, a test that failed) fails the part. The
             decision rules of C7 and D4 turn the lines into `VERBINDUNGEN-BEFUND` lines: the
             acceptances E2, E4 and E7 fail the part when not passed (the next run of the same suite
             confirms the corrected value, D4 N1), the conditions of the packages B1 to B5 and of the
             decisions of E5, E6, E10, E11 never fail it (native/crates_task_verbindungen.py).
  verbindungen-gesamt  (Linux, after the three `verbindungen` jobs) reads their line files (--lines-dir)
             and decides what needs every platform: DRIVER_RATE (80 % of the slowest platform's
             single-connection rate), the send bundle, four connections against one (MAX_EXTRA) and
             the ACK frequency (B5: at least two platforms).

  timings    only the compile stage of `workspace` (no test runs), for measuring
             where the build time goes (workflow input part=compile-timings).

Every command streams its output into the step log as it runs, each line with
the seconds since the command started, and into its log file under
native/target/crates-task-logs; a resource line (memory, commit/swap, CPU,
the largest compiler and linker processes) follows every 30 s. The compile
stage reports each finished unit with its time and writes Cargo's timing
report (native/target/cargo-timings). Every command's tail goes to the step
summary; a failing command fails the part after all of its commands ran.
"""
import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / "native"
WINDOWS = platform.system() == "Windows"
SUMMARY = os.environ.get("GITHUB_STEP_SUMMARY")
CRATES = ["--workspace", "--exclude", "smart_explorer_android", "--exclude", "smart_explorer"]


def summary(text):
    print(text, flush=True)
    if SUMMARY:
        with open(SUMMARY, "a", encoding="utf-8") as handle:
            handle.write(text + "\n")


RESOURCE_PERIOD = 30
# Compilers, linkers (MSVC link, rust-lld, GNU ld through cc/collect2, MinGW ld as
# ps truncates it) and the Windows Defender engine.
WATCHED = ("rustc", "cargo", "link", "lld", "rust-lld", "cc", "collect2", "ld", "x86_64-w64-ming",
           "msmpeng", "mpdefendercoreservice")


def resources_windows():
    import ctypes
    from ctypes import wintypes

    class MemoryStatus(ctypes.Structure):
        _fields_ = [("dwLength", wintypes.DWORD), ("dwMemoryLoad", wintypes.DWORD),
                    ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                    ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                    ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                    ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]

    status = MemoryStatus()
    status.dwLength = ctypes.sizeof(MemoryStatus)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        return "memory: GlobalMemoryStatusEx failed"
    gib = 1024 ** 3
    memory = (f"memory load {status.dwMemoryLoad}%, physical free {status.ullAvailPhys / gib:.1f} of "
              f"{status.ullTotalPhys / gib:.1f} GiB, commit free {status.ullAvailPageFile / gib:.1f} of "
              f"{status.ullTotalPageFile / gib:.1f} GiB")
    # Hard page faults and the CPU of the processes that compile, link or scan files.
    script = ("$ErrorActionPreference='SilentlyContinue';"
              "$c=(Get-Counter '\\Memory\\Pages Input/sec','\\Processor(_Total)\\% Processor Time').CounterSamples;"
              "'pages-in/s {0:N0}, cpu {1:N0}%' -f $c[0].CookedValue,$c[1].CookedValue;"
              "Get-Process | Where-Object { @('" + "','".join(WATCHED) + "') -contains $_.ProcessName.ToLower() } |"
              " Sort-Object WorkingSet64 -Descending | Select-Object -First 8 |"
              " ForEach-Object { '{0}({1}) ws {2:N0} MiB cpu {3:N0} s' -f $_.ProcessName,$_.Id,($_.WorkingSet64/1MB),$_.CPU }")
    probe = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                           encoding="utf-8", errors="replace", timeout=25)
    return memory + "; " + "; ".join(line.strip() for line in probe.stdout.splitlines() if line.strip())


def cpu_times_linux():
    fields = [int(value) for value in Path("/proc/stat").read_text().split("\n", 1)[0].split()[1:]]
    return fields[3] + fields[4], sum(fields)


def resources_linux(previous):
    info = {}
    for line in Path("/proc/meminfo").read_text().splitlines():
        key, value = line.split(":", 1)
        info[key] = int(value.split()[0]) * 1024
    gib = 1024 ** 3
    idle, total = cpu_times_linux()
    busy = ""
    if previous[0] is not None:
        spent = total - previous[0][1]
        if spent > 0:
            busy = f", cpu {100 * (1 - (idle - previous[0][0]) / spent):.0f}%"
    previous[0] = (idle, total)
    processes = subprocess.run(["ps", "-eo", "comm,pid,rss,time", "--sort=-rss"], stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, text=True, errors="replace").stdout.splitlines()[1:]
    top = [line.split() for line in processes if line.split() and line.split()[0].lower() in WATCHED][:8]
    return (f"memory free {info.get('MemAvailable', 0) / gib:.1f} of {info.get('MemTotal', 0) / gib:.1f} GiB, "
            f"swap used {(info.get('SwapTotal', 0) - info.get('SwapFree', 0)) / gib:.1f} of "
            f"{info.get('SwapTotal', 0) / gib:.1f} GiB{busy}; "
            + "; ".join(f"{name}({pid}) rss {int(rss) // 1024} MiB cpu {cpu}" for name, pid, rss, cpu in top))


class Stream:
    """One command's output: every line to the step log with the seconds since the
    command started, to its log file, and kept for the summary."""

    def __init__(self, title):
        log = NATIVE / "target" / "crates-task-logs"
        log.mkdir(parents=True, exist_ok=True)
        self.path = log / (re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-") + ".log")
        self.handle = open(self.path, "w", encoding="utf-8")
        self.lines = []
        self.started = time.monotonic()
        self.lock = threading.Lock()

    def write(self, line, keep=True):
        stamped = f"[{time.monotonic() - self.started:7.0f}s] {line}"
        with self.lock:
            if self.handle.closed:  # an abandoned reader (see run) writes no more
                return
            print(stamped, flush=True)
            self.handle.write(stamped + "\n")
            self.handle.flush()
            if keep:
                self.lines.append(line)

    def close(self):
        with self.lock:
            self.handle.close()


def sample_resources(stream, stop):
    previous = [None]
    while not stop.wait(RESOURCE_PERIOD):
        try:
            text = resources_windows() if WINDOWS else resources_linux(previous)
        except Exception as error:  # the sampler must never end the suite
            text = f"resource sample failed: {error!r}"
        stream.write("[resources] " + text, keep=False)


def cargo_message(stream, line):
    """Renders one line of `cargo --message-format json`: diagnostics as the
    compiler printed them, each finished unit as one line."""
    try:
        message = json.loads(line)
    except ValueError:
        stream.write(line)
        return
    reason = message.get("reason")
    if reason == "compiler-message":
        for rendered in (message.get("message", {}).get("rendered") or "").rstrip().splitlines():
            stream.write(rendered)
    elif reason == "compiler-artifact":
        target = message.get("target", {})
        profile = message.get("profile", {})
        kind = "+".join(target.get("kind", []))
        test = " (test)" if profile.get("test") else ""
        if message.get("fresh"):
            return
        stream.write(f"finished {message.get('package_id', '?').split('#')[-1]} {kind} {target.get('name')}{test}")
    elif reason == "build-finished":
        stream.write(f"build finished, success={message.get('success')}")


# Seconds the output of a command may stay open after the command ended. A
# process it started and left running inherited the pipe and keeps it open
# (on Windows every inheritable handle passes to every child): crates-task run
# 42308441438 waited 78 min, until the job limit, after the programs' tests
# had ended in 11 s with one leaky test; run 41437549919 spent its 3:50 h the
# same way. After this time the leftovers are named and ended.
DRAIN_AFTER_EXIT = 30


def leftover_processes():
    """Processes still running from this build's target directory (test
    binaries and the programs they started), as (pid, description)."""
    target = str(NATIVE / "target")
    found = []
    if WINDOWS:
        script = (f"$t = '{target}\\*'; Get-CimInstance Win32_Process | "
                  "Where-Object { $_.ExecutablePath -like $t } | "
                  "ForEach-Object { \"$($_.ProcessId)`t$($_.Name) $($_.CommandLine)\" }")
        probe = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL,
                               text=True, encoding="utf-8", errors="replace", timeout=60)
        for row in probe.stdout.splitlines():
            pid, _, text = row.partition("\t")
            if pid.strip().isdigit():
                found.append((int(pid), text.strip()[:300]))
        return found
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            exe = os.readlink(entry / "exe")
            if exe.startswith(target + "/"):
                arguments = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
                found.append((int(entry.name), arguments.strip()[:300]))
        except OSError:
            continue
    return found


def end_processes(processes):
    for pid, _ in processes:
        try:
            if WINDOWS:
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(pid)], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, timeout=60)
            else:
                os.kill(pid, 9)
        except (OSError, subprocess.SubprocessError):
            continue


def run(title, command, cwd=NATIVE, env=None, cargo_json=False, expect=(), collect=None):
    """Runs one command. `expect` names tests that must each be reported as passed
    exactly once (`test <path>::<name> ... ok`): a test that was renamed or lost
    its cfg then fails here instead of silently shrinking the selection. `collect`, a list,
    receives the output lines of the command (without the time stamps)."""
    summary(f"### {title}\n`{' '.join(command)}`")
    stream = Stream(title)
    stop = threading.Event()
    sampler = threading.Thread(target=sample_resources, args=(stream, stop), daemon=True)
    sampler.start()
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               stdin=subprocess.DEVNULL, text=True, encoding="utf-8", errors="replace",
                               bufsize=1)

    def pump():
        for raw in process.stdout:
            line = raw.rstrip("\n")
            if cargo_json and line.startswith("{"):
                cargo_message(stream, line)
            else:
                stream.write(line)

    reader = threading.Thread(target=pump, daemon=True)
    reader.start()
    returncode = process.wait()
    reader.join(timeout=DRAIN_AFTER_EXIT)
    if reader.is_alive():
        try:
            leftovers = leftover_processes()
        except Exception as error:  # naming the leftovers must never end the suite
            stream.write(f"[runner] listing the processes failed: {error!r}")
            leftovers = []
        stream.write(f"[runner] the command ended with exit code {returncode}, but {len(leftovers)} "
                     f"process(es) it left running still hold its output; ending them:")
        for pid, text in leftovers:
            stream.write(f"[runner]   {pid} {text}")
        summary(f"left running after the command (ended by the suite): "
                + ("; ".join(text for _, text in leftovers) or "no process of the target directory found"))
        end_processes(leftovers)
        reader.join(timeout=DRAIN_AFTER_EXIT)
        if reader.is_alive():
            stream.write("[runner] the output is still open; going on without it")
    stop.set()
    sampler.join(timeout=RESOURCE_PERIOD)
    elapsed = time.monotonic() - stream.started
    stream.close()
    lines = stream.lines
    if collect is not None:
        collect.extend(lines)
    # libtest ("test result:", "... FAILED") and cargo-nextest ("Summary [",
    # "FAIL [", "TIMEOUT [", "SIGSEGV [", "ABORT [") report lines.
    results = [line for line in lines if line.startswith("test result:") or "Summary [" in line]
    failures = [line for line in lines if re.match(r"^test .+ \.\.\. FAILED$", line)
                or re.match(r"^\s*(FAIL|TIMEOUT|SIGSEGV|SIGABRT|SIGKILL|ABORT|LEAK-FAIL) \[", line)
                or line.startswith("error") or "panicked at" in line]
    summary(f"exit code {returncode} after {elapsed / 60:.1f} min")
    if results:
        summary("\n".join(f"- {line}" for line in results[-40:]))
    if returncode != 0:
        tail = failures[-80:] or lines[-60:]
        summary("```\n" + "\n".join(tail) + "\n```")
    unreported = []
    for name in expect:
        passed = [line for line in lines if re.match(r"^test (.*::)?" + re.escape(name) + r" \.\.\. ok\b", line)]
        if len(passed) != 1:
            unreported.append(f"{name} ({len(passed)} times)")
    if unreported:
        summary("expected to pass exactly once, but not reported so: " + ", ".join(unreported))
    return returncode == 0 and not unreported


def compile_stage():
    """Builds every test binary of the crates and of the programs without running
    them, with Cargo's timing report and one line per finished unit, so the time
    of each unit is known even when the job is cut off."""
    timed = ["--no-run", "--timings", "--message-format", "json"]
    crates = run("Compile the tests of every crate", ["cargo", "test", "--locked", *CRATES, *timed], cargo_json=True)
    programs = run("Compile the programs and their integration tests",
                   ["cargo", "test", "--locked", "-p", "smart_explorer", *timed], cargo_json=True)
    return crates, programs


def part_timings():
    return all(compile_stage())


def keep_junit(name):
    """Keeps nextest's JUnit report (per-test times and outcomes) of the last run
    beside the logs; the next run overwrites target/nextest/default/junit.xml."""
    report = NATIVE / "target" / "nextest" / "default" / "junit.xml"
    if report.is_file():
        logs = NATIVE / "target" / "crates-task-logs"
        logs.mkdir(parents=True, exist_ok=True)
        (logs / f"{name}.junit.xml").write_bytes(report.read_bytes())


# The share server's limit on the lookup ids one client publishes at a time (share-server/src/limits.rs) and
# the copy the engine keeps for its renewal state (code_renewal_state.rs) must be one integer: the engine
# defers a renewal before the server would refuse a 65th lookup id (E52, E5).
PUBLISHED_DIRECTS_CONSTANTS = (
    (ROOT / "share-server" / "src" / "limits.rs", "MAX_PUBLISHED_DIRECTS_PER_CLIENT"),
    (NATIVE / "crates" / "engine" / "src" / "share" / "core" / "code_renewal_state.rs",
     "SERVER_PUBLISHED_DIRECTS_PER_CLIENT"),
)


def published_directs_check():
    """Static check on source text: both constants are integer literals and equal. Runs once, on Linux,
    next to the test register; a missing literal or a difference fails the part."""
    values = []
    for path, name in PUBLISHED_DIRECTS_CONSTANTS:
        found = re.search(r"\bconst\s+" + name + r"\s*:\s*\w+\s*=\s*(\d+)\s*;", path.read_text(encoding="utf-8"))
        if not found:
            summary(f"### Published-lookup limit\n{name} is no integer literal in {path.relative_to(ROOT)}")
            return False
        values.append((name, int(found.group(1))))
    if values[0][1] != values[1][1]:
        summary(f"### Published-lookup limit\n{values[0][0]} = {values[0][1]} differs from "
                f"{values[1][0]} = {values[1][1]}")
        return False
    summary(f"### Published-lookup limit\n{values[0][0]} and {values[1][0]} are both {values[0][1]}")
    return True


def part_workspace():
    # The test register (docs/tests/register.tsv) names every test with the requirement it
    # protects and its suite; a test without a row or a row without a test fails here. It reads
    # source text only, so Linux runs it once for both OS.
    registered = WINDOWS or run("Test register", [sys.executable, str(ROOT / "native" / "test-register.py")],
                                cwd=ROOT)
    constants = WINDOWS or published_directs_check()
    crates_built, programs_built = compile_stage()
    # The crates' tests run as their binaries were written for: each binary in
    # one process, one test after the other, as build.yml runs them. They keep
    # machine-wide state apart with process-wide serial numbers and locks (the
    # app data directory, HKCU, the volume's free space, the sync job folder);
    # in nextest's one process per test those collided (run 42308441438: 14
    # failures that one process does not have). The programs' tests run with
    # cargo-nextest (native/.config/nextest.toml), one at a time: each in its
    # own process, a test running five minutes is ended and named, and a test
    # that leaves a process running is named as leaky.
    threads = ["--test-threads=1"]
    nextest = ["cargo", "nextest", "run", "--locked", "--no-fail-fast"]
    # The crates' own tests build the layers with feature test-support (their
    # dev-dependencies); the programs and their integration tests build in a
    # separate invocation without it, so `se`/`smart_explorer` started by
    # native/tests run the production paths as in a release.
    # The local loopback fixtures of the copy/paste and Windows-remote tests
    # refuse to start unless their switch is set; they need nothing else.
    env = dict(os.environ, SMART_EXPLORER_COPY_PASTE_TASK="1", SMART_EXPLORER_WINDOWS_REMOTE_TASK="1")
    # Tests that need resources only their own suites provide are #[ignore = "<need>; Suite <name>"]:
    # a plain run does not start them, and the suite named there does (module docstring).
    # A test binary that did not compile is not run; its compile log above is the result.
    ok = crates_built and run("Tests of every crate",
                              ["cargo", "test", "--locked", *CRATES, "--no-fail-fast", "--", *threads],
                              env=env, expect=batch_expected())
    ok = (programs_built and run("Programs and their integration tests",
                                 [*nextest, "-p", "smart_explorer"])) and ok
    keep_junit("programs-and-their-integration-tests")
    # The #[ignore] tests that need their own environment reuse the crates' test binaries.
    ok = (crates_built and part_ignored(env)) and ok
    ok = (crates_built and part_root_ignored()) and ok
    ok = (programs_built and part_programs_ignored()) and ok
    return ok and registered and constants


def part_programs_ignored():
    """The program test that needs a Linux host with a systemd manager and cgroup v2: the real
    containment self-test of `se --share-exec-platform-self-test` (native/tests/se_cli/process.rs).
    It is #[ignore] so that a host without the manager cannot pass it without checking; here a
    missing manager fails it. Linux only; the Windows twin (Job Object) needs nothing and runs
    with the other program tests."""
    if WINDOWS:
        return True
    name = "real_systemd_cgroup_exec_provider_contains_and_drains_processes"
    return run("Programs: containment on a systemd host (ignored test)",
               ["cargo", "test", "--locked", "-p", "smart_explorer", "--test", "se_cli", "--no-fail-fast", "--",
                "--test-threads=1", "--ignored", name], expect=[name])


# The tests of the batch "open points of v0.5.196" (plan 2026-11-08, milestones M1 to M18), by the
# operating system whose "Tests of every crate" run must report each of them as passed exactly once.
# A test whose module was left out, lost its cfg or was renamed fails the run here instead of silently
# shrinking it. The #[ignore] tests of the batch run in IGNORED_GROUPS, ROOT_TESTS and the s4 part;
# the tests of share-server in `servers`; the Kotlin tests in `android`. docs/tests/register.tsv names
# the requirement each protects.
BATCH_TESTS = {
    "both": [
        "batches_p13_legacy_profiles_read_historical_update_state_without_mutation",
        "cli_task_installation_detects_terminal_only_and_desktop",
        "code_renewal_grenze_task_a_waiting_removal_is_reported_as_deferred",
        "code_renewal_grenze_task_more_than_four_generations_run_before_a_removal_waits",
        "code_renewal_grenze_task_the_bound_is_the_server_lookup_limit_minus_the_current_id",
        "code_renewal_grenze_task_the_deferral_ends_when_one_generation_ends",
        "fs_fehler_task_connection_root_is_named_by_virtual_path",
        "fs_tresor_task_sibling_with_shared_name_prefix_stays_shareable",
        "fs_tresor_task_spellings_of_a_mounted_drive_are_private",
        "lan_kuerzung_task_a_sighting_without_any_signed_address_is_dropped",
        "lan_kuerzung_task_no_address_outside_the_signed_list_is_ever_adopted",
        "lan_kuerzung_task_the_candidate_path_takes_a_v3_sighting_only_with_every_address_signed",
        "lan_kuerzung_task_the_signature_and_the_other_checks_are_unchanged",
        "lan_kuerzung_task_the_signed_address_stays_and_the_foreign_one_is_dropped",
        "lan_kuerzung_task_the_strict_verifier_takes_a_mixed_sighting_like_the_candidate_path",
        "login_selbst_task_device_named_twice_keeps_one_grant",
        "login_selbst_task_foreign_account_rejected",
        "login_selbst_task_grant_only_for_own_account",
        "login_selbst_task_hash_mismatch_aborts_and_keeps_old_state",
        "login_selbst_task_install_keeps_identity_and_grants",
        "login_selbst_task_last_revocation_plans_removal",
        "login_selbst_task_network_follows_changed_server",
        "login_selbst_task_revocation_reaches_an_older_running_service",
        "login_selbst_task_revocation_removes_grant_and_ends_session",
        "login_selbst_task_trickling_caller_is_cut_at_the_deadline",
        "login_selbst_task_untrusted_channel_is_unreachable_not_refused",
        "login_selbst_task_version_change_sets_up_exactly_once",
        "login_selbst_task_watch_follows_server_and_broker_restart",
        "login_ziel_task_book_survives_a_reload",
        "login_ziel_task_host_offers_only_with_steering_and_own_grant",
        "login_ziel_task_older_peer_gets_a_clear_message",
        "login_ziel_task_open_forms_keep_the_file_form",
        "login_ziel_task_revocation_deletes_the_target",
        "login_ziel_task_switches_on_secure_desktop_and_back",
        "login_ziel_task_viewer_accepts_only_the_pinned_session_of_the_device",
        "login_ziel_task_wire_stays_bounded_and_strict",
        "rdp_netz_task_state_a_sharing_the_person_set_up_is_not_switched_off_by_se",
        "rdp_netz_task_state_an_enabled_sharing_without_listener_still_fails",
        "rdp_netz_task_state_recorded_done_is_a_failure_only_without_a_listener",
        "rdp_netz_task_state_switched_off_sharing_is_never_failed",
        "rdp_netz_task_state_switched_off_sharing_with_a_done_marker_is_done",
        "rdp_netz_task_state_the_switched_off_condition_needs_all_three_facts",
        "rueckfragen_task_answered_anchor_passes_the_window_to_the_next_request",
        "rueckfragen_task_answered_notified_request_passes_the_notification_on",
        "rueckfragen_task_cli_only_request_lets_the_next_one_ask_again",
        "rueckfragen_task_each_request_of_a_device_is_decided_alone",
        "rueckfragen_task_ended_window_lets_the_next_request_open_a_new_one",
        "rueckfragen_task_equal_requests_share_one_always_answer",
        "rueckfragen_task_equal_requests_share_one_denial",
        "rueckfragen_task_once_answers_only_the_chosen_equal_request",
        "rueckfragen_task_other_grant_revision_is_no_equal_request",
        "rueckfragen_task_requests_of_two_devices_show_two_windows",
        "rueckfragen_task_ten_requests_of_one_device_show_one_window",
        "rueckfragen_task_window_ended_with_its_own_request_open_shows_nothing_again",
        "rueckfragen_task_window_whose_own_request_ended_shows_the_rest_again",
        "rueckfragen_task_withdrawn_anchor_passes_the_window_on",
        "rueckfragen_task_without_a_surface_nobody_is_asked_and_nothing_joins",
        "unbelegt_task_candidates_exclude_loopback_multicast_and_mapped",
        "unbelegt_task_entfernen_ohne_kennung_widerruft_nur_kennungslose_eintraege",
        "unbelegt_task_leere_praesenz_setzt_keinen_pin",
        "unbelegt_task_legacy_widerruf_ohne_kennung_trifft_nur_kennungslose_eintraege",
        "unbelegt_task_route_cap_counts_drops",
        "unbelegt_task_route_cap_counts_each_refused_candidate_once",
        "unbelegt_task_route_cap_keeps_lan_phase",
        "unbelegt_task_route_cap_keeps_round_open",
        "unbelegt_task_unsigned_sighting_drops_multicast",
        "unbelegt_task_wieder_erlauben_erreicht_was_der_widerruf_erreicht",
        "update_worker_task_failed_start_is_an_error_not_a_replacement",
        "update_worker_task_legacy_keeps_signed_plain_http_feeds",
        "update_worker_task_legacy_keeps_unsigned_https_feeds",
        "update_worker_task_legacy_refuses_an_unsigned_plain_http_feed",
        "update_worker_task_running_worker_starts_once_and_is_replaced",
        "update_worker_task_without_running_worker_nothing_starts",
        "vendor_grenze_task_removal_applies_to_the_next_handshake_without_resumption",
        "vendor_grenze_task_removed_key_admitted_again_keeps_connecting",
        "vendor_grenze_task_removed_key_fails_the_handshake_in_both_directions",
        "windows_watch_task_unsupported_host_switches_the_peer_to_polling",
        "windows_watch_task_unsupported_reaches_the_subscriber_distinguishably",
        # The tests of p-erg-1 to p-erg-18 (plan 2026-11-08, M21 to M34, E41) that run on every OS.
        "login_selbst_task_steering_right_gives_exactly_one_grant_for_that_device",
        "login_selbst_task_revocation_empties_the_list",
        "login_selbst_task_past_end_is_not_listed",
        "login_selbst_task_direct_and_room_of_one_device_give_one_entry",
        "login_ziel_task_target_arrives_bound_to_device_and_key",
        "login_ziel_task_without_steering_answers_without_target",
        "login_ziel_task_peer_without_feature_is_a_legacy_peer",
        "login_ziel_task_removed_contact_deletes_the_target",
        "login_selbst_task_rdp_and_login_step_make_one_elevated_call",
        "login_selbst_task_single_step_makes_one_call_with_that_step",
        "login_selbst_task_system_option_repeats_and_single_stays_valid",
        "bildschirm_sitzung_task_session_changed_kodiert_dekodiert_und_unbekannt_lesbar",
        "bildschirm_sitzung_task_plan_einmal_je_sekunde_hoechstens_30_s",
        "bildschirm_sitzung_task_wiederverbindung_ein_handshake_je_versuch_ende_nach_30_s",
        "bildschirm_sitzung_task_wiederverbindung_oeffnet_oder_endet_mit_ablehnung",
        "pin_neu_task_fifth_failure_renews_after_30_seconds_with_new_pin",
        "pin_neu_task_person_stop_creates_no_renewal",
        "pin_neu_task_paired_offer_creates_no_renewal",
        "pin_neu_task_renewal_keeps_original_deadline",
        "pin_neu_task_discoverable_output_names_hint_and_new_pin",
        "dienste_oeffnen_task_wire_names_of_open_and_close",
        "dienste_oeffnen_task_service_reply_carries_the_mapping",
        "dienste_oeffnen_task_open_refuses_a_device_without_feed",
        "dienste_oeffnen_task_close_of_an_unknown_service_is_false",
        "entdeckung_task_a_wrong_secret_changed_clear_fields_or_an_injected_address_are_refused",
        "vault_task_watch_reports_nothing_from_an_unlocked_vault_drive",
        "vault_task_watch_on_an_unlocked_vault_drive_is_a_permission_refusal",
        "rdp_diag_task_feed_diagnostics_serialized_with_field",
        "rdp_diag_task_feed_diagnostics_missing_field_is_empty",
        "rdp_diag_task_feed_diagnostics_present_field_is_read",
        "rdp_diag_task_feed_diagnostics_bounded_to_16_lines_of_300_chars",
        "rdp_diag_task_feed_diagnostics_cut_at_character_boundary",
        "update_worker_task_answering_worker_counts_as_running",
        "update_worker_task_without_proof_or_pong_does_not_count_as_running",
        "rdp_netz_task_state_no_steering_device_left_after_the_last_withdrawal",
        "rdp_netz_task_state_one_steering_device_left_keeps_steering",
        "rueckfragen_task_always_reports_the_other_equal_requests_it_answered",
        "rueckfragen_task_denial_reports_the_other_equal_requests_it_answered",
        "rueckfragen_task_once_reports_no_other_request_as_answered",
        "rueckfragen_task_only_equal_requests_count_as_answered",
        "discovery_grenze_task_over_local_limit_remembers_the_refused_exchange",
        "code_renewal_task_retiring_presence_carries_no_direct_address",
        # Named by p-erg-7 (not changed there; it fails until the lan_presence_task_tests.rs fix outside
        # p-erg-19 is applied).
        "entdeckung_task_a_resolved_sealed_record_with_a_wrong_secret_or_changed_fields_is_refused",
        # The gaps of the second integration pass (package E2 of plan 2026-11-14): N1, N2, N3, N6, N7, N9. None of
        # these tests is bound to one operating system, so both must report them.
        "rueckfragen_task_cli_always_names_the_answered_count",
        "rueckfragen_task_cli_deny_names_the_answered_count",
        "rueckfragen_task_decided_without_count_reads_as_zero",
        "vault_task_mount_status_publishes_the_drive_until_the_host_ended",
        "login_ziel_task_forget_direct_peer_persisted_deletes_the_target",
        "fs_fehler_task_error_under_a_share_names_the_virtual_root",
        "pin_neu_task_stop_in_the_pause_cancels_the_renewal",
        "pin_neu_task_cli_lists_and_stops_the_waiting_renewal",
        "pin_neu_task_android_status_names_the_waiting_renewal",
        "dienste_oeffnen_task_android_close_of_an_unknown_service_names_the_port",
    ],
    "linux": [
        "batches_p13_legacy_profile_generation_serializes_with_terminal_updates",
        "batches_p13_legacy_profiles_preserve_stale_update_lock_in_readonly_installation",
        "batches_p13_legacy_profiles_reject_same_byte_image_replacement",
        "bildschirm_sitzung_task_closed_indicator_of_a_followed_session_ends_its_sessions",
        "bildschirm_sitzung_task_consent_window_asks_in_the_followed_session",
        "bildschirm_sitzung_task_daemon_session_kind_follows_a_switch",
        "bildschirm_sitzung_task_daemon_x11_connects_to_the_followed_display",
        "bildschirm_sitzung_task_foreign_or_ambiguous_xauthority_is_refused",
        "bildschirm_sitzung_task_greeter_session_is_followed",
        "bildschirm_sitzung_task_no_active_session_gives_no_display",
        "bildschirm_sitzung_task_own_session_variables_keep_their_path",
        "bildschirm_sitzung_task_session_switch_ends_the_old_host",
        "bildschirm_sitzung_task_started_host_is_marked_and_held",
        "bildschirm_sitzung_task_wayland_session_runs_as_its_account_with_its_bus",
        "bildschirm_sitzung_task_x11_session_gives_its_display_and_owned_xauthority",
        "bildschirm_sitzung_task_x_server_must_be_root_or_the_session_account",
        "bildschirm_sitzung_task_xauthority_cookie_is_picked_for_the_display",
        "dns_android_task_empty_nameservers_answer_a_through_the_system_at_once",
        "dns_android_task_empty_nameservers_answer_aaaa_through_the_system_at_once",
        "login_selbst_task_linux_account_comes_from_the_kernel",
        "login_selbst_task_linux_gateway_account_is_refused",
        "rdp_netz_task_gnome_a_connection_during_the_grace_moves_the_release",
        "rdp_netz_task_gnome_connection_rows_count_only_accepted_loopback_on_the_rdp_port",
        "rdp_netz_task_gnome_orphan_check_keeps_a_living_owner_and_drops_a_gone_one",
        "rdp_netz_task_gnome_reach_note_names_the_session_only",
        "rdp_netz_task_gnome_status_reads_enabled_only_in_the_rdp_block",
        "rdp_netz_task_gnome_switch_on_without_a_connection_is_due_after_the_grace",
        "rdp_netz_task_gnome_switched_off_sharing_reports_the_session_reason",
        "rdp_netz_task_gnome_switched_off_with_an_active_unit_is_the_off_state",
        "rdp_netz_task_gnome_two_sessions_release_only_after_the_last_one",
        "rdp_netz_task_krdp_argument_is_loopback_and_keeps_other_arguments",
        "rdp_netz_task_krdp_argument_replaces_any_address_form",
        "rdp_netz_task_krdp_argument_with_whitespace_is_refused_not_split",
        "rdp_netz_task_krdp_drop_in_clears_then_sets_the_loopback_command",
        "rdp_netz_task_krdp_drop_in_refuses_arguments_systemd_would_change",
        "rdp_netz_task_krdp_exec_start_from_unit_text_keeps_the_boundaries",
        "rdp_netz_task_krdp_exec_start_is_read_from_show_output",
        "rdp_netz_task_krdp_unit_text_with_prefix_or_two_commands_is_refused",
        "vendor_grenze_task_default_resolver_has_no_public_fallback",
        # The tests of p-erg-1 to p-erg-18 that run on Linux only (platform as stated in their reports).
        "bildschirm_sitzung_task_fensterliste_kommt_aus_dem_sitzungskonto",
        "bildschirm_sitzung_task_abfrage_startet_mit_der_umgebung_des_sitzungskontos",
        "bildschirm_sitzung_task_kodiererangebot_kommt_aus_dem_sitzungskonto",
        "bildschirm_sitzung_task_fremde_oder_verlinkte_tokendatei_wird_nicht_gelesen",
        "bildschirm_sitzung_task_fremdes_oder_verlinktes_symbolverzeichnis_wird_nicht_genutzt",
        "bildschirm_kanal_task_a_window_token_is_refused_outside_its_scope",
        "bildschirm_kanal_task_the_indicator_lists_watches_and_ends_the_worker_sessions",
        "bildschirm_kanal_task_a_decision_answers_exactly_the_requests_of_the_window",
        "bildschirm_kanal_task_the_token_ends_with_the_window_and_with_a_session_switch",
        "bildschirm_kanal_task_the_window_reads_address_and_token_from_variable_and_stdin",
        "rdp_netz_krdp_task_unit_without_loopback_address_gives_evidence_and_note",
    ],
    "windows": [
        "batches_p13_legacy_windows_profile_pins_cli_without_sidecar",
        "fs_tresor_task_windows_identity_alone_decides",
        "fs_tresor_task_windows_verbatim_and_case_spellings_are_private",
        "login_selbst_task_windows_foreign_principals_refused",
        "rdp_netz_task_firewall_default_allow_profile_admits_the_port",
        "rdp_netz_task_firewall_disabled_profile_admits_the_port",
        "rdp_netz_task_firewall_loopback_only_rule_is_not_network_reach",
        "rdp_netz_task_firewall_open_rule_is_reported_in_evidence_and_diagnostics",
        "rdp_netz_task_firewall_program_filter_rule_counts_as_open",
        "rdp_netz_task_firewall_rule_for_another_profile_does_not_admit",
        "rdp_netz_task_firewall_unknown_network_profile_applies_every_rule",
        "rdp_netz_task_firewall_unreadable_and_missing_listener_are_not_guessed",
        "rdp_netz_task_firewall_without_network_rule_is_reported",
        "rdp_netz_task_inventory_reads_firewall_fields_and_older_output",
        "windows_watch_task_confined_reports_create_modify_rename_delete",
        "windows_watch_task_link_child_is_not_followed",
        "windows_watch_task_missing_notifications_end_with_unsupported",
        "windows_watch_task_replaced_root_is_refused",
        # The tests of p-erg-1 to p-erg-18 that run on Windows only (the grant pipe, the vault listing).
        "login_selbst_task_windows_malformed_status_pipe_name_is_dropped",
        "vault_task_windows_listing_hides_unlocked_drive_in_other_spelling",
    ],
}
SERVER_BATCH_TESTS = [
    "server_rand_task_a_copy_keeps_the_moment_it_was_taken",
    "server_rand_task_a_failed_write_keeps_the_copy_pending",
    "server_rand_task_a_registration_that_displaces_a_client_still_succeeds",
    "server_rand_task_a_scope_of_reserved_places_refuses_the_newcomer",
    "server_rand_task_an_evicted_client_reconnects_and_gets_a_place",
    "server_rand_task_concurrent_registrations_leave_every_binding_on_disk",
    "server_rand_task_failed_publication_write_stays_hidden_and_pending",
    "server_rand_task_global_limit_refuses_before_binding_and_saving",
    "server_rand_task_key_limit_is_checked_before_a_device_id_is_bound",
    "server_rand_task_overlapping_saves_leave_the_newer_copy",
    "server_rand_task_publication_reaches_the_disk_before_it_is_visible",
    "server_rand_task_reserve_current_keeps_the_thread_place_until_release",
    "server_rand_task_reserved_place_is_not_evicted_by_a_newcomer",
    "server_rand_task_restart_reads_the_newer_copy_in_either_order",
    "unbelegt_task_binding_budget_per_end_site",
    # The four share-server tests of p-erg-13 (E60, S15).
    "unbelegt_task_legacy_publish_unbound_lookup_refused",
    "unbelegt_task_legacy_watch_bound_lookup_refused",
    "unbelegt_task_legacy_watch_unbound_lookup_stays_admitted_until_bound",
    "unbelegt_task_proven_publish_unchanged",
    # S15 on the request paths (package E2 of plan 2026-11-14, gap N5).
    "unbelegt_task_legacy_request_to_bound_lookup_refused",
    "unbelegt_task_legacy_decision_to_bound_lookup_refused",
    "unbelegt_task_legacy_request_to_unbound_lookup_still_routed",
    # S15 on the signed request paths (review of package E2): request with and without the legacy
    # bridge, decision receipt.
    "unbelegt_task_legacy_bridge_request_to_bound_lookup_refused",
    "unbelegt_task_legacy_signed_request_to_bound_lookup_refused",
    "unbelegt_task_legacy_decision_receipt_for_bound_lookup_refused",
    # Existing share-server tests that p-erg-13 affects: named by the open points of its review (p-erg-18).
    # They fail until the fixes outside p-erg-19 are applied (the review says so).
    "review_task_bound_lookup_requires_access_and_only_its_owner_mutates_it",
    "retained_state_caps_reject_growth_and_cleanup_returns_to_baseline",
    "android_background_task_idle_refresh_waits_for_keepalive_tick",
    "android_background_task_route_change_first_presence_and_expiry_are_immediate",
    "android_background_task_offline_unwatch_and_leave_drop_deferred_refreshes",
    "android_background_task_waking_flushes_before_idle_ack",
    "android_background_task_keepalive_proposal_and_capability_gate",
    "android_background_task_k180_bundles_one_refresh_per_tick_and_copy_never_expires",
    "android_background_task_wall_clock_jump_sends_refresh_immediately",
]


# The new tests of the plan 2026-11-14 "Verbindungen je Zweck" (packages P1, P2, P2b, P3, P4, P5a, P6, P7, K1,
# the repair of the connection drops and the two reviews), by the operating system whose "Tests of every
# crate" run must report each of them as passed exactly once (`run(expect=...)`, as BATCH_TESTS). The
# #[ignore] measuring tests `verbindungen_messung_task_*` are not here: part `verbindungen` runs them. The
# Kotlin tests are in KOTLIN_BATCH_TESTS. docs/tests/register.tsv names the requirement each protects.
VERBINDUNGEN_TESTS = {
    "both": [
        # P1 (vendor send bundles).
        "verbindungen_task_vendor_iroh_transmit_setters_reach_noq_config",
        "verbindungen_task_vendor_transmit_defaults_setters_and_relay_cap",
        # P2 (purpose profiles, yielding controller, interactive presence) and the two yield tests of the transport review.
        "verbindungen_task_profile_accept_with_binds_purpose_and_transfer_priority",
        "verbindungen_task_profile_controller_per_purpose",
        "verbindungen_task_profile_late_bound_purpose",
        "verbindungen_task_profile_per_alpn",
        "verbindungen_task_profile_switches_change_only_their_values",
        "verbindungen_task_profiles_keep_endpoint_values",
        "verbindungen_task_yield_base_delay_from_minute_minima",
        "verbindungen_task_yield_full_window_survives_acknowledgements_in_batches",
        "verbindungen_task_yield_grows_back_below_target",
        "verbindungen_task_yield_inner_window_is_held_while_the_yielding_one_is_in_force",
        "verbindungen_task_yield_loss_halves_once_per_round_trip",
        "verbindungen_task_yield_never_above_cubic",
        "verbindungen_task_yield_shrinks_only_with_presence",
        "verbindungen_task_presence_counts_up_and_down",
        "verbindungen_task_presence_ends_on_abort_and_panic",
        "verbindungen_task_presence_retainer_holds_entry_until_last_drop",
        "verbindungen_task_bulk_no_growth_with_presence",
        # P2b (host download priority, Exec host presence).
        "verbindungen_task_host_download_priority_follows_the_m3_switch",
        "verbindungen_task_host_download_streams_are_exactly_the_data_reads",
        "verbindungen_task_host_download_streams_get_priority_minus_one_replies_keep_zero",
        "verbindungen_task_exec_host_entry_ends_on_error_abort_and_panic",
        "verbindungen_task_exec_host_terminal_job_counts_until_its_guard_drops",
        # P3 (TCP pump) and the two route-pause tests of the transport review.
        "verbindungen_task_tcp_budget_grants_growth_only_within_capacity",
        "verbindungen_task_tcp_budget_returns_after_end",
        "verbindungen_task_tcp_buffer_grows_on_full_read_within_budget",
        "verbindungen_task_tcp_buffer_stays_base_on_short_reads",
        "verbindungen_task_tcp_buffer_stays_base_without_free_budget",
        "verbindungen_task_tcp_half_close_both_ways",
        "verbindungen_task_tcp_idle_expires_without_progress",
        "verbindungen_task_tcp_idle_resets_with_progress",
        "verbindungen_task_tcp_presence_counts_while_pumping",
        "verbindungen_task_tcp_quic_pump_drains_both_halves_under_presence_and_budget",
        "verbindungen_task_tcp_route_pause_holds_idle_time",
        "verbindungen_task_tcp_route_pause_keeps_the_pending_operation",
        # P4 (picture presence, control stream priority).
        "verbindungen_task_bild_control_stream_above_picture_groups",
        "verbindungen_task_bild_presence_follows_a_preview_raised_to_the_full_view",
        "verbindungen_task_bild_presence_only_for_view_and_control",
        # P5a and P7 (station output of the bench).
        "verbindungen_task_bench_stages_field_is_left_out_while_empty",
        "verbindungen_task_bench_stages_summary_gives_p50_and_p95_in_milliseconds",
        "verbindungen_task_cli_bench_stages_in_text",
        # P6 (pointer overlay).
        "verbindungen_task_zeiger_an_inverting_pointer_stays_visible_on_every_background",
        "verbindungen_task_zeiger_two_renderings_give_color_and_alpha",
        "verbindungen_task_zeiger_a_large_pointer_is_thinned_out_to_fit_a_frame",
        "verbindungen_task_zeiger_a_publication_wakes_the_reader_of_the_feed",
        "verbindungen_task_zeiger_a_wish_before_the_capture_is_kept_until_it_can_separate",
        "verbindungen_task_zeiger_flush_writes_frames_the_daemon_reads_back",
        "verbindungen_task_zeiger_frames_round_trip_and_refuse_bad_payloads",
        "verbindungen_task_zeiger_negotiated_only_for_a_full_view_of_an_announcing_host",
        "verbindungen_task_zeiger_the_feed_gives_each_change_once_to_each_reader",
        "verbindungen_task_zeiger_the_host_process_sends_only_changes_and_only_when_switched_on",
        "verbindungen_task_zeiger_the_view_shows_only_a_visible_pointer_of_the_held_shape",
        "verbindungen_task_zeiger_update_frames_of_the_host_process_round_trip",
        "verbindungen_task_zeiger_windows_capture_stops_drawing_the_pointer_with_the_overlay",
        "verbindungen_task_zeiger_a_steering_view_shows_one_pointer_at_the_local_position",
        "verbindungen_task_zeiger_local_pointer_places_the_hot_spot_under_the_mouse",
        "verbindungen_task_zeiger_nothing_is_drawn_for_a_hidden_or_unknown_pointer",
        "verbindungen_task_zeiger_overlay_is_drawn_where_the_host_pointer_is",
        "verbindungen_task_zeiger_mobile_events_carry_what_the_app_draws",
        "verbindungen_task_zeiger_mobile_hidden_position_is_an_event_the_app_reads",
        "verbindungen_task_zeiger_mobile_shape_repeats_only_after_the_pause",
        # K1 (corrections before the suite run).
        "verbindungen_task_relay_replays_the_set_mode_choices_after_session_changed",
        # The repair of the connection drops (relay candidates of one host).
        "verbindungen_task_abbruch_signalisierungsport_wird_nicht_per_qad_zum_relay",
        "verbindungen_task_abbruch_heimrelay_ist_nie_ein_port_ohne_relay",
    ],
    "linux": [
        # P6: the X11 host (XFixes pointer without blending) exists on Linux only.
        # P6 (pointer overlay).
        "verbindungen_task_zeiger_x11_blends_without_the_overlay_and_not_with_it",
        "verbindungen_task_zeiger_x11_sends_the_shape_again_only_when_its_serial_changes",
    ],
    "windows": [],
}


# The tests of the agent plugin repair (2026-11-14: U1 daemon outside the installation,
# U2 launcher relay across updates, U3 profile refresh of an elevated helper for its own installation),
# by the operating system whose "Tests of every crate" run must report each of them as passed exactly
# once (`run(expect=...)`, as BATCH_TESTS). docs/tests/register.tsv names the requirement each protects.
PLUGIN_TESTS = {
    "both": [
        "plugin_task_daemon_of_an_installation_answers_only_for_its_own",
        "plugin_task_daemon_outside_an_installation_admits_the_named_installation",
        "plugin_task_daemon_outside_an_installation_asks_for_the_root",
        "plugin_task_daemon_outside_an_installation_names_the_reason",
        "plugin_task_daemon_refuses_a_named_root_of_another_installation",
        "plugin_task_daemon_refuses_a_relative_root_and_an_installation_being_removed",
        "plugin_task_daemon_refuses_a_root_that_is_no_installation",
        "plugin_task_elevated_helper_refreshes_only_its_own_installation",
        "plugin_task_relay_answers_each_open_request_of_an_ended_runtime_once",
        "plugin_task_relay_announces_tool_changes_only_after_the_capability_was_declared",
        "plugin_task_relay_answers_ping_and_refuses_queued_requests_without_runtime",
        "plugin_task_relay_drops_host_answers_to_an_ended_runtime",
        "plugin_task_relay_names_the_request_a_cancellation_withdraws",
        "plugin_task_relay_passes_lines_it_cannot_read_unchanged",
        "plugin_task_relay_reads_whole_lines_and_marks_the_end",
        "plugin_task_relay_replays_the_handshake_and_consumes_only_its_answer",
        "plugin_task_relay_stateless_host_needs_no_replay",
        "plugin_task_relay_waits_only_while_the_installation_is_updated",
        "plugin_task_runtime_hello_without_root_keeps_the_previous_wire_form",
    ],
    "linux": [],
    "windows": [],
}


def batch_expected():
    system = "windows" if WINDOWS else "linux"
    return (BATCH_TESTS["both"] + BATCH_TESTS[system] + VERBINDUNGEN_TESTS["both"]
            + VERBINDUNGEN_TESTS[system] + PLUGIN_TESTS["both"] + PLUGIN_TESTS[system])


# Loads the netem qdisc module. The Azure kernels of the hosted runners ship sch_netem in the package
# linux-modules-extra-<release>, which is not always installed: without it `tc qdisc ... netem` fails with
# "Specified qdisc kind is unknown". The package installation is why HOST_COMMAND_TIMEOUT is long.
NETEM_MODULE = ["sudo", "-n", "sh", "-c",
                "modprobe sch_netem 2>/dev/null || "
                "{ DEBIAN_FRONTEND=noninteractive apt-get install -y \"linux-modules-extra-$(uname -r)\" "
                "&& modprobe sch_netem; }"]
# Seconds one host command of an ignored group may take (the package installation of NETEM_MODULE).
HOST_COMMAND_TIMEOUT = 900

# Crate tests that are #[ignore] because they need an environment a plain run does not
# provide. They belonged to suites of their own (direct-open, sync-paths, gui-design,
# search-recursive-access, removed 2026-11-07) and run here with that environment, on the
# OS their suite used. Each name is a substring filter that must select exactly one
# ignored test and that test must pass (`run(expect=...)`). `profile` gives the run
# its own app-data folders (the App constructors of the tests read them, the real
# profile of the runner stays untouched); `drop` removes variables from the environment.
IGNORED_GROUPS = [
    {   # Direct opening and reconnect: the GUI's remote-edit recovery (app::remote_open) and the
        # daemon/Share reconnect without replay (share::direct_open_task_tests); Linux and Windows.
        "title": "Direct opening and reconnect (ignored tests, isolated app data)",
        "os": ("Linux", "Windows"),
        "set": {"SMART_EXPLORER_COPY_PASTE_TASK": "1"},
        "namespace": "direct_open_",  # + the first 12 characters of the candidate (Windows resource names)
        "profile": True,
        "drop": ("SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"),
        "tests": [
            "direct_open_task_pending_download_is_neither_edit_nor_recovery",
            "direct_open_task_parallel_download_and_atomic_save_do_not_block_new_open",
            "direct_open_task_new_missing_payload_and_failed_manifest_never_become_editing",
            "direct_open_task_recovery_rejects_escaped_and_nonregular_editor_paths",
            "direct_open_task_failed_and_disconnected_workers_remove_only_their_downloads",
            "direct_open_task_atomic_save_waits_for_stability_then_saves_back",
            "direct_open_task_manifest_failure_suppresses_upload_and_preserves_edit",
            "direct_open_task_save_conflict_and_failed_revision_check_preserve_remote",
            "direct_open_task_real_direct_and_room_reconnect_through_daemon_without_replay",
            "direct_open_task_reconnect_cannot_bypass_revoked_direct_access",
        ],
    },
    {   # Sync setups of every backend pair keep their locators and baselines; links and junctions
        # stay protected omissions (AGENTS.md, "preserve existing behavior"); Linux and Windows.
        "title": "Sync paths and links (ignored tests, isolated app data)",
        "os": ("Linux", "Windows"),
        "set": {"SMART_EXPLORER_COPY_PASTE_TASK": "1", "SMART_EXPLORER_ANALYTICS_TASK": "1"},
        "profile": True,
        "drop": ("SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"),
        "tests": [
            "sync_paths_task_all_backend_pairs_transfer_changes_and_keep_baselines_separate",
            "sync_paths_task_picker_setup_and_quick_actions_retain_remote_provenance",
            "sync_paths_task_split_same_paths_on_different_remotes_and_uncached_metadata",
            "sync_paths_task_saved_local_job_uses_worker_resolution_and_preserves_old_behavior",
            "sync_paths_task_real_share_cross_peer_sync_and_local_roundtrip",
            "sync_links_task_saved_job_and_gui_retain_partial_result_notice",
            "sync_links_task_cross_remote_contract_protects_counterpart_subtree",
        ],
    },
    {   # The egui harness renders the real GUI headless in light and dark mode at the minimum and
        # the desktop window size and writes one JSON mesh dump per view into the visuals folder
        # (uploaded with the logs); Linux only, as its former suite ran.
        "title": "GUI layout harness (ignored tests, visuals folder)",
        "os": ("Linux",),
        "set": {"SMART_EXPLORER_GUI_TASK": "1"},
        "visuals": "SMART_EXPLORER_GUI_VISUALS",
        "profile": True,
        "tests": [
            "gui_design_task_workspace_layout_selection_filters_and_split",
            "gui_design_task_settings_persistence_shortcut_guard_and_dialogs",
            "gui_design_task_start_page_and_chart_visuals",
            "gui_design_task_drive_failure_reaches_readable_report_and_complete_clipboard",
        ],
    },
    {   # Real ACL, impersonation token and junction fixtures, and the authenticated read helper as a
        # child process: they need the administrator token and SeBackupPrivilege (checked first);
        # Windows only.
        "title": "Windows access fixtures (ignored tests, administrator token)",
        "os": ("Windows",),
        "set": {"SMART_EXPLORER_ANALYTICS_TASK": "1", "SMART_EXPLORER_COPY_PASTE_TASK": "1"},
        "needs_backup_privilege": True,
        "profile": True,
        "tests": [
            "analytics_access_task_real_denied_directory_locked_files_and_unchanged_acl",
            "analytics_access_task_restricted_identity_never_falls_back_to_process_authority",
            "analytics_access_task_redirect_children_are_traversal_boundaries",
            "search_recursive_access_task_authenticated_helper_reuses_read_handles_in_parent",
            # The login service's grant pipe (E21): needs the elevated administrator token.
            "login_selbst_task_windows_grant_pipe_names_the_client_token",
            # A fixed pipe name held in advance does not block the grant channel (E45): needs the same token.
            "login_selbst_task_windows_held_fixed_pipe_name_does_not_block_grant_channel",
        ],
    },
    {   # An NTFS junction into a mounted vault's folder is private for the Share host (E35, S11);
        # creating the junction needs no right, the test is ignored only because it needs NTFS.
        "title": "Vault folder behind an NTFS junction (ignored test)",
        "os": ("Windows",),
        "profile": True,
        "tests": [
            "fs_tresor_task_windows_junction_into_vault_is_private",
        ],
    },
    {   # The 8.3 short name of a vault path is private (E35, S11). The test fails, with a message,
        # on a volume without 8.3 names; the group enables them on the temp volume first.
        "title": "Vault folder by its 8.3 short name (ignored test)",
        "os": ("Windows",),
        "profile": True,
        "windows_short_names": True,
        "tests": [
            "fs_tresor_task_windows_short_name_is_private",
        ],
    },
    {   # Retiring generations are announced over mDNS (E52, L9): the test's two daemons find each other on
        # the loopback interface, which must carry the MULTICAST flag and a route for 224.0.0.0/4 while the
        # test runs (assumption, plan 2026-11-08 p-erg-18 item 9). The group sets both with sudo and removes
        # them again. Linux only.
        "title": "Multicast on lo (ignored test)",
        "os": ("Linux",),
        "before": [["sudo", "-n", "ip", "link", "set", "dev", "lo", "multicast", "on"],
                   ["sudo", "-n", "ip", "route", "replace", "224.0.0.0/4", "dev", "lo"]],
        "after": [["sudo", "-n", "ip", "route", "del", "224.0.0.0/4", "dev", "lo"],
                  ["sudo", "-n", "ip", "link", "set", "dev", "lo", "multicast", "off"]],
        "profile": True,
        "tests": [
            "code_renewal_grenze_task_sixty_three_generations_are_all_announced",
        ],
    },
    {   # The handshake of an unknown peer over a mobile profile (E61, r-zugang U1): the loopback interface
        # carries 350 ms +- 50 ms delay and 2 % loss while the test runs. The group sets the qdisc with sudo
        # and removes it again whether or not the test passed; a failing command is reported as an
        # infrastructure error of the group with its own output. Linux only. The kernels of the hosted
        # runners carry sch_netem in linux-modules-extra (transfer-engine-task.yml, rdt_task/speed_net.py
        # netem_ready): the first command loads the module and installs that package when it is missing.
        "title": "Mobile profile handshake on lo (netem, ignored test)",
        "os": ("Linux",),
        "before": [NETEM_MODULE,
                   ["sudo", "-n", "tc", "qdisc", "replace", "dev", "lo", "root", "netem",
                    "delay", "350ms", "50ms", "loss", "2%"]],
        "after": [["sudo", "-n", "tc", "qdisc", "del", "dev", "lo", "root"]],
        "profile": True,
        "tests": [
            "unbelegt_task_unknown_handshake_mobile_profile",
        ],
    },
    {   # A second account must not reach the worker IPC of the first (M26, E50). The group creates the
        # second Linux account with sudo and names it in SE_TASK_FOREIGN_ACCOUNT; the test serves the worker
        # IPC of the runner's account and starts itself again as the second account, which must fail to
        # read the token, get no proof for a guessed token and never reach the „Öffnen“ arm. The test puts
        # the first account's app data into a folder of its own below the temp folder that every account
        # may enter (the profile folder of this group lies below a folder only the runner may enter), so
        # only the modes the worker gives its app data folder and its token refuse the second account.
        # Linux only.
        "title": "Second account on the worker IPC (ignored test, isolated app data)",
        "os": ("Linux",),
        "before": [["sudo", "-n", "sh", "-c",
                    "getent passwd se-task-foreign >/dev/null || useradd --system --no-create-home "
                    "--shell /usr/sbin/nologin se-task-foreign"]],
        "after": [["sudo", "-n", "sh", "-c",
                   "! getent passwd se-task-foreign >/dev/null || userdel se-task-foreign"]],
        "set": {"SE_TASK_FOREIGN_ACCOUNT": "se-task-foreign"},
        "profile": True,
        "tests": [
            "dienste_oeffnen_task_foreign_account_is_refused",
        ],
    },
    {   # Desktop „Öffnen“ and „Schließen“ on a device feed of the test process (E27, E50, M26): the tests
        # bind listeners on the device address and write the device address book of the app data folder.
        "title": "Desktop open and close of shared services (ignored tests, isolated app data)",
        "os": ("Linux", "Windows"),
        "profile": True,
        "tests": [
            "dienste_oeffnen_task_closed_service_is_not_restarted_by_a_feed_update",
            "dienste_oeffnen_task_open_refuses_a_port_the_feed_does_not_name",
            "dienste_oeffnen_task_open_and_close_over_ipc_change_the_mapping",
        ],
    },
    {   # The machine login target over two real nodes on loopback (E43, S2, p-erg-15 B8): the steering set,
        # the login broker's stand-in and the viewer's target book are process-wide, so it runs alone.
        "title": "Login target over two nodes (ignored test, isolated app data)",
        "os": ("Linux", "Windows"),
        "profile": True,
        "tests": [
            "login_ziel_task_two_nodes_deliver_the_target_only_with_steering",
        ],
    },
]

# The ignored test that needs root and the system account of an installed login service
# (native/install-login-service.sh creates the account and the record directory; the suite makes the
# same two things for the test and removes them again). Linux only.
ROOT_TESTS = ["login_selbst_task_linux_root_records_keep_installer_owner"]
LOGIN_ACCOUNT = "smart-explorer-login"
LOGIN_STATE = "/var/lib/smart-explorer-login-gateway"
LOGIN_CONFIG = "/var/lib/smart-explorer-login"


def candidate_sha():
    probe = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                           text=True)
    return probe.stdout.strip()


def backup_privilege_problem():
    """None when this process has the administrator token and SeBackupPrivilege, else the reason."""
    import ctypes

    if not ctypes.windll.shell32.IsUserAnAdmin():
        return "the real ACL fixtures need the runner's administrator token"
    privileges = subprocess.run(["whoami", "/priv", "/fo", "csv"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                text=True, errors="replace").stdout
    if "SeBackupPrivilege" not in privileges:
        return "the runner's token lacks SeBackupPrivilege"
    return None


def enable_short_names():
    """Switches 8.3 name creation on for the volume of the temp folder (new names only) and records
    the volume's state before and after, so the test's own message has its context."""
    drive = os.path.splitdrive(tempfile.gettempdir())[0] or "C:"
    for arguments in (["query", drive], ["set", drive, "0"], ["query", drive]):
        probe = subprocess.run(["fsutil", "8dot3name", *arguments], stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, text=True,
                               errors="replace", timeout=60)
        summary(f"`fsutil 8dot3name {' '.join(arguments)}` exit {probe.returncode}: "
                + " / ".join(line.strip() for line in probe.stdout.splitlines() if line.strip()))


def engine_test_binary():
    """The compiled lib test binary of smart_explorer_engine, built with the feature set of the
    ignored groups (the same `cargo test` selection, so nothing is compiled again); None with the
    reason in the summary."""
    probe = subprocess.run(["cargo", "test", "--locked", *CRATES, "--lib", "--no-run", "--message-format", "json"],
                           cwd=NATIVE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL,
                           text=True, encoding="utf-8", errors="replace")
    if probe.returncode != 0:
        summary("cargo could not report the test binaries:\n```\n" + probe.stderr[-3000:] + "\n```")
        return None
    for line in probe.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if (message.get("reason") == "compiler-artifact" and message.get("executable")
                and message.get("profile", {}).get("test")
                and message.get("target", {}).get("name") == "smart_explorer_engine"
                and "lib" in message["target"].get("kind", [])):
            return message["executable"]
    summary("no test binary of smart_explorer_engine in the cargo output")
    return None


def part_root_ignored():
    """The login records test as root (see ROOT_TESTS). Needs passwordless sudo, as the runner has."""
    if WINDOWS:
        return True
    title = "Login service records as root (ignored test, system account)"
    if subprocess.run(["sudo", "-n", "true"], stdin=subprocess.DEVNULL).returncode != 0:
        summary(f"### {title}\nnot run: no passwordless sudo")
        return False
    binary = engine_test_binary()
    if binary is None:
        summary(f"### {title}\nnot run: no engine test binary")
        return False

    def sudo(*arguments):
        return subprocess.run(["sudo", "-n", *arguments], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL).returncode

    account_existed = sudo("getent", "passwd", LOGIN_ACCOUNT) == 0
    try:
        if not account_existed and sudo("useradd", "--system", "--user-group", "--home-dir", LOGIN_STATE,
                                        "--no-create-home", "--shell", "/usr/sbin/nologin", LOGIN_ACCOUNT) != 0:
            summary(f"### {title}\nnot run: the system account {LOGIN_ACCOUNT} could not be created")
            return False
        if sudo("install", "-d", "-o", "root", "-g", "root", "-m", "0750", LOGIN_CONFIG) != 0:
            summary(f"### {title}\nnot run: {LOGIN_CONFIG} could not be created")
            return False
        return run(title, ["sudo", "-n", binary, "--test-threads=1", "--ignored", *ROOT_TESTS],
                   cwd=NATIVE / "crates" / "engine", expect=ROOT_TESTS)
    finally:
        sudo("rm", "-rf", LOGIN_CONFIG)
        if not account_existed:
            sudo("userdel", LOGIN_ACCOUNT)


def host_commands(title, step, commands):
    """Runs the host commands of one ignored group in order (`sudo -n` on the runner). The first command
    that fails ends the step: its command line and output go to the job summary as an infrastructure error
    of the group, and the step fails."""
    for command in commands:
        try:
            process = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
                                     timeout=HOST_COMMAND_TIMEOUT)
        except (OSError, subprocess.TimeoutExpired) as error:
            summary(f"### {title}\nInfrastructure error ({step}): `{' '.join(command)}` did not run: {error!r}")
            return False
        if process.returncode != 0:
            summary(f"### {title}\nInfrastructure error ({step}): `{' '.join(command)}` exited with "
                    f"{process.returncode}\n```\n{process.stdout[-4000:]}\n```")
            return False
    return True


def run_ignored_group(group, env):
    """Runs one group between its host commands: `before` first, then the tests, and `after` always,
    also when `before` or a test fails."""
    title = group["title"]
    passed = False
    try:
        passed = host_commands(title, "setup", group.get("before", ())) and run_ignored_tests(group, env)
    finally:
        cleaned = host_commands(title, "cleanup", group.get("after", ()))
    return passed and cleaned


def run_ignored_tests(group, env):
    title = group["title"]
    problem = backup_privilege_problem() if group.get("needs_backup_privilege") and WINDOWS else None
    if problem:
        summary(f"### {title}\nnot run: {problem}")
        return False
    scratch = Path(tempfile.mkdtemp(prefix="crates-task-ignored-"))
    try:
        if group.get("windows_short_names") and WINDOWS:
            enable_short_names()
        local = dict(env, RUST_BACKTRACE="1", **group.get("set", {}))
        for name in group.get("drop", ()):
            local.pop(name, None)
        if group.get("namespace"):
            local["SMART_EXPLORER_E2E_TEST_NAMESPACE"] = group["namespace"] + candidate_sha()[:12]
        if group.get("profile"):
            profile = scratch / "profile"
            local.update(APPDATA=str(profile / "roaming"), LOCALAPPDATA=str(profile / "local"),
                         XDG_CONFIG_HOME=str(profile / "config"), XDG_DATA_HOME=str(profile / "data"))
            for name in ("APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME"):
                Path(local[name]).mkdir(parents=True, exist_ok=True)
        if group.get("visuals"):
            visuals = NATIVE / "target" / "crates-task-logs" / "gui-visuals"
            visuals.mkdir(parents=True, exist_ok=True)
            local[group["visuals"]] = str(visuals)
        return run(title,
                   ["cargo", "test", "--locked", *CRATES, "--lib", "--no-fail-fast", "--",
                    "--test-threads=1", "--ignored", *group["tests"]],
                   env=local, expect=group["tests"])
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def part_ignored(env):
    ok = True
    groups = [group for group in IGNORED_GROUPS if platform.system() in group["os"]]
    summary("Run with their environment (#[ignore] tests of former suites): "
            + ", ".join(f"`{name}`" for group in groups for name in group["tests"]))
    for group in groups:
        ok = run_ignored_group(group, env) and ok
    return ok


def part_servers():
    """The packages outside the workspace, each with its own manifest and lock. One test at a
    time, as their former suites ran them (idle keepalive, relay ping and startup tests use
    process-wide timers and ports)."""
    one_at_a_time = ["--no-fail-fast", "--", "--test-threads=1"]
    ok = run("Share server: unit tests and tests/relay_startup.rs",
             ["cargo", "test", "--locked", "--manifest-path", "share-server/Cargo.toml", *one_at_a_time], cwd=ROOT,
             expect=["share_remote_task_server_and_native_discovery_wire_fixture",
                     "share_remote_task_server_discovery_lease_renewal_and_complete_lifecycle",
                     "share_remote_task_server_discovery_rejections_cancel_and_expiry",
                     "explicit_relay_disable_still_starts_signaling",
                     "invalid_expected_relay_configuration_exits_nonzero",
                     *SERVER_BATCH_TESTS])
    # Only the integration test file: the unit tests the agent binary and that file compile from the
    # shared protocol source are the storage crate's own tests, which `workspace` runs.
    ok = run("SSH agent: confined-process tests",
             ["cargo", "test", "--locked", "--manifest-path", "se-agent/Cargo.toml", "--test", "remote_drive_task",
              "--no-fail-fast", "--", "--test-threads=1", "remote_drive_task_agent_"], cwd=ROOT,
             expect=["remote_drive_task_agent_root_is_kernel_confined_and_write_new_is_exclusive",
                     "remote_drive_task_agent_rejects_a_symlinked_root_before_serving"]) and ok
    return ok


def part_windows_gnu():
    gnu = ["--no-run", "--all-targets", "--target", "x86_64-pc-windows-gnu"]
    ok = run("Test binaries of every crate, x86_64-pc-windows-gnu",
             ["cargo", "test", "--locked", *CRATES, *gnu])
    ok &= run("Programs and their test binaries, x86_64-pc-windows-gnu",
              ["cargo", "test", "--locked", "-p", "smart_explorer", *gnu])
    return ok


def rustls_verifier_maven():
    """The Maven repository inside the rustls-platform-verifier-android crate, resolved as
    android/build-release-apk.sh does; android/settings.gradle.kts refuses to configure without it
    (-PrustlsVerifierMaven). None, with the reason in the summary, when it cannot be resolved."""
    process = subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1",
                              "--filter-platform", "aarch64-linux-android",
                              "--manifest-path", str(NATIVE / "Cargo.toml")],
                             cwd=NATIVE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                             encoding="utf-8", errors="replace")
    if process.returncode != 0:
        summary("cargo metadata failed:\n```\n" + process.stderr[-4000:] + "\n```")
        return None
    paths = sorted({package["manifest_path"] for package in json.loads(process.stdout)["packages"]
                    if package["name"] == "rustls-platform-verifier-android"})
    if len(paths) != 1:
        summary(f"expected one rustls-platform-verifier-android package, found {len(paths)}")
        return None
    maven = Path(paths[0]).parent / "maven"
    if not (maven / "rustls" / "rustls-platform-verifier").is_dir():
        summary(f"rustls-platform-verifier Maven repository missing at {maven}")
        return None
    return maven


# JVM tests of the batch: class -> test methods. Gradle prints no test names, so the suite reads the
# JUnit reports of the unit test task and requires each method exactly once, without failure or skip.
KOTLIN_BATCH_TESTS = {
    "app.smartexplorer.android.ui.share.RdpDiagnosticsTaskTest": [
        "rdpDiagnosticsTaskEmptyListShowsNothing",
        "rdpDiagnosticsTaskKeepsEveryLineLiteral",
        "rdpDiagnosticsTaskKeepsOrderAndDropsOnlyBlankLines",
        # p-erg-9 (E54, M29, F13)
        "rdpDiagnosticsTaskForeignDeviceShowsOnlyItsOwnFeedLines",
        "rdpDiagnosticsTaskOwnDeviceShowsLocalLines",
        "rdpDiagnosticsTaskSelectsForeignDeviceByContactOrIdentity",
        "rdpDiagnosticsTaskShowsDroppedCandidatesOnlyWhenCounted",
    ],
    # Package E2 of plan 2026-11-14, gap N7 (E49): the waiting renewal on Android
    "app.smartexplorer.android.api.PinNeuTaskRenewalWaitingTest": [
        "pinNeuTaskRenewalWaitingIsReadWithItsStopTarget",
        "pinNeuTaskOlderCoreSendsNoRenewalWaiting",
    ],
    # p-erg-12 (E28, M32)
    "app.smartexplorer.android.service.AccessApprovalGroupingTest": [
        "rueckfragenTaskTenQuestionsOfOneDeviceAreOneNotificationWithTen",
        "rueckfragenTaskQuestionsOfTwoDevicesAreTwoNotifications",
    ],
    # Plan 2026-11-14 "Verbindungen je Zweck", P6: the pointer overlay's events through the app's decoder
    "app.smartexplorer.android.core.ScreenCursorEventTest": [
        "cursorEventsDecodeBesideTheSessionEvents",
        "aShapeWithTheWrongSizeOrNoPixelsIsSkipped",
        "theHostsNothingToDrawDecodesAsAHiddenPosition",
        "theHotSpotLiesOnTheHostPositionScaledToThePicture",
    ],
}


def kotlin_batch_problems():
    import xml.etree.ElementTree as xml

    reports = ROOT / "android" / "app" / "build" / "test-results" / "testDebugUnitTest"
    problems = []
    for name, methods in KOTLIN_BATCH_TESTS.items():
        report = reports / f"TEST-{name}.xml"
        if not report.is_file():
            problems.append(f"{name}: no JUnit report at {report}")
            continue
        cases = xml.parse(report).getroot().findall("testcase")
        for method in methods:
            found = [case for case in cases if case.get("name") == method]
            if len(found) != 1:
                problems.append(f"{name}.{method}: reported {len(found)} times")
            elif any(found[0].find(kind) is not None for kind in ("failure", "error", "skipped")):
                problems.append(f"{name}.{method}: failed or skipped")
    return problems


def part_android(abi):
    ok = run(f"Android JNI library, {abi}",
             ["cargo", "ndk", "-t", abi, "--platform", "30", "build", "--locked",
              "-p", "smart_explorer_android"])
    if abi == "arm64-v8a":
        maven = rustls_verifier_maven()
        if maven is None:
            summary("### Gradle configuration of the Android app\nnot run: no rustls verifier Maven repository")
            return False
        ok &= run("Gradle configuration of the Android app",
                  ["sh", "./gradlew", "--no-daemon", "--console=plain", "--stacktrace", ":app:help",
                   f"-PrustlsVerifierMaven={maven}"], cwd=ROOT / "android")
        ok &= run("Kotlin compile and JVM unit tests of the Android app",
                  ["sh", "./gradlew", "--no-daemon", "--console=plain", "--stacktrace", "--continue",
                   ":app:compileDebugKotlin", ":app:testDebugUnitTest",
                   # The instrumentation sources compile here too; their tests need a device and
                   # run in android-task, review-task and rechte-desktop-tresore-task.
                   ":app:compileDebugAndroidTestKotlin",
                   f"-PrustlsVerifierMaven={maven}"], cwd=ROOT / "android")
        problems = kotlin_batch_problems()
        summary("### JVM tests of the batch\n"
                + ("each reported exactly once and passed" if not problems else "\n".join(f"- {line}" for line in problems)))
        ok &= not problems
    return ok


def part_agent():
    return run("SSH-agent bundles", ["bash", "native/build-agent-bundles.sh"], cwd=ROOT)


# The S4 measurement (plan 2026-11-08, E30). The tests are #[ignore]: they measure the runner, so
# they run alone, one after the other, with the profile that ships the release optimization.
S4_THRESHOLD_PERCENT = 10.0
S4_LINE = re.compile(r"S4-MESSUNG plattform=(\S+) verfahren=(\S+) aead_mib_s=([\d.]+) "
                     r"share_mib_s=([\d.]+) anteil_prozent=([\d.]+)")


def part_s4():
    lines = []
    ok = run("S4: single-core AEAD throughput and the encryption share of a Share transfer",
             ["cargo", "test", "--locked", "--profile", "speed", "-p", "smart_explorer_engine", "--lib",
              "--no-fail-fast", "s4_messung_task_", "--", "--ignored", "--nocapture", "--test-threads=1"],
             collect=lines)
    # With --nocapture the line follows "test <name> ... " on the same output line.
    measured = [match for match in (S4_LINE.search(line) for line in lines) if match]
    if not any(re.match(r"^test result: ok\. 4 passed", line) for line in lines):
        summary("S4: the four measuring tests did not all pass")
        ok = False
    if len(measured) != 1:
        summary(f"S4: expected exactly one S4-MESSUNG line, found {len(measured)}")
        return False
    platform_name, method, aead, share, percent = measured[0].groups()
    summary(f"S4-MESSUNG plattform={platform_name} verfahren={method} aead_mib_s={aead} share_mib_s={share} "
            f"anteil_prozent={percent} schwelle_prozent={S4_THRESHOLD_PERCENT:.1f}")
    if float(percent) >= S4_THRESHOLD_PERCENT:
        summary(f"S4-BEFUND: on {platform_name} encryption takes {percent} % of a transfer, at or above the "
                f"{S4_THRESHOLD_PERCENT:.0f} % threshold (E30): the plan owner decides on the plaintext channel.")
        ok = False
    return ok


def part_e2e():
    """The code renewal across real processes (native/test-share-code-renewal-e2e.sh). The development
    `se` and the share server are built first, incrementally on a warm cache."""
    if WINDOWS:
        summary("the code renewal E2E runs on Linux only")
        return False
    ok = run("Build se (development profile)", ["cargo", "build", "--locked", "--bin", "se"])
    ok = ok and run("Build the share server (development profile)",
                    ["cargo", "build", "--locked", "--manifest-path", "share-server/Cargo.toml"], cwd=ROOT)
    if not ok:
        return False
    logs = NATIVE / "target" / "crates-task-logs" / "code-renewal-e2e"
    shutil.rmtree(logs, ignore_errors=True)
    env = dict(os.environ,
               SMART_EXPLORER_SE_BINARY=str(NATIVE / "target" / "debug" / "se"),
               SMART_EXPLORER_SHARE_SERVER_BINARY=str(ROOT / "share-server" / "target" / "debug" / "se-share-server"),
               SMART_EXPLORER_E2E_LOG_DIR=str(logs))
    lines = []
    ok = run("Code renewal across processes (E17)", ["bash", "native/test-share-code-renewal-e2e.sh"],
             cwd=ROOT, env=env, collect=lines)
    passed = [line for line in lines if line.startswith("code renewal E2E passed")]
    started = [line for line in lines if line.startswith("E2E-RENEWAL")]
    summary("\n".join(["E2E result: " + (passed[0] if passed else "no pass line")] + started))
    return ok and len(passed) == 1


# The measurements of the plan 2026-11-14 (C7, package P8). What runs where, the line checks and the decision
# rules are in native/crates_task_verbindungen.py; this part runs the commands. The tests are #[ignore]
# (`verbindungen_messung_task_*`): they measure the runner, so they run alone, one after the other, with the
# profile that ships the release optimization, as the s4 part does.
def netem_shown(arguments):
    """None when `tc qdisc show dev lo` shows the netem rule `arguments`, else what is missing."""
    shown = subprocess.run(["tc", "qdisc", "show", "dev", "lo"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True, errors="replace").stdout.lower()
    values = [token for token in arguments.lower().split() if token not in ("delay", "rate", "limit", "loss")]
    missing = [token for token in values if token not in shown]
    if "netem" not in shown or missing:
        return f"`tc qdisc show dev lo` shows `{shown.strip()}`, missing {missing or ['netem']}"
    return None


def verbindungen_group(vb, group, key, library, profiles):
    """Runs the tests of one group on its link. Returns whether the command and the link set-up passed and the
    parsed measurement lines, tagged with the codec source of the group."""
    title = f"Verbindungen: {group.title} ({key})"
    tests = [case.test for case in group.cases]
    env = dict(os.environ, SE_SPEED_PROFILE=group.profile, RUST_BACKTRACE="1")
    env.pop("SE_SCREEN_TASK_OPENH264", None)
    if group.openh264 and library is not None:
        env["SE_SCREEN_TASK_OPENH264"] = str(library)
    before, after = [], []
    if group.profile != "raw":
        arguments = profiles[group.profile][0]
        before = [NETEM_MODULE, ["sudo", "-n", "tc", "qdisc", "replace", "dev", "lo", "root", "netem",
                                 *arguments.split()]]
        after = [["sudo", "-n", "tc", "qdisc", "del", "dev", "lo", "root"]]
    lines = []
    passed = False
    try:
        if host_commands(title, "setup", before):
            problem = netem_shown(arguments) if before else None
            if problem:
                summary(f"### {title}\nInfrastructure error (setup): the link {group.profile} is not set: {problem}")
            else:
                passed = run(title,
                             ["cargo", "test", "--locked", "--profile", "speed", "-p", "smart_explorer_engine",
                              "--lib", "--no-fail-fast", *tests, "--", "--ignored", "--show-output",
                              "--test-threads=1"],
                             env=env, expect=tests, collect=lines)
    finally:
        cleaned = host_commands(title, "cleanup", after)
    return passed and cleaned, vb.parse(lines, group.source)


def part_verbindungen():
    import crates_task_verbindungen as vb

    key = vb.platform_key(platform.system(), platform.machine())
    groups = vb.groups_for(key)
    links = ", ".join(sorted({group.profile for group in groups})) or "none"
    summary(f"### Verbindungen\nplatform {key}; links: {links}")
    if not groups:
        summary(f"no measurement plan for {key}")
        return False
    try:
        profiles = vb.link_profiles() if key == "linux-x86_64" else {}
        shipped = vb.shipped_target_ms(ROOT)
    except Exception as error:  # an import or a source that changed: the part cannot decide
        summary(f"the decision rules cannot start: {error!r}")
        return False
    if not run("Verbindungen: compile the engine's tests (speed profile)",
               ["cargo", "test", "--locked", "--profile", "speed", "-p", "smart_explorer_engine", "--lib",
                "--no-run"]):
        return False
    scratch = Path(tempfile.mkdtemp(prefix="crates-task-verbindungen-"))
    try:
        library = None
        if any(group.openh264 for group in groups):
            try:
                library = vb.openh264_download(ROOT, key, scratch)
            except vb.Problem as error:
                summary(f"### OpenH264\nthe pinned library is not there: {error}")
                return False
            summary(f"### OpenH264\n{library.name if library else 'no pin for ' + key} "
                    "(SE_SCREEN_TASK_OPENH264, size and SHA-256 as pinned in openh264_pins.rs)")
        ok = True
        records = []
        problems = []
        for group in groups:
            ran, found = verbindungen_group(vb, group, key, library, profiles)
            ok = ran and ok
            records.extend(found)
            problems.extend(f"{group.title}: {text}" for text in vb.check_group(group, found, key))
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    logs = NATIVE / "target" / "crates-task-logs"
    logs.mkdir(parents=True, exist_ok=True)
    (logs / vb.LINES_FILE).write_text("".join(vb.render(record) + "\n" for record in records), encoding="utf-8")
    summary("### Measurement lines\n```\n" + "\n".join(vb.render(record) for record in records) + "\n```")
    if problems:
        summary("### A measurement did not measure\n" + "\n".join(f"- {text}" for text in problems))
        return False
    findings, failed = vb.evaluate_platform(key, records, shipped, profiles)
    summary(f"### Findings ({key}, T shipped {shipped} ms)\n```\n" + "\n".join(findings) + "\n```")
    if failed:
        summary("an acceptance of C7 (E2, E4 or E7) is not passed: the line says which value; the next run of "
                "the same suite confirms the corrected one (plan D4, N1)")
    return ok and not failed


def part_verbindungen_gesamt(directory):
    """What needs the lines of every platform; reads the line files the three `verbindungen` jobs wrote."""
    import crates_task_verbindungen as vb

    if not directory:
        summary("--lines-dir is missing")
        return False
    by_platform = {}
    files = sorted(Path(directory).rglob(vb.LINES_FILE))
    for path in files:
        for record in vb.parse(path.read_text(encoding="utf-8").splitlines()):
            by_platform.setdefault(record.get("plattform"), []).append(record)
    summary("### Line files\n" + "\n".join(f"- {path}" for path in files)
            + "\n" + "\n".join(f"- {key}: {len(by_platform.get(key, []))} lines" for key in vb.PLATFORMS))
    findings, failed = vb.evaluate_overall(by_platform)
    summary("### Findings over every platform\n```\n" + "\n".join(findings) + "\n```")
    return not failed


def main():
    # The step log takes UTF-8; Python on a Windows runner would otherwise encode the
    # piped output with the ANSI code page and fail on the first character outside it.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    parser = argparse.ArgumentParser()
    parser.add_argument("--part", required=True,
                        choices=["workspace", "timings", "windows-gnu", "android", "agent", "servers", "s4", "e2e",
                                 "verbindungen", "verbindungen-gesamt"])
    parser.add_argument("--abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a")
    parser.add_argument("--lines-dir", help="part verbindungen-gesamt: the folder with the line files")
    args = parser.parse_args()
    summary(f"## crates-task: {args.part} ({platform.system()})")
    parts = {"workspace": part_workspace, "timings": part_timings, "windows-gnu": part_windows_gnu,
             "android": lambda: part_android(args.abi), "agent": part_agent, "servers": part_servers,
             "s4": part_s4, "e2e": part_e2e, "verbindungen": part_verbindungen,
             "verbindungen-gesamt": lambda: part_verbindungen_gesamt(args.lines_dir)}
    ok = parts[args.part]()
    summary("**passed**" if ok else "**failed**")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
