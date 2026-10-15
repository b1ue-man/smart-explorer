#!/usr/bin/env python3
"""One remote-only task suite for the 2026-10-14 "real terminal" batch: `se shell`
behaves like `ssh HOST` in a terminal (docs/plaene/2026-10-14-echtes-terminal/plan.md).

Builds only the development `se` and the library test binary of this host (no
release build), then checks every milestone M2 to M8, the security signals S1 to
S9 and the behaviour that must stay (V1 to V13) once:

  fmt          rustfmt of exactly the Rust files the batch changed (stdin mode, list
               below; no crate-wide debt, no git diff against a base)
  build        `se` plus the lib tests; compiler warnings inside batch files fail
  unit         `terminal_task_` (every named test of M2 to M7 per platform),
               `shell_session_task_`, `mcp_task_`, the exec protocol, types,
               supervisor protocol, IPC, server, client and registry modules
  mcp          (Linux and Windows) M8 over the wire: `sessions` shows `kind`, a
               terminal session started under a PTY (Linux) or pywinpty (Windows)
               with `--attachable --name tt` is listed as `terminal`, `attach`
               refuses it naming `-T --attachable`, an agent still joins
               `se shell -T --attachable`
  linux-ssh    (Linux) OpenSSH container with bash, nvim and python3: `se shell
               @sshbox` in a Python PTY (30x100, TERM xterm-256color): tty, TERM,
               window size and its change, Tab completion, colours, Ctrl+C,
               nvim, a password prompt, raw mode, long output and a 130 KB paste,
               `~?` `~~`, exit status, `~.`, SIGTERM, the restored terminal, a start
               that fails, the fallback of a server with `PermitTTY no`, the
               working directory of `@label:/dir`
  linux-share  (Linux) paired Share peer (host = this runner): the same checks
               plus the real claude and codex TUIs, attaching a second terminal,
               the update stop, clean-up of background jobs after `~.`, no
               terminal content in logs or history, and revoking the Exec grant
  preserve     (Linux) pipe mode, `-T` in a terminal, `--attach` to an agent's and to
               a `-T --attachable` command session in a terminal, `se exec`, MCP
               `exec` without `session`, plain Share exec, the update stop in line mode
  windows      (Windows) paired Share peer on Windows: `se.exe shell` under pywinpty
               (ConPTY): PowerShell prompt, Tab, Ctrl+C, window size and its
               change, exit status, `~.` and the clean-up of the host's processes

Every check goes through `results.check` / `results.record`; raw PTY output of
every session is kept as `<session>.raw` and `<session>.txt` below `--log-root`.
A fixture failure (Docker, pairing, build) is recorded as a failed check, never
skipped. Every stage runs even when an earlier one failed; the summary lists all.

Helpers (results, process runner, isolated clients, MCP client, OpenSSH and Share
fixtures) come from test-ai-plugin-task.py.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

NATIVE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("ai_plugin_task", NATIVE / "test-ai-plugin-task.py")
ai = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ai)
WINDOWS = ai.WINDOWS
ensure = ai.ensure
if not WINDOWS:
    import fcntl
    import termios

# The batch base (docs/plaene/2026-10-14-echtes-terminal/plan.md E17): HEAD at the end of
# planning. The fmt stage does not diff against it (later commits of the delete batch must not
# change what is checked); it checks exactly BATCH_RUST_FILES.
ai.BATCH_BASE = "63733be530af9cfb26946a09c762e7d2f7abce9d"

# Every Rust file this batch created or changed (milestones M2 to M8 and their test fixes).
BATCH_RUST_FILES = (
    "native/src/share/core/exec_terminal.rs",
    "native/src/share/core/exec_types.rs",
    "native/src/share/core/exec_protocol.rs",
    "native/src/share/core/exec_protocol_tests.rs",
    "native/src/share/core/exec_admission.rs",
    "native/src/share/core/exec_server.rs",
    "native/src/share/core/exec_server_tests.rs",
    "native/src/share/core/exec_job.rs",
    "native/src/share/core/exec_platform.rs",
    "native/src/share/core/exec_supervisor_protocol.rs",
    "native/src/share/core/exec_client.rs",
    "native/src/share/core/exec_client_active.rs",
    "native/src/share/core/exec_client_active_tests.rs",
    "native/src/share/core/exec_client_tests.rs",
    "native/src/share/core/exec_client_heartbeat_integration_tests.rs",
    "native/src/share/core/exec_registry_tests.rs",
    "native/src/share/core/exec_grant_runtime_tests.rs",
    "native/src/share/core/exec_configuration_sync_task_tests.rs",
    "native/src/share/mod.rs",
    "native/src/share/api_exports.rs",
    "native/src/daemon/os/shared/exec_ipc.rs",
    "native/src/daemon/os/shared/exec_ipc_tests.rs",
    "native/src/cli/exec.rs",
    "native/src/cli/exec_ssh.rs",
    "native/src/cli/mcp/exec_share_job.rs",
    "native/src/cli/mcp/exec_tool.rs",
    "native/src/cli/mcp/exec_session_job.rs",
    "native/src/share/os/linux_os/exec_self_test.rs",
    "native/src/share/os/windows/exec_self_test.rs",
    "native/src/share/os/linux_os/exec_pty.rs",
    "native/src/share/os/linux_os/exec_pty_tests.rs",
    "native/src/share/os/linux_os/exec_supervisor.rs",
    "native/src/share/os/linux_os/exec.rs",
    "native/src/share/os/android/exec.rs",
    "native/src/share/os/android/exec_contain.rs",
    "native/src/share/os/windows/exec.rs",
    "native/src/share/os/windows/exec_supervisor.rs",
    "native/src/share/os/windows/exec_conpty.rs",
    "native/src/share/os/windows/exec_conpty_tests.rs",
    "native/src/share/os/windows/exec_handles.rs",
    "native/src/sftp/core/command.rs",
    "native/src/sftp/core/terminal.rs",
    "native/src/sftp/mod.rs",
    "native/src/cli/shell_session.rs",
    "native/src/cli/shell_session/client.rs",
    "native/src/cli/shell_session/dialect.rs",
    "native/src/cli/shell_session/engine.rs",
    "native/src/cli/shell_session/host.rs",
    "native/src/cli/shell_session/registry.rs",
    "native/src/cli/shell_session/server.rs",
    "native/src/cli/shell_session/tests.rs",
    "native/src/cli/shell_session/transports.rs",
    "native/src/cli/shell_session/types.rs",
    "native/src/cli/shell_session/wire.rs",
    "native/src/cli/shell_session/pty_test_transport.rs",
    "native/src/cli/shell_session/terminal_client.rs",
    "native/src/cli/shell_session/terminal_engine.rs",
    "native/src/cli/shell_session/terminal_server.rs",
    "native/src/cli/shell_session/terminal_tests.rs",
    "native/src/cli/shell_session/terminal_transports.rs",
    "native/src/cli/shell.rs",
    "native/src/cli/shell_terminal.rs",
    "native/src/cli/shell_output_filter.rs",
    "native/src/cli/shell_escape.rs",
    "native/src/cli/os/mod.rs",
    "native/src/cli/os/terminal_unix.rs",
    "native/src/cli/os/terminal_windows.rs",
    "native/src/cli/mod.rs",
    "native/src/cli/mcp/shell_registry.rs",
    "native/src/cli/mcp/sessions_tool.rs",
    "native/src/cli/mcp/catalog.rs",
)
# Compiler warnings in these files fail the build stage (paths relative to native/).
ai.NEW_FILE_PREFIXES = tuple(path.removeprefix("native/") for path in BATCH_RUST_FILES)

UNIT_FILTERS = ["terminal_task_", "shell_session_task_", "mcp_task_", "exec_protocol", "exec_types",
                "exec_supervisor_protocol", "exec_ipc", "exec_server", "exec_client", "cli::tests::",
                "cli::exec::", "exec_registry", "exec_grant_runtime", "exec_admission", "exec_job",
                "exec_platform", "exec_terminal", "shell_session::", "cli::completions"]

# Tests that must have passed on this host, per prefix. The counts are the test functions
# of the source tree (`rg -n "fn terminal_task_"`): 82 terminal_task_ tests, of which 7
# run on Linux only (exec_pty_tests), 12 on Unix only (the local-PTY module of
# terminal_tests.rs) and 14 on Windows only (11 ConPTY tests, 3 console tests); so 68 on
# Linux and 63 on Windows. shell_session_task_: 23, of which 6 are Unix only and 1 Windows
# only (22 / 17). mcp_task_: 26 on both.
EXPECTED_PASSED = {"terminal_task_": 63 if WINDOWS else 68,
                   "shell_session_task_": 17 if WINDOWS else 22,
                   "mcp_task_": 26}

# Every terminal_task_ test by milestone; each must be among the passed tests.
COMMON_TESTS = (
    ("M2 Share Exec terminal protocol (types, frames, host gate, client gate, IPC)", (
        "terminal_task_start_without_terminal_keeps_its_bytes_and_digest",
        "terminal_task_terminal_start_carries_the_key_last_and_round_trips",
        "terminal_task_size_limits_are_one_to_32767",
        "terminal_task_term_value_is_a_short_portable_name",
        "terminal_task_login_and_terminal_only_exist_together",
        "terminal_task_login_is_a_unit_mode_and_hidden_in_the_history",
        "terminal_task_resize_frame_round_trips_with_tag_22_within_256_bytes",
        "terminal_task_resize_is_refused_without_a_terminal_start",
        "terminal_task_resize_is_accepted_for_a_terminal_start_until_the_result",
        "terminal_task_server_state_refuses_a_login_start_without_terminal",
        "terminal_task_hello_ok_without_features_decodes_and_stays_byte_equal",
        "terminal_task_resize_after_the_result_is_drained_only_for_terminal_starts",
        "terminal_task_host_without_terminal_support_denies_only_terminal_starts",
        "terminal_task_resize_round_trips_and_rejects_unusable_sizes",
        "terminal_task_terminal_start_is_never_sent_to_a_host_without_the_feature",
        "terminal_task_terminal_start_and_resizes_reach_a_host_with_the_feature",
        "terminal_task_resize_without_a_terminal_start_has_no_effect",
        "terminal_task_resize_frame_becomes_a_client_resize_input",
        "terminal_task_unusable_or_malformed_resize_frames_are_refused",
        "terminal_task_existing_input_frames_keep_their_meaning",
    )),
    ("M5 SSH terminal channel", (
        "terminal_task_modes_are_valid_rfc4254_opcodes",
        "terminal_task_request_values_are_checked_before_the_channel_opens",
        "terminal_task_error_text_names_the_refused_pty_request",
    )),
    ("M6 terminal session core (all platforms)", (
        "terminal_task_start_lines_quote_and_encode_every_literal",
        "terminal_task_entries_of_older_releases_read_as_command_sessions",
        "terminal_task_terminal_entries_without_a_probe_have_no_dialect_key",
        "terminal_task_sizes_are_bounded_and_convert_to_the_transports",
        "terminal_task_frames_carry_raw_bytes_within_the_data_limit",
        "terminal_task_the_last_active_participant_sets_the_size",
    )),
    ("M7 terminal front-end (options, escape sequences, exit codes, update stop, output filter)", (
        "terminal_task_cli_parses_the_terminal_options_of_shell",
        "terminal_task_hints_name_the_session_and_the_leave_sequence",
        "terminal_task_the_escape_character_counts_at_the_start_and_after_a_newline",
        "terminal_task_two_escape_characters_send_one_and_other_bytes_send_both",
        "terminal_task_the_help_is_local_and_keeps_the_start_of_the_line",
        "terminal_task_nothing_after_the_leave_sequence_is_sent",
        "terminal_task_an_escape_character_at_the_end_of_a_read_waits_for_the_next",
        "terminal_task_reading_byte_by_byte_gives_the_same_result_as_reading_at_once",
        "terminal_task_none_makes_the_session_transparent_and_another_character_works",
        "terminal_task_the_escape_character_argument_takes_one_ascii_character_or_none",
        "terminal_task_exit_codes_follow_the_ending",
        "terminal_task_closing_messages_name_the_reason",
        "terminal_task_the_remote_term_is_the_local_one_only_when_acceptable",
        "terminal_task_window_sizes_and_notices_are_kept_in_bounds",
        "terminal_task_a_lost_session_after_an_update_request_is_the_update_stop",
        "terminal_task_the_win32_input_mode_request_never_reaches_the_local_terminal",
        "terminal_task_other_modes_of_a_request_with_9001_stay",
        "terminal_task_a_request_split_anywhere_is_filtered_like_one_read",
        "terminal_task_other_output_passes_unchanged",
        "terminal_task_held_back_bytes_are_bounded",
    )),
)
UNIX_TESTS = (
    ("M6 terminal session over a local pty (bytes, size rule, attaching, end, versions, limits)", (
        "terminal_task_bytes_pass_unchanged_in_both_directions",
        "terminal_task_large_output_arrives_complete_and_in_order",
        "terminal_task_the_remote_terminal_has_the_size_of_the_last_active_terminal",
        "terminal_task_a_second_terminal_sees_the_output_and_types_into_the_same_shell",
        "terminal_task_a_terminals_input_reaches_the_others_only_as_the_remote_echo",
        "terminal_task_notices_carry_no_control_characters",
        "terminal_task_the_end_of_the_shell_reaches_everyone_and_unlists_the_session",
        "terminal_task_the_owner_leaving_or_a_cancel_ends_the_session_for_everyone",
        "terminal_task_the_hello_version_decides_which_sessions_a_client_may_join",
        "terminal_task_frames_of_the_other_kind_of_session_end_the_connection",
        "terminal_task_a_ninth_attached_terminal_is_refused",
        "terminal_task_opening_checks_name_term_and_size_before_anything_runs",
    )),
)
LINUX_TESTS = (
    ("M3 pty on Linux hosts", (
        "terminal_task_pty_pair_starts_with_the_requested_size_and_utf8_input",
        "terminal_task_login_shell_runs_on_a_pty_of_the_requested_size",
        "terminal_task_login_shell_gets_term_a_login_name_and_no_window_variables",
        "terminal_task_control_c_ends_the_foreground_command_and_keeps_the_shell",
        "terminal_task_login_shell_exit_status_reaches_the_exit_event",
        "terminal_task_start_without_terminal_keeps_separate_pipes",
        "terminal_task_start_fails_closed_unless_login_and_terminal_come_together",
    )),
)
WINDOWS_TESTS = (
    ("M4 ConPTY on Windows hosts", (
        "terminal_task_conpty_is_available_on_supported_windows",
        "terminal_task_cmd_runs_in_a_pseudoconsole_with_vt_output",
        "terminal_task_powershell_follows_resize_and_returns_its_exit_code",
        "terminal_task_closing_the_console_ends_a_running_command",
        "terminal_task_control_c_ends_a_command_in_the_pseudoconsole",
        "terminal_task_resize_after_close_reports_a_closed_console",
        "terminal_task_coordinates_reject_unusable_sizes",
        "terminal_task_environment_block_is_sorted_without_regard_to_case",
        "terminal_task_environment_block_keeps_one_entry_per_name_ignoring_case",
        "terminal_task_empty_environment_block_is_two_nul_characters",
        "terminal_task_login_shell_is_the_absolute_system_powershell",
    )),
    ("M7 Windows console (modes, keys, UTF-8)", (
        "terminal_task_windows_console_modes_follow_the_ssh_recipe",
        "terminal_task_windows_keys_become_utf8_with_repeats_and_surrogates",
        "terminal_task_windows_output_decodes_utf8_across_chunks",
    )),
)
# M8 and the unchanged behaviour in unit form.
NAMED_OTHER = (
    ("M8 MCP: agents refuse terminal sessions and listing says the kind", (
        "mcp_task_terminal_sessions_refuse_agents",
        "mcp_task_listed_sessions_of_the_user_say_whether_agents_can_join",
        "mcp_task_catalog_is_complete_and_within_host_limits",
    )),
    ("V: command sessions, markers and the attach protocol keep working", (
        "shell_session_task_cli_parses_shell_sessions_and_attach",
        "shell_session_task_local_powershell_keeps_state_across_commands" if WINDOWS else
        "shell_session_task_attached_clients_share_state_and_activity",
    )),
)

# Words of the priority paragraph an agent must still receive (as in the earlier batches).
PRIORITY_WORDS = ("first", "before", "Read", "Write", "Edit", "Glob", "ls", "cat", "find")
ESC = "\x1b"
ANSI = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[()][0-9A-Za-z]|\x1b[@-Z\\-_]")


def plain(text):
    return ANSI.sub("", text)


# What a shell writes between the line feed that ends the echo of a typed command and the
# first line of the command's output: bash (readline) switches bracketed paste off there with
# ESC[?2004l and a carriage return. Allowed: any CSI sequences, then one carriage return.
OUTPUT_START = r"(?:\x1b\[[0-?]*[ -/]*[@-~])*\r?"


def out_line(text):
    """Pattern for `text` alone on a line of terminal output; the echo of a typed
    command that merely contains it does not match."""
    return r"(?:^|\n)" + OUTPUT_START + re.escape(text) + r"\r?\n"


# --------------------------------------------------------------------------- terminals

class Terminal:
    """A program in a pseudo-terminal. All raw output is kept (and written to a log) so
    that checks match on bytes seen since a mark."""
    shell = "posix"

    def __init__(self, name, log_dir):
        self.name = name
        self.log_dir = Path(log_dir)
        self.log_dir.mkdir(parents=True, exist_ok=True)
        self.raw = bytearray()
        self.cond = threading.Condition()
        self.last_data = time.time()
        self.counter = 0
        self.log = open(self.log_dir / f"{name}.raw", "wb")

    def feed(self, data):
        with self.cond:
            self.raw += data
            self.last_data = time.time()
            self.log.write(data)
            self.log.flush()
            self.cond.notify_all()

    def mark(self):
        with self.cond:
            return len(self.raw)

    def text(self, since=0):
        with self.cond:
            data = bytes(self.raw[since:])
        return data.decode("utf-8", errors="replace")

    def wait(self, pattern, since, timeout=60, what=None):
        """Waits until `pattern` (a regular expression) matches the output after `since`."""
        regex = re.compile(pattern)
        deadline = time.time() + timeout
        with self.cond:
            while True:
                text = bytes(self.raw[since:]).decode("utf-8", errors="replace")
                if WINDOWS:
                    text = plain(text)
                found = regex.search(text)
                if found:
                    return found
                now = time.time()
                over = self.exited() and now - self.last_data > 1.5
                if now >= deadline or over:
                    state = "the program had ended" if over else f"after {timeout}s"
                    raise AssertionError(f"{what or pattern!r} not seen ({state}) in {self.name}; "
                                         f"output so far: {plain(text)[-700:]!r}")
                self.cond.wait(timeout=max(0.05, min(1.0, deadline - now)))

    def send_text(self, data):
        return data.encode("utf-8") if isinstance(data, str) else data

    def finish_log(self):
        try:
            self.log.close()
        except OSError:
            pass
        text = plain(bytes(self.raw).decode("utf-8", errors="replace"))
        (self.log_dir / f"{self.name}.txt").write_text(text, encoding="utf-8", errors="replace")


class UnixPty(Terminal):
    """`se` in a Python-made PTY: os.openpty, the child is a session leader whose
    controlling terminal is the slave (TIOCSCTTY), TERM xterm-256color, rows x cols."""

    def __init__(self, argv, env, log_dir, name, rows=30, cols=100, cwd=None):
        super().__init__(name, log_dir)
        self.master, self.slave = os.openpty()
        self.set_size(rows, cols)
        self.process = subprocess.Popen(
            [str(part) for part in argv], stdin=self.slave, stdout=self.slave, stderr=self.slave, env=env,
            cwd=cwd, start_new_session=True, preexec_fn=_become_controlling_terminal, close_fds=True)
        # The parent keeps the slave open on purpose: the terminal settings that `se`
        # leaves behind stay readable after it ended (checks of the restored terminal).
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        while True:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                return
            if not data:
                return
            self.feed(data)

    def set_size(self, rows, cols):
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    resize = set_size

    def exited(self):
        return self.process.poll() is not None

    def send(self, data):
        data = self.send_text(data)
        while data:
            written = os.write(self.master, data)
            data = data[written:]

    def exit_code(self, timeout=60):
        try:
            code = self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            raise AssertionError(f"{self.name} did not end within {timeout}s; output so far: "
                                 f"{plain(self.text())[-700:]!r}") from None
        time.sleep(0.4)
        return code

    def signal(self, number):
        os.kill(self.process.pid, number)

    def local_flags(self):
        """The terminal settings `se` left (or holds) on the outer PTY."""
        return termios.tcgetattr(self.slave)[3]

    def restored(self):
        flags = self.local_flags()
        return bool(flags & termios.ICANON) and bool(flags & termios.ECHO)

    def raw_mode(self):
        flags = self.local_flags()
        return not (flags & termios.ICANON) and not (flags & termios.ECHO)

    def close(self):
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except OSError:
                self.process.kill()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass
        time.sleep(0.2)
        for descriptor in (self.master, self.slave):
            try:
                os.close(descriptor)
            except OSError:
                pass
        self.finish_log()


def _become_controlling_terminal():
    # Runs in the child after setsid() and after standard input became the PTY slave.
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


class WinPty(Terminal):
    """`se.exe` under pywinpty (ConPTY). pywinpty returns text, so the output is kept as
    UTF-8 and matched after removing the VT sequences ConPTY adds."""

    def __init__(self, argv, env, log_dir, name, rows=30, cols=100, cwd=None):
        super().__init__(name, log_dir)
        from winpty import PtyProcess  # installed by the workflow (pywinpty==3.0.5)
        self.proc = PtyProcess.spawn([str(part) for part in argv], cwd=cwd, env=dict(env),
                                     dimensions=(rows, cols))
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        while True:
            try:
                chunk = self.proc.read(4096)
            except EOFError:
                return
            except Exception:  # noqa: BLE001 - the reader ends with the terminal
                return
            if chunk:
                self.feed(chunk.encode("utf-8", errors="replace"))
            elif not self.proc.isalive():
                return
            else:
                time.sleep(0.02)

    def exited(self):
        return not self.proc.isalive()

    def send(self, data):
        text = data.decode("utf-8", errors="replace") if isinstance(data, bytes) else data
        try:
            self.proc.write(text)
        except EOFError:
            raise AssertionError(f"{self.name} had already ended") from None

    def resize(self, rows, cols):
        self.proc.setwinsize(rows, cols)

    def exit_code(self, timeout=60):
        deadline = time.time() + timeout
        while self.proc.isalive():
            if time.time() > deadline:
                raise AssertionError(f"{self.name} did not end within {timeout}s; output so far: "
                                     f"{plain(self.text())[-700:]!r}")
            time.sleep(0.1)
        time.sleep(0.4)
        return self.proc.exitstatus

    def close(self):
        try:
            if self.proc.isalive():
                self.proc.terminate(force=True)
            self.proc.close(force=True)
        except Exception:  # noqa: BLE001 - best-effort clean-up
            pass
        time.sleep(0.2)
        self.finish_log()


def sync(term, shell=None, timeout=60):
    """Types a command whose output differs from its echo and waits for that output: the
    shell is up and idle afterwards. Returns the mark behind it."""
    shell = shell or term.shell
    term.counter += 1
    token = f"sy{term.counter}x"
    since = term.mark()
    if shell == "powershell":
        term.send(f"Write-Output ('{token}-' + (6*7))\r")
    else:
        term.send(f"echo {token}-$((6*7))\r")
    # The typed command holds the expression, only its output holds 42.
    term.wait(re.escape(f"{token}-42"), since, timeout, what=f"sync {token}")
    return term.mark()


def open_session(se_binary, env, args, logs, name, shell="posix", ready_timeout=150, rows=30, cols=100):
    """Starts `se shell ARGS` in a terminal, waits for the remote shell and a sync."""
    terminal_class = WinPty if WINDOWS else UnixPty
    term = terminal_class([se_binary, "shell", *args], env, logs, name, rows, cols)
    term.shell = shell
    try:
        if shell == "powershell":
            term.wait(r"PS\s*[A-Za-z]:", 0, ready_timeout, what="the PowerShell prompt")
        else:
            term.wait(r"[\s\S]", 0, ready_timeout, what="any output of the remote shell")
        time.sleep(0.5)
        sync(term, shell, 90)
    except Exception:
        term.close()
        raise
    return term


def tty_env(client, extra=None):
    env = dict(client.env)
    env["TERM"] = "xterm-256color"
    for name in ("COLUMNS", "LINES", "COMPLETE"):
        env.pop(name, None)
    env.update(extra or {})
    return env


def run_checks(results, stage, term, checks):
    for name, function in checks:
        if term is None:
            results.record(stage, name, False, "no terminal session")
        else:
            results.check(stage, name, lambda function=function: function(term))


# --------------------------------------------------------------------------- checks in a POSIX terminal

def check_tty(term):
    since = sync(term)
    term.send("tty\r")
    return term.wait(r"\n" + OUTPUT_START + r"(/dev/pts/\d+)\r?\n", since, 20, what="tty output").group(1)


def check_raw_while_running(term):
    ensure(term.raw_mode(), f"the local terminal is not in raw mode (lflag {term.local_flags():#x})")


def check_term(term):
    since = sync(term)
    term.send('echo "T=$TERM"\r')
    term.wait(out_line("T=xterm-256color"), since, 20, what="TERM of the remote shell")


def wait_size(term, rows, cols, attempts=5):
    """The size is passed on asynchronously: ask again until it shows."""
    for attempt in range(1, attempts + 1):
        since = term.mark()
        term.send("stty size\r")
        try:
            term.wait(out_line(f"{rows} {cols}"), since, 5, what=f"stty size {rows} {cols}")
            return attempt
        except AssertionError:
            time.sleep(1)
    raise AssertionError(f"stty size never showed {rows} {cols} (last output {plain(term.text(since))[-300:]!r})")


def check_size(term):
    sync(term)
    first = wait_size(term, 30, 100, 2)
    term.resize(40, 120)
    second = wait_size(term, 40, 120)
    return f"30 100 (try {first}), then 40 120 after TIOCSWINSZ on the outer PTY (try {second})"


def check_tab(term):
    sync(term)
    since = term.mark()
    term.send("ech\t")
    term.wait(re.escape("echo "), since, 20, what="Tab completion of `ech`")
    term.send(" tabok\r")
    term.wait(out_line("tabok"), since, 20, what="output of the completed command")
    ensure("command not found" not in term.text(since), term.text(since)[-300:])


def check_color(term):
    since = sync(term)
    term.send("printf '\\033[31mred\\033[0m\\n'\r")
    term.wait(re.escape(f"{ESC}[31mred{ESC}[0m"), since, 20, what="ESC[31mred ESC[0m")


def check_ctrl_c(term):
    since = sync(term)
    term.send("echo go-$((6*7)); sleep 100\r")
    term.wait("go-42", since, 20, what="the start of sleep")
    time.sleep(1.0)
    since = term.mark()
    term.send("\x03")
    # The remote line discipline echoes ^C when it signalled the foreground command; typing
    # earlier would be discarded with the unread input (Linux flushes it on SIGINT).
    term.wait(re.escape("^C"), since, 15, what="^C echo")
    time.sleep(0.5)
    since = term.mark()
    term.send("echo alive-$((1+1))\r")
    term.wait(out_line("alive-2"), since, 5, what="alive-2 after Ctrl+C")


def check_nvim(term):
    since = sync(term)
    term.send("nvim -u NONE -i NONE\r")
    term.wait(re.escape(f"{ESC}[?1049h"), since, 40, what="nvim switching to the alternate screen")
    time.sleep(1.5)
    since = term.mark()
    term.send(":q!\r")
    term.wait(re.escape(f"{ESC}[?1049l"), since, 30, what="nvim leaving the alternate screen")
    sync(term)


def check_password(term):
    since = sync(term)
    term.send("python3 -c \"import getpass;print('got:'+getpass.getpass('pw: '))\"\r")
    term.wait(r"\n" + OUTPUT_START + "pw: ", since, 40, what="the password prompt")
    after_prompt = term.mark()
    term.send("secret\r")
    term.wait("got:secret", after_prompt, 20, what="got:secret")
    segment = term.text(after_prompt)
    ensure("secret" not in segment[:segment.index("got:secret")],
           f"the typed password was echoed: {segment!r}")
    sync(term)


def check_raw_stdin(term):
    since = sync(term)
    # The program puts the terminal back as it found it: bash keeps the settings that a
    # foreground program leaves behind, and a raw terminal (no ICANON, so Ctrl+D is a data
    # byte) would break the checks after this one. The echo of the typed line never holds
    # `raw ok`; only the program's output does.
    term.send("python3 -c 'import termios as t, tty; s = t.tcgetattr(0); tty.setraw(0); "
              "print(\"raw\", \"ok\"); t.tcsetattr(0, t.TCSADRAIN, s)'\r")
    term.wait(r"raw ok\r?\n", since, 30, what="raw ok")
    sync(term)


def check_large_output(term):
    """50000 numbered lines (about 290 KB) arrive complete and in order (flow control)."""
    since = sync(term)
    term.send("seq 1 50000; echo seq-end-$((6*7))\r")
    term.wait("seq-end-42", since, 120, what="the end of the long output")
    numbers = [int(number) for number in
               re.findall(r"(?:^|\n)" + OUTPUT_START + r"(\d+)\r?(?=\n)", term.text(since))]
    ensure(numbers == list(range(1, 50001)),
           f"{len(numbers)} numbered lines arrived, the first ones {numbers[:5]}, the last ones {numbers[-5:]}")
    return "50000 lines complete and in order"


def check_large_paste(term):
    """About 130 KB (more than one 64 KiB frame) typed in one go reach a program unchanged."""
    sync(term)
    term.send("stty -echo; cat > /tmp/se-paste.txt\r")
    time.sleep(1.0)
    payload = (("0123456789" * 8) + "\n") * 1600
    term.send(payload)
    time.sleep(1.0)
    term.send("\x04")
    time.sleep(1.0)
    since = term.mark()
    term.send("stty echo; sha256sum < /tmp/se-paste.txt; wc -c < /tmp/se-paste.txt; rm -f /tmp/se-paste.txt\r")
    term.wait(re.escape(hashlib.sha256(payload.encode()).hexdigest()), since, 90, what="the sha256 of the pasted data")
    term.wait(out_line(str(len(payload))), since, 30, what="the byte count of the pasted data")
    sync(term)
    return f"{len(payload)} bytes"


def check_escape_help_and_tilde(term):
    sync(term)
    since = term.mark()
    term.send("~?")
    term.wait("Supported escape sequences", since, 10, what="the local help of ~?")
    since = term.mark()
    term.send("~~echo\r")
    term.wait(r"~echo: command not found", since, 20, what="one tilde sent for ~~")
    ensure("~~echo" not in term.text(since), term.text(since)[-300:])
    sync(term)


def posix_exit(term, code=7, timeout=60):
    term.send(f"exit {code}\r")
    return term.exit_code(timeout)


def check_exit_seven(term):
    sync(term)
    code = posix_exit(term, 7)
    ensure(code == 7, f"exit code {code}")
    ensure(term.restored(), "ICANON and ECHO of the outer PTY were not restored after exit")


# --------------------------------------------------------------------------- fmt, build, unit

def stage_fmt(results):
    rustfmt = shutil.which("rustfmt")
    results.record("fmt", "rustfmt on PATH", bool(rustfmt), rustfmt)
    for relative in BATCH_RUST_FILES:
        def check(relative=relative):
            path = ai.ROOT / relative
            ensure(path.exists(), "file of the batch is missing")
            source = path.read_text(encoding="utf-8")
            formatted = ai.run([rustfmt, "--edition", "2021", "--emit", "stdout"], input_text=source)
            ensure(formatted.returncode == 0, formatted.stderr.decode()[:400])
            ensure(formatted.stdout.decode("utf-8").replace("\r\n", "\n") == source.replace("\r\n", "\n"),
                   "not rustfmt-formatted")
        results.check("fmt", relative, check)


def failure_detail(stdout, stderr):
    lines = stdout.splitlines()
    failed = [line for line in lines if line.startswith("test ") and line.rstrip().endswith("FAILED")]
    section = lines[lines.index("failures:"):] if "failures:" in lines else []
    if not failed and not section:
        return (stdout + stderr)[-4000:]
    return "\n".join(failed + [""] + section + ["--- end of stderr ---", stderr[-1000:]])


def passed_test(passed, name):
    suffix = f"{name} ... ok"
    return any(line.endswith(suffix) and (line == f"test {suffix}" or line.endswith("::" + suffix))
               for line in passed)


def stage_unit(results, log_root):
    completed = ai.run(["cargo", "test", "--locked", "--lib", "--", *UNIT_FILTERS], cwd=ai.NATIVE,
                       log=log_root / "unit.log", timeout=5400)
    stdout = completed.stdout.decode(errors="replace")
    stderr = completed.stderr.decode(errors="replace")
    output = stdout + stderr
    summary = [line for line in output.splitlines() if line.startswith("test result:")]
    results.record("unit", "cargo test --lib: " + " + ".join(UNIT_FILTERS), completed.returncode == 0,
                   failure_detail(stdout, stderr) if completed.returncode else "; ".join(summary))
    passed = [line.rstrip() for line in output.splitlines() if line.startswith("test ") and line.rstrip().endswith("ok")]
    for prefix, expected in EXPECTED_PASSED.items():
        ran = [line for line in passed if prefix in line]
        results.record("unit", f"at least {expected} {prefix} tests passed", len(ran) >= expected, f"{len(ran)} passed")
    groups = list(COMMON_TESTS)
    groups += list(UNIX_TESTS) if not WINDOWS else []
    groups += list(LINUX_TESTS) if sys.platform.startswith("linux") else []
    groups += list(WINDOWS_TESTS) if WINDOWS else []
    listed = sum(len(names) for _, names in groups if all(n.startswith("terminal_task_") for n in names))
    results.record("unit", "the named terminal_task_ tests are as many as the expected count",
                   listed == EXPECTED_PASSED["terminal_task_"], f"{listed} named, {EXPECTED_PASSED['terminal_task_']} expected")
    for title, names in groups + list(NAMED_OTHER):
        missing = [name for name in names if not passed_test(passed, name)]
        results.record("unit", f"{title}: {len(names)} named tests passed", not missing, "missing: " + ", ".join(missing))


# --------------------------------------------------------------------------- MCP, sessions and processes

class ShellProcess:
    """A running `se shell` on pipes (no terminal) whose standard input stays open until finish()."""

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

    def kill(self):
        if self.process.poll() is None:
            self.process.kill()


def shell_list(se_binary, client, log):
    listed = client.se(se_binary, "shell", "--list", "--json", log=log)
    ensure(listed.returncode == 0, listed.stderr.decode())
    return json.loads(listed.stdout)


def wait_listed(se_binary, client, log, name, present=True, timeout=90):
    return ai.poll(lambda: any(entry["session"] == name for entry in shell_list(se_binary, client, log)) == present,
                   timeout, f"session {name} {'never listed' if present else 'still listed'}")


def isolated_client(work, name, extra=None):
    """ai.Client with, on Windows, a test namespace of its own: debug builds of `se` name the
    background worker's single-instance mutex and the Credential Manager entries after
    SMART_EXPLORER_E2E_TEST_NAMESPACE (native/src/windows_test_namespace.rs). Without it two
    clients on one Windows runner share one worker slot, and the second worker never starts."""
    extra = dict(extra or {})
    if WINDOWS:
        run = os.environ.get("GITHUB_RUN_ID", "local") + "-" + os.environ.get("GITHUB_RUN_ATTEMPT", "1")
        namespace = re.sub(r"[^A-Za-z0-9_-]", "-", f"{name}-{run}")[:48]
        ensure(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,47}", namespace), f"unusable test namespace {namespace!r}")
        extra["SMART_EXPLORER_E2E_TEST_NAMESPACE"] = namespace
    return ai.Client(work, name, extra)


class ShareFixture:
    """A local Share server and two paired clients: `a` (the user's side, runs `se shell`)
    and `b` (the host, this runner) with Exec granted. Used by the mcp, linux-share and
    windows stages."""

    def __init__(self, results, se_binary, work, log_root):
        self.results = results
        self.se = se_binary
        self.work = work
        self.logs = log_root / "share-fixture"
        self.logs.mkdir(parents=True, exist_ok=True)
        self.server = None
        self.a = None
        self.b = None
        self.endpoint = None
        self.tried = False

    def start(self):
        if self.tried:
            return self.endpoint is not None
        self.tried = True
        built = ai.run(["cargo", "build", "--locked", "--manifest-path", ai.ROOT / "share-server" / "Cargo.toml",
                        "--bin", "se-share-server"], log=self.logs / "share-server-build.log", timeout=5400)
        if built.returncode != 0:
            self.results.record("share-fixture", "share-server fixture build", False, built.stderr.decode()[-2000:])
            return False
        binary = ai.ROOT / "share-server" / "target" / "debug" / ("se-share-server.exe" if WINDOWS else "se-share-server")
        port = 31000 + os.getpid() % 12000
        self.server = subprocess.Popen([binary, f"127.0.0.1:{port}"], stdout=open(self.logs / "server.log", "wb"),
                                       stderr=subprocess.STDOUT)
        # The host (b) runs a login bash on Linux so that Tab completion exists.
        host_env = {"SE_SHARE_RELAY_ONLY": "1"}
        if not WINDOWS:
            host_env["SHELL"] = "/bin/bash"
        self.a = isolated_client(self.work, "term-a", {"SE_SHARE_RELAY_ONLY": "1", "TERM": "xterm-256color"})
        self.b = isolated_client(self.work, "term-b", host_env)
        time.sleep(1)
        self.endpoint = ai.pair_with_exec(self.results, self.se, self.a, self.b, f"127.0.0.1:{port}",
                                          f"http://127.0.0.1:{port + 1}", self.logs)
        return self.endpoint is not None

    def stop(self):
        if self.a is not None:
            ai.stop_background_daemons([self.a, self.b])
        if self.server is not None:
            self.server.terminate()
            try:
                self.server.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.server.kill()

    def shell_args(self, *extra):
        """Arguments of `se shell` against the peer; on Linux the host shell is bash."""
        shell = [] if WINDOWS else ["--env", "SHELL=/bin/bash"]
        return [self.endpoint, *shell, *extra]


# --------------------------------------------------------------------------- mcp stage

def stage_mcp(results, se_binary, work, log_root, fixture):
    stage = "mcp"
    logs = log_root / "mcp"
    logs.mkdir(parents=True, exist_ok=True)
    plain_client = isolated_client(work, "term-mcp")

    def catalog(lifecycle):
        def check():
            session = ai.McpSession(se_binary, plain_client, lifecycle, logs, f"catalog-{lifecycle}")
            try:
                instructions = session.info.get("instructions", "")
                first = instructions.split("\n\n")[0]
                ensure(len(instructions) <= 2048, f"instructions {len(instructions)} characters")
                ensure(len(first) <= 512, f"first paragraph {len(first)} characters")
                missing = [word for word in PRIORITY_WORDS if word not in first]
                ensure(not missing, f"priority paragraph lacks {missing}: {first}")
                ensure("-T --attachable" in instructions, "instructions do not name `se shell -T --attachable`")
                tools = session.tools()
                ensure([tool["name"] for tool in tools] == ai.EXPECTED_TOOLS, [tool["name"] for tool in tools])
                by_name = {tool["name"]: tool for tool in tools}
                ensure("-T --attachable" in by_name["sessions"]["description"], by_name["sessions"]["description"])
                ensure("terminal" in by_name["sessions"]["description"], by_name["sessions"]["description"])
                ensure("-T --attachable" in by_name["exec"]["description"], by_name["exec"]["description"])
                ensure(all(len(tool.get("description", "")) <= 2048 for tool in tools), "a description is too long")
            finally:
                ensure(session.close() == 0, "server exit code")
            return f"{len(first)}-character priority paragraph"
        return check

    results.check(stage, "instructions and tool texts over initialize 2025-06-18 (Codex)", catalog("initialize"))
    results.check(stage, "instructions and tool texts over server/discover 2026-07-28 (Claude Code)", catalog("modern"))

    def basics():
        session = ai.McpSession(se_binary, plain_client, "modern", logs, "sessions-basics")
        try:
            ensure(session.call("sessions", {}) == {"sessions": []}, "sessions are not empty at the start")
        finally:
            ensure(session.close() == 0, "server exit code")
        listed = plain_client.se(se_binary, "shell", "--list", "--json", log=logs / "cli.log")
        ensure(listed.returncode == 0 and json.loads(listed.stdout) == [], listed.stdout)
        text = plain_client.se(se_binary, "shell", "--list", log=logs / "cli.log")
        ensure(text.returncode == 0 and b"no attachable sessions" in text.stdout, text.stdout)
        help_text = plain_client.se(se_binary, "shell", "--help", log=logs / "cli.log").stdout.decode()
        for word in ("--attachable", "--attach", "--list", "--no-tty", "--escape-char", "~.", "pseudo-terminal"):
            ensure(word in help_text, f"`se shell --help` lacks {word}")
    results.check(stage, "empty sessions list, `se shell --list` and --help name the terminal options", basics)

    def skill_text():
        source = (ai.ROOT / "native" / "src" / "cli" / "plugin" / "assets" / "SKILL.md").read_text(encoding="utf-8")
        text = source
        if not WINDOWS:
            # Linux has the claude CLI: the skill as `se plugin install claude` writes it.
            installed = plain_client.se(se_binary, "plugin", "install", "claude", "--json", timeout=600,
                                        log=logs / "plugin.log")
            ensure(installed.returncode == 0, installed.stdout.decode() + installed.stderr.decode())
            written = (plain_client.app_data / "ai-plugins" / "claude" / "smart-explorer" / "skills" / "smart-explorer"
                       / "SKILL.md")
            text = written.read_text(encoding="utf-8")
            removed = plain_client.se(se_binary, "plugin", "uninstall", "claude", "--json", timeout=600,
                                      log=logs / "plugin.log")
            ensure(removed.returncode == 0, removed.stdout.decode() + removed.stderr.decode())
        flat = " ".join(text.split())
        for needle in ("se shell -T --attachable", "full terminal session", "agents cannot join it"):
            ensure(needle in flat, f"the skill lacks {needle!r}")
        ensure("## Priority" in text, "the skill lost its priority section")
    results.check(stage, "the agent skill names terminal sessions that agents cannot join and `se shell -T --attachable`",
                  skill_text)
    ai.stop_background_daemons([plain_client])

    if not fixture.start():
        results.record(stage, "terminal session listed, refused for agents, command shell joined", False,
                       "the Share fixture (pairing with Exec) is not available")
        return
    a = fixture.a
    shell = "powershell" if WINDOWS else "posix"
    session = ai.McpSession(se_binary, a, "modern", logs, "terminal-kind")
    term = None
    try:
        def start_terminal():
            nonlocal term
            term = open_session(se_binary, tty_env(a), fixture.shell_args("--attachable", "--name", "tt"), logs,
                                "terminal-tt", shell)
            ensure(term.mark() > 0, "no output")
        results.check(stage, "se shell <peer> --attachable --name tt opens a terminal session", start_terminal)

        def listed_as_terminal():
            ensure(term is not None, "no terminal session")
            rows = session.call("sessions", {})["sessions"]
            row = next((row for row in rows if row.get("session") == "tt"), None)
            ensure(row is not None, rows)
            ensure(row.get("kind") == "terminal" and row.get("role") == "none", row)
            ensure("agents cannot join" in row.get("next", ""), row)
            return json.dumps(row)
        results.check(stage, "MCP `sessions` lists the terminal session with kind terminal", listed_as_terminal)

        def attach_refused():
            ensure(term is not None, "no terminal session")
            refused = session.call("sessions", {"action": "attach", "session": "tt"}, expect_error=True)
            ensure("terminal" in refused["error"] and "-T --attachable" in refused["error"], refused)
            exec_refused = session.call("exec", {"session": "tt", "command": "echo x"}, expect_error=True)
            ensure("give `target`" in exec_refused["error"], exec_refused)
        results.check(stage, "MCP `attach` refuses it and names `-T --attachable`; `exec` in it needs a target",
                      attach_refused)

        def cli_listing():
            ensure(term is not None, "no terminal session")
            entries = shell_list(se_binary, a, logs / "cli.log")
            entry = next((entry for entry in entries if entry["session"] == "tt"), None)
            ensure(entry is not None and entry["kind"] == "terminal", entries)
            ensure({"id", "session", "owner", "target", "kind", "created_ms"} <= set(entry), entry)
            ensure("dialect" not in entry and "token" not in entry and "port" not in entry, entry)
            listing = a.se(se_binary, "shell", "--list", log=logs / "cli.log").stdout.decode()
            fields = next(line for line in listing.splitlines() if line.startswith("tt\t")).split("\t")
            ensure(len(fields) == 6 and fields[2] == "-" and fields[5] == "terminal", fields)
            refused = a.se(se_binary, "shell", "--attach", "tt", input_text="true\n", log=logs / "cli.log")
            ensure(refused.returncode == 125 and b"terminal session" in refused.stderr,
                   (refused.returncode, refused.stderr))
        results.check(stage, "`se shell --list` (--json) shows kind, six columns, no token; --attach without a "
                             "terminal fails with exit 125", cli_listing)

        def leaves():
            ensure(term is not None, "no terminal session")
            term.send("exit\r")
            code = term.exit_code(60)
            ensure(code == 0, f"exit code {code}")
            wait_listed(se_binary, a, logs / "cli.log", "tt", present=False, timeout=30)
        results.check(stage, "the end of the shell exits 0 and unlists the session", leaves)
    finally:
        if term is not None:
            term.close()

    try:
        def command_session_kind():
            opened = ai.run_exec(session, {"target": fixture.endpoint, "session": "agent-p1", "command": "echo ok"},
                                 timeout=240)
            ensure(opened["status"] == "completed" and opened["exit_code"] == 0, opened)
            rows = session.call("sessions", {})["sessions"]
            row = next((row for row in rows if row.get("session") == "agent-p1"), None)
            ensure(row is not None and row.get("kind") == "commands" and row.get("role") == "owner", rows)
            closed = session.call("sessions", {"action": "close", "session": "agent-p1"})
            ensure(closed["closed"] is True, closed)
        results.check(stage, "an agent's own command session lists kind commands", command_session_kind)
    finally:
        session.close()

    join_command_shell(results, se_binary, fixture, logs)


def join_command_shell(results, se_binary, fixture, logs):
    """V4: an agent joins `se shell -T --attachable` as before (pipes, no terminal)."""
    stage = "mcp"
    a = fixture.a
    session = ai.McpSession(se_binary, a, "initialize", logs, "join-command-shell")
    human = None
    try:
        def joins():
            nonlocal human
            args = fixture.shell_args("-T", "--attachable", "--name", "human")
            human = ShellProcess(se_binary, a, args, logs, "human")
            wait_listed(se_binary, a, logs / "cli.log", "human", timeout=240)
            entry = next(entry for entry in shell_list(se_binary, a, logs / "cli.log") if entry["session"] == "human")
            ensure(entry["kind"] == "commands" and "dialect" in entry, entry)
            attached = session.call("sessions", {"action": "attach", "session": "human"})
            ensure(attached["role"] == "attached" and attached["session"] == "human", attached)
            if WINDOWS:
                set_state = "$env:M = 'agent'"
                show = "Write-Output ($env:M + '|' + 'shared')\n"
            else:
                set_state = "export M=agent; cd /tmp"
                show = 'printf "%s|" "$M"; pwd\n'
            ran = ai.run_exec(session, {"session": "human", "command": set_state})
            ensure(ran["exit_code"] == 0, ran)
            human.send(show)
            line = human.read_line().strip()
            ensure(line == ("agent|shared" if WINDOWS else "agent|/tmp"), f"the terminal did not see the agent's state: {line!r}")
            code, _ = human.finish("exit 4\n")
            ensure(code == 4, f"se shell exit code {code}: {human.stderr()}")
            stderr = human.stderr()
            ensure("se mcp (pid" in stderr and "attached" in stderr, stderr)
        results.check(stage, "an agent joins `se shell -T --attachable` as before: shared state, exit 4 ends it", joins)
    finally:
        if human is not None:
            human.kill()
        session.close()


# --------------------------------------------------------------------------- Linux: OpenSSH container

PREPARE_SCRIPT = r'''
set -eu
if [ "$SE_INSTALL" = 1 ]; then
  apk add --no-cache bash neovim python3 >/dev/null
  if command -v usermod >/dev/null 2>&1; then
    usermod -s /bin/bash "$SE_USER"
  else
    sed -i "s#^\($SE_USER:.*:\)[^:]*\$#\1/bin/bash#" /etc/passwd
  fi
  grep -q "^$SE_USER:.*:/bin/bash\$" /etc/passwd
fi
CONF=$(readlink -f /config/sshd/sshd_config 2>/dev/null || true)
[ -f "$CONF" ] || CONF=/etc/ssh/sshd_config
grep -vi '^[[:space:]]*#\{0,1\}[[:space:]]*PermitTTY' "$CONF" > /tmp/sshd_config.new || true
{ echo "PermitTTY $SE_PERMIT_TTY"; cat /tmp/sshd_config.new; } > "$CONF"
echo "sshd config: $CONF"; grep -in permittty "$CONF"
pid=
for f in /var/run/sshd.pid /run/sshd.pid /config/sshd.pid; do
  if [ -s "$f" ]; then pid=$(cat "$f"); break; fi
done
if [ -z "$pid" ] || [ ! -d "/proc/$pid" ]; then
  pid=
  for d in /proc/[0-9]*; do
    args=$(tr '\0' ' ' < "$d/cmdline" 2>/dev/null || true)
    case "$args" in *sshd\ -D*|*sshd\ -e*) pid=${d#/proc/}; break;; esac
  done
fi
[ -n "$pid" ]
kill -HUP "$pid"
sleep 2
echo "sshd $pid reloaded"
'''


def start_ssh_container(log_root, tag):
    """Another OpenSSH container (ai.docker_ssh removes a container of its own fixed name)."""
    user, password = "seuser", base64.b16encode(os.urandom(12)).decode().lower()
    name = f"se-terminal-ssh-{tag}-{os.getpid()}"
    ai.run(["docker", "rm", "-f", name], timeout=120)
    started = ai.run(["docker", "run", "-d", "--name", name, "-p", "127.0.0.1::2222",
                      "-e", f"PUID={os.getuid()}", "-e", f"PGID={os.getgid()}", "-e", "PASSWORD_ACCESS=true",
                      "-e", f"USER_NAME={user}", "-e", f"USER_PASSWORD={password}",
                      "lscr.io/linuxserver/openssh-server:latest"], timeout=600, log=log_root / "docker.log")
    ensure(started.returncode == 0, started.stderr.decode())
    port = int(ai.run(["docker", "port", name, "2222/tcp"]).stdout.decode().strip().rsplit(":", 1)[1])
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


def prepare_container(container, user, install, permit_tty, log):
    """bash, neovim and python3 for the user (Alpine), bash as login shell, and PermitTTY."""
    prepared = ai.run(["docker", "exec", "-u", "root", "-e", f"SE_USER={user}", "-e", f"SE_INSTALL={1 if install else 0}",
                       "-e", f"SE_PERMIT_TTY={'yes' if permit_tty else 'no'}", container, "sh", "-c", PREPARE_SCRIPT],
                      timeout=900, log=log)
    ensure(prepared.returncode == 0, (prepared.stdout + prepared.stderr).decode(errors="replace")[-1500:])
    return prepared.stdout.decode(errors="replace").strip().replace("\n", "; ")


def add_connection(client, se_binary, port, user, password, label, root, log):
    added = client.se(se_binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", str(port),
                      "--user", user, "--root", root, "--label", label, "--password-stdin",
                      input_text=password + "\n", log=log)
    ensure(added.returncode == 0, added.stderr.decode())


class SshFixture:
    """The OpenSSH containers and the isolated client of the Linux SSH stages. The preserve
    stage reuses them after the Share stage, so they are removed by main()."""

    def __init__(self):
        self.client = None
        self.logs = None
        self.containers = []
        self.ready = False

    def stop(self):
        for name in self.containers:
            ai.run(["docker", "rm", "-f", name], timeout=120)
        if self.client is not None:
            ai.stop_background_daemons([self.client])


def stage_linux_ssh(results, se_binary, work, log_root, fixture):
    stage = "linux-ssh"
    logs = log_root / "linux-ssh"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "cli.log"
    client = isolated_client(work, "term-ssh", {"TERM": "xterm-256color"})
    fixture.client = client
    fixture.logs = logs
    try:
        container, port, user, password = ai.docker_ssh(work, log_root)
        fixture.containers.append(container)
    except Exception as error:  # noqa: BLE001 - recorded as a failed fixture
        results.record(stage, "OpenSSH container fixture", False, error)
        return
    try:
        detail = prepare_container(container, user, True, True, logs / "prepare.log")
        results.record(stage, "container: bash, neovim, python3, bash as login shell, PermitTTY yes", True, detail)
        add_connection(client, se_binary, port, user, password, "sshbox", "/config", log)
        warm = client.se(se_binary, "exec", "@sshbox", "--", "mkdir", "-p", "work", log=log, timeout=240)
        ensure(warm.returncode == 0, warm.stderr.decode())
        results.record(stage, "saved connection @sshbox (root /config) added, host key accepted without a terminal", True)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "container preparation and saved connection @sshbox", False,
                       f"{type(error).__name__}: {error}")
        return
    fixture.ready = True
    try:
        add_connection(client, se_binary, port, user, password, "sshroot", "/", log)
        results.record(stage, "saved connection @sshroot (root /) added", True)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "saved connection @sshroot (root /) added", False, f"{type(error).__name__}: {error}")
    env = tty_env(client)
    ssh_main_session(results, se_binary, env, logs)
    ssh_other_endings(results, se_binary, env, logs)
    ssh_working_directory(results, se_binary, env, logs)
    ssh_refused_terminal(results, se_binary, work, log_root, logs, fixture.containers)
    ai.stop_background_daemons([client])


def ssh_main_session(results, se_binary, env, logs):
    stage = "linux-ssh"
    term = None
    try:
        term = open_session(se_binary, env, ["@sshbox"], logs, "main")
        results.record(stage, "se shell @sshbox opens a terminal session (30x100, TERM xterm-256color)", True)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "se shell @sshbox opens a terminal session (30x100, TERM xterm-256color)", False,
                       f"{type(error).__name__}: {error}")
    try:
        run_checks(results, stage, term, [
            ("(a) tty prints /dev/pts/N", check_tty),
            ("the local terminal is in raw mode while the session runs", check_raw_while_running),
            ("(b) TERM is xterm-256color", check_term),
            ("(c) stty size 30 100, then 40 120 after the outer PTY was resized", check_size),
            ("(d) Tab completes a command (ech + Tab)", check_tab),
            ("(e) colours reach the screen unchanged (ESC[31m ... ESC[0m)", check_color),
            ("(f) Ctrl+C ends sleep and the shell answers within 5 s", check_ctrl_c),
            ("(g) nvim uses the alternate screen (1049h/1049l)", check_nvim),
            ("(h) a password prompt does not echo and the program gets the input", check_password),
            ("(i) a program may switch stdin to raw mode", check_raw_stdin),
            ("(S6) 50000 lines of output arrive complete and in order", check_large_output),
            ("(S6) a 130 KB paste (more than one 64 KiB frame) reaches cat unchanged", check_large_paste),
            ("`~?` shows the local help and `~~` sends one tilde", check_escape_help_and_tilde),
            ("(j) exit 7 gives exit code 7 and (l) restores ICANON and ECHO", check_exit_seven),
        ])
    finally:
        if term is not None:
            term.close()


def ssh_other_endings(results, se_binary, env, logs):
    stage = "linux-ssh"

    def leave():
        term = open_session(se_binary, env, ["@sshbox"], logs, "leave")
        try:
            since = term.mark()
            term.send("~.")
            code = term.exit_code(60)
            ensure(code == 130, f"exit code {code}")
            ensure("session closed" in term.text(since), term.text(since)[-300:])
            ensure(term.restored(), "ICANON and ECHO of the outer PTY were not restored after ~.")
        finally:
            term.close()
    results.check(stage, "(k) ~. at the start of a line ends the session with exit code 130, terminal restored", leave)

    def sigterm():
        term = open_session(se_binary, env, ["@sshbox"], logs, "sigterm")
        try:
            ensure(term.raw_mode(), "not in raw mode before SIGTERM")
            since = term.mark()
            term.signal(signal.SIGTERM)
            code = term.exit_code(60)
            ensure(code == 143, f"exit code {code}")
            ensure("request of the system" in term.text(since), term.text(since)[-300:])
            ensure(term.restored(), "ICANON and ECHO of the outer PTY were not restored after SIGTERM")
        finally:
            term.close()
    results.check(stage, "(l) SIGTERM to se: exit code 143, message, ICANON and ECHO restored", sigterm)

    def escape_none():
        term = open_session(se_binary, env, ["@sshbox", "-e", "none"], logs, "escape-none")
        try:
            since = term.mark()
            term.send("~.\r")
            term.wait(r"~\.: command not found", since, 20, what="~. passed on to the remote shell")
            sync(term)
            ensure(not term.exited(), "se ended on ~. although the escape sequences are off")
            code = posix_exit(term, 0)
            ensure(code == 0, f"exit code {code}")
        finally:
            term.close()
    results.check(stage, "--escape-char none passes ~. on to the remote shell", escape_none)

    def failed_start():
        term = UnixPty([se_binary, "shell", "@no-such-connection"], env, logs, "failed-start")
        try:
            code = term.exit_code(60)
            ensure(code != 0, f"exit code {code}")
            ensure("no-such-connection" in term.text(), plain(term.text())[-300:])
            ensure(term.restored(), "the terminal was changed although no session started")
        finally:
            term.close()
    results.check(stage, "(S5) a start that fails leaves the terminal as it was and exits non-zero", failed_start)


def ssh_working_directory(results, se_binary, env, logs):
    stage = "linux-ssh"

    def pwd_in(selector, expected):
        def check():
            term = open_session(se_binary, env, [selector], logs, "cwd-" + re.sub(r"\W", "_", selector))
            try:
                since = term.mark()
                term.send("pwd\r")
                term.wait(out_line(expected), since, 20, what=f"pwd {expected}")
                posix_exit(term, 0)
            finally:
                term.close()
        return check
    results.check(stage, "(n) @sshroot:/tmp (connection root /) starts in /tmp", pwd_in("@sshroot:/tmp", "/tmp"))
    results.check(stage, "(n) @sshbox:/work (connection root /config) starts in /config/work",
                  pwd_in("@sshbox:/work", "/config/work"))


def ssh_refused_terminal(results, se_binary, work, log_root, logs, containers):
    stage = "linux-ssh"
    client = isolated_client(work, "term-ssh-deny", {"TERM": "xterm-256color"})
    try:
        container, port, user, password = start_ssh_container(log_root, "deny")
        containers.append(container)
        detail = prepare_container(container, user, False, False, logs / "prepare-deny.log")
        add_connection(client, se_binary, port, user, password, "sshdeny", "/config", logs / "cli.log")
        warm = client.se(se_binary, "exec", "@sshdeny", "--", "true", log=logs / "cli.log", timeout=240)
        ensure(warm.returncode == 0, warm.stderr.decode())
    except Exception as error:  # noqa: BLE001
        results.record(stage, "(m) second container with PermitTTY no", False, f"{type(error).__name__}: {error}")
        return

    def fallback():
        term = UnixPty([se_binary, "shell", "@sshdeny"], tty_env(client), logs, "fallback")
        try:
            term.wait(r"refused a terminal \(PTY\); using the command shell", 0, 120,
                      what="the fallback hint (is PermitTTY no in effect? " + detail + ")")
            term.wait(r"sshdeny:", 0, 60, what="the command shell prompt")
            ensure(not term.raw_mode(), "the outer terminal is in raw mode in the command shell")
            since = term.mark()
            term.send("echo ok\r")
            term.wait(out_line("ok"), since, 60, what="ok from the command shell")
            term.send("exit\r")
            code = term.exit_code(60)
            ensure(code == 0, f"exit code {code}")
        finally:
            term.close()
    results.check(stage, "(m) a server with PermitTTY no: hint on stderr, then the line mode works", fallback)
    ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- Linux: preserve

def stage_preserve(results, se_binary, fixture):
    """V1 to V4, V6 and V10 (line mode) against the OpenSSH container of the linux-ssh stage."""
    stage = "preserve"
    if not fixture.ready:
        results.record(stage, "OpenSSH container and saved connection @sshbox from the linux-ssh stage", False,
                       "the fixture is not available")
        return
    client, logs = fixture.client, fixture.logs
    log = logs / "cli.log"
    env = tty_env(client)

    def pipe_mode():
        script = "cd /tmp\nexport X=5\nprintf '%s:' \"$X\"; pwd\nfalse\nexit 4\n"
        shell = client.se(se_binary, "shell", "@sshbox", log=log, timeout=180, input_text=script)
        ensure(shell.returncode == 4, (shell.returncode, shell.stderr))
        ensure(shell.stdout.decode() == "5:/tmp\n", shell.stdout)
        ensure(b"\x1b[" not in shell.stdout, "control sequences in the piped output")
    results.check(stage, "(V1) piped `se shell @sshbox`: state over lines, exit 4, no terminal output", pipe_mode)

    def no_tty_in_terminal():
        term = UnixPty([se_binary, "shell", "@sshbox", "-T"], env, logs, "no-tty")
        try:
            term.wait(r"sshbox:[^\r\n]*\$ ", 0, 90, what="the line mode prompt on stderr")
            ensure(not term.raw_mode(), "-T put the terminal into raw mode")
            since = term.mark()
            term.send("tty\r")
            term.wait("not a tty", since, 60, what="`not a tty` from the remote command")
            # A line typed while a command runs goes to that command's standard input: type
            # `exit 3` only after `tty` ended and the line mode showed its next prompt.
            term.wait(r"not a tty[\s\S]*sshbox:[^\r\n]*\$ ", since, 60, what="the prompt after `tty`")
            term.send("exit 3\r")
            code = term.exit_code(60)
            ensure(code == 3, f"exit code {code}")
        finally:
            term.close()
    results.check(stage, "(V2) `se shell @sshbox -T` in a terminal: line mode, prompt sshbox:, remote tty is `not a tty`",
                  no_tty_in_terminal)

    session = ai.McpSession(se_binary, client, "initialize", logs, "preserve")
    owner = None
    try:
        def attach_agent_session():
            opened = ai.run_exec(session, {"target": "@sshbox", "session": "agent1",
                                           "command": "export A=from-agent; cd /tmp"})
            ensure(opened["exit_code"] == 0, opened)
            term = UnixPty([se_binary, "shell", "--attach", "agent1"], env, logs, "attach-agent")
            try:
                term.wait(r"attached to session agent1", 0, 90, what="the attach line")
                ensure(not term.raw_mode(), "the terminal is in raw mode in the line mode")
                since = term.mark()
                term.send('printf "%s|" "$A"; pwd\r')
                term.wait(r"from-agent\|/tmp", since, 90, what="the agent's state in the joined shell")
                term.send("\x04")
                code = term.exit_code(60)
                ensure(code == 0, f"exit code {code}")
            finally:
                term.close()
            rows = session.call("sessions", {})["sessions"]
            row = next((row for row in rows if row.get("session") == "agent1"), None)
            ensure(row is not None and row.get("kind") == "commands", rows)
        results.check(stage, "(V3) `se shell --attach` to an agent's session in a terminal: line mode, shared state, Ctrl+D detaches",
                      attach_agent_session)

        def attach_command_shell():
            nonlocal owner
            owner = ShellProcess(se_binary, client, ["@sshbox", "-T", "--attachable", "--name", "cmdsh"], logs, "cmdsh")
            wait_listed(se_binary, client, log, "cmdsh")
            entry = next(entry for entry in shell_list(se_binary, client, log) if entry["session"] == "cmdsh")
            ensure(entry["kind"] == "commands" and entry["dialect"] == "posix", entry)
            term = UnixPty([se_binary, "shell", "--attach", "cmdsh"], env, logs, "attach-cmdsh")
            try:
                term.wait(r"attached to session cmdsh", 0, 90, what="the attach line")
                # The line mode prints its prompt after the attach line; a line typed before it
                # would put the prompt in front of the output.
                term.wait(r"attached to session cmdsh[^\n]*\n[\s\S]*sshbox:[^\r\n]*\$ ", 0, 90,
                          what="the line mode prompt after the attach line")
                since = term.mark()
                term.send("echo joined-$((2+2))\r")
                term.wait(out_line("joined-4"), since, 90, what="joined-4")
                term.send("\x04")
                ensure(term.exit_code(60) == 0, "attached shell did not detach with 0")
            finally:
                term.close()
            ensure(owner.process.poll() is None, "the owner ended when the attached shell detached")
            code, _ = owner.finish("exit 0\n")
            ensure(code == 0, f"owner exit code {code}")
        results.check(stage, "(V3) `se shell --attach` to a `-T --attachable` command shell in a terminal keeps working",
                      attach_command_shell)

        def exec_unchanged():
            echo = client.se(se_binary, "exec", "@sshbox", "--", "echo", "cli ok", log=log)
            ensure(echo.returncode == 0 and echo.stdout == b"cli ok\n", (echo.returncode, echo.stdout))
            code = client.se(se_binary, "exec", "@sshbox", "--shell", "exit 5", log=log)
            ensure(code.returncode == 5, code.returncode)
            plain_result = ai.run_exec(session, {"target": "@sshbox", "command": "echo plain"})
            ensure(plain_result["stdout"]["text"] == "plain\n" and "session" not in plain_result, plain_result)
            timed = client.se(se_binary, "exec", "@sshbox", "--timeout", "2", "--", "sleep", "30", log=log)
            ensure(timed.returncode == 124, timed.returncode)
        results.check(stage, "(V6) `se exec @sshbox` and MCP `exec` without `session` are unchanged (output, 5, 124)",
                      exec_unchanged)

        def join_terminal_shell():
            term = open_session(se_binary, env, ["@sshbox", "--attachable", "--name", "terminal-tt"], logs, "mcp-terminal")
            try:
                rows = session.call("sessions", {})["sessions"]
                row = next((row for row in rows if row.get("session") == "terminal-tt"), None)
                ensure(row is not None and row["kind"] == "terminal", rows)
                refused = session.call("sessions", {"action": "attach", "session": "terminal-tt"}, expect_error=True)
                ensure("terminal" in refused["error"] and "-T --attachable" in refused["error"], refused)
                posix_exit(term, 0)
            finally:
                term.close()
        results.check(stage, "(V4/S2) the MCP server lists an SSH terminal session and refuses to join it", join_terminal_shell)
    finally:
        if owner is not None:
            owner.kill()
        session.close()

    def update_stop_line_mode():
        marker = client.app_data / "sync" / "daemon.stop"
        shell = ShellProcess(se_binary, client, ["@sshbox"], logs, "update-line")
        try:
            shell.send("echo up-$((1+1))\n")
            ensure(shell.read_line() == "up-2\n", "the line mode shell did not answer")
            marker.parent.mkdir(parents=True, exist_ok=True)
            marker.write_text("stop")
            code = shell.process.wait(timeout=60)
            ensure(code == 125, f"exit code {code}")
            ensure("being updated" in shell.stderr(), shell.stderr())
        finally:
            marker.unlink(missing_ok=True)
            shell.kill()
    results.check(stage, "(V10) an update stop ends the line-mode shell with the notice and exit code 125", update_stop_line_mode)
    ai.stop_background_daemons([client])


# --------------------------------------------------------------------------- Linux: Share peer

def file_contains(root, needle, skip_names=(".bash_history",)):
    """Paths below `root` whose bytes contain `needle`."""
    found = []
    for directory, _, names in os.walk(root):
        for name in names:
            path = Path(directory) / name
            if name in skip_names:
                continue
            try:
                # Regular files only: reading a FIFO would block, a socket cannot be read.
                if path.is_symlink() or not path.is_file() or path.stat().st_size > 64 * 1024 * 1024:
                    continue
                if needle in path.read_bytes():
                    found.append(str(path))
            except OSError:
                continue
    return found


def check_tui(term, name):
    executable = shutil.which(name)
    ensure(executable, f"{name} is not on PATH of the runner")
    directories = []
    for tool in (executable, shutil.which("node")):
        if tool and str(Path(tool).parent) not in directories:
            directories.append(str(Path(tool).parent))
    # A login shell resets PATH: the line adds the directories of the tool and of node.
    since = sync(term)
    term.send(f'HOME=$(mktemp -d) PATH="{":".join(directories)}:$PATH" timeout --foreground -k 5 -s INT 20 {name}; echo rc=$?\r')
    done = term.wait(r"rc=(\d+)", since, 180, what=f"the end of {name}")
    text = term.text(since)
    # What the program drew: after the shell switched bracketed paste off for the command and
    # before the rc line. The prompt and the echo of the typed line (which also hold CSI
    # sequences) are outside of it; without that marker the whole stretch is used.
    drawn = text[:done.start()].split(f"{ESC}[?2004l", 1)[-1]
    ensure(f"{ESC}[" in drawn, f"{name} drew no terminal control sequences: {plain(text)[-500:]!r}")
    lowered = text.lower()
    for bad in ("raw mode is not supported", "not a terminal", "not a tty"):
        ensure(bad not in lowered, f"{name} reported `{bad}`: {plain(text)[-500:]!r}")
    sync(term)
    return f"rc={done.group(1)}, {len(drawn)} bytes drawn by {name}"


def stage_linux_share(results, se_binary, work, log_root, fixture):
    stage = "linux-share"
    logs = log_root / "linux-share"
    logs.mkdir(parents=True, exist_ok=True)
    if not fixture.start():
        results.record(stage, "Share fixture (paired clients, Exec granted)", False, "the fixture is not available")
        return
    a, b, endpoint = fixture.a, fixture.b, fixture.endpoint
    env = tty_env(a)
    canary = "CANARY" + base64.b16encode(os.urandom(6)).decode()
    canary_ran = []
    owner = None
    try:
        try:
            owner = open_session(se_binary, env, fixture.shell_args("--attachable", "--name", "tt"), logs, "owner")
            owner.send("unset HISTFILE\r")
            sync(owner)
            results.record(stage, "se shell <peer> --attachable --name tt opens a terminal session", True)
        except Exception as error:  # noqa: BLE001
            results.record(stage, "se shell <peer> --attachable --name tt opens a terminal session", False,
                           f"{type(error).__name__}: {error}")
            owner = None

        def canary_check(term):
            since = sync(term)
            term.send(f"echo {canary}-$((6*7))\r")
            term.wait(out_line(f"{canary}-42"), since, 30, what="the canary line")
            canary_ran.append(True)
        run_checks(results, stage, owner, [
            ("(a) tty prints /dev/pts/N", check_tty),
            ("the local terminal is in raw mode while the session runs", check_raw_while_running),
            ("(c) stty size 30 100, then 40 120 after the outer PTY was resized", check_size),
            ("(d) Tab completes a command (ech + Tab)", check_tab),
            ("(f) Ctrl+C ends sleep and the shell answers within 5 s", check_ctrl_c),
            ("(g) nvim uses the alternate screen (1049h/1049l)", check_nvim),
            ("(S6) 50000 lines of output arrive complete and in order", check_large_output),
            ("(S6) a 130 KB paste (more than one 64 KiB frame) reaches cat unchanged", check_large_paste),
            ("a typed canary line runs (for the log and history checks below)", canary_check),
        ])
        share_attach(results, se_binary, env, logs, fixture, owner)
        share_cleanup(results, owner)
        share_second_sessions(results, se_binary, env, logs, fixture)
        share_no_terminal_content(results, b, fixture, canary, bool(canary_ran))
        share_plain_exec(results, se_binary, fixture, logs)
        share_update_stop(results, se_binary, env, logs, fixture)
        share_revoke(results, se_binary, env, logs, fixture)
    finally:
        if owner is not None:
            owner.close()


def share_attach(results, se_binary, env, logs, fixture, owner):
    stage = "linux-share"
    a = fixture.a

    def attach_both():
        ensure(owner is not None, "no owner session")
        hint = owner.text(0)
        ensure("can be joined with: se shell --attach tt" in hint, "the owner printed no `--attach tt` hint")
        since_owner = owner.mark()
        joined = UnixPty([se_binary, "shell", "--attach", "tt"], env, logs, "attached")
        try:
            joined.wait(r"attached to session tt", 0, 90, what="the attach line")
            owner.wait(r"attached", since_owner, 30, what="the owner's notice that a terminal attached")
            ensure(joined.raw_mode(), "the attached terminal is not in raw mode")
            time.sleep(0.5)
            since_joined = joined.mark()
            since_owner = owner.mark()
            joined.send("echo from-b-$((20+22))\r")
            joined.wait(out_line("from-b-42"), since_joined, 60, what="from-b-42 at the attached terminal")
            owner.wait(out_line("from-b-42"), since_owner, 60, what="from-b-42 at the owner")
            since_owner = owner.mark()
            joined.send("~.")
            code = joined.exit_code(60)
            ensure(code == 0, f"the attached terminal's exit code {code}")
            ensure("detached from the session" in joined.text(), joined.text()[-300:])
            ensure(joined.restored(), "the attached terminal's ICANON and ECHO were not restored")
            owner.wait(r"detached", since_owner, 30, what="the owner's notice that the terminal detached")
            since_owner = owner.mark()
            owner.send("echo a-after-$((40+2))\r")
            owner.wait(out_line("a-after-42"), since_owner, 30, what="the owner works after the detach")
        finally:
            joined.close()
    results.check(stage, "a second terminal attaches: both see and type, the owner sees `attached`, ~. detaches with 0, "
                         "the owner continues", attach_both)

    def attach_without_terminal():
        refused = a.se(se_binary, "shell", "--attach", "tt", input_text="true\n", log=logs / "cli.log")
        ensure(refused.returncode == 125 and b"terminal session" in refused.stderr, (refused.returncode, refused.stderr))
    results.check(stage, "`se shell --attach tt` on pipes fails with `terminal session` and exit 125", attach_without_terminal)


def share_cleanup(results, owner):
    stage = "linux-share"

    def leave_with_background_job():
        ensure(owner is not None, "no owner session")
        owner.send("sleep 61 &\r")
        sync(owner)
        found = subprocess.run(["pgrep", "-f", "[s]leep 61"], stdout=subprocess.DEVNULL)
        ensure(found.returncode == 0, "the background sleep is not running on the host")
        since = owner.mark()
        owner.send("~.")
        code = owner.exit_code(60)
        ensure(code == 130, f"exit code {code}")
        ensure("session closed" in owner.text(since), owner.text(since)[-300:])
        ensure(owner.restored(), "ICANON and ECHO were not restored after ~.")
        ai.poll(lambda: subprocess.run(["pgrep", "-f", "[s]leep 61"], stdout=subprocess.DEVNULL).returncode == 1,
                15, "the background sleep of the ended session still runs on the host")
    results.check(stage, "(k) ~. of the owner: exit 130, terminal restored, the host's background job is gone within 15 s",
                  leave_with_background_job)


def share_second_sessions(results, se_binary, env, logs, fixture):
    stage = "linux-share"
    session = {}
    try:
        session["term"] = open_session(se_binary, env, fixture.shell_args(), logs, "tui")
        results.record(stage, "se shell <peer> opens a terminal session for the TUI programs", True)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "se shell <peer> opens a terminal session for the TUI programs", False,
                       f"{type(error).__name__}: {error}")
    term = session.get("term")
    try:
        run_checks(results, stage, term, [
            ("claude starts its terminal interface (CSI sequences, no raw-mode or tty error)",
             lambda t: check_tui(t, "claude")),
            ("codex starts its terminal interface (CSI sequences, no raw-mode or tty error)",
             lambda t: check_tui(t, "codex")),
            ("(j) exit 7 gives exit code 7 and restores ICANON and ECHO", check_exit_seven),
        ])
    finally:
        if term is not None:
            term.close()


def share_no_terminal_content(results, b, fixture, canary, canary_ran):
    stage = "linux-share"
    se_binary = fixture.se

    def no_content():
        # Without the canary line in a terminal session, finding no canary proves nothing.
        ensure(canary_ran, "the canary line never ran in the terminal session")
        history = b.se(se_binary, "share", "exec", "history", "--json")
        text = history.stdout.decode(errors="replace")
        ensure(history.returncode == 0, history.stderr.decode())
        ensure("<shell>" in text, "the host history does not show `<shell>` for the terminal session")
        ensure(canary not in text, "terminal content appears in the host's Exec history")
        needle = canary.encode()
        found = []
        for client in (fixture.a, b):
            found += file_contains(client.root, needle)
        found += file_contains(fixture.logs, needle)
        ensure(not found, f"terminal content found in files: {found[:5]}")
    results.check(stage, "(V13/S8) no terminal content in logs or files of either client and the server; the host "
                         "history shows <shell>", no_content)


def share_plain_exec(results, se_binary, fixture, logs):
    stage = "preserve"
    a = fixture.a

    def plain_exec():
        cli = a.se(se_binary, "exec", fixture.endpoint, "--", "echo", "cli-ok", log=logs / "cli.log", timeout=240)
        ensure(cli.returncode == 0 and cli.stdout == b"cli-ok\n", (cli.returncode, cli.stdout, cli.stderr))
        code = a.se(se_binary, "exec", fixture.endpoint, "--shell", "exit 3", log=logs / "cli.log", timeout=240)
        ensure(code.returncode == 3, code.returncode)
        session = ai.McpSession(se_binary, a, "initialize", logs, "share-plain")
        try:
            plain_result = ai.run_exec(session, {"target": fixture.endpoint, "command": "printf plain"}, timeout=240)
            ensure(plain_result["stdout"]["text"] == "plain" and "session" not in plain_result, plain_result)
            first = ai.run_exec(session, {"target": fixture.endpoint, "session": "p1",
                                          "command": "export S=share; cd /"}, timeout=240)
            ensure(first["exit_code"] == 0 and first["cwd"] == "/", first)
            second = ai.run_exec(session, {"session": "p1", "command": 'printf %s "$S"; pwd'})
            ensure(second["stdout"]["text"] == "share/\n", second)
        finally:
            session.close()
    results.check(stage, "(V6/V7) `se exec <peer>`, MCP `exec` and an agent's command session on a Share peer are unchanged",
                  plain_exec)


def share_update_stop(results, se_binary, env, logs, fixture):
    stage = "linux-share"
    a = fixture.a
    marker = a.app_data / "sync" / "daemon.stop"

    def update_stop():
        term = open_session(se_binary, env, fixture.shell_args(), logs, "update-stop")
        try:
            ensure(term.raw_mode(), "not in raw mode before the update request")
            since = term.mark()
            marker.parent.mkdir(parents=True, exist_ok=True)
            marker.write_text("stop")
            code = term.exit_code(60)
            ensure(code == 125, f"exit code {code}")
            ensure("being updated" in term.text(since), term.text(since)[-300:])
            ensure(term.restored(), "ICANON and ECHO were not restored after the update stop")
        finally:
            marker.unlink(missing_ok=True)
            term.close()
    results.check(stage, "(V10/S5) an update stop ends the terminal session: notice, exit code 125, terminal restored",
                  update_stop)
    settle_worker(fixture, se_binary, logs)


def settle_worker(fixture, se_binary, logs, timeout=90):
    """The update request also stops the client's Share worker; the next use starts it again.
    Waits (best effort, no verdict) until it is connected so that later checks start clean."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        status = fixture.a.se(se_binary, "share", "status", "--json", log=logs / "cli.log")
        try:
            worker = json.loads(status.stdout)["worker"]
            if worker["reachable"] and worker["running"] and worker["connected"]:
                return True
        except (ValueError, KeyError, TypeError):
            pass
        time.sleep(2)
    return False


def share_revoke(results, se_binary, env, logs, fixture):
    stage = "linux-share"
    b = fixture.b

    def revoke():
        term = open_session(se_binary, env, fixture.shell_args(), logs, "revoked")
        try:
            since = term.mark()
            disabled = b.se(se_binary, "share", "grants", "exec", "disable", "--json", log=logs / "cli.log")
            ensure(disabled.returncode == 0 and json.loads(disabled.stdout).get("persisted"), disabled.stdout)
            code = term.exit_code(60)
            ensure(code == 125, f"exit code {code}: {plain(term.text(since))[-300:]!r}")
            ensure(term.restored(), "ICANON and ECHO were not restored after the grant was removed")
        finally:
            term.close()
    results.check(stage, "(V8/S1) removing the peer's Exec grant ends the terminal session within 60 s: exit 125, "
                         "terminal restored", revoke)


# --------------------------------------------------------------------------- Windows

def powershell_pids():
    script = ("Get-Process -Name powershell -ErrorAction SilentlyContinue | "
              "Where-Object { $_.Id -ne $PID } | ForEach-Object { $_.Id }")
    listed = ai.run(["powershell", "-NoProfile", "-Command", script], timeout=90)
    return {int(token) for token in listed.stdout.decode().split() if token.isdigit()}


def stage_windows(results, se_binary, work, log_root, fixture):
    stage = "windows"
    logs = log_root / "windows"
    logs.mkdir(parents=True, exist_ok=True)
    if not fixture.start():
        results.record(stage, "Share fixture (paired clients, Exec granted)", False, "the fixture is not available")
        return
    env = tty_env(fixture.a)
    term = None
    try:
        term = open_session(se_binary, env, fixture.shell_args(), logs, "powershell", "powershell", rows=30, cols=100)
        results.record(stage, "se.exe shell <peer> under pywinpty shows the PowerShell prompt (PS)", True)
    except Exception as error:  # noqa: BLE001
        results.record(stage, "se.exe shell <peer> under pywinpty shows the PowerShell prompt (PS)", False,
                       f"{type(error).__name__}: {error}")

    def windows_tab(t):
        since = sync(t)
        t.send("Write-Outp\t")
        t.wait("Write-Output", since, 40, what="Tab completion of Write-Outp")
        t.send(" ('tab'+'ok')\r")
        t.wait("tabok", since, 40, what="the output of the completed command")
        ensure("is not recognized" not in plain(t.text(since)), plain(t.text(since))[-300:])

    def windows_ctrl_c(t):
        since = sync(t)
        t.send("Start-Sleep 100\r")
        time.sleep(3)
        t.send("\x03")
        time.sleep(1.5)
        since = t.mark()
        t.send("Write-Output ('alive-' + (1+1))\r")
        t.wait("alive-2", since, 30, what="alive-2 after Ctrl+C")

    def windows_size(t):
        since = sync(t)
        t.send('"w1=$($Host.UI.RawUI.WindowSize.Width);"\r')
        t.wait("w1=100;", since, 30, what="window width 100")
        t.resize(40, 120)
        for attempt in "abcdef":
            time.sleep(2)
            since = t.mark()
            t.send('"w2%s=$($Host.UI.RawUI.WindowSize.Width);"\r' % attempt)
            try:
                t.wait(f"w2{attempt}=120;", since, 8, what="window width 120")
                return f"width 120 on try {attempt}"
            except AssertionError:
                continue
        raise AssertionError("the window width never became 120")

    def windows_exit(t):
        sync(t)
        t.send("exit 7\r")
        code = t.exit_code(90)
        ensure(code == 7, f"exit status {code}")

    try:
        run_checks(results, stage, term, [
            ("Tab completes Write-Outp and the command runs (no `is not recognized`)", windows_tab),
            ("Ctrl+C ends Start-Sleep and PowerShell answers", windows_ctrl_c),
            ("window width 100, then 120 after setwinsize(40, 120)", windows_size),
            ("exit 7 gives exit status 7", windows_exit),
        ])
    finally:
        if term is not None:
            term.close()

    def host_cleanup():
        before = powershell_pids()
        t = open_session(se_binary, env, fixture.shell_args(), logs, "cleanup", "powershell")
        try:
            started = powershell_pids() - before
            ensure(started, "no new powershell.exe process while the terminal session runs")
            t.send("~.")
            code = t.exit_code(90)
            ensure(code == 130, f"exit code {code}")
            ai.poll(lambda: not (powershell_pids() & started), 60,
                    f"the host's PowerShell processes {sorted(started)} survived ~.")
        finally:
            t.close()
    results.check(stage, "~. ends the session with 130 and the host's PowerShell process is gone within 60 s", host_cleanup)


# --------------------------------------------------------------------------- main

def compiler_errors(log_root):
    """The error lines of build.log (`--message-format short`: `path:line:col: error[code]: text`),
    so that a failed build names its errors in the job log, not only in the artifact."""
    log = log_root / "build.log"
    if not log.exists():
        return []
    text = log.read_text(encoding="utf-8", errors="replace").replace("\\", "/")
    return [line for line in text.splitlines() if re.search(r"(?:^|: )error(?:\[E\d{4}\])?: ", line)]


def guarded(results, stage, function, *arguments):
    """Runs a stage; a crash outside the individual checks is recorded as a failed check
    so that the later stages still run and the summary lists everything."""
    try:
        return function(*arguments)
    except Exception as error:  # noqa: BLE001 - recorded, never swallowed silently
        results.record(stage, "the stage ran to its end", False, f"{type(error).__name__}: {error}")
        return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    args = parser.parse_args()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = ai.Results(log_root)
    guarded(results, "fmt", stage_fmt, results)
    built = guarded(results, "build", ai.stage_build, results, log_root)
    if not (built and all(built)):
        errors = compiler_errors(log_root)
        if errors:
            results.record("build", "compiler errors from build.log", False, "\n".join(errors[:40]))
    built = bool(built and built[0])
    guarded(results, "unit", stage_unit, results, log_root)
    se_binary = ai.NATIVE / "target" / "debug" / ("se.exe" if WINDOWS else "se")
    if not built or not se_binary.exists():
        results.record("e2e", "development se binary", False, se_binary)
        return results.finish()
    work = Path(tempfile.mkdtemp(prefix="se-terminal-"))
    fixture = ShareFixture(results, se_binary, work, log_root)
    ssh_fixture = SshFixture()
    try:
        guarded(results, "mcp", stage_mcp, results, se_binary, work, log_root, fixture)
        if WINDOWS:
            guarded(results, "windows", stage_windows, results, se_binary, work, log_root, fixture)
        else:
            guarded(results, "linux-ssh", stage_linux_ssh, results, se_binary, work, log_root, ssh_fixture)
            guarded(results, "linux-share", stage_linux_share, results, se_binary, work, log_root, fixture)
            guarded(results, "preserve", stage_preserve, results, se_binary, ssh_fixture)
    finally:
        ssh_fixture.stop()
        fixture.stop()
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
