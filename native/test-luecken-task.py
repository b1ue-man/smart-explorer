#!/usr/bin/env python3
"""One remote-only task suite for the gap-closing batch (2026-10-16).

Plan and expected results: docs/plaene/2026-10-16-luecken-schliessen/plan.md (D, E).
Builds only the development `se`, the library test binary and the development
`se-share-server` of this host (no release build), then checks every
milestone once:

  fmt      rustfmt of the batch's Rust files (stdin mode, no crate-wide debt)
  build    `se` plus the lib tests; compiler warnings inside new batch files fail
  unit     `luecken_task_` tests (M1 sealed secrets and their porting, M2 exec
           account/programs/environment/audit/penalties and the profile
           encoding for older readers, M3 SSH host keys and logins, M4 hybrid
           key exchange, L1 FTP on suppaftp 10.0.2) and the directly affected
           modules; the two provider tests that need real servers run in
           `ftps` (below) and in the sync reliability suite
  ftps     (Linux) L1: the strict vsftpd FTPS server of the sync reliability
           suite (TLS on login and data, this run's CA) against the lib test
           that writes through both upload paths, reads back and reopens the
           saved location
  ftp      (Linux) L1 through `se` against pyftpdlib: a saved FTP location
           lists, downloads and uploads byte-exact; mv and cp keep an existing
           file without --force; targets with CR or LF are refused and the
           server never runs the smuggled command; rm -r removes a tree
  server   (Linux) candidate share server: `cargo build --locked`, its tests
           (relay_startup.rs), versions of both Cargo.lock files, `cargo audit`
           of both locks with the ignores of build.yml
           (native/luecken-task-server.py)
  hostkeys (Linux) static: every older suite that connects to a fresh SFTP/SSH
           server confirms its host key first (M6)
  secrets  (Linux) a plaintext store of an older version is ported on first use:
           sealed records, no plaintext byte left, leftover stages removed, a
           record an older `se` writes later is ported again, doctor names the
           key protector; (Windows) the Credential Manager store still opens
  ssh      (Linux) OpenSSH container: an unknown key is refused with its
           fingerprint, a wrong fingerprint is refused, the confirmed key is
           pinned and forgotten again; OpenSSH known_hosts (also hashed) vouch
           for a key and against a different one; `@revoked` refuses a pinned
           key; a pin of an older version keeps working; an ssh-agent login
           stores no secret
  exec     (Linux) two paired clients over the candidate share server: Exec off
           by default, an Exec grant written before run-as existed keeps the
           full shell of the worker's account, program list refusals (shell,
           terminal, LD_PRELOAD/PATH/GIT_PAGER, other programs), locale
           variables allowed, the caller's and the worker's PATH never resolve
           the program, audit log (two classes) survives a worker restart
           without shell text, disable revokes a running command at once,
           run-as refused on a non-root worker; with passwordless sudo a root
           worker runs a device's commands as a local account (own environment
           and home, `/` when the home is missing) and as root again for
           `standard`
  update   (Linux) a real update: the published `se` 0.5.180 from the update
           feed of the base commit builds the old state (Share pair with an
           Exec grant, saved SFTP password connection with a silently pinned
           host key of a server with several key types, plaintext secrets),
           then the candidate replaces it in place and every old record is
           sealed by the worker start alone, nothing else changes
           (native/luecken-task-update.py)
  android  (--job android) the Android build stage of android/test-android-task.sh:
           dependency tree, NDK check and build, APKs, JVM unit tests
  android-device  (--job android-device, emulator) the published APK 0.5.180
           is updated to the candidate APK: sealed records under the Android
           Keystore, identity and saved connection unchanged
           (native/luecken-task-android.py, LueckenUpgradeTaskTest.kt)

Helpers (results, process runner, isolated clients) come from
test-ai-plugin-task.py. Every stage runs even when an earlier one failed; the
summary lists them all.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

NATIVE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("ai_plugin_task", NATIVE / "test-ai-plugin-task.py")
ai = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ai)
# The shared fmt/build helpers read these module globals: the batch starts
# after this commit, and a compiler warning inside one of these new files fails.
ai.BATCH_BASE = "01546a6307887f14d6e0b9a700d86a4c476bdb67"
ai.NEW_FILE_PREFIXES = ("src/share/core/pq_kx", "src/share/core/auth_penalty.rs",
                        "src/share/core/exec_policy_wire", "src/share/core/exec_policy_tests",
                        "src/share/core/exec_profile_compat_tests", "src/share/core/exec_program_path",
                        "src/share/os/shared/exec_audit", "src/share/os/linux_os/exec_account.rs",
                        "src/share/os/linux_os/exec_program_search.rs",
                        "src/share/os/linux_os/exec_supervisor_account.rs", "src/creds/core/sealed_record",
                        "src/creds/os/key_protector.rs", "src/creds/os/linux_systemd_creds.rs",
                        "src/creds/os/linux_file_store", "src/sftp/core/host_key",
                        "src/sftp/core/session_auth.rs", "src/sftp/os/shared/known_hosts",
                        "src/sftp/os/ssh_agent_",
                        "src/app/core/host_key_ui.rs", "src/cli/connections/host_keys.rs",
                        "src/cli/host_key_prompt.rs", "src/cli/setup_tests.rs",
                        "src/ftp/core/luecken_task_ftp")
WINDOWS = ai.WINDOWS
ensure = ai.ensure
Client = ai.Client
UNIT_FILTERS = ["luecken_task_", "creds::", "sftp::", "share::exec", "share::node", "share::handshake_limits",
                "share::server_admission", "share::auth_penalty", "share::fair_admission", "share::profile",
                "connect::", "daemon::ipc_protocol", "exec_grant_journal", "cli::", "app::share_exec_ui",
                "app::host_key_ui", "mobile::domains", "mobile::init", "mobile::pool", "ftp::"]
# Lib tests inside those filters that need real servers from a manifest
# (SE_SYNC_PROVIDER_MANIFEST): the FTPS one runs in the `ftps` stage, the
# provider matrix (under `connect::`) only in the sync reliability suite.
UNIT_SKIPS = ["sync_reliability_task_provider_ftps", "sync_reliability_task_provider_matrix"]
FTPS_PROVIDER_TEST = ("ftp::sync_reliability_task_provider_ftps_tests::"
                      "sync_reliability_task_provider_ftps_both_writers_confirm_bytes_after_tls_close")
# The `luecken_task_` test functions of native/src (125: `grep -c "fn luecken_task_"`)
# and what each platform compiles. Windows lacks the Linux/Android file store
# and its tests (`creds/os/linux_*`, 20), the `mobile` module (3: it is built
# for `any(target_os = "android", all(unix, test))`), the Linux account and
# program-search adapters (2) and three tests gated with `cfg(unix)` (3).
EXPECTED_LINUX = 125
EXPECTED_WINDOWS = 97
NOT_ON_WINDOWS_FILES = ("creds/os/linux_file_store_port_tests.rs", "creds/os/linux_file_store_tests.rs",
                        "creds/os/linux_os.rs", "mobile/os/shared/init.rs",
                        "mobile/os/shared/domains/host_keys.rs", "share/os/linux_os/exec_account.rs",
                        "share/os/linux_os/exec_program_search.rs")
NOT_ON_WINDOWS_TESTS = ("luecken_task_openssh_files_are_read_through_links_like_ssh_does",
                        "luecken_task_a_listed_applet_does_not_open_the_other_applets_of_its_binary",
                        "luecken_task_audit_files_are_owner_only")
MARKER = re.compile(r"\[se-host-key-unknown host=(\S+) port=(\d+) type=(\S+) fp=(SHA256:[^\]\s]+)\]")
LEGACY_MAGIC = b"SESEC01\0"
SEALED_MAGIC = b"SESEC02\0"
RUN_AS_USER = "se-luecken-runas"
PAIR_CHECK = "two clients paired; Exec off by default, refused without confirmation, then granted"
# The development share server of the `server` stage; the Share pairs of the
# `exec` and `update` stages run against it.
SHARE_SERVER = {"binary": None}


def helper(name):
    """Loads native/luecken-task-<name>.py (its functions take this module as `lib`)."""
    spec = importlib.util.spec_from_file_location("luecken_task_" + name, NATIVE / f"luecken-task-{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def source_test_names():
    """Names of the `luecken_task_` functions this platform compiles, from the sources."""
    names = set()
    src = NATIVE / "src"
    for path in src.rglob("*.rs"):
        relative = path.relative_to(src).as_posix()
        if WINDOWS and relative in NOT_ON_WINDOWS_FILES:
            continue
        for name in re.findall(r"\bfn (luecken_task_\w+)", path.read_text(encoding="utf-8", errors="replace")):
            if not (WINDOWS and name in NOT_ON_WINDOWS_TESTS):
                names.add(name)
    return names


def stage_unit(results, log_root):
    skips = [argument for name in UNIT_SKIPS for argument in ("--skip", name)]
    completed = ai.run(["cargo", "test", "--locked", "--lib", "--", *UNIT_FILTERS, *skips],
                       cwd=ai.NATIVE, log=log_root / "unit.log", timeout=5400)
    output = (completed.stdout + completed.stderr).decode(errors="replace")
    summary = [line for line in output.splitlines() if line.startswith("test result:")]
    results.record("unit", "luecken_task_ + affected modules", completed.returncode == 0,
                   failure_detail(output) if completed.returncode else "; ".join(summary))
    passed = [match.group(1) for match in re.finditer(r"^test (\S*luecken_task_\S*) \.\.\. ok$", output, re.MULTILINE)]
    expected = EXPECTED_WINDOWS if WINDOWS else EXPECTED_LINUX
    missing = sorted(source_test_names() - {name.rsplit("::", 1)[-1] for name in passed})
    results.record("unit", "luecken_task_ tests ran", len(passed) >= expected and not missing,
                   f"{len(passed)} of {expected}" + (f"; not run: {', '.join(missing[:30])}" if missing else ""))


def failure_detail(output):
    """Failed test names and their panics first, then the tail."""
    lines = output.splitlines()
    failed = [line for line in lines if line.startswith("test ") and line.endswith("FAILED")]
    panics = [line for line in lines if "panicked at" in line or line.startswith("thread '")]
    return "\n".join(failed[:40] + panics[:40]) + "\n...\n" + output[-3000:]


def text(completed):
    return (completed.stdout + b"\n" + completed.stderr).decode(errors="replace")


def wait_stopped(clients, timeout=30):
    """Ends the clients' background `se` processes and waits until they are gone."""
    ai.stop_background_daemons(clients)
    markers = {f"XDG_DATA_HOME={client.root / 'data'}".encode() for client in clients}
    deadline = time.time() + timeout
    while time.time() < deadline:
        alive = False
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                if set((entry / "environ").read_bytes().split(b"\0")) & markers:
                    alive = True
                    break
            except OSError:
                continue
        if not alive:
            return
        time.sleep(0.5)
    raise AssertionError("background se processes did not stop")


