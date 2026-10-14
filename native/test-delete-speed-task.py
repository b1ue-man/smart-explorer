#!/usr/bin/env python3
"""One remote-only task suite for the fast folder delete batch (2026-10-14).

Plan and expected results: docs/plaene/2026-10-14-ordner-loeschen-schnell/plan.md (D).
Builds only the development `se` and the library test binary of this host (no
release build), then checks every milestone once:

  fmt     rustfmt of the batch's Rust files (stdin mode, no crate-wide debt)
  build   `se` plus the lib tests; compiler warnings inside new batch files fail
  unit    `delete_speed_task_` tests: vfs fast path and listed tree delete (M1),
          Share wire/host/client (M2), agent codec, se-agent handler and daemon
          (M3), protocol backends (M4)
  share   (Linux) two paired clients over a local signalling/relay server: a
          peer folder with nested files and a link to an outside folder goes in
          one host-side run (`se rm -r` through the background service and MCP
          `remove`), the link target (inside the export) survives; a folder holding a protected app-private entry is refused
          with nothing deleted, which only the host-side tree delete does; a
          read-only export refuses and keeps everything
  sftp    (Linux) an OpenSSH server: nested tree with a link to an outside
          folder deleted by the listed, pipelined tree delete
  ftp     (Linux) pyftpdlib: nested tree deleted without per-entry stats

Helpers (results, process runner, isolated clients, MCP client, OpenSSH
container) come from test-ai-plugin-task.py. Every stage runs even when an
earlier one failed; the summary lists them all.
"""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
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
# The shared fmt/build helpers read these module globals. The build helper
# matches each fragment anywhere in a warning line, message text included, so
# every fragment names a file path: a bare `remove_tree` also matched the old
# dead-code warning "function `remove_tree` is never used" in the unchanged
# `remove_tree` function of src/share/core/fs_delete.rs.
ai.BATCH_BASE = "ad99f3a4678c7af34c9cbd1c74cb59dbf4b5846f"
ai.NEW_FILE_PREFIXES = ("src/vfs/core/tree_remove_", "src/daemon/os/shared/backend_remove_tree.rs",
                        "src/share/core/peer_remove_tree", "src/share/core/remove_tree_wire",
                        "src/share/os/shared/host_remove_tree", "delete_speed_task_tests.rs",
                        "fs_delete_count_task_tests.rs", "src/agent/core/ext_stream.rs",
                        "src/agent/core/tree_remove.rs", "src/agent_proto/os/shared/tree_remove.rs",
                        "src/sftp/core/tree_remove.rs", "src/smb/core/tree_remove.rs",
                        "src/ftp/core/tree_remove.rs", "src/webdav/core/tree_remove.rs")
WINDOWS = ai.WINDOWS
ensure = ai.ensure
Client = ai.Client
McpSession = ai.McpSession
TREE_FILES = 1500


def stage_unit(results, log_root):
    completed = ai.run(["cargo", "test", "--locked", "--lib", "--", "delete_speed_task_"],
                       cwd=ai.NATIVE, log=log_root / "unit.log", timeout=5400)
    output = (completed.stdout + completed.stderr).decode(errors="replace")
    summary = [line for line in output.splitlines() if line.startswith("test result:")]
    results.record("unit", "delete_speed_task_", completed.returncode == 0,
                   failure_detail(output) if completed.returncode else "; ".join(summary))
    ran = [line for line in output.splitlines() if line.startswith("test ") and "delete_speed_task_" in line]
    expected = EXPECTED_WINDOWS if WINDOWS else EXPECTED_LINUX
    results.record("unit", "delete_speed_task_ tests ran", len(ran) >= expected, f"{len(ran)} of {expected}")


# Tests that compile on each platform: vfs 6, Share 19 (17 without the two
# symlink tests), agent protocol/agent/service 18 (16), SFTP 3, SMB 1, FTP 3,
# WebDAV 4.
EXPECTED_LINUX = 54
EXPECTED_WINDOWS = 50


