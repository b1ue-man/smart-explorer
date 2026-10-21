#!/usr/bin/env python3
"""Windows side of the S5 stages (se Desktop) of the batch suite ``rechte-desktop-tresore-task`` (plan.md D3
``windows-desktop``; strang-desktop.md DSK-M5, DSK-M15).

Used by ``native/rdt_task/desktop.py`` and runnable by hand on the Windows runner
(``python native/desktop-task-windows.py probe``). Runs only on the remote runner (AGENTS.md); it builds
nothing.

``progman()``  the pre-check of D 586: ``FindWindowW("Progman", NULL)`` with the reported outcome. Without
               Explorer's desktop window the checks with Explorer reference are not run (assumption A7).
``session()``  session id and whether an input desktop exists (the window checks need an interactive
               session, NR DN21).
"""
from __future__ import annotations

import ctypes
import json
import os
import sys

DESKTOP_READOBJECTS = 0x0001
# Checks that need Explorer's desktop (Win+D handled by Explorer; assumption A7, U32).
EXPLORER_CHECKS = ("desktop_task_windows_show_desktop_keeps_surface",)


def progman() -> tuple[bool, str]:
    if os.name != "nt":
        return False, "nur unter Windows"
    user32 = ctypes.windll.user32
    user32.FindWindowW.restype = ctypes.c_void_p
    handle = user32.FindWindowW("Progman", None)
    if not handle:
        return False, "FindWindowW(\"Progman\") liefert NULL: kein Explorer-Desktop auf dem Runner (Annahme A7)"
    return True, f"FindWindowW(\"Progman\") = 0x{handle:x}: Explorer-Desktop vorhanden"


def session() -> tuple[bool, str]:
    if os.name != "nt":
        return False, "nur unter Windows"
    kernel32 = ctypes.windll.kernel32
    user32 = ctypes.windll.user32
    sid = ctypes.c_ulong(0)
    kernel32.ProcessIdToSessionId(kernel32.GetCurrentProcessId(), ctypes.byref(sid))
    handle = user32.OpenInputDesktop(0, False, DESKTOP_READOBJECTS)
    if handle:
        user32.CloseDesktop(handle)
    if sid.value == 0 or not handle:
        return False, f"Sitzung {sid.value} ohne Eingabedesktop"
    return True, f"interaktive Sitzung {sid.value}"


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "probe":
        print(json.dumps({"progman": progman(), "session": session()}, indent=2, ensure_ascii=False))
        sys.exit(0)
    print(__doc__)
    sys.exit(2)
