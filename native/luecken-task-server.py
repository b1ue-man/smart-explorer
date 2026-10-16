"""Server stage of the gap-closing suite (Linux): the candidate share server.

Called only by native/test-luecken-task.py (`lib` is that module); no entry
point of its own.

  * `cargo build --locked` of the development `se-share-server` (the very
    binary the Share pairs of the `exec` and `update` stages run against) and
    `cargo test --locked` of share-server (unit tests and
    share-server/tests/relay_startup.rs);
  * the dependency versions the batch promises, read from both Cargo.lock
    files (native: iroh 1.3.0, rustls 0.23.46, russh 0.64.1, no aws-lc;
    share-server: rustls and rustls-webpki above the advisories, iroh-base
    1.1.0 or newer, no `iroh` client package);
  * `cargo audit` of both lock files with exactly the `--ignore` entries that
    .github/workflows/build.yml passes (read from that file, so the suite and
    the push CI never drift apart).
"""
import re
import tomllib
from pathlib import Path

SERVER_TARGET = Path("share-server") / "target" / "debug" / "se-share-server"
BUILD_WORKFLOW = Path(".github") / "workflows" / "build.yml"
# Versions that carry a published advisory and were lifted by this batch.
NATIVE_EXACT = {"iroh": "1.3.0", "iroh-base": "1.3.0", "iroh-relay": "1.3.0", "russh": "0.64.1",
                "ml-kem": "0.3.2"}
NATIVE_AT_LEAST = {"rustls": "0.23.46"}
NATIVE_NOT = {"h2": "0.4.15", "lru": "0.18.0", "webbrowser": "1.2.1"}
FORBIDDEN_CRATES = ("aws-lc-rs", "aws-lc-sys", "aws-lc-fips-sys")
SERVER_AT_LEAST = {"rustls": "0.23.46", "rustls-webpki": "0.103.14", "iroh-base": "1.1.0"}


def version_key(text):
    found = re.match(r"(\d+)\.(\d+)\.(\d+)", text)
    return tuple(int(part) for part in found.groups()) if found else (0, 0, 0)


def locked_versions(path):
    packages = {}
    for package in tomllib.loads(Path(path).read_text(encoding="utf-8")).get("package", []):
        packages.setdefault(package["name"], []).append(package["version"])
    return packages


def native_lock_checks(lib, results):
    stage = "server"
    root = lib.ai.ROOT

    def native():
        packages = locked_versions(root / "native" / "Cargo.lock")
        for name, wanted in NATIVE_EXACT.items():
            lib.ensure(packages.get(name) == [wanted], f"{name} is {packages.get(name)}, expected [{wanted}]")
        for name, wanted in NATIVE_AT_LEAST.items():
            versions = packages.get(name, [])
            lib.ensure(versions and all(version_key(v) >= version_key(wanted) for v in versions),
                       f"{name} is {versions}, expected at least {wanted}")
        for name, bad in NATIVE_NOT.items():
            lib.ensure(bad not in packages.get(name, []), f"{name} {bad} (advisory) is locked again")
        present = [name for name in FORBIDDEN_CRATES if name in packages]
        lib.ensure(not present, f"aws-lc packages in the lock: {present}")
        return ", ".join(f"{name} {packages[name][0]}" for name in ("iroh", "rustls", "russh"))
    results.check(stage, "native/Cargo.lock: iroh 1.3.0, rustls >= 0.23.46, russh 0.64.1, no aws-lc", native)

    def server():
        packages = locked_versions(root / "share-server" / "Cargo.lock")
        for name, wanted in SERVER_AT_LEAST.items():
            versions = packages.get(name, [])
            lib.ensure(versions and all(version_key(v) >= version_key(wanted) for v in versions),
                       f"{name} is {versions}, expected at least {wanted}")
        lib.ensure("iroh" not in packages, "the iroh client package is locked in share-server")
        present = [name for name in FORBIDDEN_CRATES if name in packages]
        lib.ensure(not present, f"aws-lc packages in the lock: {present}")
        return ", ".join(f"{name} {packages[name][0]}" for name in SERVER_AT_LEAST)
    results.check(stage, "share-server/Cargo.lock: rustls, rustls-webpki, iroh-base above the advisories", server)


