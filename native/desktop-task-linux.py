#!/usr/bin/env python3
"""Linux side of the S5 stages (se Desktop) of the batch suite ``rechte-desktop-tresore-task`` (plan.md D3
``linux-desktop``, ``linux-wayland``, ``linux-gnome``; strang-desktop.md DSK-M15).

Used by ``native/rdt_task/desktop.py`` and runnable by hand on a Linux runner
(``python native/desktop-task-linux.py probe`` prints the tools and the GNOME Shell version). Runs only on
the remote runner (AGENTS.md); it builds nothing.

The X11, layer-shell and GNOME checks start their own Xvfb, window managers, Sway and GNOME Shell
(``x11_task_env.rs``, ``layer_task_env.rs``, ``gnome_headless_tests.rs``) and end with the panic
"nicht ausgeführt: <Grund>" when a tool is missing; the suite reports that as not run. This helper adds
what the tests cannot do themselves:

``fedora_command``  ``desktop_task_gnome_headless_variant_b`` needs GNOME Shell 49 to 51: it runs the
                    candidate's test program inside the container ``fedora:43`` (assumptions A18, A19), with
                    the repository, the test program and the GUI program mounted read-only at their runner
                    paths, so compile-time paths stay valid.
``probe``           which tools the stages find.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

FEDORA_IMAGE = "fedora:43"
# GNOME Shell 49 headless with the session bus, Mesa (llvmpipe/lavapipe) and the schemas the extension uses.
FEDORA_PACKAGES = ("gnome-shell", "dbus-daemon", "dbus-tools", "mesa-dri-drivers", "mesa-vulkan-drivers",
                   "gsettings-desktop-schemas", "glib2", "libwayland-client", "libxkbcommon", "xorg-x11-server-Xwayland")
TOOLS = ("Xvfb", "openbox", "xfwm4", "xfdesktop", "xrandr", "xdotool", "dbus-run-session", "dbus-daemon", "sway",
         "grim", "wtype", "wl-copy", "readelf", "node", "gnome-shell", "docker", "git")


def which(name: str) -> str | None:
    return shutil.which(name)


def gnome_shell_version() -> str | None:
    binary = which("gnome-shell")
    if not binary:
        return None
    completed = subprocess.run([binary, "--version"], capture_output=True, text=True, check=False, timeout=60)
    return completed.stdout.strip() or None


def fedora_command(test_exe: Path, paths: list[str], env: dict[str, str], mounts: list[Path], workdir: Path) -> list[str]:
    """``docker run`` that installs GNOME Shell in ``fedora:43`` and runs ``paths`` of the test program
    (``--exact``, ``--include-ignored``, one thread)."""
    command = ["docker", "run", "--rm", "--init", "--shm-size=1g"]
    for mount in dict.fromkeys(str(path) for path in mounts):
        command += ["-v", f"{mount}:{mount}:ro"]
    for key, value in env.items():
        command += ["-e", f"{key}={value}"]
    command += ["-w", str(workdir), FEDORA_IMAGE, "bash", "-c"]
    tests = " ".join(f"'{path}'" for path in paths)
    script = (f"dnf -y -q install {' '.join(FEDORA_PACKAGES)} >/tmp/dnf.log 2>&1 "
              "|| { echo 'nicht ausgeführt: dnf install im Container fedora:43 scheiterte'; tail -n 20 /tmp/dnf.log; exit 3; }; "
              "gnome-shell --version; "
              f"'{test_exe}' --exact {tests} --include-ignored --test-threads=1")
    return command + [script]


def probe() -> dict:
    found = {name: which(name) for name in TOOLS}
    found["gnome-shell --version"] = gnome_shell_version()
    return found


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "probe":
        print(json.dumps(probe(), indent=2))
        sys.exit(0)
    print(__doc__)
    sys.exit(2)