def failure_detail(output):
    """Failed test names and their panics first, then the tail."""
    lines = output.splitlines()
    failed = [line for line in lines if line.startswith("test ") and line.endswith("FAILED")]
    panics = [line for line in lines if "panicked at" in line or line.startswith("thread '")]
    return "\n".join(failed[:40] + panics[:40]) + "\n...\n" + output[-3000:]


# --------------------------------------------------------------------------- fixtures

def make_tree(root, files=TREE_FILES, outside=None):
    """Nested folders with `files` small files; a link to `outside` when given."""
    for index in range(files):
        folder = root / f"d{index % 10}" / f"e{index % 7}"
        folder.mkdir(parents=True, exist_ok=True)
        (folder / f"f{index}.txt").write_text(str(index))
    (root / "empty").mkdir(parents=True, exist_ok=True)
    if outside is not None and not WINDOWS:
        os.symlink(outside, root / "d0" / "outside-link")


def errors(completed):
    return completed.stderr.decode(errors="replace")


def timed(function):
    started = time.monotonic()
    value = function()
    return value, time.monotonic() - started


# --------------------------------------------------------------------------- Share (Linux)

def pair(results, se_binary, a, b, signal, relay, log):
    """Pairs A with B (Direct, write right by default); returns A's endpoint of B."""

    def worker_ready(client):
        status = json.loads(client.se(se_binary, "share", "status", "--json", log=log).stdout)
        worker = status["worker"]
        return worker["reachable"] and worker["running"] and worker["connected"] and \
            (worker.get("relay_url") or "").rstrip("/") == relay

    code = json.loads(b.se(se_binary, "share", "identity", "--json", log=log).stdout)["direct_code"]
    for client in (b, a):
        configured = client.se(se_binary, "share", "configure", "--server", f"tcp://{signal}",
                               "--allow-plaintext", log=log)
        ensure(configured.returncode == 0, errors(configured))
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
    return endpoint