def write_private(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(data)
    os.chmod(path, 0o600)


def files_containing(root, needle):
    found = []
    for path in Path(root).rglob("*"):
        try:
            if path.is_file() and not path.is_symlink() and needle in path.read_bytes():
                found.append(str(path))
        except OSError:
            continue
    return found


# --------------------------------------------------------------------------- share server fixture (Linux)

def free_port_pair():
    """A loopback port whose successor is free as well (signaling port and relay port)."""
    for _ in range(100):
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
    raise AssertionError("no free pair of loopback ports")


def start_share_server(binary, logs, name):
    """Starts the development share server; returns (process, signaling address, relay URL)."""
    port = free_port_pair()
    server = subprocess.Popen([binary, f"127.0.0.1:{port}"], stdout=open(logs / f"share-server-{name}.log", "wb"),
                              stderr=subprocess.STDOUT)
    deadline = time.time() + 30
    while time.time() < deadline:
        if server.poll() is not None:
            raise AssertionError(f"the share server exited with {server.returncode}")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return server, f"127.0.0.1:{port}", f"http://127.0.0.1:{port + 1}"
        except OSError:
            time.sleep(0.2)
    server.terminate()
    raise AssertionError("the share server did not listen")


# --------------------------------------------------------------------------- OpenSSH fixture (Linux)

def start_openssh(log_root, public_key):
    user, password = "seuser", base64.b16encode(os.urandom(12)).decode().lower()
    name = f"se-luecken-ssh-{os.getpid()}"
    ai.run(["docker", "rm", "-f", name], timeout=120)
    started = ai.run(["docker", "run", "-d", "--name", name, "-p", "127.0.0.1::2222",
                      "-e", f"PUID={os.getuid()}", "-e", f"PGID={os.getgid()}", "-e", "PASSWORD_ACCESS=true",
                      "-e", f"USER_NAME={user}", "-e", f"USER_PASSWORD={password}",
                      "-e", f"PUBLIC_KEY={public_key}",
                      "lscr.io/linuxserver/openssh-server:latest"], timeout=600, log=log_root / "docker.log")
    ensure(started.returncode == 0, started.stderr.decode())
    mapped = ai.run(["docker", "port", name, "2222/tcp"]).stdout.decode().strip().splitlines()[0]
    port = int(mapped.rsplit(":", 1)[1])
    deadline = time.time() + 120
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
                if connection.recv(4).startswith(b"SSH-"):
                    break
        except OSError:
            time.sleep(1)
    else:
        raise AssertionError("OpenSSH container did not answer")
    time.sleep(2)
    return name, port, user, password


def openssh_fingerprint(key_base64):
    digest = hashlib.sha256(base64.b64decode(key_base64)).digest()
    return "SHA256:" + base64.b64encode(digest).decode().rstrip("=")


def scan_host_keys(port, work, log):
    """`ssh-keyscan` lines of the container and the fingerprint of each key type."""
    lines = []
    for _ in range(10):
        scanned = ai.run(["ssh-keyscan", "-p", str(port), "127.0.0.1"], log=log, timeout=60)
        lines = [line for line in scanned.stdout.decode().splitlines()
                 if line.strip() and not line.startswith("#")]
        if lines:
            break
        time.sleep(1)
    ensure(lines, "ssh-keyscan returned no host key")
    by_type = {line.split()[1]: openssh_fingerprint(line.split()[2]) for line in lines}
    path = work / "keyscan.txt"
    path.write_text("\n".join(lines) + "\n")
    listed = ai.run(["ssh-keygen", "-l", "-E", "sha256", "-f", path], log=log)
    keygen = {part for line in listed.stdout.decode().splitlines() for part in line.split()
              if part.startswith("SHA256:")}
    ensure(set(by_type.values()) == keygen, f"own fingerprints {by_type} differ from ssh-keygen {keygen}")
    return lines, by_type


def add_password_connection(client, se_binary, port, user, password, label, log):
    added = client.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                      "--user", user, "--root", "/config", "--label", label, "--password-stdin",
                      input_text=password + "\n", log=log)
    ensure(added.returncode == 0, text(added))


def stage_ssh(results, se_binary, work, log_root):
    stage = "ssh"
    logs = log_root / "ssh"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "cli.log"
    agent_key = work / "agent_key"
    generated = ai.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "se-luecken-agent", "-f", agent_key],
                       log=log)
    if generated.returncode != 0:
        results.record(stage, "agent key fixture", False, text(generated))
        return
    public_key = (work / "agent_key.pub").read_text().strip()
    try:
        container, port, user, password = start_openssh(log_root, public_key)
        lines, by_type = scan_host_keys(port, work, log)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "OpenSSH container fixture", False, f"{type(error).__name__}: {error}")
        return
    clients = []
    presented = {}

    def client(name, extra=None):
        made = Client(work, name, extra)
        clients.append(made)
        return made

    try:
        host_key_checks(results, se_binary, client("hostkey"), port, user, password, by_type, presented, log)
        openssh_checks(results, se_binary, client, work, port, user, password, lines, log)
        revoked_checks(results, se_binary, client, port, user, password, lines, log)
        old_pin_and_secret_checks(results, se_binary, client("legacy"), port, user, password, by_type,
                                  presented, log)
        agent_checks(results, se_binary, client, work, port, user, by_type, presented, log)
    finally:
        ai.stop_background_daemons(clients)
        ai.run(["docker", "rm", "-f", container], timeout=120)


