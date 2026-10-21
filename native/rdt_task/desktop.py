"""S5 (se Desktop, Windows and Linux) in the batch suite: stages, fixture and suite checks (plan.md D3;
strang-desktop.md DSK-M15; package U41).

Stages (``STAGES``):
  desktop-linux-x11      linux-desktop: DSK-M3 process checks (headless surfaces), DSK-M6 X11 checks (the tests
                         start Xvfb, openbox, xfwm4/xfdesktop themselves), last ``desktop_task_no_hang_restart_during_suite``.
  desktop-linux-wayland  linux-wayland: DSK-M7 layer checks (the tests start Sway themselves), ``readelf`` and the
                         lock comparison with ``da2646d2`` (needs the full Git history in the checkout).
  desktop-gnome          linux-gnome: variant A on the runner's GNOME Shell 46, variant B in ``fedora:43``
                         (``native/desktop-task-linux.py``), the private session bus check.
  desktop-gui-harness    windows-desktop: GUI harness checks (``SMART_EXPLORER_GUI_TASK``) and the feed checks.
  desktop-windows        windows-desktop: pre-check ``FindWindowW("Progman")`` (reported as its own entry), the
                         Explorer-bound check only with Progman (A7), DSK-M3 process checks, DSK-M5 window and
                         registry checks, last ``desktop_task_no_hang_restart_during_suite``.
  kde                    shared with ``screen.py`` (its function runs the S5 entry there).

A test that ends with the panic "nicht ausgeführt: <Grund>" is reported as not run (``screen.run_entries``).

Fixture ``session-bus``: ``DBUS_SESSION_BUS_ADDRESS`` of a private ``dbus-daemon --session`` and
``SE_DESKTOP_TASK_SESSION_BUS=1`` (Linux).

``desktop_task_no_hang_restart_during_suite`` (assumption A45): while the window and process checks of the
stage run, a watcher reads every ``shell.log`` below the stage's group folders (the entries set ``TMPDIR``,
``TMP`` and ``TEMP`` to ``{tmp}``, so the tests' temporary app data lies there). The check passes when at least
one shell log was seen and none has a ``surface_hung`` line. ``desktop_task_supervisor_kills_hung_surface``
provokes a hang on purpose and therefore runs without these variables, outside the watched folders.

Suite checks (``CHECKS``): ``desktop_task_installer_static``, ``desktop_task_install_linux_static``,
``desktop_task_docs_todo_entries_present`` (prepare: static), ``desktop_task_docs_inventory_lists_every_cli_command``
(linux-core: docs-cli), ``desktop_task_no_hang_restart_during_suite``, ``desktop_task_windows_progman_precheck``.
"""
from __future__ import annotations

import os
import re
import shutil
import threading
import time
from pathlib import Path

from . import common, screen
from .common import FAILED, LINUX, NATIVE, NOT_RUN, PASSED, REPO

NO_HANG = "desktop_task_no_hang_restart_during_suite"
PROGMAN = "desktop_task_windows_progman_precheck"
VARIANT_B = "desktop_task_gnome_headless_variant_b"
DESKTOP_DOC = REPO / "docs" / "oberflaechen" / "desktop-launcher.md"
TODO_FILE = REPO / "docs" / "TODO.md"
INSTALL_LINUX = REPO / "install-linux.sh"
# strang-desktop.md C7: the eight TODO ids of S5.
TODO_IDS = ("PIN-TASKBAR", "DESKTOP-EMBED", "DESKTOP-LAYER-IME", "DESKTOP-WAYLAND-DND", "DESKTOP-LINUX-DRAGOUT",
            "DESKTOP-GNOME-LEGACY", "DESKTOP-GNOME-XML-WALLPAPER", "LINUX-UNINSTALL")
SHELL_LOGS = ("shell.log", "shell.log.1")
HUNG = "\tsurface_hung\t"


# --------------------------------------------------------------------------- shell log watcher (A45)

class ShellLogWatcher:
    def __init__(self, ctx: common.JobContext, stage: str):
        self.base = ctx.work / "groups"
        self.stage = stage
        self.offsets: dict[Path, int] = {}
        self.lines = 0
        self.hung: list[str] = []
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def scan(self) -> None:
        if not self.base.is_dir():
            return
        for group in self.base.glob(f"{self.stage}-*"):
            for directory, _, files in os.walk(group):
                for name in files:
                    if name not in SHELL_LOGS:
                        continue
                    path = Path(directory) / name
                    try:
                        with open(path, "rb") as handle:
                            handle.seek(self.offsets.get(path, 0))
                            data = handle.read()
                            self.offsets[path] = handle.tell()
                    except OSError:
                        continue
                    for line in data.decode("utf-8", "replace").splitlines():
                        self.lines += 1
                        if HUNG in line:
                            self.hung.append(f"{path}: {line.strip()}")

    def loop(self) -> None:
        while not self.stop_event.wait(0.25):
            try:
                self.scan()
            except OSError:
                continue

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> dict:
        self.stop_event.set()
        self.thread.join(timeout=10)
        self.scan()
        return {"files": len(self.offsets), "lines": self.lines, "hung": list(self.hung)}


