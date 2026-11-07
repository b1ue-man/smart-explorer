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


def run(title, command, cwd=NATIVE, env=None, cargo_json=False, expect=()):
    """Runs one command. `expect` names tests that must each be reported as passed
    exactly once (`test <path>::<name> ... ok`): a test that was renamed or lost
    its cfg then fails here instead of silently shrinking the selection."""
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


def part_workspace():
    # The test register (docs/tests/register.tsv) names every test with the requirement it
    # protects and its suite; a test without a row or a row without a test fails here. It reads
    # source text only, so Linux runs it once for both OS.
    registered = WINDOWS or run("Test register", [sys.executable, str(ROOT / "native" / "test-register.py")],
                                cwd=ROOT)
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
                              env=env)
    ok = (programs_built and run("Programs and their integration tests",
                                 [*nextest, "-p", "smart_explorer"])) and ok
    keep_junit("programs-and-their-integration-tests")
    # The #[ignore] tests that need their own environment reuse the crates' test binaries.
    ok = (crates_built and part_ignored(env)) and ok
    ok = (programs_built and part_programs_ignored()) and ok
    return ok and registered


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
        ],
    },
]


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


def run_ignored_group(group, env):
    title = group["title"]
    problem = backup_privilege_problem() if group.get("needs_backup_privilege") and WINDOWS else None
    if problem:
        summary(f"### {title}\nnot run: {problem}")
        return False
    scratch = Path(tempfile.mkdtemp(prefix="crates-task-ignored-"))
    try:
        local = dict(env, RUST_BACKTRACE="1", **group["set"])
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
                     "invalid_expected_relay_configuration_exits_nonzero"])
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
    return ok


def part_agent():
    return run("SSH-agent bundles", ["bash", "native/build-agent-bundles.sh"], cwd=ROOT)


def main():
    # The step log takes UTF-8; Python on a Windows runner would otherwise encode the
    # piped output with the ANSI code page and fail on the first character outside it.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    parser = argparse.ArgumentParser()
    parser.add_argument("--part", required=True,
                        choices=["workspace", "timings", "windows-gnu", "android", "agent", "servers"])
    parser.add_argument("--abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a")
    args = parser.parse_args()
    summary(f"## crates-task: {args.part} ({platform.system()})")
    parts = {"workspace": part_workspace, "timings": part_timings, "windows-gnu": part_windows_gnu,
             "android": lambda: part_android(args.abi), "agent": part_agent, "servers": part_servers}
    ok = parts[args.part]()
    summary("**passed**" if ok else "**failed**")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
