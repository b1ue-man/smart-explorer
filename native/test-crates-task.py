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
             the tests that moved between crates still pass.
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
import subprocess
import sys
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
            print(stamped, flush=True)
            self.handle.write(stamped + "\n")
            self.handle.flush()
            if keep:
                self.lines.append(line)

    def close(self):
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


def run(title, command, cwd=NATIVE, env=None, cargo_json=False):
    summary(f"### {title}\n`{' '.join(command)}`")
    stream = Stream(title)
    stop = threading.Event()
    sampler = threading.Thread(target=sample_resources, args=(stream, stop), daemon=True)
    sampler.start()
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, encoding="utf-8", errors="replace", bufsize=1)
    for raw in process.stdout:
        line = raw.rstrip("\n")
        if cargo_json and line.startswith("{"):
            cargo_message(stream, line)
        else:
            stream.write(line)
    returncode = process.wait()
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
    return returncode == 0


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
    crates_built, programs_built = compile_stage()
    # cargo-nextest runs every test in its own process with the limits of
    # native/.config/nextest.toml: a test running five minutes is ended and
    # named. The tests of the programs (package smart_explorer) run one at a
    # time there (test group machine-wide); the crates' tests run in parallel
    # processes.
    nextest = ["cargo", "nextest", "run", "--locked", "--no-fail-fast"]
    # The crates' own tests build the layers with feature test-support (their
    # dev-dependencies); the programs and their integration tests build in a
    # separate invocation without it, so `se`/`smart_explorer` started by
    # native/tests run the production paths as in a release.
    # The local loopback fixtures of the copy/paste and Windows-remote tests
    # refuse to start unless their switch is set; they need nothing else.
    env = dict(os.environ, SMART_EXPLORER_COPY_PASTE_TASK="1", SMART_EXPLORER_WINDOWS_REMOTE_TASK="1")
    # Tests that need resources only their own suites provide: real protocol
    # servers (SE_SYNC_PROVIDER_MANIFEST), a live Google account
    # (SE_DRIVE_TEST_CLIENT_ID), a candidate se binary installed by the
    # remote runner (batches P13 retirement) and the network namespace with
    # exactly one private interface of the review suite (S09 LAN-link
    # transport: on a runner with several private interfaces the product
    # refuses with "LAN-Link hat kein eindeutiges privates Interface").
    skips = ["sync_reliability_task_provider", "sync_reliability_task_options_share_private_versions",
             "sync_reliability_task_old_jobs_crossremote", "sync_reliability_task_live_",
             "batches_p13_linux_installed_cli_retirement_reexecs_only_with_its_bound_root",
             "batches_p13_linux_retirement_handoff_cannot_select_a_foreign_installation",
             "review_task_s09_transport_close_withdraw_disable_discard_cached_facts",
             "review_task_s09_transport_expired_or_unknown_facts_fail_closed",
             "review_task_s09_transport_path_revisions_reject_returned_path_evidence",
             "review_task_s09_transport_real_private_tls_round_has_only_status_rights",
             "review_task_s09_transport_rejects_wrong_pin_and_replayed_confirm"]
    summary("Not run here, they need their suite's resources: " + ", ".join(f"`{name}`" for name in skips))
    skip_args = [arg for name in skips for arg in ("--skip", name)]
    # A test binary that did not compile is not run; its compile log above is the result.
    ok = crates_built and run("Tests of every crate", [*nextest, *CRATES, "--", *skip_args], env=env)
    keep_junit("tests-of-every-crate")
    # nextest does not run doc tests; cargo test runs them (two in the core crate).
    ok = (crates_built and run("Doc tests of every crate",
                               ["cargo", "test", "--locked", *CRATES, "--doc", "--no-fail-fast"])) and ok
    ok = (programs_built and run("Programs and their integration tests",
                                 [*nextest, "-p", "smart_explorer"])) and ok
    keep_junit("programs-and-their-integration-tests")
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
                   f"-PrustlsVerifierMaven={maven}"], cwd=ROOT / "android")
    return ok


def part_agent():
    return run("SSH-agent bundles", ["bash", "native/build-agent-bundles.sh"], cwd=ROOT)


def main():
    # The step log takes UTF-8; Python on a Windows runner would otherwise encode the
    # piped output with the ANSI code page and fail on the first character outside it.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    parser = argparse.ArgumentParser()
    parser.add_argument("--part", required=True, choices=["workspace", "timings", "windows-gnu", "android", "agent"])
    parser.add_argument("--abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a")
    args = parser.parse_args()
    summary(f"## crates-task: {args.part} ({platform.system()})")
    parts = {"workspace": part_workspace, "timings": part_timings, "windows-gnu": part_windows_gnu,
             "android": lambda: part_android(args.abi), "agent": part_agent}
    ok = parts[args.part]()
    summary("**passed**" if ok else "**failed**")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
