#!/usr/bin/env python3
"""Host side of the se Launcher instrumentations (``LauncherTaskTest``) in the batch suite
``rechte-desktop-tresore-task`` (plan.md D3 ``android-device``, ``android-device-api30``; strang-desktop.md
DSK-M9 to DSK-M13, DSK-M15).

Used by ``native/rdt_task/launcher.py`` and runnable by hand against a running emulator
(``python native/launcher-task-android.py home-holder``). Runs only on the remote runner (AGENTS.md); it
builds nothing.

What the host has to do around single test methods (the tests do everything else themselves: role
holder, ``appwidget grantbind``, notification listener, work profile and private space):

``PHASES``  methods that run more than once with host steps in between. ``launcher_task_survives_package_replace``
            (U35, assumption A30): ``-e launcher_replace_phase before``, then ``adb install -r`` of the same
            app APK, then ``-e launcher_replace_phase after``. Further phased methods of U40 are added here
            when U40 hands them over.
``helper_apk_args``  ``launcher_task_package_removed_clears_items`` needs a small app with a launcher activity
            (``-e launcher_helper_apk <path on the device> -e launcher_helper_package <package>``). The suite
            passes them only when the artifact ``rdt-apks`` contains ``launcher-helper.apk`` and
            ``launcher-helper.package`` (one line: the package name); otherwise the test reports "nicht
            ausgeführt" itself (assumption failure).
``home role``  the stage restores the HOME role holder of the emulator image at its end, so later stages
            (``android-bestand``) find the system launcher again.
"""
from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

HOME_ROLE = "android.app.role.HOME"
REPLACE = "launcher_task_survives_package_replace"
HELPER_APK = "launcher-helper.apk"
HELPER_PACKAGE = "launcher-helper.package"
DEVICE_TMP = "/data/local/tmp"

# method -> list of steps; a step is ("run", [(arg, value), ...]) or ("reinstall-app", []).
PHASES: dict[str, list[tuple[str, list[tuple[str, str]]]]] = {
    REPLACE: [("run", [("launcher_replace_phase", "before")]),
              ("reinstall-app", []),
              ("run", [("launcher_replace_phase", "after")])],
}


def home_holder_args() -> list[str]:
    return ["shell", "cmd", "role", "get-role-holders", "--user", "0", HOME_ROLE]


def restore_home_args(package: str) -> list[str]:
    return ["shell", "cmd", "role", "add-role-holder", "--user", "0", HOME_ROLE, package]


def parse_holder(output: str) -> str | None:
    for token in output.replace(",", " ").split():
        if "." in token and not token.startswith("["):
            return token.strip("[]")
    return None


def helper_apk(apks_dir: Path | None) -> tuple[Path, str] | None:
    """(local APK, package name) of the helper app, when the artifact carries it."""
    if not apks_dir or not apks_dir.is_dir():
        return None
    apk = next(iter(sorted(apks_dir.rglob(HELPER_APK))), None)
    package_file = next(iter(sorted(apks_dir.rglob(HELPER_PACKAGE))), None)
    if apk is None or package_file is None:
        return None
    package = package_file.read_text(encoding="utf-8").strip()
    return (apk, package) if package else None


def app_apk(apks_dir: Path | None) -> Path | None:
    """The app APK the device job installed (same order as ``common.Device.setup``)."""
    if not apks_dir or not apks_dir.is_dir():
        return None
    apks = [apk for apk in sorted(apks_dir.rglob("*.apk"))
            if "androidtest" not in apk.name.lower() and apk.name != HELPER_APK]
    apks.sort(key=lambda apk: (0 if "debug" in apk.name.lower() else 1,
                               0 if ("x86_64" in apk.name or "universal" in apk.name) else 1, apk.name))
    return apks[0] if apks else None


def main() -> int:
    adb = shutil.which("adb")
    if not adb:
        print("adb fehlt", file=sys.stderr)
        return 2
    if len(sys.argv) > 1 and sys.argv[1] == "home-holder":
        completed = subprocess.run([adb, *home_holder_args()], capture_output=True, text=True, check=False, timeout=60)
        print(parse_holder(completed.stdout) or completed.stdout.strip())
        return completed.returncode
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
