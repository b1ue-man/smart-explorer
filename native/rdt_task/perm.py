"""S1 (Exec-Rechte) in the batch suite: stages and suite checks (plan D3; strang-rechte.md D.1).

Stages (all library tests by exact full path; options and fixtures come from expected_signals.txt):

linux-core:perm-pair          PRM-M2, PRM-M3, PRM-M4 (``perm_task_pending_surface_cli_only_without_display``
                              without DISPLAY/WAYLAND_DISPLAY: fixture ``nodisplay``)
linux-root:perm-linux-root    PRM-M5 ``#[ignore]`` tests as root: ``sudo -E`` with
                              ``SMART_EXPLORER_PERM_ROOT_TASK=1``, ``SMART_EXPLORER_PERM_SE_BIN`` = absolute
                              debug ``se``, ``--include-ignored``, one test thread (handoff U18)
linux-root:perm-linux-helper  PRM-M6 without root: ``SMART_EXPLORER_PERM_HELPER_TASK=1``, the tests call
                              ``sudo -n`` and ``python3`` themselves (fixture ``sudo``); the Fedora/SELinux
                              case is ``not_run`` (no runner, assumption A5)
linux-2204:perm-2204          ``perm_task_exec_linux_ubuntu2204_starts`` with ``SMART_EXPLORER_PERM_2204_TASK=1``;
                              the stage names the runner's Ubuntu and systemd version
windows-core:perm-windows     PRM-M7 (U24) and the static/runtime Windows checks below
android-device(-api30)        ``PermTaskTest`` instrumentation (generic, common.Device)

Suite checks: ``perm_task_inventory_paths_exist`` (PRM-M11), ``perm_task_installer_uninstall_removes_service_static``
(C5, U36), ``perm_task_windows_cmd_start_line_answers_powershell_dialect`` (main agent, 2026-10-17),
and two checks that are always ``not_run`` because no runner offers the platform
(``perm_task_linux_helper_fedora_selinux``, ``perm_task_windows_home_and_microsoft_accounts``).
"""
from __future__ import annotations

import re
from pathlib import Path

from . import common
from .common import FAILED, NATIVE, NOT_RUN, PASSED, REPO, WINDOWS

SHARE_EXEC_INVENTORY = REPO / "docs" / "oberflaechen" / "share-exec.md"
INVENTORY_INDEX = REPO / "docs" / "oberflaechen" / "INDEX.md"
TERMINAL_TRANSPORTS = NATIVE / "src" / "cli" / "shell_session" / "terminal_transports.rs"
START_LINE_TEST = "fn shell_session_task_share_session_needs_one_start_line"
RUST_STRING = r'"((?:[^"\\]|\\.)*)"'
DIALECT_MARKER = "SE_DIALECT_x_"


def stage_perm_2204(ctx: common.JobContext, stage: str) -> None:
    """Runs the stage and names the runner's Ubuntu and systemd versions (assumption on A10, C2)."""
    version = ""
    os_release = Path("/etc/os-release")
    if os_release.is_file():
        match = re.search(r'^VERSION_ID="?([^"\n]+)"?', os_release.read_text(encoding="utf-8"), re.M)
        version = match.group(1) if match else ""
    systemd = ctx.run(["systemctl", "--version"], stage=stage, timeout=60)
    note = f"Runner: Ubuntu {version or 'unbekannt'}; {common.first_line(systemd.output) or 'systemctl fehlt'}"
    if version != "22.04":
        note += " (erwartet Ubuntu 22.04 mit systemd 249)"
    ctx.state.setdefault("stage_notes", {})[stage] = note
    ctx.run_default(stage)


# --------------------------------------------------------------------------- static checks

def check_inventory_paths_exist(ctx: common.JobContext) -> tuple[str, str]:
    """PRM-M11: share-exec.md has the section of this batch, every Datei:Zeile reference
    resolves, and INDEX.md defines the cost classes (strang-rechte.md PRM-M11, C6)."""
    if not SHARE_EXEC_INVENTORY.is_file():
        return FAILED, "docs/oberflaechen/share-exec.md fehlt"
    text = SHARE_EXEC_INVENTORY.read_text(encoding="utf-8", errors="replace")
    problems = []
    if not re.search(r"Stand nach (dem )?Stapel Rechte", text):
        problems.append("Abschnitt 'Stand nach dem Stapel Rechte (v0.5.183)' fehlt (C6, PRM-M11; Paket U42)")
    count, unresolved = common.Citations().check_text(text)
    if not count:
        problems.append("keine Belege der Form Datei:Zeile")
    problems.extend(unresolved)
    index = INVENTORY_INDEX.read_text(encoding="utf-8", errors="replace") if INVENTORY_INDEX.is_file() else ""
    if not re.search(r"Kostenklasse", index):
        problems.append("docs/oberflaechen/INDEX.md definiert keine Kostenklassen C1-C3 (PRM-M11)")
    if problems:
        return FAILED, f"{len(problems)} Befunde:\n" + "\n".join(problems[:60])
    return PASSED, f"{count} Belege in share-exec.md aufgelöst, Abschnitt des Stapels und Kostenklassen vorhanden"


