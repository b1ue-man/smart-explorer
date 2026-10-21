#!/usr/bin/env python3
"""Emulator preparation of the S4 instrumentations (``ScreenTaskTest``) of the batch suite
``rechte-desktop-tresore-task`` (plan.md D3 ``android-device``, ``android-device-api30``; strang-bildschirm.md
SCR-M8, SCR-M9, SCR-M12).

Used by ``native/rdt_task/screen.py`` (``DEVICE_SETUP``) and runnable by hand against a running emulator
(``python native/screen-task-android.py setup`` with ``adb`` on ``PATH``). Runs only on the remote runner
(AGENTS.md); it builds nothing.

``ScreenTaskTest`` switches the accessibility service on itself and allows ``PROJECT_MEDIA`` itself; the
setup here does the same once before the stage, so a refused command shows up in the stage log with its
output instead of only as a failed assertion. No second device takes part (the class says so).
"""
from __future__ import annotations

import shutil
import subprocess
import sys

PACKAGE = "app.smartexplorer.android"
ACCESSIBILITY_SERVICE = f"{PACKAGE}/app.smartexplorer.android.screen.ScreenControlService"


def setup_commands(package: str = PACKAGE) -> list[tuple[str, list[str]]]:
    """(description, adb arguments) in order; each may fail without ending the stage."""
    return [
        ("PROJECT_MEDIA erlauben", ["shell", "appops", "set", package, "PROJECT_MEDIA", "allow"]),
        ("Bedienungshilfe eintragen", ["shell", "settings", "put", "secure", "enabled_accessibility_services",
                                        ACCESSIBILITY_SERVICE]),
        ("Bedienungshilfen einschalten", ["shell", "settings", "put", "secure", "accessibility_enabled", "1"]),
        ("Bildschirm wach halten", ["shell", "svc", "power", "stayon", "true"]),
        ("Bildschirm entsperren", ["shell", "wm", "dismiss-keyguard"]),
    ]


def main() -> int:
    adb = shutil.which("adb")
    if not adb:
        print("adb fehlt", file=sys.stderr)
        return 2
    if len(sys.argv) < 2 or sys.argv[1] != "setup":
        print(__doc__)
        return 2
    code = 0
    for description, args in setup_commands():
        completed = subprocess.run([adb, *args], capture_output=True, text=True, check=False, timeout=120)
        print(f"{description}: Exitcode {completed.returncode} {' '.join((completed.stdout + completed.stderr).split())}")
        code = code or completed.returncode
    return code


if __name__ == "__main__":
    sys.exit(main())
