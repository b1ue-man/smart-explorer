#!/usr/bin/env python3
"""Task-level suite of the crate split (plan docs/plaene/2026-11-01-idealzustand,
D13) and the shared task setup (D12). Run only on GitHub Actions
(.github/workflows/crates-task.yml); never on a developer workstation.

Parts:
  workspace  every crate of the workspace except the Android-only bridge:
             build all targets, run every test (Linux and Windows); on
             Windows also check the GNU target. Proves that the layers
             core/storage/transfer/engine, the GUI, the CLI, the mobile
             facade and the programs compile and link (every port has its
             provider, the lower crates' own tests their stand-ins) and that
             the tests that moved between crates still pass.
  android    the Android JNI library (native/android-bridge over the facade
             and the mobile crate) for arm64-v8a: compile and link the cdylib.
  agent      the SSH-agent bundles through native/build-agent-bundles.sh, the
             path the release's prepare job takes (se-agent includes the
             shared protocol sources from crates/storage and crates/core).

Every command's tail goes to the step summary; a failing command fails the
part after all of its commands ran.
"""
import argparse
import os
import platform
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / "native"
WINDOWS = platform.system() == "Windows"
SUMMARY = os.environ.get("GITHUB_STEP_SUMMARY")
EXCLUDE = ["--workspace", "--exclude", "smart_explorer_android"]


def summary(text):
    print(text, flush=True)
    if SUMMARY:
        with open(SUMMARY, "a", encoding="utf-8") as handle:
            handle.write(text + "\n")


def run(title, command, cwd=NATIVE, env=None):
    summary(f"### {title}\n`{' '.join(command)}`")
    process = subprocess.run(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    output = process.stdout
    log = NATIVE / "target" / "crates-task-logs"
    log.mkdir(parents=True, exist_ok=True)
    (log / (re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-") + ".log")).write_text(output, encoding="utf-8")
    sys.stdout.write(output[-20000:])
    lines = output.splitlines()
    results = [line for line in lines if line.startswith("test result:")]
    failures = [line for line in lines if re.match(r"^test .+ \.\.\. FAILED$", line)
                or line.startswith("error") or "panicked at" in line]
    summary(f"exit code {process.returncode}")
    if results:
        summary("\n".join(f"- {line}" for line in results[-40:]))
    if process.returncode != 0:
        tail = failures[-80:] or lines[-60:]
        summary("```\n" + "\n".join(tail) + "\n```")
    return process.returncode == 0


def part_workspace():
    env = dict(os.environ)
    ok = run("Build every crate and target", ["cargo", "build", "--locked", *EXCLUDE, "--all-targets"])
    threads = ["--test-threads=1"] if WINDOWS else []
    ok &= run("Every test of the workspace",
              ["cargo", "test", "--locked", *EXCLUDE, "--no-fail-fast", "--", *threads], env=env)
    if WINDOWS:
        ok &= run("Check of the Windows GNU target",
                  ["cargo", "check", "--locked", *EXCLUDE, "--all-targets", "--target", "x86_64-pc-windows-gnu"])
    return ok


def part_android():
    return run("Android JNI library, arm64-v8a",
               ["cargo", "ndk", "-t", "arm64-v8a", "--platform", "30", "build", "--locked",
                "-p", "smart_explorer_android"])


def part_agent():
    return run("SSH-agent bundles", ["bash", "native/build-agent-bundles.sh"], cwd=ROOT)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--part", required=True, choices=["workspace", "android", "agent"])
    args = parser.parse_args()
    summary(f"## crates-task: {args.part} ({platform.system()})")
    ok = {"workspace": part_workspace, "android": part_android, "agent": part_agent}[args.part]()
    summary("**passed**" if ok else "**failed**")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
