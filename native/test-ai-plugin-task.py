#!/usr/bin/env python3
"""One remote-only task suite for the AI agent plugin batch (2026-10-12), which since
2026-11-07 also holds the stages of the former analysis-plugin suite (2026-10-14: priority
paragraph, persistent shell sessions, attaching shells and sessions).

Plan and expected results: docs/plaene/2026-10-12-ki-agenten-plugin/plan.md (D) and
docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md. Builds only the development
`se` and the library test binary of this host (no release build), then checks every
milestone once:

  build    `se` plus the lib tests (rustfmt and clippy: build.yml, docs/TODO.md H1)
  unit     `mcp_task_` tests and the directly affected `cli::` and handoff tests
  mcp      `se mcp` over both lifecycles (Codex `initialize` 2025-06-18,
           Claude Code `server/discover` 2026-07-28): catalog (25 tools, read-only
           hints, `alwaysLoad` on exactly the 12 file tools, the priority
           paragraph within the first 512 characters of the instructions, the
           `session` and `attachable` arguments of `exec`), stray-print
           protection, the `sessions` tool and its refusals, `se shell --list`,
           local file tools, jobs and cancellation, the `se` tool,
           status/connections, update stop, session cleanup
  ssh      (Linux) saved SFTP remote in an OpenSSH container (its host key is
           confirmed with the `ssh-keyscan` fingerprint before the first
           connection): remote files,
           `exec` streams/exit codes/argv quoting/cwd/env/stdin/timeout/cancel,
           process-group termination, `se exec @label`
  ssh-sessions
           (Linux) a persistent shell session keeps export/cd/functions, exit
           codes, input and argv quoting; `se shell --attach` joins an agent's
           session and `sessions attach` joins a terminal started with
           --attachable; no attach without --attachable; timeout and cancel end
           the session and its remote process group; the end of the MCP
           connection removes its sessions; plain `exec` unchanged
  share    (Linux) two paired clients with a local Share server: `exec` on the
           peer, cancellation, peer history
  share-sessions
           (Linux) sessions on a paired Share peer with Exec: state, `se shell`
           exit code, plain `exec` unchanged, ending when the grant is removed
  plugin   real `claude` and `codex` CLIs: install, validate, list, health
           check, refresh after a version change, uninstall, config output,
           and `se plugin install claude` for one host

Every stage runs even when an earlier one failed; the summary lists them all.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import queue
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / "native"
WINDOWS = os.name == "nt"
# The catalog of native/crates/cli/src/cli/mcp/catalog.rs `tools()`, in its order.
EXPECTED_TOOLS = [
    "status", "connections", "list", "stat", "read", "write", "edit", "mkdir", "remove",
    "copy", "move", "search", "exec", "job_status", "job_input", "job_cancel", "se", "sessions",
    "screen", "screen_window", "screen_input", "tcp_forward", "apps", "rdp", "login",
]
# Tools whose annotation says readOnlyHint (catalog.rs `read_only(..)`).
READ_ONLY = {"status", "connections", "list", "stat", "read", "search", "job_status", "screen"}
# Tools Claude Code loads upfront (`_meta["anthropic/alwaysLoad"]`, catalog.rs `UPFRONT`).
UPFRONT = {"connections", "list", "stat", "read", "write", "edit", "mkdir", "remove", "copy", "move",
           "search", "job_status"}
# Words of the priority paragraph an agent must receive: the order (before) and the host tools it
# ranks below Smart Explorer.
PRIORITY_WORDS = ("first", "before", "Read", "Write", "Edit", "Glob", "ls", "cat", "find")
# Path fragments of files in which a compiler warning fails the build stage. Empty here: since
# 2026-11-07 no suite gates files "created since a base commit"; build.yml gates clippy. The
# terminal suite sets its own fixed list.
NEW_FILE_PREFIXES = ()
UNIT_FILTERS = ["mcp_task_", "cli::", "daemon::handoff"]


class Results:
    def __init__(self, log_root):
        self.log_root = log_root
        self.items = []

    def record(self, stage, name, ok, detail=""):
        self.items.append({"stage": stage, "check": name, "ok": bool(ok), "detail": str(detail)[:4000]})
        print(f"[{'PASS' if ok else 'FAIL'}] {stage}: {name}" + (f" — {detail}" if detail and not ok else ""),
              flush=True)

    def check(self, stage, name, function):
        try:
            detail = function()
            self.record(stage, name, True, detail or "")
        except Exception as error:  # noqa: BLE001 - every failure becomes a recorded check
            self.record(stage, name, False, f"{type(error).__name__}: {error}")

    def finish(self):
        (self.log_root / "summary.json").write_text(json.dumps(self.items, indent=2))
        failed = [item for item in self.items if not item["ok"]]
        print(f"\n{len(self.items) - len(failed)} passed, {len(failed)} failed")
        for item in failed:
            print(f"  FAILED {item['stage']}: {item['check']}: {item['detail'][:500]}")
        return 1 if failed else 0


def run(command, *, cwd=None, env=None, timeout=1800, input_text=None, log=None):
    started = time.time()
    completed = subprocess.run(
        [str(part) for part in command], cwd=cwd, env=env, timeout=timeout,
        input=input_text.encode() if input_text is not None else None,
        stdin=subprocess.DEVNULL if input_text is None else None,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    if log is not None:
        with open(log, "ab") as handle:
            handle.write(f"$ {' '.join(map(str, command))}  ({time.time() - started:.1f}s, exit {completed.returncode})\n".encode())
            handle.write(completed.stdout + b"\n--- stderr ---\n" + completed.stderr + b"\n")
    return completed


def ensure(condition, message):
    if not condition:
        raise AssertionError(message)


# --------------------------------------------------------------------------- build

def cargo_with_warnings(results, name, command, log):
    completed = run(command, cwd=NATIVE, log=log, timeout=7200)
    output = (completed.stdout + completed.stderr).decode(errors="replace").replace("\\", "/")
    new_warnings = [line for line in output.splitlines()
                    if ": warning:" in line and any(prefix in line for prefix in NEW_FILE_PREFIXES)]
    results.record("build", name, completed.returncode == 0,
                   output[-3000:] if completed.returncode else "")
    if NEW_FILE_PREFIXES:
        results.record("build", f"{name}: no warnings in the named files", not new_warnings,
                       "\n".join(new_warnings[:40]))
    return completed.returncode == 0


def stage_build(results, log_root):
    log = log_root / "build.log"
    built = cargo_with_warnings(results, "cargo build --bin se",
                                ["cargo", "build", "--locked", "--bin", "se", "--message-format", "short"], log)
    tests = cargo_with_warnings(results, "cargo test --lib --no-run",
                                ["cargo", "test", "--locked", "--no-fail-fast", "--workspace", "--exclude", "smart_explorer_android", "--exclude", "smart_explorer", "--lib", "--no-run", "--message-format", "short"], log)
    return built, tests


def stage_unit(results, log_root):
    completed = run(["cargo", "test", "--locked", "--no-fail-fast", "--workspace", "--exclude", "smart_explorer_android", "--exclude", "smart_explorer", "--lib", "--", *UNIT_FILTERS],
                    cwd=NATIVE, log=log_root / "unit.log", timeout=5400)
    output = (completed.stdout + completed.stderr).decode(errors="replace")
    summary = [line for line in output.splitlines() if line.startswith("test result:")]
    results.record("unit", "mcp_task_ + cli:: + daemon::handoff", completed.returncode == 0,
                   output[-4000:] if completed.returncode else "; ".join(summary))
    ran = [line for line in output.splitlines() if line.startswith("test ") and "mcp_task_" in line]
    # 27 mcp_task_ tests exist (2026-11-07: native/crates/cli, native/crates/transfer).
    results.record("unit", "mcp_task_ tests ran", len(ran) >= 27, f"{len(ran)} mcp_task_ tests")


# --------------------------------------------------------------------------- environment

class Client:
    """An isolated home and app-data directory for `se`."""

    def __init__(self, root, name, extra=None):
        self.root = Path(root) / name
        for directory in ("home", "data", "config", "runtime"):
            (self.root / directory).mkdir(parents=True, exist_ok=True)
            if not WINDOWS:
                os.chmod(self.root / directory, 0o700)
        self.env = dict(os.environ)
        self.env.update({
            "HOME": str(self.root / "home"), "USERPROFILE": str(self.root / "home"),
            "XDG_DATA_HOME": str(self.root / "data"), "XDG_CONFIG_HOME": str(self.root / "config"),
            "XDG_RUNTIME_DIR": str(self.root / "runtime"),
            "APPDATA": str(self.root / "data"), "LOCALAPPDATA": str(self.root / "data"),
        })
        self.env.pop("COMPLETE", None)
        self.env.update(extra or {})

    @property
    def app_data(self):
        return self.root / "data" / "smart_explorer"

    def se(self, se_binary, *args, input_text=None, timeout=180, log=None):
        return run([se_binary, *args], env=self.env, input_text=input_text, timeout=timeout, log=log)


def stop_background_daemons(clients):
    """Ends `se --sync-daemon` processes started inside the isolated clients."""
    if WINDOWS:
        script = ("Get-CimInstance Win32_Process -Filter \"Name='se.exe'\" | "
                  "Where-Object { $_.CommandLine -match '--sync-daemon|--sync-guardian' } | "
                  "ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }")
        run(["powershell", "-NoProfile", "-Command", script], timeout=60)
        return
    markers = {f"XDG_DATA_HOME={client.root / 'data'}".encode() for client in clients}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            environ = set((entry / "environ").read_bytes().split(b"\0"))
        except OSError:
            continue
        if environ & markers:
            try:
                os.kill(int(entry.name), 15)
            except OSError:
                pass


# --------------------------------------------------------------------------- MCP client

class McpError(Exception):
    pass


class McpSession:
    MODERN = "2026-07-28"

    def __init__(self, se_binary, client, lifecycle, log_dir, name, extra_env=None):
        self.lifecycle = lifecycle
        env = dict(client.env)
        env.update(extra_env or {})
        self.stderr_path = Path(log_dir) / f"{name}.stderr.log"
        self.transcript = open(Path(log_dir) / f"{name}.jsonl", "w", encoding="utf-8")
        self.process = subprocess.Popen([str(se_binary), "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=open(self.stderr_path, "wb"), env=env)
        self.messages = queue.Queue()
        self.garbage = []
        self.notifications = []
        self.next_id = 1
        self.pending = {}
        threading.Thread(target=self._read, daemon=True).start()
        self.info = self._start()

    def _read(self):
        for line in self.process.stdout:
            text = line.decode("utf-8", errors="replace").strip()
            if not text:
                continue
            self.transcript.write(text + "\n")
            self.transcript.flush()
            try:
                self.messages.put(json.loads(text))
            except json.JSONDecodeError:
                self.garbage.append(text)
        self.messages.put(None)

    def _meta(self, extra=None):
        if self.lifecycle != "modern":
            return dict(extra or {})
        meta = {
            "io.modelcontextprotocol/protocolVersion": self.MODERN,
            "io.modelcontextprotocol/clientInfo": {"name": "se-ai-plugin-suite", "version": "1"},
            "io.modelcontextprotocol/clientCapabilities": {},
        }
        meta.update(extra or {})
        return meta

    def send(self, message):
        self.transcript.write(json.dumps(message) + "\n")
        self.process.stdin.write((json.dumps(message) + "\n").encode())
        self.process.stdin.flush()

    def request(self, method, params=None, timeout=300, meta=None):
        request_id = self.next_id
        self.next_id += 1
        params = dict(params or {})
        merged = self._meta(meta)
        if merged:
            params["_meta"] = merged
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        deadline = time.time() + timeout
        while True:
            if request_id in self.pending:
                return self._unwrap(self.pending.pop(request_id))
            remaining = deadline - time.time()
            if remaining <= 0:
                raise TimeoutError(f"{method} got no answer within {timeout}s")
            try:
                message = self.messages.get(timeout=remaining)
            except queue.Empty:
                continue
            if message is None:
                raise McpError(f"server closed the connection during {method}")
            if "id" in message and ("result" in message or "error" in message):
                self.pending[message["id"]] = message
            else:
                self.notifications.append(message)

    @staticmethod
    def _unwrap(message):
        if "error" in message:
            raise McpError(json.dumps(message["error"]))
        return message["result"]

    def _start(self):
        if self.lifecycle == "initialize":
            result = self.request("initialize", {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "se-ai-plugin-suite", "version": "1"},
            }, timeout=60)
            ensure(result.get("protocolVersion") == "2025-06-18", f"negotiated {result.get('protocolVersion')}")
            self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            return result
        return self.request("server/discover", {}, timeout=60)

    def tools(self):
        return self.request("tools/list", {})["tools"]

    def call(self, name, arguments=None, expect_error=False, timeout=600, progress_token=None):
        meta = {"progressToken": progress_token} if progress_token is not None else None
        result = self.request("tools/call", {"name": name, "arguments": arguments or {}}, timeout=timeout,
                              meta=meta)
        structured = result.get("structuredContent")
        ensure(structured is not None, f"{name}: no structuredContent in {result}")
        text = result["content"][0]["text"]
        ensure(json.loads(text) == structured, f"{name}: text content differs from structuredContent")
        if bool(result.get("isError")) != expect_error:
            raise AssertionError(f"{name} {'succeeded' if expect_error else 'failed'}: {structured}")
        return structured

    def wait_job(self, structured, timeout=600):
        """Follows a job answer with job_status until it ended."""
        deadline = time.time() + timeout
        while structured.get("status") == "running":
            ensure(time.time() < deadline, f"job {structured.get('job_id')} still running")
            structured = self.call("job_status", {"job_id": structured["job_id"], "wait_seconds": 20})
        return structured

    def close(self, timeout=60):
        try:
            self.process.stdin.close()
        except OSError:
            pass
        try:
            self.process.wait(timeout=timeout)
        finally:
            self.transcript.close()
        ensure(not self.garbage, f"non-JSON lines on the protocol channel: {self.garbage[:3]}")
        return self.process.returncode


def path_text(path):
    return str(path)


# --------------------------------------------------------------------------- MCP stage

def stage_mcp(results, se_binary, work, log_root):
    client = Client(work, "mcp")
    logs = log_root / "mcp"
    logs.mkdir(parents=True, exist_ok=True)
    stage = "mcp"

    def catalog(lifecycle):
        def check():
            session = McpSession(se_binary, client, lifecycle, logs, f"catalog-{lifecycle}")
            try:
                if lifecycle == "modern":
                    ensure("instructions" in session.info, f"discover result {session.info}")
                else:
                    ensure(session.info.get("serverInfo", {}).get("name") == "smart-explorer", session.info)
                    ensure(len(session.info.get("instructions", "")) > 100, "instructions missing")
                instructions = session.info.get("instructions", "")
                first = instructions.split("\n\n")[0]
                ensure(len(instructions) <= 2048, f"instructions {len(instructions)} characters")
                ensure(len(first) <= 512, f"first paragraph {len(first)} characters")
                missing = [word for word in PRIORITY_WORDS if word not in first]
                ensure(not missing, f"priority paragraph lacks {missing}: {first}")
                ensure("sessions" in instructions, "instructions do not name the sessions tool")
                tools = session.tools()
                ensure([tool["name"] for tool in tools] == EXPECTED_TOOLS, [tool["name"] for tool in tools])
                for tool in tools:
                    read_only = tool.get("annotations", {}).get("readOnlyHint") is True
                    ensure(read_only == (tool["name"] in READ_ONLY), f"{tool['name']} readOnlyHint {read_only}")
                    ensure(tool["inputSchema"]["type"] == "object", tool["name"])
                    upfront = tool.get("_meta", {}).get("anthropic/alwaysLoad")
                    ensure(upfront is (True if tool["name"] in UPFRONT else None),
                           f"{tool['name']}: alwaysLoad {upfront!r}")
                    ensure(len(tool.get("description", "")) <= 2048, f"{tool['name']} description length")
                exec_tool = next(tool for tool in tools if tool["name"] == "exec")
                ensure({"session", "attachable"} <= set(exec_tool["inputSchema"]["properties"]), exec_tool)
                ensure("target" not in exec_tool["inputSchema"].get("required", []), exec_tool["inputSchema"])
                try:
                    session.request("tools/call", {"name": "no_such_tool", "arguments": {}})
                    raise AssertionError("unknown tool was accepted")
                except McpError:
                    pass
            finally:
                ensure(session.close() == 0, "server exit code")
            return f"{len(tools)} tools, {len(first)}-character priority paragraph, {len(UPFRONT)} upfront tools"
        return check

    results.check(stage, "catalog, instructions and alwaysLoad over initialize 2025-06-18 (Codex)",
                  catalog("initialize"))
    results.check(stage, "catalog, instructions and alwaysLoad over server/discover 2026-07-28 (Claude Code)",
                  catalog("modern"))

    def stray_print():
        session = McpSession(se_binary, client, "initialize", logs, "stray", {"SE_MCP_TEST_STRAY_PRINT": "1"})
        session.tools()
        session.close()
        ensure("se mcp test: stray standard output" in session.stderr_path.read_text(errors="replace"),
               "stray print did not reach stderr")
    results.check(stage, "stray stdout reaches stderr, protocol stays clean", stray_print)

    def sessions_basics():
        session = McpSession(se_binary, client, "modern", logs, "sessions-basics")
        try:
            listed = session.call("sessions", {})
            ensure(listed == {"sessions": []}, listed)
            missing = session.call("exec", {"session": "nothing", "command": "true"}, expect_error=True)
            ensure("give `target`" in missing["error"], missing)
            kept_open = session.call("exec", {"session": "s", "target": "@none", "command": "cat",
                                              "keep_stdin_open": True}, expect_error=True)
            ensure("keep_stdin_open" in kept_open["error"], kept_open)
            bad_name = session.call("exec", {"session": "a b", "target": "@none", "command": "true"},
                                    expect_error=True)
            ensure("session name" in bad_name["error"], bad_name)
            unknown = session.call("sessions", {"action": "attach", "session": "nobody"}, expect_error=True)
            ensure("no attachable session" in unknown["error"], unknown)
            plain = session.call("exec", {"command": "true"}, expect_error=True)
            ensure("target" in plain["error"], plain)
        finally:
            ensure(session.close() == 0, "server exit code")
        shell_listing = client.se(se_binary, "shell", "--list", "--json", log=logs / "cli.log")
        ensure(shell_listing.returncode == 0 and json.loads(shell_listing.stdout) == [], shell_listing.stdout)
        shell_help = client.se(se_binary, "shell", "--help", log=logs / "cli.log")
        ensure(b"--attachable" in shell_help.stdout and b"--attach" in shell_help.stdout, shell_help.stdout[:400])
    results.check(stage, "sessions tool, exec session refusals, se shell --list and --help", sessions_basics)

    session = McpSession(se_binary, client, "modern", logs, "files")
    base = Path(tempfile.mkdtemp(prefix="se-mcp-files-", dir=work))
    try:
        file_checks(results, session, base, se_binary, client)
        job_checks(results, session, base)
        tool_checks(results, session, se_binary, client)
    finally:
        results.check(stage, "session ends cleanly and removes its directory",
                      lambda: ensure(session.close() == 0 and not any(
                          (client.app_data / "mcp" / "sessions").iterdir()), "session directory left behind"))
    update_stop_checks(results, se_binary, client, logs)
    stop_background_daemons([client])


def file_checks(results, session, base, se_binary, client):
    stage = "mcp-files"
    target = path_text(base / "notes" / "a.txt")
    state = {}

    def write_create():
        result = session.call("write", {"target": target, "content": "hello\nworld\n"})
        ensure(result["created"] is True and result["bytes"] == 12, result)
        ensure((base / "notes" / "a.txt").read_text() == "hello\nworld\n", "content on disk")
        state["etag"] = result["etag"]
    results.check(stage, "write creates file and parents", write_create)

    def read_text():
        result = session.call("read", {"target": target})
        ensure(result["content"] == "hello\nworld\n" and result["etag"] == state["etag"], result)
        ensure(result["next_offset"] is None, result)
        part = session.call("read", {"target": target, "offset": 6, "length": 3})
        ensure(part["content"] == "wor" and part["next_offset"] == 9, part)
    results.check(stage, "read text, offsets and etag", read_text)

    def write_guards():
        refused = session.call("write", {"target": target, "content": "x"}, expect_error=True)
        ensure("overwrite" in refused["error"], refused)
        stale = session.call("write", {"target": target, "content": "x", "if_match": "0000"}, expect_error=True)
        ensure(stale.get("etag") == state["etag"], stale)
        ensure((base / "notes" / "a.txt").read_text() == "hello\nworld\n", "refused write changed the file")
    results.check(stage, "write refuses existing files and stale etags", write_guards)

    def edit():
        result = session.call("edit", {"target": target, "if_match": state["etag"],
                                       "edits": [{"old_text": "world", "new_text": "there"}]})
        ensure(result["replacements"] == 1, result)
        ensure((base / "notes" / "a.txt").read_text() == "hello\nthere\n", "edited content")
        (base / "notes" / "b.txt").write_text("x x")
        ambiguous = session.call("edit", {"target": path_text(base / "notes" / "b.txt"),
                                          "edits": [{"old_text": "x", "new_text": "y"}]}, expect_error=True)
        ensure("occurs 2 times" in ambiguous["error"], ambiguous)
        stale = session.call("edit", {"target": target, "if_match": state["etag"],
                                      "edits": [{"old_text": "hello", "new_text": "bye"}]}, expect_error=True)
        ensure("etag" in stale, stale)
    results.check(stage, "edit replaces exactly once, refuses ambiguity and stale etags", edit)

    def binary():
        payload = bytes(range(256)) * 4
        written = session.call("write", {"target": path_text(base / "bin.dat"),
                                         "content": base64.b64encode(payload).decode(), "encoding": "base64"})
        ensure(written["bytes"] == len(payload), written)
        hint = session.call("read", {"target": path_text(base / "bin.dat")}, expect_error=True)
        ensure("base64" in hint["error"], hint)
        read = session.call("read", {"target": path_text(base / "bin.dat"), "encoding": "base64"})
        ensure(base64.b64decode(read["content"]) == payload, "base64 roundtrip")
    results.check(stage, "binary files: hint for text, exact base64 roundtrip", binary)

    def relative():
        refused = session.call("stat", {"target": "relative/file.txt"}, expect_error=True)
        ensure("absolute" in refused["error"], refused)
    results.check(stage, "relative local paths are rejected", relative)

    def mkdir_list_stat():
        first = session.call("mkdir", {"target": path_text(base / "d" / "e")})
        again = session.call("mkdir", {"target": path_text(base / "d" / "e")})
        ensure(first["created"] is True and again["created"] is False, (first, again))
        many = base / "many"
        many.mkdir()
        for index in range(60):
            (many / f"f{index:03}.txt").write_text(str(index))
        names, offset, pages = [], 0, 0
        while offset is not None:
            page = session.call("list", {"target": path_text(many), "offset": offset, "max_bytes": 1024})
            names += [entry["name"] for entry in page["entries"]]
            offset, pages = page["next_offset"], pages + 1
        ensure(names == sorted(f"f{index:03}.txt" for index in range(60)), names[:5])
        ensure(pages > 1, "listing did not page")
        meta = session.call("stat", {"target": path_text(many)})
        ensure(meta["type"] == "dir", meta)
        return f"{pages} pages"
    results.check(stage, "mkdir, paged list, stat", mkdir_list_stat)

    def copy_move_search_remove():
        refused = session.call("copy", {"source": path_text(base / "many"),
                                        "destination": path_text(base / "copy")}, expect_error=True)
        ensure(refused["status"] == "failed" and "recursive" in refused["error"], refused)
        copied = session.wait_job(session.call("copy", {"source": path_text(base / "many"),
                                                        "destination": path_text(base / "copy"),
                                                        "recursive": True}))
        ensure(copied["status"] == "completed", copied)
        ensure(sorted(os.listdir(base / "copy")) == sorted(os.listdir(base / "many")), "copied tree")
        moved = session.wait_job(session.call("move", {"source": path_text(base / "copy"),
                                                       "destination": path_text(base / "moved"),
                                                       "recursive": True}))
        ensure(moved["status"] == "completed" and not (base / "copy").exists(), moved)
        hits = session.wait_job(session.call("search", {"target": path_text(base / "many"),
                                                        "query": "f00*.txt", "glob": True}))
        ensure(hits["status"] == "completed" and hits["hits_total"] == 10, hits)
        ensure(len(hits["hits"]["items"]) == 10, hits["hits"])
        refused = session.call("remove", {"target": path_text(base / "moved")}, expect_error=True)
        ensure("recursive" in refused["error"], refused)
        session.call("remove", {"target": path_text(base / "moved"), "recursive": True})
        ensure(not (base / "moved").exists(), "removed tree")
        root = "C:\\" if WINDOWS else "/"
        guarded = session.call("remove", {"target": root, "recursive": True}, expect_error=True)
        ensure("root" in guarded["error"], guarded)
    results.check(stage, "copy, move, search, remove keep se cp/mv/rm rules", copy_move_search_remove)


def job_checks(results, session, base):
    stage = "mcp-jobs"

    def cancel_copy():
        big = base / "big.bin"
        with open(big, "wb") as handle:
            handle.truncate(2 * 1024 ** 3)
        started = session.call("copy", {"source": path_text(big), "destination": path_text(base / "big-copy.bin"),
                                        "wait_seconds": 0})
        ensure(started["status"] in ("running", "completed"), started)
        if started["status"] == "running":
            ensure("job_status" in started["next"], started)
            listed = session.call("job_status", {})
            ensure(any(job["job_id"] == started["job_id"] for job in listed["jobs"]), listed)
            ended = session.call("job_cancel", {"job_id": started["job_id"], "wait_seconds": 60})
            ensure(ended["status"] in ("cancelled", "completed"), ended)
        leftovers = [name for name in os.listdir(base) if "big" in name and name not in ("big.bin", "big-copy.bin")]
        ensure(not leftovers, f"stage files left: {leftovers}")
        if (base / "big-copy.bin").exists():
            ensure((base / "big-copy.bin").stat().st_size == big.stat().st_size, "partial destination file")
        big.unlink()
        return started["status"]
    results.check(stage, "copy job: running answer, job list, cancel leaves no stage", cancel_copy)

    def progress():
        many = base / "progress"
        many.mkdir()
        (many / "f.bin").write_bytes(os.urandom(1024))
        answer = session.call("copy", {"source": path_text(many), "destination": path_text(base / "progress-copy"),
                                       "recursive": True, "wait_seconds": 30}, progress_token="p1")
        ensure(answer["status"] == "completed", answer)
        ensure(answer["progress"]["files_written"] == 1, answer)
    results.check(stage, "job result reports progress counters", progress)


def tool_checks(results, session, se_binary, client):
    stage = "mcp-tools"

    def se_tool():
        answer = session.wait_job(session.call("se", {"args": ["connections", "list", "--json"]}))
        ensure(answer["status"] == "completed" and answer["exit_code"] == 0, answer)
        direct = client.se(se_binary, "connections", "list", "--json")
        ensure(json.loads(answer["stdout"]["text"]) == json.loads(direct.stdout), "se tool output differs")
        refused = session.call("se", {"args": ["mcp"]}, expect_error=True)
        ensure("itself" in refused["error"], refused)
        doctor = session.wait_job(session.call("se", {"args": ["doctor", "--json"]}))
        ensure(json.loads(doctor["stdout"]["text"])["version"], doctor)
    results.check(stage, "se tool runs CLI commands with their exact output", se_tool)

    def status_connections():
        status = session.call("status", {})
        version = client.se(se_binary, "--version").stdout.decode().split()[-1]
        ensure(status["installation"]["version"] == version, status["installation"])
        ensure(status["mcp"]["version"] == version, status["mcp"])
        listed = session.call("connections", {})
        ensure(isinstance(listed["connections"], list), listed)
    results.check(stage, "status and connections", status_connections)


def update_stop_checks(results, se_binary, client, logs):
    stage = "mcp-update"
    marker = client.app_data / "sync" / "daemon.stop"

    def stale_then_new():
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text("stop")
        stale = client.app_data / "mcp" / "sessions" / "999999-stale"
        stale.mkdir(parents=True, exist_ok=True)
        (stale / "session.lock").write_text("")
        session = McpSession(se_binary, client, "initialize", logs, "update-stop")
        ensure(not stale.exists(), "stale session directory survived the start")
        time.sleep(3)
        ensure(len(session.tools()) == len(EXPECTED_TOOLS), "server ended on a stale stop request")
        time.sleep(1.2)
        marker.write_text("stop")
        started = time.time()
        session.process.wait(timeout=30)
        took = time.time() - started
        session.close()
        ensure("being updated" in session.stderr_path.read_text(errors="replace"), "no update notice on stderr")
        return f"ended {took:.1f}s after the update request"
    try:
        results.check(stage, "stale stop ignored, new update stop ends the server < 30 s", stale_then_new)
    finally:
        marker.unlink(missing_ok=True)


# --------------------------------------------------------------------------- SSH stage (Linux)

def docker_ssh(work, log_root):
    user, password = "seuser", base64.b16encode(os.urandom(12)).decode().lower()
    name = f"se-ai-plugin-ssh-{os.getpid()}"
    run(["docker", "rm", "-f", name], timeout=120)
    started = run(["docker", "run", "-d", "--name", name, "-p", "127.0.0.1::2222",
                   "-e", f"PUID={os.getuid()}", "-e", f"PGID={os.getgid()}", "-e", "PASSWORD_ACCESS=true",
                   "-e", f"USER_NAME={user}", "-e", f"USER_PASSWORD={password}",
                   "lscr.io/linuxserver/openssh-server:latest"], timeout=600, log=log_root / "docker.log")
    ensure(started.returncode == 0, started.stderr.decode())
    port = int(run(["docker", "port", name, "2222/tcp"]).stdout.decode().strip().rsplit(":", 1)[1])
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


# Host-key types in the order Smart Explorer's SSH client prefers them (Ed25519,
# the ECDSA curves, then RSA): of the types a server offers, the first one of this
# list is the key it presents.
SSH_HOST_KEY_ORDER = ("ssh-ed25519", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521",
                      "ssh-rsa")


def scan_ssh_host_keys(host, port, log=None):
    """`{key type: SHA256 fingerprint}` of the host keys a freshly started SSH
    server presents, from `ssh-keyscan` and `ssh-keygen -lf -`."""
    lines = []
    for _ in range(10):
        scanned = run(["ssh-keyscan", "-p", str(port), host], log=log, timeout=60)
        lines = [line for line in scanned.stdout.decode().splitlines()
                 if line.strip() and not line.startswith("#")]
        if lines:
            break
        time.sleep(1)
    ensure(lines, f"ssh-keyscan returned no host key of {host}:{port}")
    fingerprints = {}
    for line in lines:
        listed = run(["ssh-keygen", "-l", "-E", "sha256", "-f", "-"], input_text=line + "\n", log=log)
        found = [part for part in listed.stdout.decode().split() if part.startswith("SHA256:")]
        ensure(listed.returncode == 0 and found,
               f"ssh-keygen gave no SHA256 fingerprint: {(listed.stdout + listed.stderr).decode(errors='replace')[:300]}")
        fingerprints[line.split()[1]] = found[0]
    return fingerprints


def trust_ssh_host_key(client, se_binary, host, port, log=None):
    """Confirms the host key of a freshly started SSH server before the first
    connection: since the gap-closing batch (2026-10-16) Smart Explorer no
    longer stores an unknown key silently. The fingerprint comes from
    `ssh-keyscan`; `se connections trust-host-key` pins it only when the server
    presents exactly that key again, so the types are tried in the client's
    preference order and a fingerprint of another type is refused without
    storing anything. Returns `(key type, fingerprint)` of the pinned key."""
    fingerprints = scan_ssh_host_keys(host, port, log)
    refused = []
    for algorithm in SSH_HOST_KEY_ORDER:
        fingerprint = fingerprints.get(algorithm)
        if fingerprint is None:
            continue
        trusted = client.se(se_binary, "connections", "trust-host-key", f"{host}:{port}",
                            "--fingerprint", fingerprint, log=log)
        if trusted.returncode == 0:
            return algorithm, fingerprint
        refused.append(f"{algorithm} {fingerprint}: {(trusted.stdout + trusted.stderr).decode(errors='replace')[-300:]}")
    raise AssertionError(f"no host key of {host}:{port} could be confirmed (scanned {sorted(fingerprints)}): "
                         + " | ".join(refused))


def stage_ssh(results, se_binary, work, log_root):
    stage = "ssh"
    client = Client(work, "ssh")
    logs = log_root / "ssh"
    logs.mkdir(parents=True, exist_ok=True)
    try:
        container, port, user, password = docker_ssh(work, log_root)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "OpenSSH container fixture", False, error)
        return
    try:
        results.check(stage, "fresh server's host key confirmed before the first connection",
                      lambda: " ".join(trust_ssh_host_key(client, se_binary, "127.0.0.1", port, logs / "cli.log")))
        added = client.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", user, "--root", "/config", "--label", "sshbox", "--password-stdin",
                          input_text=password + "\n", log=logs / "cli.log")
        results.record(stage, "saved SFTP connection added", added.returncode == 0, added.stderr.decode())
        session = McpSession(se_binary, client, "initialize", logs, "ssh")
        local = Path(tempfile.mkdtemp(prefix="se-mcp-ssh-", dir=work))
        try:
            ssh_file_checks(results, session, local)
            ssh_exec_checks(results, session)
        finally:
            session.close()
        ssh_cli_checks(results, se_binary, client, logs)
    finally:
        run(["docker", "rm", "-f", container], timeout=120)
        stop_background_daemons([client])


def ssh_file_checks(results, session, local):
    stage = "ssh-files"

    def files():
        written = session.call("write", {"target": "@sshbox:/mcp/a.txt", "content": "remote text\n"})
        ensure(written["created"] is True, written)
        read = session.call("read", {"target": "@sshbox:/mcp/a.txt"})
        ensure(read["content"] == "remote text\n", read)
        copied = session.wait_job(session.call("copy", {"source": "@sshbox:/mcp", "destination": path_text(local),
                                                        "recursive": True}))
        ensure(copied["status"] == "completed", copied)
        ensure((local / "mcp" / "a.txt").read_text() == "remote text\n", "downloaded content")
        listing = session.call("list", {"target": "@sshbox:/mcp"})
        ensure([entry["name"] for entry in listing["entries"]] == ["a.txt"], listing)
    results.check(stage, "remote write, read, list and download", files)


def run_exec(session, arguments, timeout=120):
    return session.wait_job(session.call("exec", arguments), timeout=timeout)


def ssh_exec_checks(results, session):
    stage = "ssh-exec"

    def streams():
        result = run_exec(session, {"target": "@sshbox", "command": "echo out; echo err >&2; exit 3"})
        ensure(result["status"] == "completed" and result["exit_code"] == 3, result)
        ensure(result["stdout"]["text"] == "out\n" and result["stderr"]["text"] == "err\n", result)
        ensure(result["cli_exit_code"] == 3, result)
    results.check(stage, "stdout/stderr stay separate, exit status kept", streams)

    def quoting_cwd_env_stdin():
        argv = run_exec(session, {"target": "@sshbox", "argv": ["printf", "%s|", "a b", "$(id)", "it's"]})
        ensure(argv["stdout"]["text"] == "a b|$(id)|it's|", argv)
        # `@label:/dir` is below the connection's root (/config) like every target.
        cwd = run_exec(session, {"target": "@sshbox:/mcp", "argv": ["pwd"]})
        ensure(cwd["stdout"]["text"].strip() == "/config/mcp", cwd)
        env = run_exec(session, {"target": "@sshbox", "command": 'printf %s "$MODE"', "env": {"MODE": "x y"}})
        ensure(env["stdout"]["text"] == "x y", env)
        stdin = run_exec(session, {"target": "@sshbox", "command": "cat", "stdin": "line1\nline2\n"})
        ensure(stdin["stdout"]["text"] == "line1\nline2\n", stdin)
    results.check(stage, "argv quoting, cwd from target, env, stdin", quoting_cwd_env_stdin)

    def interactive_input():
        started = session.call("exec", {"target": "@sshbox", "command": "head -n1", "keep_stdin_open": True,
                                        "wait_seconds": 1})
        ensure(started["status"] == "running", started)
        session.call("job_input", {"job_id": started["job_id"], "data": "typed\n"})
        ended = session.wait_job(started)
        ensure(ended["stdout"]["text"] == "typed\n", ended)
    results.check(stage, "job_input feeds a running command", interactive_input)

    def timeout_and_cancel():
        timed = run_exec(session, {"target": "@sshbox", "command": "sleep 30", "timeout_seconds": 2})
        ensure(timed["status"] == "timed_out" and timed["cli_exit_code"] == 124, timed)
        started = session.call("exec", {"target": "@sshbox", "command": "sleep 61", "wait_seconds": 1})
        ensure(started["status"] == "running", started)
        ended = session.call("job_cancel", {"job_id": started["job_id"], "wait_seconds": 30})
        ensure(ended["status"] == "cancelled", ended)
        left = run_exec(session, {"target": "@sshbox", "command": "pgrep -f '[s]leep 61' || echo none"})
        ensure(left["stdout"]["text"].strip() == "none", left)
    results.check(stage, "timeout 124, cancel ends the remote process group", timeout_and_cancel)

    def se_tool_cancel():
        started = session.call("se", {"args": ["exec", "@sshbox", "--", "sleep", "62"], "wait_seconds": 2})
        ensure(started["status"] == "running", started)
        ended = session.call("job_cancel", {"job_id": started["job_id"], "wait_seconds": 40})
        ensure(ended["status"] == "cancelled", ended)
        left = run_exec(session, {"target": "@sshbox", "command": "pgrep -f '[s]leep 62' || echo none"})
        ensure(left["stdout"]["text"].strip() == "none", left)
    results.check(stage, "se tool cancel stops `se exec` like Ctrl+C", se_tool_cancel)


def ssh_cli_checks(results, se_binary, client, logs):
    stage = "ssh-cli"

    def cli():
        echo = client.se(se_binary, "exec", "@sshbox", "--", "echo", "cli ok", log=logs / "cli.log")
        ensure(echo.returncode == 0 and echo.stdout.decode() == "cli ok\n", (echo.returncode, echo.stderr))
        code = client.se(se_binary, "exec", "@sshbox", "--shell", "exit 5", log=logs / "cli.log")
        ensure(code.returncode == 5, code.returncode)
        timed = client.se(se_binary, "exec", "@sshbox", "--timeout", "2", "--", "sleep", "30", log=logs / "cli.log")
        ensure(timed.returncode == 124, timed.returncode)
        cwd = client.se(se_binary, "exec", "@sshbox:/mcp", "--", "pwd", log=logs / "cli.log")
        ensure(cwd.stdout.decode().strip() == "/config/mcp", cwd.stdout)
    results.check(stage, "se exec @label: output, exit codes, timeout, cwd", cli)


# --------------------------------------------------------------------------- Share stage (Linux)

def stage_share(results, se_binary, work, log_root):
    stage = "share"
    logs = log_root / "share"
    logs.mkdir(parents=True, exist_ok=True)
    built = run(["cargo", "build", "--locked", "--manifest-path", ROOT / "share-server" / "Cargo.toml",
                 "--bin", "se-share-server"], log=logs / "share-server-build.log", timeout=5400)
    if built.returncode != 0:
        results.record(stage, "share-server fixture build", False, built.stderr.decode()[-2000:])
        return
    server_binary = ROOT / "share-server" / "target" / "debug" / "se-share-server"
    port = 31000 + os.getpid() % 12000
    server = subprocess.Popen([server_binary, f"127.0.0.1:{port}"], stdout=open(logs / "server.log", "wb"),
                              stderr=subprocess.STDOUT)
    clients = [Client(work, name, {"SE_SHARE_RELAY_ONLY": "1"}) for name in ("share-a", "share-b")]
    a, b = clients
    try:
        time.sleep(1)
        endpoint = pair_with_exec(results, se_binary, a, b, f"127.0.0.1:{port}", f"http://127.0.0.1:{port + 1}", logs)
        if endpoint is None:
            return
        session = McpSession(se_binary, a, "initialize", logs, "share")
        try:
            def remote():
                result = run_exec(session, {"target": endpoint, "command": "printf SHARE_OK; printf e >&2; exit 4"})
                ensure(result["status"] == "completed" and result["exit_code"] == 4, result)
                ensure(result["stdout"]["text"] == "SHARE_OK" and result["stderr"]["text"] == "e", result)
            results.check(stage, "exec on a Share peer: streams and exit code", remote)

            def cancel():
                started = session.call("exec", {"target": endpoint, "command": "sleep 63", "wait_seconds": 2})
                ensure(started["status"] == "running", started)
                ended = session.call("job_cancel", {"job_id": started["job_id"], "wait_seconds": 30})
                ensure(ended["status"] == "cancelled", ended)
                deadline = time.time() + 45
                while time.time() < deadline:
                    history = b.se(se_binary, "share", "exec", "history", "--json")
                    if history.returncode == 0 and any(entry.get("direction") == "incoming"
                                                       and entry.get("job", {}).get("state") == "cancelled"
                                                       for entry in json.loads(history.stdout)):
                        return
                    time.sleep(0.5)
                raise AssertionError("the peer never recorded the cancelled incoming exec")
            results.check(stage, "job_cancel ends the peer's contained process", cancel)
        finally:
            session.close()
    finally:
        stop_background_daemons(clients)
        server.terminate()
        server.wait(timeout=30)


def poll(function, timeout, message):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            value = function()
            if value:
                return value
        except Exception as error:  # noqa: BLE001 - polled until the deadline
            last = error
        time.sleep(0.5)
    raise AssertionError(f"{message} (last: {last})")


def pair_with_exec(results, se_binary, a, b, signal, relay, logs):
    stage = "share"
    log = logs / "pairing.log"

    def worker_ready(client):
        status = json.loads(client.se(se_binary, "share", "status", "--json", log=log).stdout)
        worker = status["worker"]
        return worker["reachable"] and worker["running"] and worker["connected"] and \
            (worker.get("relay_url") or "").rstrip("/") == relay

    try:
        code = json.loads(b.se(se_binary, "share", "identity", "--json", log=log).stdout)["direct_code"]
        for client in (b, a):
            # The fixture server speaks plaintext; addresses without a scheme mean TLS.
            configured = client.se(se_binary, "share", "configure", "--server", f"tcp://{signal}",
                                   "--allow-plaintext", log=log)
            ensure(configured.returncode == 0, configured.stderr.decode())
            poll(lambda client=client: worker_ready(client), 90, "Share worker did not connect")
        added = json.loads(a.se(se_binary, "connections", "add-peer", "--code", code, "--name", "Target", "--json",
                                log=log).stdout)
        endpoint, request_id = added["endpoint"], added["request_id"]

        def accepted():
            inbox = json.loads(b.se(se_binary, "share", "request", "--json", log=log).stdout)
            if any(entry.get("decision", {}).get("state") == "pending" for entry in inbox.get("requests", [])):
                b.se(se_binary, "share", "request", "accept", "--json", log=log)
            shown = json.loads(a.se(se_binary, "share", "request", "show", request_id, "--json", log=log).stdout)
            return shown["decision"]["state"] == "accepted" and shown["authorization"]["active"]
        poll(accepted, 90, "the access request was never accepted")
        enabled = json.loads(b.se(se_binary, "share", "grants", "exec", "enable", "--yes", "--json",
                                  log=log).stdout)
        ensure(enabled.get("persisted") and enabled.get("applied"), enabled)
        results.record(stage, "two clients paired, Exec granted", True, endpoint)
        return endpoint
    except Exception as error:  # noqa: BLE001
        results.record(stage, "two clients paired, Exec granted", False, f"{type(error).__name__}: {error}")
        return None


# --------------------------------------------------------------------------- shell sessions (Linux)
# Persistent shell sessions and attaching, formerly the analysis-plugin suite (2026-10-14).

class ShellProcess:
    """A running `se shell` whose standard input stays open until finish()."""

    def __init__(self, se_binary, client, args, log_dir, name):
        self.stderr_path = Path(log_dir) / f"{name}.stderr.log"
        self.process = subprocess.Popen([str(se_binary), "shell", *args], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=open(self.stderr_path, "wb"),
                                        env=client.env)
        self.lines = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.process.stdout:
            self.lines.put(line.decode("utf-8", errors="replace"))
        self.lines.put(None)

    def send(self, text):
        self.process.stdin.write(text.encode())
        self.process.stdin.flush()

    def read_line(self, timeout=90):
        line = self.lines.get(timeout=timeout)
        ensure(line is not None, f"se shell ended early: {self.stderr()}")
        return line

    def finish(self, text="", timeout=120):
        if text:
            self.send(text)
        self.process.stdin.close()
        code = self.process.wait(timeout=timeout)
        rest = []
        while True:
            line = self.lines.get(timeout=30)
            if line is None:
                break
            rest.append(line)
        return code, "".join(rest)

    def stderr(self):
        return self.stderr_path.read_text(errors="replace")


def shell_list(se_binary, client, log):
    listed = client.se(se_binary, "shell", "--list", "--json", log=log)
    ensure(listed.returncode == 0, listed.stderr.decode())
    return json.loads(listed.stdout)


def wait_listed(se_binary, client, log, name, present=True, timeout=90):
    return poll(lambda: any(entry["session"] == name for entry in shell_list(se_binary, client, log)) == present,
                   timeout, f"session {name} {'never listed' if present else 'still listed'}")


def stage_ssh_sessions(results, se_binary, work, log_root):
    stage = "ssh-sessions"
    client = Client(work, "ssh-sessions")
    logs = log_root / "ssh-sessions"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "cli.log"
    try:
        container, port, user, password = docker_ssh(work, log_root)
    except Exception as error:  # noqa: BLE001 - recorded as a failed fixture
        results.record(stage, "OpenSSH container fixture", False, error)
        return
    try:
        results.check(stage, "fresh server's host key confirmed before the first connection",
                      lambda: " ".join(trust_ssh_host_key(client, se_binary, "127.0.0.1", port, log)))
        added = client.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", user, "--root", "/config", "--label", "sshbox", "--password-stdin",
                          input_text=password + "\n", log=log)
        results.record(stage, "saved SFTP connection added", added.returncode == 0, added.stderr.decode())
        session = McpSession(se_binary, client, "initialize", logs, "ssh-sessions")
        try:
            ssh_session_state(results, session)
            ssh_attach_both_ways(results, session, se_binary, client, logs)
            ssh_session_ends(results, session)
        finally:
            session.close()
        ssh_cli_and_cleanup(results, se_binary, client, logs)
    finally:
        run(["docker", "rm", "-f", container], timeout=120)
        stop_background_daemons([client])


def ssh_session_state(results, session):
    stage = "ssh-sessions"

    def state():
        first = run_exec(session, {"target": "@sshbox", "session": "s1",
                                      "command": "export A=1; cd /tmp; f() { echo fn-$1; }"})
        ensure(first["status"] == "completed" and first["exit_code"] == 0, first)
        ensure(first["cwd"] == "/tmp" and first["session"] == "s1" and not first["session_ended"], first)
        second = run_exec(session, {"session": "s1", "command": 'printf %s "$A"; f x; pwd'})
        ensure(second["stdout"]["text"] == "1fn-x\n/tmp\n", second)
        failed = run_exec(session, {"session": "s1", "command": "echo out; echo err >&2; false"})
        ensure(failed["exit_code"] == 1 and failed["cli_exit_code"] == 1, failed)
        ensure(failed["stdout"]["text"] == "out\n" and failed["stderr"]["text"] == "err\n", failed)
        piped = run_exec(session, {"session": "s1", "command": "cat", "stdin": "x\ny"})
        ensure(piped["stdout"]["text"] == "x\ny\n", piped)
        argv = run_exec(session, {"session": "s1", "argv": ["printf", "%s|", "a b", "$(id)", "it's"]})
        ensure(argv["stdout"]["text"] == "a b|$(id)|it's|", argv)
        placed = run_exec(session, {"session": "s1", "cwd": "/", "env": {"MODE": "x y"},
                                       "command": 'printf %s "$MODE"; pwd'})
        ensure(placed["stdout"]["text"] == "x y/\n" and placed["cwd"] == "/", placed)
        rooted = run_exec(session, {"target": "@sshbox:/", "session": "rooted", "command": "pwd"})
        ensure(rooted["stdout"]["text"].strip() == "/config", rooted)
        missing = session.call("exec", {"target": "@sshbox:/no-such-dir", "session": "lost", "command": "pwd"},
                               expect_error=True)
        ensure(missing["status"] == "failed" and "working directory" in missing["error"], missing)
        listed = session.call("sessions", {})["sessions"]
        ensure({"s1", "rooted"} <= {row["session"] for row in listed if row.get("role") == "owner"}, listed)
        plain = run_exec(session, {"target": "@sshbox", "command": "echo plain"})
        ensure(plain["stdout"]["text"] == "plain\n" and "session" not in plain, plain)
    results.check(stage, "session keeps export/cd/functions, exit codes, stdin, argv, cwd/env; plain exec unchanged",
                  state)


def ssh_attach_both_ways(results, session, se_binary, client, logs):
    stage = "ssh-attach"
    log = logs / "cli.log"

    def shell_joins_session():
        opened = run_exec(session, {"target": "@sshbox", "session": "agent1",
                                       "command": "export A=from-agent; cd /tmp"})
        ensure(opened["exit_code"] == 0, opened)
        entries = [entry for entry in shell_list(se_binary, client, log) if entry["session"] == "agent1"]
        ensure(len(entries) == 1 and entries[0]["owner"].startswith("se mcp (pid "), entries)
        ensure("token" not in entries[0] and "port" not in entries[0], entries[0])
        joined = client.se(se_binary, "shell", "--attach", "agent1", log=log, timeout=180,
                           input_text='printf "%s|" "$A"; pwd\nexport B=from-shell\n')
        ensure(joined.returncode == 0, joined.stderr.decode())
        ensure(joined.stdout.decode() == "from-agent|/tmp\n", joined.stdout)
        after = run_exec(session, {"session": "agent1", "command": 'printf %s "$B"'})
        ensure(after["stdout"]["text"] == "from-shell", after)
        activity = session.call("sessions", {"action": "activity", "session": "agent1"})["activity"]
        ensure("export B=from-shell" in activity and "se shell (pid" in activity, activity)
    results.check(stage, "se shell --attach joins an agent's session: shared state, detach keeps it, activity",
                  shell_joins_session)

    def session_joins_shell():
        human = ShellProcess(se_binary, client, ["@sshbox", "--attachable", "--name", "human"], logs, "human")
        try:
            wait_listed(se_binary, client, log, "human")
            attached = session.call("sessions", {"action": "attach", "session": "human"})
            ensure(attached["role"] == "attached" and attached["session"] == "human", attached)
            ran = run_exec(session, {"session": "human", "command": "export M=agent; cd /tmp"})
            ensure(ran["exit_code"] == 0 and ran["cwd"] == "/tmp", ran)
            human.send('printf "%s|" "$M"; pwd\n')
            ensure(human.read_line() == "agent|/tmp\n", "the terminal did not see the agent's state")
            code, rest = human.finish("exit 4\n")
            ensure(code == 4, f"se shell exit code {code}: {human.stderr()}")
            stderr = human.stderr()
            ensure("se mcp (pid" in stderr and "attached" in stderr, stderr)
            gone = session.call("exec", {"session": "human", "command": "true"}, expect_error=True)
            ensure("give `target`" in gone["error"], gone)
        finally:
            if human.process.poll() is None:
                human.process.kill()
    results.check(stage, "sessions attach joins se shell --attachable: shared state, its activity, exit 4 ends it",
                  session_joins_shell)

    def no_consent():
        private = ShellProcess(se_binary, client, ["@sshbox", "--name", "private"], logs, "private")
        try:
            private.send("echo ready\n")
            ensure(private.read_line() == "ready\n", "private shell did not start")
            ensure(all(entry["session"] != "private" for entry in shell_list(se_binary, client, log)),
                   "a shell without --attachable is listed")
            refused = client.se(se_binary, "shell", "--attach", "private", log=log, input_text="true\n")
            ensure(refused.returncode != 0 and b"no attachable session" in refused.stderr, refused.stderr)
            denied = session.call("sessions", {"action": "attach", "session": "private"}, expect_error=True)
            ensure("no attachable session" in denied["error"], denied)
            code, _ = private.finish()
            ensure(code == 0, f"exit code {code}")
        finally:
            if private.process.poll() is None:
                private.process.kill()
    results.check(stage, "a shell without --attachable cannot be joined", no_consent)


def ssh_session_ends(results, session):
    stage = "ssh-session-end"

    def timeout_and_cancel():
        timed = run_exec(session, {"target": "@sshbox", "session": "t1", "command": "sleep 30",
                                      "timeout_seconds": 2})
        ensure(timed["status"] == "timed_out" and timed["cli_exit_code"] == 124 and timed["session_ended"], timed)
        started = session.call("exec", {"target": "@sshbox", "session": "c1", "command": "sleep 61",
                                        "wait_seconds": 1})
        ensure(started["status"] == "running", started)
        ended = session.wait_job(session.call("job_cancel", {"job_id": started["job_id"], "wait_seconds": 30}))
        ensure(ended["status"] == "cancelled" and ended["session_ended"], ended)
        left = run_exec(session, {"target": "@sshbox", "command": "pgrep -f '[s]leep 61' || echo none"})
        ensure(left["stdout"]["text"].strip() == "none", left)
        exited = run_exec(session, {"target": "@sshbox", "session": "x1", "command": "exit 5"})
        ensure(exited["status"] == "completed" and exited["exit_code"] == 5 and exited["session_ended"], exited)
        closed = session.call("sessions", {"action": "close", "session": "s1"})
        ensure(closed == {"session": "s1", "closed": True, "detached": False}, closed)
    results.check(stage, "timeout 124 and cancel end the session and its process group; exit N; close",
                  timeout_and_cancel)


def ssh_cli_and_cleanup(results, se_binary, client, logs):
    stage = "ssh-cli"
    log = logs / "cli.log"

    def piped_shell():
        script = "cd /tmp\nexport X=5\nprintf '%s:' \"$X\"; pwd\nfalse\nexit 4\n"
        shell = client.se(se_binary, "shell", "@sshbox", log=log, timeout=180, input_text=script)
        ensure(shell.returncode == 4, (shell.returncode, shell.stderr))
        ensure(shell.stdout.decode() == "5:/tmp\n", shell.stdout)
        exec_cli = client.se(se_binary, "exec", "@sshbox", "--", "echo", "cli ok", log=log)
        ensure(exec_cli.returncode == 0 and exec_cli.stdout == b"cli ok\n", exec_cli.stdout)
    results.check(stage, "se shell @label: state over lines, exit 4; se exec unchanged", piped_shell)

    def mcp_end_removes_sessions():
        other = McpSession(se_binary, client, "modern", logs, "ending")
        try:
            opened = run_exec(other, {"target": "@sshbox", "session": "e1", "command": "true"})
            ensure(opened["exit_code"] == 0, opened)
            wait_listed(se_binary, client, log, "e1")
        finally:
            other.close()
        wait_listed(se_binary, client, log, "e1", present=False, timeout=30)
    results.check(stage, "the end of the MCP connection ends and unlists its sessions", mcp_end_removes_sessions)


def stage_share_sessions(results, se_binary, work, log_root):
    stage = "share-sessions"
    logs = log_root / "share-sessions"
    logs.mkdir(parents=True, exist_ok=True)
    built = run(["cargo", "build", "--locked", "--manifest-path", ROOT / "share-server" / "Cargo.toml",
                    "--bin", "se-share-server"], log=logs / "share-server-build.log", timeout=5400)
    if built.returncode != 0:
        results.record(stage, "share-server fixture build", False, built.stderr.decode()[-2000:])
        return
    server_binary = ROOT / "share-server" / "target" / "debug" / "se-share-server"
    # Another port than stage_share: the server of that stage ended a moment ago.
    port = 31000 + (os.getpid() + 40) % 12000
    server = subprocess.Popen([server_binary, f"127.0.0.1:{port}"], stdout=open(logs / "server.log", "wb"),
                              stderr=subprocess.STDOUT)
    clients = [Client(work, name, {"SE_SHARE_RELAY_ONLY": "1"}) for name in ("sessions-a", "sessions-b")]
    a, b = clients
    try:
        time.sleep(1)
        endpoint = pair_with_exec(results, se_binary, a, b, f"127.0.0.1:{port}", f"http://127.0.0.1:{port + 1}",
                                     logs)
        if endpoint is None:
            return
        session = McpSession(se_binary, a, "initialize", logs, "share-sessions")
        try:
            def state():
                first = run_exec(session, {"target": endpoint, "session": "p1",
                                              "command": "export S=share; cd /"}, timeout=240)
                ensure(first["exit_code"] == 0 and first["cwd"] == "/", first)
                second = run_exec(session, {"session": "p1", "command": 'printf %s "$S"; pwd'})
                ensure(second["stdout"]["text"] == "share/\n", second)
            results.check(stage, "session on a Share peer keeps export and cd", state)

            def cli_shell():
                shell = a.se(se_binary, "shell", endpoint, log=logs / "cli.log", timeout=240,
                             input_text='export Q=1\nprintf %s "$Q"\nexit 3\n')
                ensure(shell.returncode == 3, (shell.returncode, shell.stderr))
                ensure(shell.stdout.decode() == "1", shell.stdout)
                plain = run_exec(session, {"target": endpoint, "command": "printf plain"})
                ensure(plain["stdout"]["text"] == "plain", plain)
                closed = session.call("sessions", {"action": "close", "session": "p1"})
                ensure(closed["closed"] is True, closed)
            results.check(stage, "se shell on a Share peer exits with exit 3; plain exec unchanged; close", cli_shell)

            def revoke():
                opened = run_exec(session, {"target": endpoint, "session": "p2", "command": "true"}, timeout=240)
                ensure(opened["exit_code"] == 0, opened)
                disabled = b.se(se_binary, "share", "grants", "exec", "disable", "--json", log=logs / "cli.log")
                ensure(disabled.returncode == 0 and json.loads(disabled.stdout).get("persisted"), disabled.stdout)

                def ended():
                    rows = session.call("sessions", {})["sessions"]
                    return next((row.get("ended") for row in rows if row["session"] == "p2"), None)
                message = poll(ended, 60, "the session did not end after the Exec grant was removed")
                refused = session.call("exec", {"session": "p2", "command": "true"}, expect_error=True)
                ensure("give `target`" in refused["error"], refused)
                return message
            results.check(stage, "removing the peer's Exec grant ends the session", revoke)
        finally:
            session.close()
    finally:
        stop_background_daemons(clients)
        server.terminate()
        server.wait(timeout=30)


# --------------------------------------------------------------------------- plugin stage

def host_cli(name):
    return shutil.which(name)


def stage_plugin(results, se_binary, work, log_root):
    stage = "plugin"
    logs = log_root / "plugin"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "commands.log"
    client = Client(work, "plugin")
    claude, codex = host_cli("claude"), host_cli("codex")
    results.record(stage, "claude and codex CLIs on PATH", claude and codex, f"claude={claude} codex={codex}")
    if not (claude and codex):
        return
    executable = str(Path(se_binary).resolve())
    skill = Path(client.env["HOME"]) / ".agents" / "skills" / "smart-explorer"
    marketplace = client.app_data / "ai-plugins" / "claude"

    def install():
        installed = client.se(se_binary, "plugin", "install", "--json", timeout=600, log=log)
        ensure(installed.returncode == 0, installed.stdout.decode() + installed.stderr.decode())
        reports = {report["host"]: report for report in json.loads(installed.stdout)}
        ensure(reports["claude"]["installed"] and reports["codex"]["installed"], reports)
        ensure(reports["codex"]["verified"], reports["codex"])
        mcp = json.loads((marketplace / "smart-explorer" / ".mcp.json").read_text())
        ensure(mcp["mcpServers"]["se"] == {"command": executable, "args": ["mcp"]}, mcp)
        ensure((skill / "SKILL.md").read_text().startswith("---\nname: smart-explorer\n"), "codex skill")
    results.check(stage, "se plugin install: Claude marketplace plugin and Codex entry", install)

    def host_view():
        listed = run([claude, "plugin", "list", "--json"], env=client.env, log=log, timeout=180)
        ensure(listed.returncode == 0 and b"smart-explorer" in listed.stdout, listed.stdout.decode()[:800])
        validated = run([claude, "plugin", "validate", marketplace / "smart-explorer"], env=client.env, log=log)
        ensure(validated.returncode == 0, validated.stdout.decode() + validated.stderr.decode())
        entry = run([codex, "mcp", "get", "smart-explorer", "--json"], env=client.env, log=log, timeout=120)
        ensure(entry.returncode == 0 and json.dumps(executable)[1:-1].encode() in entry.stdout,
               entry.stdout.decode()[:800])
        status = json.loads(client.se(se_binary, "plugin", "status", "--json", log=log).stdout)
        hosts = {report["host"]: report for report in status}
        ensure(hosts["claude"]["plugin_listed"] and hosts["claude"]["files"]["current"], hosts["claude"])
        ensure(hosts["codex"]["server"]["current"] and hosts["codex"]["skill"]["current"], hosts["codex"])
    results.check(stage, "hosts list, validate and report the installation", host_view)

    def health():
        checked = run([claude, "mcp", "list"], env=client.env, log=log, timeout=240)
        text = (checked.stdout + checked.stderr).decode(errors="replace")
        lines = [line for line in text.splitlines() if "smart-explorer" in line]
        ensure(not lines or any("onnected" in line for line in lines), "\n".join(lines))
        return "listed: " + ("; ".join(lines) or "plugin servers not shown by `claude mcp list`")
    results.check(stage, "claude mcp list health check reaches se mcp", health)

    def refresh():
        for directory in (marketplace, skill):
            marker = directory / ".se-generated.json"
            data = json.loads(marker.read_text())
            data["version"] = "0.0.1"
            marker.write_text(json.dumps(data))
        session = McpSession(se_binary, client, "initialize", logs, "refresh")
        session.close()
        version = client.se(se_binary, "--version").stdout.decode().split()[-1]
        for directory in (marketplace, skill):
            ensure(json.loads((directory / ".se-generated.json").read_text())["version"] == version, directory)
        plugin = json.loads((marketplace / "smart-explorer" / ".claude-plugin" / "plugin.json").read_text())
        ensure(plugin["version"] == version, plugin)
    results.check(stage, "se mcp refreshes generated files written by another version", refresh)

    def config():
        generic = json.loads(client.se(se_binary, "plugin", "config", "generic").stdout)
        ensure(generic["mcpServers"]["smart-explorer"]["command"] == executable, generic)
        codex_toml = client.se(se_binary, "plugin", "config", "codex").stdout.decode()
        ensure("[mcp_servers.smart-explorer]" in codex_toml, codex_toml)
    results.check(stage, "se plugin config prints client entries", config)

    def uninstall():
        removed = client.se(se_binary, "plugin", "uninstall", "--json", timeout=600, log=log)
        ensure(removed.returncode == 0, removed.stdout.decode() + removed.stderr.decode())
        ensure(not marketplace.exists() and not skill.exists(), "generated files left")
        entry = run([codex, "mcp", "get", "smart-explorer", "--json"], env=client.env, log=log, timeout=120)
        ensure(entry.returncode != 0, "codex entry survived")
        listed = run([claude, "plugin", "list", "--json"], env=client.env, log=log, timeout=180)
        ensure(b"smart-explorer@smart-explorer" not in listed.stdout, listed.stdout.decode()[:800])
    results.check(stage, "se plugin uninstall removes entries and generated files", uninstall)

    def claude_only():
        # One named host (formerly the analysis-plugin suite): only Claude Code's plugin is written,
        # the host validates it, and uninstalling that host removes the generated files again.
        plugin = marketplace / "smart-explorer"
        installed = client.se(se_binary, "plugin", "install", "claude", "--json", timeout=600, log=log)
        ensure(installed.returncode == 0, installed.stdout.decode() + installed.stderr.decode())
        ensure((plugin / ".claude-plugin" / "plugin.json").is_file() and (plugin / ".mcp.json").is_file(),
               "plugin files missing")
        ensure(not skill.exists(), "installing claude wrote the Codex skill")
        validated = run([claude, "plugin", "validate", plugin], env=client.env, log=log, timeout=180)
        ensure(validated.returncode == 0, validated.stdout.decode() + validated.stderr.decode())
        removed = client.se(se_binary, "plugin", "uninstall", "claude", "--json", timeout=600, log=log)
        ensure(removed.returncode == 0, removed.stdout.decode() + removed.stderr.decode())
        ensure(not plugin.exists(), "generated files left")
    results.check(stage, "se plugin install claude: only that host, the plugin validates, uninstall removes it",
                  claude_only)
    stop_background_daemons([client])


# --------------------------------------------------------------------------- main

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    args = parser.parse_args()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = Results(log_root)
    built, _ = stage_build(results, log_root)
    stage_unit(results, log_root)
    se_binary = NATIVE / "target" / "debug" / ("se.exe" if WINDOWS else "se")
    if not built or not se_binary.exists():
        results.record("e2e", "development se binary", False, se_binary)
        return results.finish()
    work = Path(tempfile.mkdtemp(prefix="se-ai-plugin-"))
    try:
        stage_mcp(results, se_binary, work, log_root)
        if not WINDOWS:
            stage_ssh(results, se_binary, work, log_root)
            stage_ssh_sessions(results, se_binary, work, log_root)
            stage_share(results, se_binary, work, log_root)
            stage_share_sessions(results, se_binary, work, log_root)
        stage_plugin(results, se_binary, work, log_root)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