def audit_ignores(lib):
    """`--ignore` entries of the two `cargo audit` lines of build.yml."""
    workflow = (lib.ai.ROOT / BUILD_WORKFLOW).read_text(encoding="utf-8")
    ignores = {}
    for name, pattern in (("native", r"cargo audit --file Cargo\.lock([^\n]*)"),
                          ("share-server", r"cargo audit --file \.\./share-server/Cargo\.lock([^\n]*)")):
        found = re.search(pattern, workflow)
        lib.ensure(found, f"build.yml has no `cargo audit` line for {name}")
        ignores[name] = re.findall(r"--ignore\s+(RUSTSEC-\d{4}-\d{4})", found.group(1))
    return ignores


def ensure_cargo_audit(lib, logs):
    probe = lib.ai.run(["cargo", "audit", "--version"], timeout=120)
    if probe.returncode != 0:
        installed = lib.ai.run(["cargo", "install", "cargo-audit", "--locked"], log=logs / "cargo-audit-install.log",
                               timeout=3600)
        lib.ensure(installed.returncode == 0, lib.text(installed)[-1500:])


def audit_checks(lib, results, logs):
    stage = "server"
    root = lib.ai.ROOT
    try:
        ignores = audit_ignores(lib)
        ensure_cargo_audit(lib, logs)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "cargo audit tooling", False, f"{type(error).__name__}: {error}")
        return
    for name, lock in (("native", root / "native" / "Cargo.lock"), ("share-server", root / "share-server" / "Cargo.lock")):
        flags = [part for rustsec in ignores[name] for part in ("--ignore", rustsec)]
        done = lib.ai.run(["cargo", "audit", "--file", lock, *flags], cwd=root, log=logs / f"audit-{name}.log",
                          timeout=1800)
        shown = f"{name} (ignored as in build.yml: {', '.join(ignores[name]) or 'none'})"
        results.record(stage, f"cargo audit {shown}", done.returncode == 0,
                       lib.text(done)[-3000:] if done.returncode else "")


def test_checks(lib, results, logs):
    stage = "server"
    manifest = lib.ai.ROOT / "share-server" / "Cargo.toml"
    tested = lib.ai.run(["cargo", "test", "--locked", "--manifest-path", manifest, "--", "--test-threads=1"],
                        cwd=lib.ai.ROOT, log=logs / "share-server-test.log", timeout=5400)
    output = lib.text(tested)
    passed = sum(int(count) for count in re.findall(r"test result: ok\. (\d+) passed", output))
    results.record(stage, "cargo test --locked share-server", tested.returncode == 0,
                   lib.failure_detail(output) if tested.returncode else f"{passed} tests passed")
    startup = [name for name in ("explicit_relay_disable_still_starts_signaling",
                                 "invalid_expected_relay_configuration_exits_nonzero")
               if f"test {name} ... ok" in output]
    results.record(stage, "share-server/tests/relay_startup.rs ran", len(startup) == 2,
                   f"ran: {startup}")


def stage_server(lib, results, log_root):
    """Returns the development share server binary, or None when it cannot be built."""
    logs = log_root / "server"
    logs.mkdir(parents=True, exist_ok=True)
    root = lib.ai.ROOT
    built = lib.ai.run(["cargo", "build", "--locked", "--manifest-path", root / "share-server" / "Cargo.toml",
                        "--bin", "se-share-server", "--message-format", "short"], cwd=root,
                       log=logs / "share-server-build.log", timeout=5400)
    results.record("server", "cargo build --locked share-server (candidate se-share-server)",
                   built.returncode == 0, lib.text(built)[-3000:] if built.returncode else "")
    native_lock_checks(lib, results)
    if built.returncode == 0:
        test_checks(lib, results, logs)
    audit_checks(lib, results, logs)
    binary = root / SERVER_TARGET
    return binary if built.returncode == 0 and binary.exists() else None