def host_key_checks(results, se_binary, hostkey, port, user, password, by_type, presented, log):
    stage = "ssh"
    store = hostkey.app_data / "known_hosts_sftp.txt"
    pin_line = f"127.0.0.1:{port} "

    def pinned():
        return store.exists() and any(line.startswith(pin_line) for line in store.read_text().splitlines())

    def unknown():
        add_password_connection(hostkey, se_binary, port, user, password, "box", log)
        listed = hostkey.se(se_binary, "ls", "@box:/", log=log)
        ensure(listed.returncode != 0, "a server with an unknown host key was accepted")
        found = MARKER.search(text(listed))
        ensure(found, f"no host-key marker in: {text(listed)[-1500:]}")
        host, marker_port, algorithm, fingerprint = found.groups()
        ensure(host == "127.0.0.1" and int(marker_port) == port, found.group(0))
        ensure(by_type.get(algorithm) == fingerprint, f"{algorithm} {fingerprint} is not the server's key {by_type}")
        ensure("trust-host-key" in text(listed), "the error does not name the confirming command")
        ensure(not pinned(), "the unknown key was stored without confirmation")
        presented.update(algorithm=algorithm, fingerprint=fingerprint)
    results.check(stage, "unknown host key: refused with type and fingerprint, nothing stored", unknown)

    def wrong_fingerprint():
        fingerprint = presented.get("fingerprint")
        ensure(fingerprint, "no presented fingerprint from the previous check")
        wrong = "SHA256:" + base64.b64encode(bytes(32)).decode().rstrip("=")
        refused = hostkey.se(se_binary, "connections", "trust-host-key", "box", "--fingerprint", wrong, log=log)
        ensure(refused.returncode != 0, "a fingerprint the server does not present was pinned")
        ensure(fingerprint in text(refused), "the refusal does not show the presented fingerprint")
        ensure(not pinned(), "a wrong fingerprint was stored")
    results.check(stage, "trust-host-key refuses a fingerprint the server does not present", wrong_fingerprint)

    def trust_and_forget():
        fingerprint = presented.get("fingerprint")
        ensure(fingerprint, "no presented fingerprint from the first check")
        trusted = hostkey.se(se_binary, "connections", "trust-host-key", "box", "--fingerprint", fingerprint,
                             log=log)
        ensure(trusted.returncode == 0 and "pinned" in text(trusted), text(trusted))
        ensure(f"{pin_line}{fingerprint}" in store.read_text().splitlines(), "pin line missing")
        listed = hostkey.se(se_binary, "ls", "@box:/", log=log)
        ensure(listed.returncode == 0, text(listed))
        forgotten = hostkey.se(se_binary, "connections", "forget-host-key", f"127.0.0.1:{port}", log=log)
        ensure(forgotten.returncode == 0 and "forgotten" in text(forgotten), text(forgotten))
        ensure(not pinned(), "the pin is still stored")
        # Without a terminal there is nobody to ask: the fingerprint is shown instead.
        asked = hostkey.se(se_binary, "connections", "trust-host-key", "box", log=log)
        ensure(asked.returncode != 0 and fingerprint in text(asked) and "--fingerprint" in text(asked),
               text(asked))
        ensure(not pinned(), "a key was pinned without confirmation")
    results.check(stage, "confirmed fingerprint is pinned and connects; forget removes the pin", trust_and_forget)

    def mcp_refuses():
        # An entry point that never has a terminal: it must answer at once with the fingerprint and the
        # way to confirm, and store nothing (V9).
        session = ai.McpSession(se_binary, hostkey, "initialize", log.parent, "luecken-hostkey")
        try:
            started = time.time()
            shown = json.dumps(session.call("list", {"target": "@box:/"}, expect_error=True, timeout=120))
            ensure(time.time() - started < 100, "the MCP list of an unpinned host hung")
            ensure("SHA256:" in shown and "trust-host-key" in shown, shown[-1500:])
            ensure(not pinned(), "the MCP list stored the unknown key")
        finally:
            session.close()
    results.check(stage, "MCP list of an unpinned host fails at once with the fingerprint and the confirming "
                  "command, nothing stored", mcp_refuses)


def openssh_checks(results, se_binary, client, work, port, user, password, lines, log):
    stage = "ssh"

    def known_hosts(target, content, hashed=False):
        path = target.root / "home" / ".ssh" / "known_hosts"
        path.parent.mkdir(parents=True, exist_ok=True)
        os.chmod(path.parent, 0o700)
        path.write_text(content)
        if hashed:
            converted = ai.run(["ssh-keygen", "-H", "-f", path], log=log)
            ensure(converted.returncode == 0, text(converted))
            ensure("127.0.0.1" not in path.read_text(), "ssh-keygen -H left a plain host name")
        return path

    def vouched(name, hashed):
        target = client(name)
        known_hosts(target, "\n".join(lines) + "\n", hashed)
        add_password_connection(target, se_binary, port, user, password, "box", log)
        listed = target.se(se_binary, "ls", "@box:/", log=log)
        ensure(listed.returncode == 0, text(listed))
        store = target.app_data / "known_hosts_sftp.txt"
        ensure(not store.exists() or f"127.0.0.1:{port} " not in store.read_text(),
               "a key vouched for by OpenSSH was copied into Smart Explorer's pins")
    results.check(stage, "OpenSSH known_hosts vouches for the server's key", lambda: vouched("openssh", False))
    results.check(stage, "hashed OpenSSH known_hosts (ssh-keygen -H) vouches for the key",
                  lambda: vouched("openssh-hashed", True))

    def changed():
        other_key = work / "other_host_key"
        generated = ai.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", other_key], log=log)
        ensure(generated.returncode == 0, text(generated))
        algorithm, key = (work / "other_host_key.pub").read_text().split()[:2]
        target = client("openssh-changed")
        known_hosts(target, f"[127.0.0.1]:{port} {algorithm} {key}\n")
        add_password_connection(target, se_binary, port, user, password, "box", log)
        listed = target.se(se_binary, "ls", "@box:/", log=log)
        ensure(listed.returncode != 0, "a key different from the one in known_hosts was accepted")
        ensure("changed" in text(listed) and not MARKER.search(text(listed)),
               f"not reported as a changed key: {text(listed)[-1500:]}")
    results.check(stage, "a different key in OpenSSH known_hosts refuses the connection", changed)


def revoked_checks(results, se_binary, client, port, user, password, lines, log):
    """`@revoked` in the OpenSSH known_hosts refuses a key Smart Explorer has pinned (D3/E6)."""
    stage = "ssh"
    keys = {line.split()[1]: line.split()[2] for line in lines}

    def revoked_pin():
        target = client("revoked")
        add_password_connection(target, se_binary, port, user, password, "box", log)
        algorithm, fingerprint = ai.trust_ssh_host_key(target, se_binary, "127.0.0.1", port, log)
        listed = target.se(se_binary, "ls", "@box:/", log=log)
        ensure(listed.returncode == 0, f"the pinned key does not connect: {text(listed)}")
        pins = target.app_data / "known_hosts_sftp.txt"
        pinned = pins.read_bytes()
        path = target.root / "home" / ".ssh" / "known_hosts"
        path.parent.mkdir(parents=True, exist_ok=True)
        os.chmod(path.parent, 0o700)
        others = [name for name in keys if name != algorithm]
        ensure(others, f"the server presents only {sorted(keys)}")
        # Another key of this host revoked: the pinned key still connects.
        path.write_text(f"@revoked [127.0.0.1]:{port} {others[0]} {keys[others[0]]}\n")
        unaffected = target.se(se_binary, "ls", "@box:/", log=log)
        ensure(unaffected.returncode == 0, f"a revoked OTHER key refused the pinned one: {text(unaffected)}")
        # The pinned key revoked, for the host and for every host.
        for revoking in (f"[127.0.0.1]:{port}", "*"):
            path.write_text(f"@revoked {revoking} {algorithm} {keys[algorithm]}\n")
            refused = target.se(se_binary, "ls", "@box:/", log=log)
            ensure(refused.returncode != 0, f"a revoked key with a Smart Explorer pin connected ({revoking})")
            shown = text(refused)
            ensure("revoked" in shown and fingerprint in shown and not MARKER.search(shown),
                   f"not reported as a revoked key: {shown[-1500:]}")
            ensure(pins.read_bytes() == pinned, "the pin changed")
            # `se exec @label` is no way around it.
            denied = target.se(se_binary, "exec", "@box", "--", "echo", "LUECKEN_MUST_NOT_RUN", log=log)
            ensure(denied.returncode != 0 and b"LUECKEN_MUST_NOT_RUN" not in denied.stdout,
                   f"a revoked key ran a command: {text(denied)}")
        path.unlink()
        lifted = target.se(se_binary, "ls", "@box:/", log=log)
        ensure(lifted.returncode == 0, f"the pin does not work again once the revocation is gone: {text(lifted)}")
        return f"{algorithm} {fingerprint}"
    results.check(stage, "@revoked in ~/.ssh/known_hosts refuses a pinned key (host and wildcard), other keys and "
                  "a lifted revocation leave the pin working", revoked_pin)

    def exec_over_ssh():
        target = client("revoked")
        echoed = target.se(se_binary, "exec", "@box", "--", "echo", "ssh exec ok", log=log)
        ensure(echoed.returncode == 0 and echoed.stdout.decode() == "ssh exec ok\n", text(echoed))
        code = target.se(se_binary, "exec", "@box", "--shell", "exit 5", log=log)
        ensure(code.returncode == 5, code.returncode)
        session = ai.McpSession(se_binary, target, "initialize", log.parent, "luecken-ssh")
        try:
            result = ai.run_exec(session, {"target": "@box", "command": "echo out; exit 3"})
            ensure(result["status"] == "completed" and result["exit_code"] == 3, result)
        finally:
            session.close()
    results.check(stage, "se exec @label and the MCP exec tool run over the confirmed pin", exec_over_ssh)