def stage_share(results, se_binary, work, log_root):
    stage = "share"
    logs = log_root / "share"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "cli.log"
    built = ai.run(["cargo", "build", "--locked", "--manifest-path", ai.ROOT / "share-server" / "Cargo.toml",
                    "--bin", "se-share-server"], log=logs / "share-server-build.log", timeout=5400)
    if built.returncode != 0:
        results.record(stage, "share-server fixture build", False, built.stderr.decode()[-2000:])
        return
    server_binary = ai.ROOT / "share-server" / "target" / "debug" / "se-share-server"
    port = 31000 + os.getpid() % 12000
    server = subprocess.Popen([server_binary, f"127.0.0.1:{port}"], stdout=open(logs / "server.log", "wb"),
                              stderr=subprocess.STDOUT)
    clients = [Client(work, name, {"SE_SHARE_RELAY_ONLY": "1"}) for name in ("share-a", "share-b")]
    a, b = clients
    try:
        time.sleep(1)
        try:
            endpoint = pair(results, se_binary, a, b, f"127.0.0.1:{port}", f"http://127.0.0.1:{port + 1}", log)
            results.record(stage, "two clients paired (Direct, default rights)", True, endpoint)
        except Exception as error:  # noqa: BLE001
            results.record(stage, "two clients paired (Direct, default rights)", False,
                           f"{type(error).__name__}: {error}")
            return
        exported = Path(tempfile.mkdtemp(prefix="se-delete-export-", dir=work))
        # Inside the export but outside the deleted folder: the host refuses
        # links whose target leaves the export (existing destructive policy).
        outside = exported / "outside"
        outside.mkdir()
        (outside / "keep.txt").write_text("keep")
        added = b.se(se_binary, "share", "exports", "add", str(exported), "--label", "del", log=log)
        results.record(stage, "B exports a folder", added.returncode == 0, errors(added))
        peer = f"{endpoint.rstrip('/')}/del"

        def se(*args, timeout=900):
            return a.se(se_binary, *args, log=log, timeout=timeout)

        def visible():
            listed = se("ls", peer, timeout=180)
            return listed.returncode == 0
        try:
            ai.poll(visible, 120, "the export is not visible to the peer")
        except AssertionError as error:
            results.record(stage, "export visible to the peer", False, error)
            return

        def tree_delete():
            make_tree(exported / "tree", outside=outside)
            removed, seconds = timed(lambda: se("rm", "-r", "--force", f"{peer}/tree"))
            ensure(removed.returncode == 0, errors(removed))
            ensure(not os.path.lexists(exported / "tree"), "tree still exists on the host")
            ensure((outside / "keep.txt").read_text() == "keep", "the link target was touched")
            print(f"[INFO] share: {TREE_FILES} files deleted in {seconds:.1f} s", flush=True)
        results.check(stage, "se rm -r deletes a nested peer folder, links are not followed", tree_delete)

        def guarded():
            make_tree(exported / "guarded", files=40)
            private = exported / "guarded" / "d1" / ".se-versions"
            private.mkdir(parents=True)
            (private / "v1").write_text("version")
            before = sorted(str(path.relative_to(exported)) for path in (exported / "guarded").rglob("*"))
            refused = se("rm", "-r", "--force", f"{peer}/guarded")
            ensure(refused.returncode != 0, "a folder with an app-private entry was deleted")
            after = sorted(str(path.relative_to(exported)) for path in (exported / "guarded").rglob("*"))
            # The host checks the whole tree before the first change; an entry
            # walk of the client would have removed the visible files already.
            ensure(after == before, f"{len(before) - len(after)} entries were deleted before the refusal")
        results.check(stage, "protected entry: the host refuses before deleting anything (one host-side run)",
                      guarded)

        def mcp_remove():
            make_tree(exported / "via-mcp", files=200)
            session = McpSession(se_binary, a, "initialize", logs, "share")
            try:
                result = session.call("remove", {"target": f"{peer}/via-mcp", "recursive": True})
                ensure(not os.path.lexists(exported / "via-mcp"), f"folder still exists: {result}")
            finally:
                session.close()
        results.check(stage, "MCP remove deletes a peer folder", mcp_remove)

        def read_only():
            make_tree(exported / "kept", files=20)
            changed = b.se(se_binary, "share", "exports", "set", "del", "--read-only", log=log)
            ensure(changed.returncode == 0, errors(changed))
            try:
                probes = iter(range(1000))

                def refused_now():
                    # A harmless write shows when the host applies the change.
                    probe = se("mkdir", f"{peer}/ro-probe-{next(probes)}", timeout=180)
                    return probe.returncode != 0
                ai.poll(refused_now, 60, "the export never became read-only")
                attempt = se("rm", "-r", "--force", f"{peer}/kept", timeout=180)
                ensure(attempt.returncode != 0, "a read-only export accepted a folder delete")
                ensure(len(list((exported / "kept").rglob("*.txt"))) == 20, "files deleted from a read-only export")
            finally:
                b.se(se_binary, "share", "exports", "set", "del", "--write", log=log)
        results.check(stage, "read-only export refuses the folder delete and keeps every file", read_only)
    finally:
        ai.stop_background_daemons(clients)
        server.terminate()
        server.wait(timeout=30)


# --------------------------------------------------------------------------- SFTP (Linux)

REMOTE_TREE = ("mkdir -p /config/outside && printf keep > /config/outside/keep.txt && "
               "for i in $(seq 0 {count}); do d=/config/deltree/d$((i % 10))/e$((i % 7)); "
               "mkdir -p $d && printf $i > $d/f$i.txt; done && mkdir -p /config/deltree/empty && "
               "ln -s /config/outside /config/deltree/d0/outside-link")


