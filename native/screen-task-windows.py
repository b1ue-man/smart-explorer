#!/usr/bin/env python3
"""Windows preconditions of the S4 stage ``windows-screen: screen-windows`` (plan.md D3; strang-bildschirm.md
SCR-M3, SCR-M4, SCR-M6, SCR-M12).

Used by ``native/rdt_task/screen.py`` and runnable by hand on the Windows runner
(``python native/screen-task-windows.py probe``). Runs only on the remote runner (AGENTS.md); it starts no
build. Every function answers ``(ok, reason)``; ``ok=False`` turns the entries that need it into
"nicht ausgeführt" with that reason, never into passed.

  interactive_desktop()   the job runs in an interactive session with an input desktop (WGC, SendInput,
                          UI Automation need it; NR DN21).
  media_foundation(run)   the H.264 coders of Media Foundation exist; on Windows Server the feature
                          ``Server-Media-Foundation`` is installed once without a restart (S4 K6). Without
                          them ``screen_task_mf_encode_decode`` is not run.
  elevated_window()       a window of higher integrity beside a medium-integrity test process. A process
                          that is itself elevated (the GitHub runner account) cannot provide that, so the
                          answer is then "not available" with that reason (NR N7).
"""
from __future__ import annotations

import ctypes
import json
import os
import subprocess
import sys
from pathlib import Path

DESKTOP_READOBJECTS = 0x0001
SM_REMOTESESSION = 0x1000
SYSTEM32 = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32"
# H.264 encoder (MFT) and decoder of Media Foundation, plus its platform library.
MF_FILES = ("mfplat.dll", "mfh264enc.dll", "msmpeg2vdec.dll")


def is_windows() -> bool:
    return os.name == "nt"


def interactive_desktop() -> tuple[bool, str]:
    if not is_windows():
        return False, "nur unter Windows"
    user32 = ctypes.windll.user32
    kernel32 = ctypes.windll.kernel32
    session = ctypes.c_ulong(0)
    kernel32.ProcessIdToSessionId(kernel32.GetCurrentProcessId(), ctypes.byref(session))
    if session.value == 0:
        return False, "der Job läuft in Sitzung 0 (Dienst) ohne interaktiven Desktop"
    handle = user32.OpenInputDesktop(0, False, DESKTOP_READOBJECTS)
    if not handle:
        return False, f"OpenInputDesktop scheitert (Fehler {kernel32.GetLastError()}): kein Eingabedesktop in Sitzung {session.value}"
    user32.CloseDesktop(handle)
    remote = user32.GetSystemMetrics(SM_REMOTESESSION)
    return True, f"interaktive Sitzung {session.value} mit Eingabedesktop" + (" (Remotesitzung)" if remote else "")


def media_foundation_files() -> list[str]:
    return [name for name in MF_FILES if not (SYSTEM32 / name).is_file()]


def media_foundation(run=None) -> tuple[bool, str]:
    """``run(cmd) -> (code, output)``; installs the server feature when the files are missing."""
    if not is_windows():
        return False, "nur unter Windows"
    missing = media_foundation_files()
    if not missing:
        return True, "Media Foundation mit H.264-Kodierer und -Dekodierer vorhanden"
    note = ""
    if run is not None:
        code, output = run(["powershell", "-NoProfile", "-NonInteractive", "-Command",
                            "Install-WindowsFeature Server-Media-Foundation -ErrorAction Stop | "
                            "Select-Object Success,RestartNeeded,ExitCode | ConvertTo-Json -Compress"])
        note = f"; Install-WindowsFeature Server-Media-Foundation: Exitcode {code}, {' '.join(output.split())[:300]}"
        missing = media_foundation_files()
        if not missing:
            return True, "Media Foundation nach Install-WindowsFeature ohne Neustart vorhanden" + note
    return False, (f"Media Foundation fehlt auf dem Runner ({', '.join(missing)} nicht in {SYSTEM32}){note}; "
                   "ohne Neustart nicht nachrüstbar (S4 K6)")


def is_elevated() -> bool:
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except (AttributeError, OSError):
        return False


def elevated_window() -> tuple[bool, str]:
    if not is_windows():
        return False, "nur unter Windows"
    if is_elevated():
        return False, ("der Testprozess läuft selbst erhöht (Administrator-Konto des Runners); ein Fenster höherer "
                       "Integrität neben einem Prozess mittlerer Integrität lässt sich hier nicht herstellen (NR N7)")
    return False, ("ein erhöhtes Fenster neben dem Testprozess bräuchte eine UAC-Bestätigung, die ein Runner nicht "
                   "geben kann (NR N7)")


def run_command(cmd: list) -> tuple[int, str]:
    completed = subprocess.run(cmd, capture_output=True, text=True, check=False, timeout=1800)
    return completed.returncode, completed.stdout + completed.stderr


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "probe":
        print(json.dumps({"interactive_desktop": interactive_desktop(), "media_foundation_missing":
                          media_foundation_files() if is_windows() else None, "elevated": is_elevated()
                          if is_windows() else None}, indent=2, ensure_ascii=False))
        sys.exit(0)
    print(__doc__)
    sys.exit(2)
