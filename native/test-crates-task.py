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
             (-PrustlsVerifierMaven, android/build-release-apk.sh).
  agent      the SSH-agent bundles through native/build-agent-bundles.sh, the
             path the release's prepare job takes (se-agent includes the
             shared protocol sources from crates/storage and crates/core).

Every command's tail goes to the step summary; a failing command fails the
part after all of its commands ran.
"""
import argparse
import json
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
CRATES = ["--workspace", "--exclude", "smart_explorer_android", "--exclude", "smart_explorer"]


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
    threads = ["--test-threads=1"] if WINDOWS else []
    # The crates' own tests build the layers with feature test-support (their
    # dev-dependencies); the programs and their integration tests build in a
    # separate invocation without it, so `se`/`smart_explorer` started by
    # native/tests run the production paths as in a release.
    ok = run("Tests of every crate",
             ["cargo", "test", "--locked", *CRATES, "--no-fail-fast", "--", *threads])
    ok &= run("Programs and their integration tests",
              ["cargo", "test", "--locked", "-p", "smart_explorer", "--no-fail-fast", "--", *threads])
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
    return ok


def part_agent():
    return run("SSH-agent bundles", ["bash", "native/build-agent-bundles.sh"], cwd=ROOT)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--part", required=True, choices=["workspace", "windows-gnu", "android", "agent"])
    parser.add_argument("--abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a")
    args = parser.parse_args()
    summary(f"## crates-task: {args.part} ({platform.system()})")
    parts = {"workspace": part_workspace, "windows-gnu": part_windows_gnu,
             "android": lambda: part_android(args.abi), "agent": part_agent}
    ok = parts[args.part]()
    summary("**passed**" if ok else "**failed**")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