def check_installer_uninstall_removes_service(ctx: common.JobContext) -> tuple[str, str]:
    """C5 row PRM-M7: the uninstaller removes the Windows account service and its copy with a UAC
    prompt (``se.exe share exec accounts remove`` elevated) and prints service name and folder
    when the prompt is refused."""
    if not common.INSTALLER.is_file():
        return FAILED, "native/installer.nsi fehlt"
    uninstall = common.nsis_uninstall_lines(common.INSTALLER.read_text(encoding="utf-8", errors="replace"))
    calls = [index for index, (_, line) in enumerate(uninstall)
             if "share exec accounts remove" in line.lower() and "se.exe" in line.lower()]
    if not calls:
        return FAILED, ('die Deinstallation ruft "se.exe share exec accounts remove" nicht auf '
                        "(C5, PRM-M7; Paket U36)")
    index = calls[0]
    window = uninstall[max(0, index - 12):index + 25]
    problems = []
    if not any(re.search(r"\brunas\b|\buac\b", line, re.I) for _, line in window):
        problems.append("keine Erhöhung (runas bzw. UAC) beim Aufruf")
    if not any(line.lower().startswith("detailprint") for _, line in uninstall[index + 1:index + 25]):
        problems.append("kein DetailPrint mit Dienstname und Ordner nach dem Aufruf (Fall: Abfrage abgelehnt)")
    if problems:
        return FAILED, f"installer.nsi:{uninstall[index][0]}: " + "; ".join(problems) + " (Paket U36)"
    return PASSED, (f"installer.nsi:{uninstall[index][0]} entfernt den Kontodienst erhöht und meldet den Fall "
                    "der Ablehnung; ob se-remote nur auf ausdrückliche Wahl entfernt wird, prüft dieser "
                    "statische Test nicht")


# --------------------------------------------------------------------------- runtime checks

def start_line_from_test() -> str | None:
    """The start line of ``se shell -T`` exactly as the Rust test
    ``shell_session_task_share_session_needs_one_start_line`` pins it (cmd part + "'; " + POSIX part)."""
    if not TERMINAL_TRANSPORTS.is_file():
        return None
    text = TERMINAL_TRANSPORTS.read_text(encoding="utf-8", errors="replace")
    start = text.find(START_LINE_TEST)
    if start < 0:
        return None
    body = text[start:]
    cmd = re.search(r"assert_eq!\(\s*cmd,\s*" + RUST_STRING, body, re.S)
    posix = re.search(r"assert_eq!\(\s*posix,\s*" + RUST_STRING, body, re.S)
    if not cmd or not posix:
        return None

    def unescape(value: str) -> str:
        return re.sub(r"\\(.)", lambda match: {"n": "\n", "t": "\t", "r": "\r"}.get(match.group(1), match.group(1)),
                      value)

    return unescape(cmd.group(1)) + "'; " + unescape(posix.group(1))


def check_windows_cmd_start_line(ctx: common.JobContext) -> tuple[str, str]:
    """windows-core: the start line runs under ``cmd /D /S /C``; PowerShell reads the input
    ``echo SE_DIALECT_x_$?`` and answers ``SE_DIALECT_x_True`` (proposal of the main agent)."""
    if not WINDOWS:
        return NOT_RUN, "nur auf dem Windows-Runner"
    line = start_line_from_test()
    if line is None:
        return NOT_RUN, ("nicht umgesetzt: die Startzeile ist nicht aus dem Test "
                         "shell_session_task_share_session_needs_one_start_line lesbar")
    completed = ctx.run(["cmd", "/D", "/S", "/C", line], stage="perm-windows", env=ctx.base_env(),
                        input_text=f"echo {DIALECT_MARKER}$?\n", timeout=180)
    if f"{DIALECT_MARKER}True" in completed.output:
        return PASSED, f"cmd /D /S /C mit der Startzeile antwortet {DIALECT_MARKER}True (PowerShell)"
    return FAILED, (f"keine Antwort {DIALECT_MARKER}True ({completed.describe()}); Startzeile: {line}\n"
                    + common.excerpt(completed.output, 1500))


def check_linux_helper_fedora_selinux(ctx: common.JobContext) -> tuple[str, str]:
    return NOT_RUN, ("kein Fedora-Runner: dass init_t die Programmkopie /usr/local/sbin/smart-explorer-exec "
                     "unter SELinux ausführen darf (Annahme A5 aus S1), bleibt ungeprüft")


def check_windows_home_and_microsoft_accounts(ctx: common.JobContext) -> tuple[str, str]:
    return NOT_RUN, ("der Runner windows-2025 ist Windows Server mit lokalen Konten: Windows 11 Home sowie "
                     "Konten mit Microsoft- oder Entra-Anmeldung (Annahmen A6, A7 aus S1) bleiben ungeprüft")


STAGES = {"perm-2204": stage_perm_2204}
CHECKS = {
    "perm_task_inventory_paths_exist": check_inventory_paths_exist,
    "perm_task_installer_uninstall_removes_service_static": check_installer_uninstall_removes_service,
    "perm_task_windows_cmd_start_line_answers_powershell_dialect": check_windows_cmd_start_line,
    "perm_task_linux_helper_fedora_selinux": check_linux_helper_fedora_selinux,
    "perm_task_windows_home_and_microsoft_accounts": check_windows_home_and_microsoft_accounts,
}