def stage_sftp(results, se_binary, work, log_root):
    stage = "sftp"
    client = Client(work, "sftp")
    logs = log_root / "sftp"
    logs.mkdir(parents=True, exist_ok=True)
    try:
        container, port, user, password = ai.docker_ssh(work, log_root)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "OpenSSH container fixture", False, error)
        return
    log = logs / "cli.log"

    def se(*args):
        return client.se(se_binary, *args, log=log, timeout=900)

    def remote(script):
        return ai.run(["docker", "exec", "-u", f"{os.getuid()}:{os.getgid()}", container, "sh", "-c", script],
                      log=log)

    try:
        made = remote(REMOTE_TREE.format(count=TREE_FILES - 1))
        results.record(stage, "remote tree with a link to an outside folder", made.returncode == 0, errors(made))
        added = client.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", user, "--root", "/config", "--label", "box", "--password-stdin",
                          input_text=password + "\n", log=log)
        results.record(stage, "saved SFTP connection added", added.returncode == 0, errors(added))

        def tree_delete():
            removed, seconds = timed(lambda: se("rm", "-r", "--force", "@box:/deltree"))
            ensure(removed.returncode == 0, errors(removed))
            gone = remote("test ! -e /config/deltree && test ! -L /config/deltree")
            ensure(gone.returncode == 0, "the remote tree still exists")
            kept = remote("cat /config/outside/keep.txt")
            ensure(kept.stdout.decode() == "keep", "the link target was touched")
            print(f"[INFO] sftp: {TREE_FILES} files deleted in {seconds:.1f} s", flush=True)
        results.check(stage, "se rm -r deletes a nested SFTP folder, links are not followed", tree_delete)
    finally:
        ai.run(["docker", "rm", "-f", container], timeout=120)
        ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- FTP (Linux)

def stage_ftp(results, se_binary, work, log_root):
    stage = "ftp"
    client = Client(work, "ftp")
    log = log_root / "ftp.log"
    served = Path(tempfile.mkdtemp(prefix="se-delete-ftp-", dir=work))
    make_tree(served / "deltree", files=300)
    password = base64.b16encode(os.urandom(8)).decode().lower()
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    installed = ai.run([sys.executable, "-m", "pip", "install", "pyftpdlib>=1.5.9,<3"], log=log, timeout=600)
    if installed.returncode != 0:
        results.record(stage, "pyftpdlib fixture", False, errors(installed))
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
        added = client.se(se_binary, "connections", "add", "ftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", "ftpuser", "--root", "/", "--label", "ftpbox", "--password-stdin",
                          input_text=password + "\n", log=log)
        results.record(stage, "saved FTP connection added", added.returncode == 0, errors(added))

        def tree_delete():
            removed, seconds = timed(lambda: client.se(se_binary, "rm", "-r", "--force", "@ftpbox:/deltree",
                                                       log=log, timeout=900))
            ensure(removed.returncode == 0, errors(removed))
            ensure(not (served / "deltree").exists(), "the FTP tree still exists")
            print(f"[INFO] ftp: 300 files deleted in {seconds:.1f} s", flush=True)
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


# --------------------------------------------------------------------------- main

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    args = parser.parse_args()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = ai.Results(log_root)
    ai.stage_fmt(results, log_root)
    built, _ = ai.stage_build(results, log_root)
    stage_unit(results, log_root)
    se_binary = ai.NATIVE / "target" / "debug" / ("se.exe" if WINDOWS else "se")
    if not built or not se_binary.exists():
        results.record("e2e", "development se binary", False, se_binary)
        return results.finish()
    if WINDOWS:
        return results.finish()
    work = Path(tempfile.mkdtemp(prefix="se-delete-"))
    try:
        stage_share(results, se_binary, work, log_root)
        stage_sftp(results, se_binary, work, log_root)
        stage_ftp(results, se_binary, work, log_root)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