def old_pin_and_secret_checks(results, se_binary, legacy, port, user, password, by_type, presented, log):
    """An installation of an older version: a pin and a plaintext password record."""
    stage = "secrets"
    store = legacy.app_data / "secrets-v1"
    account = f"sftp://{user}@127.0.0.1:{port}/config"
    digest = hashlib.sha256(b"smart_explorer\0" + account.encode()).digest()
    record = store / f"{digest.hex()}.secret"
    secret = password.encode()

    def legacy_record():
        body = LEGACY_MAGIC + bytes([1]) + digest + len(secret).to_bytes(4, "big") + secret
        return body + hashlib.sha256(body).digest()

    def doctor():
        report = legacy.se(se_binary, "doctor", "--json", log=log)
        status = json.loads(report.stdout)["credential_store"]
        ensure(status["state"] == "ok", status)
        return status["detail"]

    def ported():
        add_password_connection(legacy, se_binary, port, user, password, "box", log)
        ensure(record.exists(), f"no record for {account}; files: {sorted(p.name for p in store.iterdir())}")
        ensure(record.read_bytes().startswith(SEALED_MAGIC), "a new record is not sealed")
        ensure(not files_containing(legacy.root, secret), "a new password is stored in plaintext")
        records = sorted(path.name for path in store.iterdir() if path.name.endswith(".secret"))
        ensure(records == [record.name], f"unexpected records {records}")
        # Back to the layout of an older version: plaintext records only, no
        # key, plus a stage file an interrupted write left behind.
        wait_stopped([legacy])
        shutil.rmtree(store)
        key_file = store.with_suffix(".kek")
        if key_file.exists():
            key_file.unlink()
        store.mkdir(mode=0o700)
        write_private(record, legacy_record())
        write_private(store / ".stage-luecken-interrupted", b"LEFTOVER " + secret)
        fingerprint = presented.get("fingerprint") or by_type.get("ssh-ed25519")
        ensure(fingerprint, "no host key fingerprint to pin")
        # The pin format of older versions: `host:port SHA256:...`.
        write_private(legacy.app_data / "known_hosts_sftp.txt", f"127.0.0.1:{port} {fingerprint}\n".encode())
        listed = legacy.se(se_binary, "ls", "@box:/", log=log)
        ensure(listed.returncode == 0, f"the ported password or the old pin did not work: {text(listed)}")
        ensure(record.read_bytes().startswith(SEALED_MAGIC), "the plaintext record was not sealed")
        ensure(not (store / ".stage-luecken-interrupted").exists(), "the leftover stage file is still there")
        leaks = files_containing(legacy.root, secret)
        ensure(not leaks, f"plaintext password still on disk: {leaks}")
        ensure(oct(store.stat().st_mode & 0o777) == oct(0o700), oct(store.stat().st_mode))
        ensure(oct(record.stat().st_mode & 0o777) == oct(0o600), oct(record.stat().st_mode))
        detail = doctor()
        ensure("plaintext_records=0" in detail and "protector=none-yet" not in detail, detail)
        print(f"[INFO] secrets: {detail}", flush=True)
    results.check(stage, "plaintext store of an older version: ported, sealed, no plaintext left, old pin works",
                  ported)

    def mixed_versions():
        ensure(record.exists(), "no record from the previous check")
        # An older `se` that still runs writes a plaintext record again.
        write_private(record, legacy_record())
        detail = doctor()
        ensure(record.read_bytes().startswith(SEALED_MAGIC), "the late plaintext record was not sealed")
        ensure(not files_containing(legacy.root, secret), "plaintext password still on disk")
        ensure("plaintext_records=0" in detail, detail)
        listed = legacy.se(se_binary, "ls", "@box:/", log=log)
        ensure(listed.returncode == 0, text(listed))
    results.check(stage, "a plaintext record an older se writes later is ported on the next start", mixed_versions)


def agent_checks(results, se_binary, client, work, port, user, by_type, presented, log):
    stage = "ssh"
    socket_path = work / "agent.sock"
    started = ai.run(["ssh-agent", "-s", "-a", socket_path], log=log)
    if started.returncode != 0:
        results.record(stage, "ssh-agent fixture", False, text(started))
        return
    agent_pid = re.search(r"SSH_AGENT_PID=(\d+)", started.stdout.decode())
    environment = dict(os.environ, SSH_AUTH_SOCK=str(socket_path))
    try:
        added = ai.run(["ssh-add", work / "agent_key"], env=environment, log=log)
        if added.returncode != 0:
            results.record(stage, "ssh-agent fixture", False, text(added))
            return
        target = client("agent", {"SSH_AUTH_SOCK": str(socket_path)})

        def agent_login():
            made = target.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                             "--user", user, "--root", "/config", "--label", "agentbox", "--ssh-agent", log=log)
            ensure(made.returncode == 0, text(made))
            fingerprint = presented.get("fingerprint") or by_type.get("ssh-ed25519")
            trusted = target.se(se_binary, "connections", "trust-host-key", "agentbox", "--fingerprint",
                                fingerprint, log=log)
            ensure(trusted.returncode == 0, text(trusted))
            listed = target.se(se_binary, "ls", "@agentbox:/", log=log)
            ensure(listed.returncode == 0, text(listed))
            account = f"sftp://{user}@127.0.0.1:{port}/config"
            digest = hashlib.sha256(b"smart_explorer\0" + account.encode()).hexdigest()
            ensure(not (target.app_data / "secrets-v1" / f"{digest}.secret").exists(),
                   "an ssh-agent login stored a secret")
        results.check(stage, "ssh-agent login connects and stores no secret", agent_login)
    finally:
        if agent_pid:
            try:
                os.kill(int(agent_pid.group(1)), 15)
            except OSError:
                pass


# --------------------------------------------------------------------------- Share Exec (Linux)