def watched(ctx: common.JobContext, stage: str, entries: list, extra_env: dict | None = None) -> None:
    watcher = ShellLogWatcher(ctx, stage)
    watcher.start()
    try:
        screen.run_entries(ctx, stage, entries, extra_env)
    finally:
        ctx.state.setdefault("shell_watch", {})[stage] = watcher.stop()


# --------------------------------------------------------------------------- fixture

def fixture_session_bus(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "nur unter Linux"
    session = screen.helper("screen-task-linux").start_session_bus(fixtures.make_dir("session-bus"),
                                                                   fixtures.ctx.base_env())
    screen.add_session(fixtures, session)
    if session.ok:
        fixtures.provide(name, env=dict(session.env, SE_DESKTOP_TASK_SESSION_BUS="1"))
    return session.ok, session.reason


# --------------------------------------------------------------------------- stages

def stage_desktop_linux_x11(ctx: common.JobContext, stage: str) -> None:
    entries = ctx.entries(stage)
    last = screen.by_name(entries, NO_HANG)
    watched(ctx, stage, [entry for entry in entries if entry is not last])
    if last is not None:
        ctx.run_check(stage, last)


def stage_desktop_linux_wayland(ctx: common.JobContext, stage: str) -> None:
    screen.run_entries(ctx, stage, ctx.entries(stage))


def run_variant_b(ctx: common.JobContext, stage: str, entry) -> None:
    """GNOME Shell 49 in ``fedora:43`` (A18, A19)."""
    started = time.monotonic()
    if not LINUX:
        ctx.record(entry.name, stage, NOT_RUN, "nur auf dem Linux-Runner")
        return
    if not ctx.cargo.build_lib():
        ctx.record(entry.name, stage, FAILED, "Bau der Bibliothekstests gescheitert: " + ctx.cargo.lib_error)
        return
    paths = ctx.cargo.by_leaf.get(entry.name, [])
    if len(paths) != 1:
        status = NOT_RUN if (not paths and entry.pending) else FAILED
        ctx.record(entry.name, stage, status, entry.pending_reason() if status == NOT_RUN else
                   "fehlt im Testprogramm dieser Plattform")
        return
    gui = ctx.cargo.bin_path("smart_explorer")
    if gui is None:
        ctx.record(entry.name, stage, FAILED, ctx.cargo.bin_errors.get("smart_explorer", "Bau von smart_explorer gescheitert"))
        return
    if not shutil.which("docker"):
        ctx.record(entry.name, stage, NOT_RUN, "docker fehlt im Runner: Container fedora:43 nicht startbar (A18)")
        return
    tool = screen.helper("desktop-task-linux")
    exe = Path(ctx.cargo.test_exe)
    env = {"SE_DESKTOP_TASK_GNOME": "1", "SE_DESKTOP_TASK_GUI": str(gui), "RUST_BACKTRACE": "1",
           "CARGO_MANIFEST_DIR": str(NATIVE), "CARGO_PKG_NAME": "smart_explorer", "CARGO_CRATE_NAME": "smart_explorer"}
    command = tool.fedora_command(exe, paths, env, [REPO, exe.parent, gui.parent], NATIVE)
    parser = common.LibtestParser(time.monotonic())
    completed = ctx.run(command, stage=stage, timeout=100 * 60, on_line=parser.feed)
    result = parser.results.get(paths[0])
    if result is None:
        text = completed.output
        match = screen.NOT_RUN_PANIC.search(text)
        if match or "error while loading shared libraries" in text or completed.code == 125:
            reason = (f"nicht ausgeführt: {match.group(1).strip()}" if match
                      else "Container fedora:43 oder das Testprogramm darin startete nicht: " + common.excerpt(text, 600))
            ctx.record(entry.name, stage, NOT_RUN, reason, time.monotonic() - started)
        else:
            ctx.record(entry.name, stage, FAILED, f"kein Ergebnis ({completed.describe()}): " + common.excerpt(text, 1200),
                       time.monotonic() - started)
        return
    status, reason, _ = result
    if status == FAILED:
        block = {match.group(1): match.group(2) for match in common.FAILURE_BLOCK.finditer(completed.output)}
        reason = block.get(paths[0]) or common.excerpt(completed.output, 1200)
        match = screen.NOT_RUN_PANIC.search(reason)
        if match:
            status, reason = NOT_RUN, f"vom Test gemeldet: nicht ausgeführt: {match.group(1).strip()}"
    ctx.record(entry.name, stage, status, reason or f"{paths[0]} in {tool.FEDORA_IMAGE}", time.monotonic() - started)


def stage_desktop_gnome(ctx: common.JobContext, stage: str) -> None:
    entries = ctx.entries(stage)
    variant_b = screen.by_name(entries, VARIANT_B)
    screen.run_entries(ctx, stage, [entry for entry in entries if entry is not variant_b])
    if variant_b is not None and not ctx.recorded(variant_b.name):
        run_variant_b(ctx, stage, variant_b)


def stage_desktop_gui_harness(ctx: common.JobContext, stage: str) -> None:
    screen.run_entries(ctx, stage, ctx.entries(stage))


def stage_desktop_windows(ctx: common.JobContext, stage: str) -> None:
    entries = ctx.entries(stage)
    precheck = screen.by_name(entries, PROGMAN)
    last = screen.by_name(entries, NO_HANG)
    if precheck is not None:
        ctx.run_check(stage, precheck)
    tool = screen.helper("desktop-task-windows")
    present, reason = tool.progman()
    ctx.state.setdefault("stage_notes", {})[stage] = reason
    rest = [entry for entry in entries if entry not in (precheck, last)]
    if not present:
        explorer = [entry for entry in rest if entry.name in tool.EXPLORER_CHECKS]
        ctx.not_run_all(stage, f"Prüfung mit Explorer-Bezug: {reason}", explorer)
        rest = [entry for entry in rest if entry not in explorer]
    watched(ctx, stage, rest)
    if last is not None:
        ctx.run_check(stage, last)


# --------------------------------------------------------------------------- checks

def check_no_hang(ctx: common.JobContext) -> tuple[str, str]:
    """A45: no surface restarted for a missing heartbeat while the stage's window checks ran."""
    stage = ctx.current_stage or ""
    summary = ctx.state.get("shell_watch", {}).get(stage)
    if summary is None:
        return NOT_RUN, f"die Fensterprüfungen der Stufe {stage} liefen nicht (keine Beobachtung der Wächterprotokolle)"
    if summary["hung"]:
        return FAILED, f"{len(summary['hung'])} Neustarts wegen fehlender Lebenszeichen:\n" + "\n".join(summary["hung"][:30])
    if not summary["files"]:
        return NOT_RUN, ("kein Wächterprotokoll (shell.log) während der Fensterprüfungen beobachtet: die Prüfungen "
                         "liefen nicht oder legten ihre App-Daten nicht unter TMPDIR/TMP/TEMP der Stufe ab")
    return PASSED, (f"{summary['files']} Wächterprotokolle mit {summary['lines']} Zeilen gelesen: kein surface_hung "
                    "(ohne die absichtlich hängende Fläche aus desktop_task_supervisor_kills_hung_surface)")


def check_progman(ctx: common.JobContext) -> tuple[str, str]:
    present, reason = screen.helper("desktop-task-windows").progman()
    return (PASSED if present else NOT_RUN), reason


def nsis_lines(text: str) -> list[tuple[int, str]]:
    return [(number, line.strip()) for number, line in enumerate(text.splitlines(), 1)]


def section_range(lines: list[tuple[int, str]], pattern: str) -> tuple[int, int] | None:
    start = next((index for index, (_, line) in enumerate(lines) if re.match(pattern, line, re.I)), None)
    if start is None:
        return None
    end = next((index for index in range(start + 1, len(lines))
                if lines[index][1].lower().startswith(("sectionend", "functionend"))), len(lines) - 1)
    return start, end


def check_installer_static(ctx: common.JobContext) -> tuple[str, str]:
    """DSK-M3/C5 and F5: optional section with ``desktop enable --json``, ``/DESKTOP=1``, ``desktop restart`` in
    SEC_MAIN, ``desktop remove`` before ``--unregister`` without Abort; the unchanged lines of F5."""
    if not common.INSTALLER.is_file():
        return FAILED, "native/installer.nsi fehlt"
    lines = nsis_lines(common.INSTALLER.read_text(encoding="utf-8", errors="replace"))
    normal = [(number, common.nsis_normal(line)) for number, line in lines]
    problems = []
    main = section_range(lines, r'^Section\s+"[^"]*"\s+SEC_MAIN\b')
    desktop = section_range(lines, r'^Section\s+/o\s+"se Desktop \(optional\)"\s+SEC_DESKTOP\b')
    if main is None:
        problems.append("Section SEC_MAIN fehlt")
    if desktop is None:
        problems.append('Section /o "se Desktop (optional)" SEC_DESKTOP fehlt')
    if main and desktop and desktop[0] < main[1]:
        problems.append("SEC_DESKTOP steht nicht nach SEC_MAIN")
    if desktop and not any("\\se.exe desktop enable --json" in line for _, line in normal[desktop[0]:desktop[1] + 1]
                           if line.startswith("execwait")):
        problems.append("SEC_DESKTOP ruft ExecWait se.exe desktop enable --json nicht auf")
    if main:
        body = normal[main[0]:main[1] + 1]
        if not any(line.startswith("execwait") and "\\se.exe desktop restart" in line for _, line in body):
            problems.append("SEC_MAIN ruft ExecWait se.exe desktop restart nicht auf")
        if not any(line.startswith("nsexec::exec") and "taskkill" in line for _, line in body):
            problems.append("F5: taskkill in SEC_MAIN fehlt")
    oninit = section_range(lines, r"^Function\s+\.onInit\b")
    # nsis_normal turns "/" into "\\": the switch /DESKTOP= reads \\desktop= here.
    if oninit is None or not any("\\desktop=" in line for _, line in normal[oninit[0]:oninit[1] + 1]):
        problems.append("/DESKTOP=1 wird in .onInit nicht gelesen")
    elif not any("selectsection ${sec_desktop}" in line for _, line in normal[oninit[0]:oninit[1] + 1]):
        problems.append("/DESKTOP=1 wählt SEC_DESKTOP nicht aus")
    if oninit is None or not any("\\installdokany=" in line for _, line in normal[oninit[0]:oninit[1] + 1]):
        problems.append("F5: /INSTALLDOKANY=1 in .onInit fehlt")
    if not any(line.lower().startswith("function .oninstsuccess") for _, line in lines):
        problems.append("F5: Function .onInstSuccess fehlt")
    if not any(line.lower().startswith("requestexecutionlevel user") for _, line in lines):
        problems.append("F5: RequestExecutionLevel user fehlt")
    uninstall = section_range(lines, r'^Section\s+"Uninstall"')
    if uninstall is None:
        problems.append("Section Uninstall fehlt")
    else:
        body = normal[uninstall[0]:uninstall[1] + 1]
        index = {key: next((i for i, (_, line) in enumerate(body)
                            if not line.startswith((";", "#")) and marker(line)), None) for key, marker in (
            ("taskkill", lambda line: "taskkill" in line),
            ("remove", lambda line: line.startswith("execwait") and "\\se.exe desktop remove" in line),
            ("unregister", lambda line: "--unregister" in line),
            ("uplink", lambda line: "--lan-uplink-cleanup" in line),
            ("cli_path", lambda line: "--uninstall-cli-path" in line))}
        if index["remove"] is None:
            problems.append("Deinstallation ruft ExecWait se.exe desktop remove nicht auf")
        elif index["unregister"] is None or not (index["taskkill"] or 0) < index["remove"] < index["unregister"]:
            problems.append("desktop remove steht nicht nach taskkill und vor --unregister")
        elif any(line.startswith("abort") for _, line in body[index["remove"]:index["unregister"]]):
            problems.append("zwischen desktop remove und --unregister steht ein Abort")
        for key, label in (("uplink", "--lan-uplink-cleanup"), ("cli_path", "--uninstall-cli-path")):
            position = index[key]
            if position is None:
                problems.append(f"F5: {label} fehlt in der Deinstallation")
            elif not any(line.startswith("abort") for _, line in body[position:position + 8]):
                problems.append(f"F5: kein Abort nach {label}")
        if index["uplink"] is not None and index["cli_path"] is not None and index["uplink"] > index["cli_path"]:
            problems.append("F5: Reihenfolge --lan-uplink-cleanup vor --uninstall-cli-path geändert")
    if problems:
        return FAILED, "; ".join(problems) + " (C5, DSK-M3, F5; Paket U36)"
    return PASSED, ("SEC_DESKTOP (/o) mit desktop enable --json, /DESKTOP=1, desktop restart in SEC_MAIN, desktop remove "
                    "vor --unregister ohne Abort; F5-Zeilen unverändert vorhanden")


def check_install_linux_static(ctx: common.JobContext) -> tuple[str, str]:
    """DSK-M3/C5: ``--with-desktop`` (usage and option), refusal together with ``--cli-only``, ``desktop enable``
    only with ``--with-desktop``, ``desktop restart`` for every desktop app install."""
    if not INSTALL_LINUX.is_file():
        return FAILED, "install-linux.sh fehlt"
    script = INSTALL_LINUX.read_text(encoding="utf-8", errors="replace")
    problems = []
    usage = re.search(r"usage:[^\n]*--with-desktop", script)
    if not usage:
        problems.append("die Usage-Zeile nennt --with-desktop nicht")
    if not re.search(r"--with-desktop\)\s*WITH_DESKTOP=1", script):
        problems.append("Option --with-desktop setzt WITH_DESKTOP=1 nicht")
    if not re.search(r'"\$CLI_BIN" desktop restart', script):
        problems.append("kein Aufruf \"$CLI_BIN\" desktop restart")
    enable = re.search(r'if \[ "\$WITH_DESKTOP" = 1 \];.*?"\$CLI_BIN" desktop enable', script, re.S)
    if not enable:
        problems.append("desktop enable läuft nicht nur mit --with-desktop")
    if LINUX:
        home = ctx.work / "install-linux-desktop" / "home"
        home.mkdir(parents=True, exist_ok=True)
        env = dict(ctx.base_env(), HOME=str(home), XDG_DATA_HOME=str(home.parent / "data"),
                   XDG_CONFIG_HOME=str(home.parent / "config"), XDG_BIN_HOME=str(home.parent / "bin"))
        refused = ctx.run(["bash", INSTALL_LINUX, "--dry-run", "--with-desktop", "--cli-only"], stage="static",
                          env=env, timeout=120)
        if refused.code != 2 or "--cli-only" not in refused.output:
            problems.append(f"--with-desktop --cli-only wird nicht mit Exitcode 2 und Hinweis abgelehnt ({refused.describe()})")
        written = [str(path.relative_to(home)) for path in home.rglob("*")]
        if written:
            problems.append("die Ablehnung schrieb in HOME: " + ", ".join(written[:10]))
    if problems:
        return FAILED, "; ".join(problems) + " (C5, DSK-M3; Paket U36)"
    return PASSED, ("--with-desktop in Usage und Optionen, Ablehnung zusammen mit --cli-only (Exitcode 2), desktop enable "
                    "nur mit --with-desktop, desktop restart bei jeder Installation der Desktop-App")


def check_docs_todo_entries(ctx: common.JobContext) -> tuple[str, str]:
    """DSK-M14: the eight TODO ids of strang-desktop.md C7 stand in docs/TODO.md."""
    if not TODO_FILE.is_file():
        return FAILED, "docs/TODO.md fehlt"
    text = TODO_FILE.read_text(encoding="utf-8", errors="replace")
    missing = [item for item in TODO_IDS if not re.search(rf"(?<![\w-]){re.escape(item)}(?![\w-])", text)]
    if missing:
        return FAILED, f"in docs/TODO.md fehlen {len(missing)} von 8 Kennungen (C7, DSK-M14; Paket U43): " + ", ".join(missing)
    return PASSED, "alle 8 Kennungen aus C7 stehen in docs/TODO.md"


def check_docs_inventory_desktop(ctx: common.JobContext) -> tuple[str, str]:
    """DSK-M14: ``se desktop --help`` against docs/oberflaechen/desktop-launcher.md."""
    return screen.cli_docs_check(ctx, "desktop", DESKTOP_DOC, "U43")


FIXTURES = {"session-bus": fixture_session_bus}
STAGES = {
    "desktop-linux-x11": stage_desktop_linux_x11,
    "desktop-linux-wayland": stage_desktop_linux_wayland,
    "desktop-gnome": stage_desktop_gnome,
    "desktop-gui-harness": stage_desktop_gui_harness,
    "desktop-windows": stage_desktop_windows,
}
CHECKS = {
    "desktop_task_installer_static": check_installer_static,
    "desktop_task_install_linux_static": check_install_linux_static,
    "desktop_task_docs_todo_entries_present": check_docs_todo_entries,
    "desktop_task_docs_inventory_lists_every_cli_command": check_docs_inventory_desktop,
    "desktop_task_no_hang_restart_during_suite": check_no_hang,
    "desktop_task_windows_progman_precheck": check_progman,
}
