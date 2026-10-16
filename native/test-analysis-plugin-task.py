#!/usr/bin/env python3
"""One remote-only task suite for the 2026-10-14 batch: analysis job id, plugin
priority, persistent shell sessions and attaching shells and sessions (R4).

Plan and expected results: docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md
(D and the R4 extension with its attach part). Builds only the development `se`
and the library test binary of this host (no release build), then checks every
milestone once:

  fmt       rustfmt of the batch's Rust files (stdin mode, no crate-wide debt)
  build     `se` plus the lib tests; compiler warnings inside batch files fail
  unit      M1: `analysis_id_task_` (id format; remote analysis and duplicate
            search between two real loopback Iroh nodes with this client's id,
            with the base64url id of clients 0.5.170 to 0.5.177, and the refusal
            of a path-like id), the directly affected wire and host analysis
            tests; M2: `mcp_task_` (catalog limits and `_meta`, skill
            frontmatter, session job states); S1/attach: `shell_session_task_`
            (markers, both dialects, state machine, frames, owner-only entries,
            and on a real local shell - bash on Linux, Windows PowerShell on
            Windows - state across commands, exit codes, streams, input,
            attaching in both roles, cancel and close); the existing `cli`
            tests (exec parsing and exit codes)
  mcp       M2 over the wire: `se mcp` under both lifecycles delivers the
            priority paragraph within the first 512 characters of its
            instructions and `_meta["anthropic/alwaysLoad"]` on exactly the file
            tools; the `sessions` tool and its refusals
  plugin    M2 as installed: `se plugin install claude` writes a skill and
            manifest that state the priority, and Claude Code validates the plugin
  ssh       (Linux) S2/S3 and attaching against an OpenSSH container: a session
            keeps export/cd/variables, exit codes, input and argv quoting; a
            terminal attaches to an agent's session and the agent to a terminal
            started with --attachable; no attach without --attachable; timeout
            and cancel end the session and its remote process group; the end of
            the MCP connection removes its sessions; plain `exec` unchanged
  share     (Linux) S2/S3 on a paired Share peer with Exec: session state,
            `se shell` exit code, plain `exec` unchanged

Helpers (results, process runner, isolated clients, MCP client, OpenSSH and
Share fixtures) come from test-ai-plugin-task.py. Every stage runs even when an
earlier one failed; the summary lists them all.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time

NATIVE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("ai_plugin_task", NATIVE / "test-ai-plugin-task.py")
ai = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ai)
# The shared fmt/build helpers read these module globals.
ai.BATCH_BASE = "fa6043efebeff6ad80971206d89e4162085e7f75"
ai.NEW_FILE_PREFIXES = ("src/share/core/analysis_request_id", "src/share/core/peer_storage_analysis.rs",
                        "src/share/core/peer_duplicates.rs", "src/share/os/shared/analysis_tasks.rs",
                        "src/cli/mcp", "src/cli/plugin", "src/cli/shell", "src/cli/mod.rs",
                        "src/cli/exec.rs")
WINDOWS = ai.WINDOWS
ensure = ai.ensure

UNIT_FILTERS = ["analysis_id_task_", "remote_drive_task_wire_tests", "storage_analysis_host", "mcp_task_",
                "shell_session_task_", "cli::tests::", "cli::exec::"]
# Tests per prefix that must have passed on this host: six shell-session tests
# run a local bash (Unix), one runs Windows PowerShell (Windows).
EXPECTED_PASSED = {"analysis_id_task_": 3, "mcp_task_": 17, "shell_session_task_": 17 if WINDOWS else 22}
# New tests of this batch that must be among them by name.
NAMED_TESTS = ("mcp_task_session_outcomes_map_to_job_states", "mcp_task_session_activity_log_is_bounded",
               "shell_session_task_cli_parses_shell_sessions_and_attach",
               "shell_session_task_attached_clients_share_state_and_activity" if not WINDOWS else
               "shell_session_task_local_powershell_keeps_state_across_commands")
UPFRONT = {"connections", "list", "stat", "read", "write", "edit", "mkdir", "remove", "copy", "move",
           "search", "job_status"}
# Words of the priority paragraph an agent must receive: the order (before)
# and the host tools it ranks below Smart Explorer.
PRIORITY_WORDS = ("first", "before", "Read", "Write", "Edit", "Glob", "ls", "cat", "find")


def failure_detail(stdout, stderr):
    """What a failed `cargo test` run reports first: the failed tests and the
    `failures:` section with their panic messages (both on stdout), then the
    end of stderr. Without either (the tests did not run, for example after a
    compile error) it reports the end of stdout and stderr."""
    lines = stdout.splitlines()
    failed = [line for line in lines if line.startswith("test ") and line.rstrip().endswith("FAILED")]
    section = lines[lines.index("failures:"):] if "failures:" in lines else []
    if not failed and not section:
        return (stdout + stderr)[-4000:]
    return "\n".join(failed + [""] + section + ["--- end of stderr ---", stderr[-1000:]])


def stage_unit(results, log_root):
    completed = ai.run(["cargo", "test", "--locked", "--lib", "--", *UNIT_FILTERS],
                       cwd=ai.NATIVE, log=log_root / "unit.log", timeout=5400)
    stdout = completed.stdout.decode(errors="replace")
    stderr = completed.stderr.decode(errors="replace")
    output = stdout + stderr
    summary = [line for line in output.splitlines() if line.startswith("test result:")]
    results.record("unit", " + ".join(UNIT_FILTERS), completed.returncode == 0,
                   failure_detail(stdout, stderr) if completed.returncode else "; ".join(summary))
    passed = [line for line in output.splitlines() if line.startswith("test ") and line.rstrip().endswith("ok")]
    for prefix, expected in EXPECTED_PASSED.items():
        ran = [line for line in passed if prefix in line]
        results.record("unit", f"{prefix} tests passed", len(ran) >= expected, f"{len(ran)} passed")
    for name in NAMED_TESTS:
        results.record("unit", f"{name} passed", any(name in line for line in passed), name)


def stage_mcp(results, se_binary, work, log_root):
    stage = "mcp"
    client = ai.Client(work, "mcp")
    logs = log_root / "mcp"
    logs.mkdir(parents=True, exist_ok=True)

    def catalog(lifecycle):
        def check():
            session = ai.McpSession(se_binary, client, lifecycle, logs, f"catalog-{lifecycle}")
            try:
                instructions = session.info.get("instructions", "")
                first = instructions.split("\n\n")[0]
                ensure(len(instructions) <= 2048, f"instructions {len(instructions)} characters")
                ensure(len(first) <= 512, f"first paragraph {len(first)} characters")
                missing = [word for word in PRIORITY_WORDS if word not in first]
                ensure(not missing, f"priority paragraph lacks {missing}: {first}")
                ensure("sessions" in instructions, "instructions do not name the sessions tool")
                tools = session.tools()
                ensure([tool["name"] for tool in tools] == ai.EXPECTED_TOOLS, [tool["name"] for tool in tools])
                for tool in tools:
                    upfront = tool.get("_meta", {}).get("anthropic/alwaysLoad")
                    expected = True if tool["name"] in UPFRONT else None
                    ensure(upfront is expected, f"{tool['name']}: alwaysLoad {upfront!r}")
                    ensure(len(tool.get("description", "")) <= 2048, f"{tool['name']} description length")
                exec_tool = next(tool for tool in tools if tool["name"] == "exec")
                ensure({"session", "attachable"} <= set(exec_tool["inputSchema"]["properties"]), exec_tool)
                ensure("target" not in exec_tool["inputSchema"].get("required", []), exec_tool["inputSchema"])
            finally:
                ensure(session.close() == 0, "server exit code")
            return f"{len(first)}-character priority paragraph, {len(UPFRONT)} upfront tools"
        return check

    results.check(stage, "instructions and alwaysLoad over initialize 2025-06-18 (Codex)", catalog("initialize"))
    results.check(stage, "instructions and alwaysLoad over server/discover 2026-07-28 (Claude Code)",
                  catalog("modern"))

    def sessions_basics():
        session = ai.McpSession(se_binary, client, "modern", logs, "sessions-basics")
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
        shell_list = client.se(se_binary, "shell", "--list", "--json", log=logs / "cli.log")
        ensure(shell_list.returncode == 0 and json.loads(shell_list.stdout) == [], shell_list.stdout)
        shell_help = client.se(se_binary, "shell", "--help", log=logs / "cli.log")
        ensure(b"--attachable" in shell_help.stdout and b"--attach" in shell_help.stdout, shell_help.stdout[:400])
    results.check(stage, "sessions tool, exec session refusals, se shell --list and --help", sessions_basics)
    ai.stop_background_daemons([client])


def stage_plugin(results, se_binary, work, log_root):
    stage = "plugin"
    logs = log_root / "plugin"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "commands.log"
    client = ai.Client(work, "plugin")
    claude = shutil.which("claude")
    results.record(stage, "claude CLI on PATH", bool(claude), f"claude={claude}")
    if not claude:
        return
    plugin = client.app_data / "ai-plugins" / "claude" / "smart-explorer"

    def install():
        installed = client.se(se_binary, "plugin", "install", "claude", "--json", timeout=600, log=log)
        ensure(installed.returncode == 0, installed.stdout.decode() + installed.stderr.decode())
        skill = (plugin / "skills" / "smart-explorer" / "SKILL.md").read_text(encoding="utf-8")
        description = next(line for line in skill.splitlines() if line.startswith("description: "))
        missing = [word for word in PRIORITY_WORDS if word not in description]
        ensure(not missing, f"skill description lacks {missing}: {description}")
        ensure("## Priority" in skill, "skill has no priority section")
        ensure("sessions" in skill and "se shell --attach" in skill, "skill does not explain sessions")
        manifest = json.loads((plugin / ".claude-plugin" / "plugin.json").read_text(encoding="utf-8"))
        ensure("before the built-in file tools" in manifest["description"], manifest)
        validated = ai.run([claude, "plugin", "validate", plugin], env=client.env, log=log, timeout=180)
        ensure(validated.returncode == 0, validated.stdout.decode() + validated.stderr.decode())
    results.check(stage, "se plugin install claude: skill and manifest state the priority, plugin validates",
                  install)

    def uninstall():
        removed = client.se(se_binary, "plugin", "uninstall", "claude", "--json", timeout=600, log=log)
        ensure(removed.returncode == 0, removed.stdout.decode() + removed.stderr.decode())
        ensure(not plugin.exists(), "generated files left")
    results.check(stage, "se plugin uninstall claude removes the generated plugin", uninstall)
    ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- shell sessions

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
    return ai.poll(lambda: any(entry["session"] == name for entry in shell_list(se_binary, client, log)) == present,
                   timeout, f"session {name} {'never listed' if present else 'still listed'}")


def stage_ssh_sessions(results, se_binary, work, log_root):
    stage = "ssh-sessions"
    client = ai.Client(work, "ssh-sessions")
    logs = log_root / "ssh-sessions"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "cli.log"
    try:
        container, port, user, password = ai.docker_ssh(work, log_root)
    except Exception as error:  # noqa: BLE001 - recorded as a failed fixture
        results.record(stage, "OpenSSH container fixture", False, error)
        return
    try:
        results.check(stage, "fresh server's host key confirmed before the first connection",
                      lambda: " ".join(ai.trust_ssh_host_key(client, se_binary, "127.0.0.1", port, log)))
        added = client.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                          "--user", user, "--root", "/config", "--label", "sshbox", "--password-stdin",
                          input_text=password + "\n", log=log)
        results.record(stage, "saved SFTP connection added", added.returncode == 0, added.stderr.decode())
        session = ai.McpSession(se_binary, client, "initialize", logs, "ssh-sessions")
        try:
            ssh_session_state(results, session)
            ssh_attach_both_ways(results, session, se_binary, client, logs)
            ssh_session_ends(results, session)
        finally:
            session.close()
        ssh_cli_and_cleanup(results, se_binary, client, logs)
    finally:
        ai.run(["docker", "rm", "-f", container], timeout=120)
        ai.stop_background_daemons([client])


def ssh_session_state(results, session):
    stage = "ssh-sessions"

    def state():
        first = ai.run_exec(session, {"target": "@sshbox", "session": "s1",
                                      "command": "export A=1; cd /tmp; f() { echo fn-$1; }"})
        ensure(first["status"] == "completed" and first["exit_code"] == 0, first)
        ensure(first["cwd"] == "/tmp" and first["session"] == "s1" and not first["session_ended"], first)
        second = ai.run_exec(session, {"session": "s1", "command": 'printf %s "$A"; f x; pwd'})
        ensure(second["stdout"]["text"] == "1fn-x\n/tmp\n", second)
        failed = ai.run_exec(session, {"session": "s1", "command": "echo out; echo err >&2; false"})
        ensure(failed["exit_code"] == 1 and failed["cli_exit_code"] == 1, failed)
        ensure(failed["stdout"]["text"] == "out\n" and failed["stderr"]["text"] == "err\n", failed)
        piped = ai.run_exec(session, {"session": "s1", "command": "cat", "stdin": "x\ny"})
        ensure(piped["stdout"]["text"] == "x\ny\n", piped)
        argv = ai.run_exec(session, {"session": "s1", "argv": ["printf", "%s|", "a b", "$(id)", "it's"]})
        ensure(argv["stdout"]["text"] == "a b|$(id)|it's|", argv)
        placed = ai.run_exec(session, {"session": "s1", "cwd": "/", "env": {"MODE": "x y"},
                                       "command": 'printf %s "$MODE"; pwd'})
        ensure(placed["stdout"]["text"] == "x y/\n" and placed["cwd"] == "/", placed)
        rooted = ai.run_exec(session, {"target": "@sshbox:/", "session": "rooted", "command": "pwd"})
        ensure(rooted["stdout"]["text"].strip() == "/config", rooted)
        missing = session.call("exec", {"target": "@sshbox:/no-such-dir", "session": "lost", "command": "pwd"},
                               expect_error=True)
        ensure(missing["status"] == "failed" and "working directory" in missing["error"], missing)
        listed = session.call("sessions", {})["sessions"]
        ensure({"s1", "rooted"} <= {row["session"] for row in listed if row.get("role") == "owner"}, listed)
        plain = ai.run_exec(session, {"target": "@sshbox", "command": "echo plain"})
        ensure(plain["stdout"]["text"] == "plain\n" and "session" not in plain, plain)
    results.check(stage, "session keeps export/cd/functions, exit codes, stdin, argv, cwd/env; plain exec unchanged",
                  state)


def ssh_attach_both_ways(results, session, se_binary, client, logs):
    stage = "ssh-attach"
    log = logs / "cli.log"

    def shell_joins_session():
        opened = ai.run_exec(session, {"target": "@sshbox", "session": "agent1",
                                       "command": "export A=from-agent; cd /tmp"})
        ensure(opened["exit_code"] == 0, opened)
        entries = [entry for entry in shell_list(se_binary, client, log) if entry["session"] == "agent1"]
        ensure(len(entries) == 1 and entries[0]["owner"].startswith("se mcp (pid "), entries)
        ensure("token" not in entries[0] and "port" not in entries[0], entries[0])
        joined = client.se(se_binary, "shell", "--attach", "agent1", log=log, timeout=180,
                           input_text='printf "%s|" "$A"; pwd\nexport B=from-shell\n')
        ensure(joined.returncode == 0, joined.stderr.decode())
        ensure(joined.stdout.decode() == "from-agent|/tmp\n", joined.stdout)
        after = ai.run_exec(session, {"session": "agent1", "command": 'printf %s "$B"'})
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
            ran = ai.run_exec(session, {"session": "human", "command": "export M=agent; cd /tmp"})
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
        timed = ai.run_exec(session, {"target": "@sshbox", "session": "t1", "command": "sleep 30",
                                      "timeout_seconds": 2})
        ensure(timed["status"] == "timed_out" and timed["cli_exit_code"] == 124 and timed["session_ended"], timed)
        started = session.call("exec", {"target": "@sshbox", "session": "c1", "command": "sleep 61",
                                        "wait_seconds": 1})
        ensure(started["status"] == "running", started)
        ended = session.wait_job(session.call("job_cancel", {"job_id": started["job_id"], "wait_seconds": 30}))
        ensure(ended["status"] == "cancelled" and ended["session_ended"], ended)
        left = ai.run_exec(session, {"target": "@sshbox", "command": "pgrep -f '[s]leep 61' || echo none"})
        ensure(left["stdout"]["text"].strip() == "none", left)
        exited = ai.run_exec(session, {"target": "@sshbox", "session": "x1", "command": "exit 5"})
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
        other = ai.McpSession(se_binary, client, "modern", logs, "ending")
        try:
            opened = ai.run_exec(other, {"target": "@sshbox", "session": "e1", "command": "true"})
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
    built = ai.run(["cargo", "build", "--locked", "--manifest-path", ai.ROOT / "share-server" / "Cargo.toml",
                    "--bin", "se-share-server"], log=logs / "share-server-build.log", timeout=5400)
    if built.returncode != 0:
        results.record(stage, "share-server fixture build", False, built.stderr.decode()[-2000:])
        return
    server_binary = ai.ROOT / "share-server" / "target" / "debug" / "se-share-server"
    port = 31000 + os.getpid() % 12000
    server = subprocess.Popen([server_binary, f"127.0.0.1:{port}"], stdout=open(logs / "server.log", "wb"),
                              stderr=subprocess.STDOUT)
    clients = [ai.Client(work, name, {"SE_SHARE_RELAY_ONLY": "1"}) for name in ("sessions-a", "sessions-b")]
    a, b = clients
    try:
        time.sleep(1)
        endpoint = ai.pair_with_exec(results, se_binary, a, b, f"127.0.0.1:{port}", f"http://127.0.0.1:{port + 1}",
                                     logs)
        if endpoint is None:
            return
        session = ai.McpSession(se_binary, a, "initialize", logs, "share-sessions")
        try:
            def state():
                first = ai.run_exec(session, {"target": endpoint, "session": "p1",
                                              "command": "export S=share; cd /"}, timeout=240)
                ensure(first["exit_code"] == 0 and first["cwd"] == "/", first)
                second = ai.run_exec(session, {"session": "p1", "command": 'printf %s "$S"; pwd'})
                ensure(second["stdout"]["text"] == "share/\n", second)
            results.check(stage, "session on a Share peer keeps export and cd", state)

            def cli_shell():
                shell = a.se(se_binary, "shell", endpoint, log=logs / "cli.log", timeout=240,
                             input_text='export Q=1\nprintf %s "$Q"\nexit 3\n')
                ensure(shell.returncode == 3, (shell.returncode, shell.stderr))
                ensure(shell.stdout.decode() == "1", shell.stdout)
                plain = ai.run_exec(session, {"target": endpoint, "command": "printf plain"})
                ensure(plain["stdout"]["text"] == "plain", plain)
                closed = session.call("sessions", {"action": "close", "session": "p1"})
                ensure(closed["closed"] is True, closed)
            results.check(stage, "se shell on a Share peer exits with exit 3; plain exec unchanged; close", cli_shell)

            def revoke():
                opened = ai.run_exec(session, {"target": endpoint, "session": "p2", "command": "true"}, timeout=240)
                ensure(opened["exit_code"] == 0, opened)
                disabled = b.se(se_binary, "share", "grants", "exec", "disable", "--json", log=logs / "cli.log")
                ensure(disabled.returncode == 0 and json.loads(disabled.stdout).get("persisted"), disabled.stdout)

                def ended():
                    rows = session.call("sessions", {})["sessions"]
                    return next((row.get("ended") for row in rows if row["session"] == "p2"), None)
                message = ai.poll(ended, 60, "the session did not end after the Exec grant was removed")
                refused = session.call("exec", {"session": "p2", "command": "true"}, expect_error=True)
                ensure("give `target`" in refused["error"], refused)
                return message
            results.check(stage, "removing the peer's Exec grant ends the session", revoke)
        finally:
            session.close()
    finally:
        ai.stop_background_daemons(clients)
        server.terminate()
        server.wait(timeout=30)


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
    work = Path(tempfile.mkdtemp(prefix="se-analysis-plugin-"))
    try:
        stage_mcp(results, se_binary, work, log_root)
        stage_plugin(results, se_binary, work, log_root)
        if not WINDOWS:
            stage_ssh_sessions(results, se_binary, work, log_root)
            stage_share_sessions(results, se_binary, work, log_root)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