class RootClient:
    """An isolated `se` whose commands and worker run as root (passwordless sudo)."""

    def __init__(self, root, name, extra=None):
        self.root = Path(root) / name
        for directory in ("home", "data", "config", "runtime"):
            ai.run(["sudo", "-n", "install", "-d", "-m", "0700", "-o", "root", "-g", "root",
                    self.root / directory], timeout=60)
        self.env = {
            "HOME": str(self.root / "home"), "XDG_DATA_HOME": str(self.root / "data"),
            "XDG_CONFIG_HOME": str(self.root / "config"), "XDG_RUNTIME_DIR": str(self.root / "runtime"),
            "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        }
        self.env.update(extra or {})

    @property
    def app_data(self):
        return self.root / "data" / "smart_explorer"

    def se(self, se_binary, *args, input_text=None, timeout=180, log=None):
        assignments = [f"{name}={value}" for name, value in self.env.items()]
        return ai.run(["sudo", "-n", "env", "-i", *assignments, se_binary, *args], input_text=input_text,
                      timeout=timeout, log=log)

    def stop(self):
        script = ("import os, sys\nmarkers = {m.encode() for m in sys.argv[1:]}\n"
                  "for entry in os.listdir('/proc'):\n"
                  "    if not entry.isdigit():\n        continue\n"
                  "    try:\n        environ = set(open(f'/proc/{entry}/environ', 'rb').read().split(b'\\0'))\n"
                  "    except OSError:\n        continue\n"
                  "    if environ & markers:\n"
                  "        try:\n            os.kill(int(entry), 15)\n        except OSError:\n            pass\n")
        ai.run(["sudo", "-n", "python3", "-I", "-c", script, f"XDG_DATA_HOME={self.root / 'data'}"], timeout=60)


def pair_with_exec(se_binary, a, b, signal, relay, log):
    """Pairs A with B (Direct) and enables Exec for A on B; returns A's endpoint of B."""

    def worker_ready(client):
        status = json.loads(client.se(se_binary, "share", "status", "--json", log=log).stdout)
        worker = status["worker"]
        return worker["reachable"] and worker["running"] and worker["connected"] and \
            (worker.get("relay_url") or "").rstrip("/") == relay

    code = json.loads(b.se(se_binary, "share", "identity", "--json", log=log).stdout)["direct_code"]
    for client in (b, a):
        configured = client.se(se_binary, "share", "configure", "--server", f"tcp://{signal}",
                               "--allow-plaintext", log=log)
        ensure(configured.returncode == 0, text(configured))
        ai.poll(lambda client=client: worker_ready(client), 90, "Share worker did not connect")
    added = json.loads(a.se(se_binary, "connections", "add-peer", "--code", code, "--name", "Target", "--json",
                            log=log).stdout)
    endpoint, request_id = added["endpoint"], added["request_id"]

    def accepted():
        inbox = json.loads(b.se(se_binary, "share", "request", "--json", log=log).stdout)
        if any(entry.get("decision", {}).get("state") == "pending" for entry in inbox.get("requests", [])):
            b.se(se_binary, "share", "request", "accept", "--json", log=log)
        shown = json.loads(a.se(se_binary, "share", "request", "show", request_id, "--json", log=log).stdout)
        return shown["decision"]["state"] == "accepted" and shown["authorization"]["active"]
    ai.poll(accepted, 90, "the access request was never accepted")
    # Exec is off for every device until its owner enables it locally and confirms (V3):
    # file access never includes it, and the warning is not skipped without a terminal.
    ensure(grants(b, se_binary, log)["enabled"] is False, "Exec is on before anybody enabled it")
    denied = a.se(se_binary, "exec", endpoint, "--", "id", "-u", log=log, timeout=120)
    ensure(denied.returncode != 0 and not denied.stdout.strip(), f"Exec ran on a device that is off: {text(denied)}")
    for answer in (None, "yes\n", "\n"):
        unconfirmed = b.se(se_binary, "share", "grants", "exec", "enable", input_text=answer, log=log)
        ensure(unconfirmed.returncode != 0, f"Exec was enabled with the answer {answer!r}")
        ensure(grants(b, se_binary, log)["enabled"] is False, f"Exec is on after the answer {answer!r}")
    # Only the typed word YES (the warning is shown on stderr) enables it, from the local CLI.
    typed = b.se(se_binary, "share", "grants", "exec", "enable", "--json", input_text="YES\n", log=log)
    ensure(typed.returncode == 0, text(typed))
    enabled = json.loads(typed.stdout)
    ensure(enabled.get("persisted") and enabled.get("applied"), enabled)
    ensure(b"WARNING" in typed.stderr, "no warning was shown before enabling Exec")
    return endpoint, worker_ready


def grants(b, se_binary, log):
    listed = b.se(se_binary, "share", "grants", "exec", "--json", log=log)
    ensure(listed.returncode == 0, text(listed))
    values = json.loads(listed.stdout)
    ensure(len(values) == 1, values)
    return values[0]


def set_grant(b, se_binary, log, *options):
    return b.se(se_binary, "share", "grants", "exec", "enable", "--yes", "--json", *options, log=log)


def exec_out(a, se_binary, endpoint, log, *command, timeout=180):
    return a.se(se_binary, "exec", endpoint, *command, log=log, timeout=timeout)


def exec_until_ok(a, se_binary, endpoint, log, *command):
    """The first command after a grant change: the peer may reconnect first."""
    last = []

    def attempt():
        done = exec_out(a, se_binary, endpoint, log, *command)
        last[:] = [done]
        return done if done.returncode == 0 else None
    try:
        return ai.poll(attempt, 120, f"exec {' '.join(command)} never succeeded")
    except AssertionError as error:
        raise AssertionError(f"{error}: {text(last[0])[-1500:] if last else 'no attempt'}") from None


def audit_entries(b, se_binary, log):
    shown = b.se(se_binary, "share", "exec", "log", "--json", "--lines", "500", log=log)
    ensure(shown.returncode == 0, text(shown))
    return json.loads(shown.stdout)


def exec_grants_in(value):
    """Every Exec grant object inside a share_profiles.json document."""
    found = []
    if isinstance(value, dict):
        grant = value.get("exec")
        if isinstance(grant, dict) and "policy_revision" in grant:
            found.append(grant)
        for child in value.values():
            found.extend(exec_grants_in(child))
    elif isinstance(value, list):
        for child in value:
            found.extend(exec_grants_in(child))
    return found


def stage_exec(results, se_binary, work, log_root):
    stage = "exec"
    logs = log_root / "exec"
    logs.mkdir(parents=True, exist_ok=True)
    binary = SHARE_SERVER["binary"]
    if binary is None:
        results.record(stage, "candidate share server fixture", False, "the server stage did not build it")
        return
    try:
        server, signal, relay = start_share_server(binary, logs, "exec")
    except Exception as error:  # noqa: BLE001
        results.record(stage, "candidate share server fixture", False, f"{type(error).__name__}: {error}")
        return
    try:
        exec_standard_checks(results, se_binary, work, logs, signal, relay)
        if ai.run(["sudo", "-n", "true"], timeout=30).returncode == 0:
            exec_root_checks(results, se_binary, work, logs, signal, relay)
        else:
            results.record(stage, "root worker checks need passwordless sudo", False, "sudo -n true failed")
    finally:
        server.terminate()
        server.wait(timeout=30)


def exec_standard_checks(results, se_binary, work, logs, signal, relay):
    import pwd

    stage = "exec"
    log = logs / "standard.log"
    worker_user = pwd.getpwuid(os.getuid()).pw_name
    # A `uname` of this folder stands first on the host worker's own PATH: a grant that resolves
    # a program through any PATH would run it instead of the system program (no other check runs
    # `uname`, so the full-shell checks are not affected).
    planted = work / "planted-path"
    planted.mkdir(parents=True, exist_ok=True)
    (planted / "uname").write_text("#!/bin/sh\necho FAKE_UNAME_LUECKEN\n")
    (planted / "uname").chmod(0o755)
    host_path = f"{planted}:{os.environ.get('PATH', '/usr/local/bin:/usr/bin:/bin')}"
    clients = [Client(work, "exec-a", {"SE_SHARE_RELAY_ONLY": "1"}),
               Client(work, "exec-b", {"SE_SHARE_RELAY_ONLY": "1", "PATH": host_path})]
    a, b = clients
    try:
        try:
            endpoint, worker_ready = pair_with_exec(se_binary, a, b, signal, relay, log)
            results.record(stage, PAIR_CHECK, True, endpoint)
        except Exception as error:  # noqa: BLE001
            results.record(stage, PAIR_CHECK, False, f"{type(error).__name__}: {error}")
            return

        def ported_grant():
            # The grant as a version before run-as wrote it: no run_as, no programs.
            wait_stopped([b])
            profiles = b.app_data / "share_profiles.json"
            document = json.loads(profiles.read_text())
            stored = exec_grants_in(document)
            ensure(stored, "no Exec grant in share_profiles.json")
            before = [(grant["enabled"], grant["policy_revision"]) for grant in stored]
            for grant in stored:
                grant.pop("run_as", None)
                grant.pop("programs", None)
            profiles.write_text(json.dumps(document, indent=2))
            ai.poll(lambda: worker_ready(b), 120, "the restarted worker did not connect")
            grant = grants(b, se_binary, log)
            ensure(grant["enabled"] and grant["run_as"] == "standard" and grant["programs"] == [], grant)
            after = [(grant["enabled"], grant["policy_revision"])
                     for grant in exec_grants_in(json.loads(profiles.read_text()))]
            ensure(after == before, f"enabled/revision changed by the port: {before} -> {after}")
            ran = exec_until_ok(a, se_binary, endpoint, log, "--", "id", "-un")
            ensure(ran.stdout.decode().strip() == worker_user, ran.stdout)
            shell = exec_out(a, se_binary, endpoint, log, "--shell", "printf LUECKEN_SHELL_TEXT_%s 7")
            ensure(shell.returncode == 0 and shell.stdout.decode() == "LUECKEN_SHELL_TEXT_7", text(shell))
        results.check(stage, "grant from before run-as: still enabled, full shell of the worker's account",
                      ported_grant)

        def program_list():
            changed = set_grant(b, se_binary, log, "--allow-program", "/usr/bin/id")
            ensure(changed.returncode == 0, text(changed))
            grant = grants(b, se_binary, log)
            ensure(grant["programs"] == ["/usr/bin/id"] and grant["run_as"] == "standard", grant)
            allowed = exec_until_ok(a, se_binary, endpoint, log, "--", "/usr/bin/id", "-un")
            ensure(allowed.stdout.decode().strip() == worker_user, text(allowed))
            shell = exec_out(a, se_binary, endpoint, log, "--shell", "id -un")
            ensure(shell.returncode != 0 and "listed programs" in text(shell), text(shell))
            other = exec_out(a, se_binary, endpoint, log, "--", "/usr/bin/whoami")
            ensure(other.returncode != 0 and "listed programs" in text(other), text(other))
        results.check(stage, "program list: listed program runs, shell and other programs are refused", program_list)

        def program_environment():
            # Only locale and terminal variables of the caller reach a job with a program list.
            locale = exec_until_ok(a, se_binary, endpoint, log, "--env", "LANG=C.UTF-8", "--env", "LC_ALL=C",
                                   "--", "/usr/bin/id", "-un")
            ensure(locale.stdout.decode().strip() == worker_user, text(locale))
            for name, value in (("LD_PRELOAD", "/tmp/luecken-preload.so"), ("PATH", "/tmp/luecken-bin"),
                                ("GIT_PAGER", "/bin/sh")):
                refused = exec_out(a, se_binary, endpoint, log, "--env", f"{name}={value}", "--", "/usr/bin/id", "-un")
                shown = text(refused)
                ensure(refused.returncode != 0 and not refused.stdout.strip(), f"{name} reached a job: {shown}")
                ensure("listed programs" in shown and name in shown and "not allowed" in shown, shown)
            plain = exec_out(a, se_binary, endpoint, log, "--env", "LANG=/tmp/luecken-locale", "--", "/usr/bin/id")
            ensure(plain.returncode != 0 and "plain" in text(plain), f"a path as a locale was accepted: {text(plain)}")
        results.check(stage, "program list: LANG/LC_* allowed; LD_PRELOAD, PATH, GIT_PAGER and path-like values "
                      "refused with the variable's name", program_environment)

        def program_resolution():
            changed = set_grant(b, se_binary, log, "--allow-program", "uname")
            ensure(changed.returncode == 0, text(changed))
            ensure(grants(b, se_binary, log)["programs"] == ["uname"], "the bare program name was not stored")
            ran = exec_until_ok(a, se_binary, endpoint, log, "--", "uname", "-s")
            ensure("FAKE_UNAME_LUECKEN" not in text(ran) and ran.stdout.decode().strip() == "Linux",
                   f"the program was resolved through a PATH: {text(ran)}")
            started = [entry for entry in audit_entries(b, se_binary, log)
                       if entry.get("event") == "started" and entry.get("resolved_program")]
            resolved = started[-1]["resolved_program"] if started else ""
            ensure(resolved.startswith("/") and resolved.endswith("/uname") and not resolved.startswith(str(planted)),
                   f"resolved program: {resolved!r}")
            restored = set_grant(b, se_binary, log, "--allow-program", "/usr/bin/id")
            ensure(restored.returncode == 0, text(restored))
            return resolved
        results.check(stage, "program list: a bare program name resolves in the fixed system path, never through "
                      "the worker's or the caller's PATH", program_resolution)

        def audit():
            entries = audit_entries(b, se_binary, log)
            events = [entry.get("event") for entry in entries]
            started = [entry for entry in entries if entry.get("event") == "started"]
            ensure(started and all(str(entry.get("account", "")).startswith("standard") for entry in started),
                   started[:3])
            ensure("finished" in events, events)
            refused = [entry for entry in entries if entry.get("event") == "start_refused"]
            ensure(len(refused) >= 2 and all("listed programs" in entry.get("reason", "") for entry in refused),
                   refused)
            ensure(all(entry.get("device_id") and entry.get("fingerprint") for entry in started + refused),
                   "an entry lacks the device")
            env_reasons = " ".join(entry.get("reason", "") for entry in refused
                                   if "environment variable" in entry.get("reason", ""))
            ensure(all(name in env_reasons for name in ("LD_PRELOAD", "PATH", "GIT_PAGER")), env_reasons)
            ensure("luecken-preload" not in env_reasons and "luecken-bin" not in env_reasons,
                   "an environment value was logged")
            files = [b.app_data / "exec-audit.log", b.app_data / "exec-audit.log.1"]
            raw = b"".join(path.read_bytes() for path in files if path.exists())
            ensure(b"LUECKEN_SHELL_TEXT" not in raw and b"LUECKEN_SHELL_TEXT" not in json.dumps(entries).encode(),
                   "shell command text was logged")
        results.check(stage, "audit log: starts, ends and refusals with device, never shell text", audit)

        def audit_survives_restart():
            def execution_ids(entries):
                return {entry["exec_id"] for entry in entries
                        if entry.get("event") in ("started", "finished", "start_refused")}

            def lines_of(*names):
                found = []
                for name in names:
                    path = b.app_data / name
                    if path.exists():
                        found.extend(json.loads(line) for line in path.read_text().splitlines() if line.strip())
                return found
            before = execution_ids(audit_entries(b, se_binary, log))
            ensure(before, "no execution entries before the restart")
            wait_stopped([b])
            ai.poll(lambda: worker_ready(b), 120, "the restarted worker did not connect")
            after = audit_entries(b, se_binary, log)
            ensure(before <= execution_ids(after), f"entries lost by the restart: {before - execution_ids(after)}")
            events = {entry.get("event") for entry in after}
            ensure({"started", "finished", "start_refused"} <= events, events)
            executions = lines_of("exec-audit.log", "exec-audit.log.1")
            rejections = lines_of("exec-audit-rejected.log", "exec-audit-rejected.log.1")
            ensure(executions and {entry["event"] for entry in executions} <= {"start_refused", "started", "finished"},
                   "the execution log holds another class")
            ensure(rejections and {entry["event"] for entry in rejections} <= {"auth_rejected", "penalty_refused"},
                   f"rejected log: {rejections[:3]}")
            ensure(all(entry.get("remote_node") and entry.get("reason", "x") for entry in rejections), rejections[:3])
            ensure(oct((b.app_data / "exec-audit.log").stat().st_mode & 0o777) == oct(0o600), "audit log mode")
            return f"{len(executions)} execution entries, {len(rejections)} rejections"
        results.check(stage, "audit log survives a worker restart; executions and rejections of strangers stay in "
                      "separate files (0600)", audit_survives_restart)

        def run_as_refused():
            refused = set_grant(b, se_binary, log, "--run-as", "nobody")
            ensure(refused.returncode != 0, "a non-root worker accepted run-as")
            grant = grants(b, se_binary, log)
            ensure(grant["run_as"] == "standard" and grant["programs"] == ["/usr/bin/id"], grant)
            full = set_grant(b, se_binary, log, "--full")
            ensure(full.returncode == 0, text(full))
            ensure(grants(b, se_binary, log)["programs"] == [], "the program list was not removed")
            shell = exec_until_ok(a, se_binary, endpoint, log, "--shell", "id -un")
            ensure(shell.stdout.decode().strip() == worker_user, text(shell))
        results.check(stage, "run-as refused on a non-root worker, grant unchanged; --full restores the shell",
                      run_as_refused)

        def disable_revokes_running():
            running = subprocess.Popen([str(se_binary), "exec", endpoint, "--", "sleep", "120"], env=a.env,
                                       stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                def started():
                    return any(entry.get("event") == "started" and entry.get("program") == "sleep"
                               for entry in audit_entries(b, se_binary, log))
                ai.poll(started, 90, "the long command never started")
                ensure(running.poll() is None, "the long command ended by itself")
                off = b.se(se_binary, "share", "grants", "exec", "disable", "--json", log=log)
                ensure(off.returncode == 0, text(off))
                try:
                    running.wait(timeout=60)
                except subprocess.TimeoutExpired:
                    raise AssertionError("disabling Exec did not end the running command") from None
                ensure(running.returncode != 0, "the revoked command ended normally")
            finally:
                if running.poll() is None:
                    running.kill()
                running.wait()
            back = set_grant(b, se_binary, log)
            ensure(back.returncode == 0, text(back))
            ran = exec_until_ok(a, se_binary, endpoint, log, "--", "id", "-un")
            ensure(ran.stdout.decode().strip() == worker_user, text(ran))
        results.check(stage, "disabling Exec ends a running command at once; enabling again restores the full shell",
                      disable_revokes_running)
    finally:
        ai.stop_background_daemons(clients)


def exec_root_checks(results, se_binary, work, logs, signal, relay):
    stage = "exec-root"
    log = logs / "root.log"
    created = ai.run(["sudo", "-n", "useradd", "--create-home", "--shell", "/bin/sh", RUN_AS_USER], log=log)
    if created.returncode not in (0, 9):  # 9: the account already exists
        results.record(stage, "run-as account fixture", False, text(created))
        return
    a = Client(work, "root-a", {"SE_SHARE_RELAY_ONLY": "1"})
    b = RootClient(work, "root-b", {"SE_SHARE_RELAY_ONLY": "1"})
    try:
        try:
            endpoint, _ = pair_with_exec(se_binary, a, b, signal, relay, log)
            results.record(stage, "client paired with a root worker; Exec off by default, then granted", True, endpoint)
        except Exception as error:  # noqa: BLE001
            results.record(stage, "client paired with a root worker; Exec off by default, then granted", False,
                           f"{type(error).__name__}: {error}")
            return

        def standard_is_root():
            ran = exec_until_ok(a, se_binary, endpoint, log, "--", "id", "-u")
            ensure(ran.stdout.decode().strip() == "0", ran.stdout)
        results.check(stage, "standard on a root worker is root, as before", standard_is_root)

        def run_as_account():
            changed = set_grant(b, se_binary, log, "--run-as", RUN_AS_USER)
            ensure(changed.returncode == 0, text(changed))
            ensure(grants(b, se_binary, log)["run_as"] == RUN_AS_USER, "run-as was not stored")
            script = 'id -un; id -u; printf "%s\\n" "$HOME"; pwd; env | grep -c "^XDG_DATA_HOME=" || true'
            ran = exec_until_ok(a, se_binary, endpoint, log, "--shell", script)
            name, uid, home, cwd, leaked = ran.stdout.decode().split()
            ensure(name == RUN_AS_USER and uid != "0", ran.stdout)
            ensure(home == f"/home/{RUN_AS_USER}" and cwd == home, ran.stdout)
            ensure(leaked == "0", "the root worker's environment reached the account")
            entries = audit_entries(b, se_binary, log)
            ensure(any(entry.get("event") == "started" and str(entry.get("account", "")).startswith(RUN_AS_USER)
                       for entry in entries), "no audit entry names the run-as account")
        results.check(stage, "run-as: the account's own id, environment and home", run_as_account)

        def missing_home():
            changed = set_grant(b, se_binary, log, "--run-as", "nobody")
            ensure(changed.returncode == 0, text(changed))
            # nobody's login shell is nologin, which (as with sshd) runs no
            # shell command; the argv form starts the program directly.
            ran = exec_until_ok(a, se_binary, endpoint, log, "--", "/bin/sh", "-c", "id -u; pwd")
            uid, cwd = ran.stdout.decode().split()
            ensure(uid != "0" and cwd == "/", ran.stdout)
            asked = exec_out(a, se_binary, endpoint, log, "--cwd", "/nonexistent-luecken", "--", "id", "-u")
            ensure(asked.returncode != 0, "a missing requested working directory was replaced")
        results.check(stage, "run-as account without a home starts in /, a requested missing cwd fails",
                      missing_home)

        def back_to_standard():
            changed = set_grant(b, se_binary, log, "--run-as", "standard")
            ensure(changed.returncode == 0, text(changed))
            ran = exec_until_ok(a, se_binary, endpoint, log, "--", "id", "-u")
            ensure(ran.stdout.decode().strip() == "0", text(ran))
        results.check(stage, "run-as standard returns to the worker's account", back_to_standard)

        def root_secrets():
            report = b.se(se_binary, "doctor", "--json", log=log)
            status = json.loads(report.stdout)["credential_store"]
            ensure(status["state"] == "ok", status)
            # The system scope is used where the binary exists and the store
            # shares its file system with the host secret in /var/lib/systemd.
            binary = any(os.path.isfile(path) for path in ("/usr/bin/systemd-creds", "/bin/systemd-creds"))
            same_device = os.stat(work).st_dev == os.stat("/var/lib/systemd").st_dev
            if binary and same_device:
                ensure("protector=systemd-creds-system" in status["detail"], status)
            print(f"[INFO] root secrets: {status['detail']}", flush=True)
        results.check(stage, "root worker seals its secrets with the systemd host key", root_secrets)
    finally:
        ai.stop_background_daemons([a])
        b.stop()
        ai.run(["sudo", "-n", "userdel", "--remove", RUN_AS_USER], log=log, timeout=60)


# --------------------------------------------------------------------------- older suites (static)

OLDER_PYTHON_SUITES = ("test-ai-plugin-task.py", "test-analysis-plugin-task.py", "test-delete-speed-task.py",
                       "test-links-task.py", "test-terminal-task.py")


def stage_hostkeys(results):
    """M6: every older suite that connects to a fresh SSH/SFTP server confirms its host key first.

    Since this batch Smart Explorer never stores an unknown key silently, so a
    call site without the confirmation would fail against a fresh container.
    The scan is static and cheap: it cannot prove the order of calls at run
    time, only that each place that adds an SFTP connection (or opens an
    SFTP backend, or starts the servers) carries the confirmation next to it.
    """
    stage = "hostkeys"
    native = ai.NATIVE
    root = ai.ROOT

    def python_suites():
        problems = []
        for name in OLDER_PYTHON_SUITES:
            source = (native / name).read_text(encoding="utf-8")
            lines = source.splitlines()
            sites = [source.count("\n", 0, found.start()) for found in re.finditer(r'"add",\s*"sftp"', source)]
            ensure(sites, f"{name}: no `connections add sftp` call site found (the scan is out of date)")
            for line in sites:
                starts = [index for index in range(line + 1) if lines[index].startswith("def ")]
                body = "\n".join(lines[starts[-1]:line]) if starts else ""
                if "trust_ssh_host_key(" not in body and "trust-host-key" not in body:
                    problems.append(f"{name}:{line + 1}")
        ensure(not problems, f"SFTP connections added without confirming the host key: {problems}")
        return f"{len(OLDER_PYTHON_SUITES)} suites"
    results.check(stage, "Python suites confirm the host key in the function that adds the SFTP connection",
                  python_suites)

    def providers():
        source = (native / "sync-reliability-providers.py").read_text(encoding="utf-8")
        ensure(source.count("trust_ssh_host_key(") >= 3, "the SSH servers of the sync providers are not confirmed")
        return "sync-reliability-providers.py"
    results.check(stage, "sync provider fixtures confirm their SSH servers", providers)

    def rust_support():
        source = (native / "tests" / "transfer_task" / "support.rs").read_text(encoding="utf-8")
        calls = [found.start() for found in re.finditer(r"confirm_sftp_host_key\(", source)]
        connect = source.index("SftpBackend::connect")
        ensure(len(calls) >= 2 and calls[-1] < connect,
               "support.rs opens an SFTP backend before confirming the host key")
        engine = (native / "test-transfer-engine-task.sh").read_text(encoding="utf-8")
        ensure("ssh-keyscan" in engine and "SE_TASK_SFTP_FINGERPRINTS" in engine,
               "the transfer engine suite does not hand the fingerprints to the Rust fixtures")
    results.check(stage, "transfer engine fixtures confirm the SFTP container before opening a backend", rust_support)

    def android_servers():
        servers = (root / "android" / "test-servers" / "servers.sh").read_text(encoding="utf-8")
        ensure("ssh-keyscan" in servers and "seSftpFingerprints" in servers and "seSshFingerprints" in servers,
               "servers.sh does not scan and pass the host keys")
        tasks = root / "android" / "app" / "src" / "androidTest" / "java" / "app" / "smartexplorer" / "android" / "task"
        support = (tasks / "TaskSupport.kt").read_text(encoding="utf-8")
        ensure(re.search(r"private suspend fun ensure\([^)]*\)[^{]*\{\s*confirmHostKey\(", support),
               "Servers.ensure does not confirm the host key before connecting")
        unconfirmed = []
        for path in sorted(tasks.glob("*.kt")):
            if path.name in ("TaskSupport.kt", "LueckenUpgradeTaskTest.kt"):
                continue
            source = path.read_text(encoding="utf-8")
            if re.search(r"Servers\.(sftpInput|agentSftpInput)\(", source) and \
                    "confirmHostKey" not in source and "trustReportedHostKey" not in source:
                unconfirmed.append(path.name)
        ensure(not unconfirmed, f"Android tests connect with Servers.*Input() without confirming: {unconfirmed}")
    results.check(stage, "Android fixtures scan the servers' host keys and confirm before the first connection",
                  android_servers)


# --------------------------------------------------------------------------- FTP and FTPS (Linux, L1)

def stage_ftps(results, log_root):
    """L1: suppaftp 10.0.2 against the strict FTPS server of the sync reliability suite.

    vsftpd (delfer/alpine-ftp-server) forces TLS on login and data, with a
    certificate of this run's CA (sync-reliability-providers.py: certificate,
    passive_ports, Owner). The one FTPS lib test writes 0, 37 and 128 KiB
    through the spooled writer and the sized stage writer (TLS closed before
    the 226 is read), reads every file back and reopens the saved location.
    """
    stage = "ftps"
    logs = log_root / "ftps"
    logs.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location("sync_reliability_providers",
                                                  NATIVE / "sync-reliability-providers.py")
    providers = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(providers)
    # Manifest, keys and the saved connection stay outside the uploaded logs.
    root = Path(tempfile.mkdtemp(prefix="se-luecken-ftps-"))
    root.chmod(0o700)
    profile = root / "profile"
    env = dict(os.environ, XDG_CONFIG_HOME=str(profile / "config"), XDG_DATA_HOME=str(profile / "data"),
               XDG_CACHE_HOME=str(profile / "cache"), RUST_BACKTRACE="1")
    owner = providers.Owner(root, logs, env)
    try:
        def server():
            providers.certificate(owner)
            user, password = "lueckenftps", base64.b16encode(os.urandom(18)).decode().lower()
            low, high = providers.passive_ports()
            options = ["-p", "127.0.0.1::21", "-p", f"127.0.0.1:{low}-{high}:{low}-{high}",
                       "-e", f"USERS={user}|{password}", "-v", f"{root}:/fixture:ro"]
            command = ["vsftpd", "/etc/vsftpd/vsftpd.conf", "-obackground=NO",
                       f"-opasv_min_port={low}", f"-opasv_max_port={high}", "-opasv_address=127.0.0.1",
                       "-ossl_enable=YES", "-oforce_local_logins_ssl=YES", "-oforce_local_data_ssl=YES",
                       "-orequire_ssl_reuse=NO", "-orsa_cert_file=/fixture/server.pem",
                       "-orsa_private_key_file=/fixture/server.key"]
            name = owner.docker("ftps", "delfer/alpine-ftp-server:latest", options, command)
            port = owner.port(name, 21)
            providers.wait_port(port, b"220")
            manifest = root / "providers.json"
            record = providers.connection("ftps", "ftps", port, user, password, f"/ftp/{user}")
            manifest.write_text(json.dumps({"providers": [record], "zip": str(root / "unused.zip")}),
                                encoding="utf-8")
            manifest.chmod(0o600)
            env.update(SE_SYNC_PROVIDER_MANIFEST=str(manifest), SE_SYNC_FIXTURE_CA_DER=str(root / "ca.der"))
            return f"vsftpd FTPS on 127.0.0.1:{port}"
        results.check(stage, "strict FTPS server (TLS on login and data, this run's CA)", server)
        if "SE_SYNC_PROVIDER_MANIFEST" in env:
            # The lib test binary of the `unit` stage; no new build.
            completed = ai.run(["cargo", "test", "--locked", "--lib", "--", "--exact", FTPS_PROVIDER_TEST,
                                "--test-threads=1"], cwd=ai.NATIVE, env=env, log=logs / "cargo-test.log",
                               timeout=3600)
            output = (completed.stdout + completed.stderr).decode(errors="replace")
            passed = completed.returncode == 0 and re.search(r"^test result: ok\. 1 passed", output, re.MULTILINE)
            results.record(stage, "FTPS: both upload paths confirm their bytes after the TLS close, "
                           "read back, saved location reopens", bool(passed),
                           "" if passed else failure_detail(output))
    finally:
        try:
            owner.close()
        except Exception as error:  # noqa: BLE001
            results.record(stage, "FTPS fixture cleanup", False, error)
        shutil.rmtree(root, ignore_errors=True)


def stage_ftp(results, se_binary, work, log_root):
    """L1 through `se` against pyftpdlib (as the FTP stages of the delete-speed and links suites)."""
    stage = "ftp"
    client = Client(work, "ftp")
    log = log_root / "ftp.log"
    served = Path(tempfile.mkdtemp(prefix="se-luecken-ftp-", dir=work))
    local = Path(tempfile.mkdtemp(prefix="se-luecken-ftp-local-", dir=work))
    payload = os.urandom(300 * 1024)
    (served / "keep.txt").write_text("victim")
    (served / "data.bin").write_bytes(payload)
    (served / "tree" / "sub").mkdir(parents=True)
    (served / "tree" / "sub" / "x.txt").write_text("deep")
    (local / "up.bin").write_bytes(payload[::-1])
    password = base64.b16encode(os.urandom(8)).decode().lower()
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    installed = ai.run([sys.executable, "-m", "pip", "install", "pyftpdlib>=1.5.9,<3"], log=log, timeout=600)
    if installed.returncode != 0:
        results.record(stage, "pyftpdlib fixture", False, text(installed))
        return
    server = subprocess.Popen([sys.executable, "-m", "pyftpdlib", "-i", "127.0.0.1", "-p", str(port), "-w",
                               "-d", str(served), "-u", "ftpuser", "-P", password],
                              stdout=open(log_root / "ftp-server.log", "wb"), stderr=subprocess.STDOUT)
    try:
        deadline = time.time() + 60
        while time.time() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
                    if connection.recv(4).startswith(b"220"):
                        break
            except OSError:
                time.sleep(0.5)
        else:
            raise AssertionError("FTP server did not answer")

        def se(*args):
            return client.se(se_binary, *args, log=log, timeout=600)

        added = client.se(se_binary, "connections", "add", "ftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", "ftpuser", "--root", "/", "--label", "ftpbox", "--password-stdin",
                          input_text=password + "\n", log=log)
        results.record(stage, "saved FTP connection added", added.returncode == 0, text(added))

        def browse_and_transfer():
            listed = se("ls", "@ftpbox:/")
            ensure(listed.returncode == 0, text(listed))
            names = {line.split("\t")[-1] for line in listed.stdout.decode().splitlines() if line.strip()}
            ensure({"keep.txt", "data.bin", "tree"} <= names, sorted(names))
            fetched = se("cp", "@ftpbox:/data.bin", str(local / "data.bin"))
            ensure(fetched.returncode == 0, text(fetched))
            ensure((local / "data.bin").read_bytes() == payload, "downloaded bytes differ")
            sent = se("cp", str(local / "up.bin"), "@ftpbox:/up.bin")
            ensure(sent.returncode == 0, text(sent))
            ensure((served / "up.bin").read_bytes() == payload[::-1], "uploaded bytes differ")
        results.check(stage, "saved FTP location lists, downloads and uploads byte-exact", browse_and_transfer)

        def no_replace():
            moved = se("mv", "@ftpbox:/up.bin", "@ftpbox:/keep.txt")
            ensure(moved.returncode != 0, "mv replaced an existing FTP file without --force")
            copied = se("cp", str(local / "up.bin"), "@ftpbox:/keep.txt")
            ensure(copied.returncode != 0, "cp replaced an existing FTP file without --force")
            ensure((served / "keep.txt").read_text() == "victim", "the existing file changed")
            ensure((served / "up.bin").read_bytes() == payload[::-1], "the move source changed")
        results.check(stage, "mv and cp keep an existing FTP file without --force (no-replace)", no_replace)

        def injection():
            for target in ("@ftpbox:/x\r\nDELE /keep.txt", "@ftpbox:/x\nDELE /keep.txt",
                           "@ftpbox:/x\rDELE /keep.txt"):
                for command in (("ls", target), ("mkdir", target), ("rm", "--force", target),
                                ("cp", str(local / "up.bin"), target)):
                    refused = se(*command)
                    ensure(refused.returncode != 0, f"se {command[0]} accepted {target!r}")
            ensure((served / "keep.txt").read_text() == "victim", "a smuggled DELE ran")
            created = sorted(name for name in os.listdir(served) if name.startswith("x"))
            ensure(not created, f"entries created from a refused target: {created!r}")
        results.check(stage, "targets with CR or LF are refused; the server never runs a smuggled command",
                      injection)

        def tree_delete():
            removed = se("rm", "-r", "--force", "@ftpbox:/tree")
            ensure(removed.returncode == 0, text(removed))
            ensure(not (served / "tree").exists(), "the FTP tree still exists")
        results.check(stage, "se rm -r deletes a nested FTP folder", tree_delete)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "FTP fixture", False, error)
    finally:
        server.terminate()
        try:
            server.wait(timeout=30)
        except subprocess.TimeoutExpired:
            server.kill()
        ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- Windows and Android

def stage_windows_secrets(results, se_binary, work, log_root):
    client = Client(work, "secrets")

    def doctor():
        report = client.se(se_binary, "doctor", "--json", log=log_root / "doctor.log")
        status = json.loads(report.stdout)["credential_store"]
        ensure(status["state"] == "ok", status)
        # No format change on Windows: no sealed file store and no key protector appear.
        ensure("protector=" not in str(status.get("detail", "")), status)
        ensure(not (client.app_data / "secrets-v1").exists(), "Windows created a file secret store")
    try:
        results.check("secrets", "Windows Credential Manager store still opens, no file store (V10)", doctor)
    finally:
        ai.stop_background_daemons([client])


def stage_android(results, log_root):
    # The Android script keeps its step logs and Gradle reports in this folder.
    environment = dict(os.environ, SMART_EXPLORER_TASK_LOG_ROOT=str(log_root / "android-logs"))
    completed = ai.run(["bash", ai.ROOT / "android" / "test-android-task.sh", "android-build",
                        "--out", log_root / "apk"], env=environment, log=log_root / "android-build.log",
                       timeout=8400)
    output = (completed.stdout + completed.stderr).decode(errors="replace")
    results.record("android", "Android tree, NDK check/build, APKs, JVM unit tests", completed.returncode == 0,
                   output[-4000:] if completed.returncode else "")


# --------------------------------------------------------------------------- main

def stage_android_device(results, args, log_root):
    """The emulator job: build with the release certificate, then update the published APK."""
    android = helper("android")
    environment = dict(os.environ)
    candidate = environment.get("CANDIDATE_SHA", "")
    apks = Path(args.apks)
    if args.phase == "build":
        results.check("android-device", "candidate and instrumentation APK signed with the release certificate; "
                      "published APK of the base commit verified",
                      lambda: json.dumps(android.build(this_module(), log_root, apks, environment, candidate))[:600])
    else:
        try:
            android.device(this_module(), results, log_root, apks, environment, candidate)
        except Exception as error:  # noqa: BLE001
            results.record("android-device", "emulator stage", False, f"{type(error).__name__}: {error}")


def this_module():
    return sys.modules[__name__]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    parser.add_argument("--job", choices=("host", "android", "android-device"), default="host")
    parser.add_argument("--phase", choices=("build", "device"), default="device",
                        help="android-device: `build` with the signing secrets, then `device` inside the emulator")
    parser.add_argument("--apks", help="android-device: folder of the APKs built by the `build` phase")
    args = parser.parse_args()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = ai.Results(log_root)
    if args.job == "android":
        stage_android(results, log_root)
        return results.finish()
    if args.job == "android-device":
        if not args.apks:
            parser.error("--job android-device requires --apks")
        stage_android_device(results, args, log_root)
        return results.finish()
    ai.stage_fmt(results, log_root)
    built, _ = ai.stage_build(results, log_root)
    stage_unit(results, log_root)
    if not WINDOWS:
        SHARE_SERVER["binary"] = helper("server").stage_server(this_module(), results, log_root)
        stage_hostkeys(results)
        stage_ftps(results, log_root)
    se_binary = ai.NATIVE / "target" / "debug" / ("se.exe" if WINDOWS else "se")
    if not built or not se_binary.exists():
        results.record("e2e", "development se binary", False, se_binary)
        return results.finish()
    work = Path(tempfile.mkdtemp(prefix="se-luecken-"))
    try:
        if WINDOWS:
            stage_windows_secrets(results, se_binary, work, log_root)
        else:
            stage_ssh(results, se_binary, work, log_root)
            stage_ftp(results, se_binary, work, log_root)
            stage_exec(results, se_binary, work, log_root)
            helper("update").stage_update(this_module(), results, se_binary, work, log_root,
                                          SHARE_SERVER["binary"])
    finally:
        if not WINDOWS:
            ai.run(["sudo", "-n", "rm", "-rf", work / "root-b"], timeout=120)
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
