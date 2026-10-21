#!/usr/bin/env python3
"""Linux environments of the S4 stages (screen) of the batch suite ``rechte-desktop-tresore-task``.

Used by ``native/rdt_task/screen.py`` (plan.md D3; strang-bildschirm.md SCR-M12) and runnable by hand on
a Linux runner (``python native/screen-task-linux.py probe`` prints which tools exist). Runs only on the
remote runner (AGENTS.md); it starts no build.

Each ``start_*`` function returns ``Session(ok, reason, env, processes)``: ``env`` is what a test needs
(``WAYLAND_DISPLAY``, ``DBUS_SESSION_BUS_ADDRESS`` ...), ``processes`` are the started programs (each in
its own process group, so the caller ends them with ``common.kill_tree``). A missing program or a
compositor that does not start gives ``ok=False`` and the exact reason; the stage then reports the
entries as "nicht ausgeführt" with that reason, never as passed.

Environments:
  openbox          EWMH window manager on an existing X display (SCR-M5 window list "ewmh").
  wayland-portal   headless Sway 1.9 (WLR_BACKENDS=headless, WLR_RENDERER=pixman), PipeWire,
                   WirePlumber, xdg-desktop-portal with xdg-desktop-portal-wlr (chooser_type=none,
                   output_name of the headless output, XDPW_PERSIST_MODE=permanent) on a private
                   session bus (SCR-M5 Wayland, U19).
  kwin             headless KWin (``kwin_wayland --virtual``) on a private session bus, with a desktop
                   file that grants the test program org_kde_plasma_window_management (S4 K3) and one
                   ordinary window titled ``KDE_WINDOW_TITLE`` (``foot``) for the window list.
  kde-portal       xdg-desktop-portal-kde, PipeWire and a permission store entry for RemoteDesktop on
                   the bus of ``kwin`` (SCR-M6 KDE).
  netem            ``tc qdisc ... netem loss 2% delay 12.5ms`` on ``lo`` (S4 A17).
  h264-high        H.264 High test stream without B frames, made with ffmpeg/x264 (SCR-M3, A4).
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

NETEM_RULE = ["netem", "loss", "2%", "delay", "12.5ms"]
HEADLESS_OUTPUT = "HEADLESS-1"
KDE_WINDOW_TITLE = "se task window"
LIBEXEC = ("/usr/libexec", "/usr/lib/x86_64-linux-gnu/libexec", "/usr/lib")
TOOLS = ("Xvfb", "openbox", "ffmpeg", "tc", "modprobe", "sway", "grim", "pipewire", "wireplumber",
         "dbus-daemon", "dbus-send", "kwin_wayland", "swaymsg", "foot")


class Session:
    def __init__(self, ok: bool, reason: str, env: dict | None = None, processes: list | None = None):
        self.ok = ok
        self.reason = reason
        self.env = env or {}
        self.processes = processes or []


def which(name: str) -> str | None:
    found = shutil.which(name)
    if found:
        return found
    for base in LIBEXEC:
        candidate = Path(base) / name
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


def spawn(command: list, env: dict, log: Path) -> subprocess.Popen:
    log.parent.mkdir(parents=True, exist_ok=True)
    handle = open(log, "ab")
    return subprocess.Popen([str(part) for part in command], env=env, stdout=handle, stderr=subprocess.STDOUT,
                            stdin=subprocess.DEVNULL, start_new_session=True)


def wait_for(check, timeout: float, process: subprocess.Popen | None = None) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if check():
            return True
        if process is not None and process.poll() is not None:
            return bool(check())
        time.sleep(0.2)
    return bool(check())


def tail(path: Path, limit: int = 600) -> str:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""
    return " ".join(text[-limit:].split())


def exit_text(process: subprocess.Popen) -> str:
    code = process.poll()
    return "läuft" if code is None else f"beendet mit {code}"


def end_all(processes: list) -> None:
    for process in reversed(processes):
        if process.poll() is None:
            try:
                os.killpg(process.pid, 15)
            except (ProcessLookupError, PermissionError):
                pass
    for process in processes:
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, 9)
            except (ProcessLookupError, PermissionError):
                pass


# --------------------------------------------------------------------------- X11

def start_openbox(display: str, root: Path, base_env: dict) -> Session:
    binary = which("openbox")
    if not binary:
        return Session(False, "openbox fehlt (Paket openbox)")
    env = dict(base_env, DISPLAY=display)
    env.pop("WAYLAND_DISPLAY", None)
    process = spawn([binary, "--sm-disable"], env, root / "openbox.log")
    xprop = which("xprop")

    def running() -> bool:
        if process.poll() is not None:
            return False
        if not xprop:
            return True
        probe = subprocess.run([xprop, "-root", "_NET_SUPPORTING_WM_CHECK"], env=env, capture_output=True,
                               text=True, check=False)
        return "window id" in probe.stdout
    if not wait_for(running, 20, process):
        end_all([process])
        return Session(False, f"openbox startete nicht auf {display}: {tail(root / 'openbox.log')}")
    return Session(True, f"openbox auf {display}", {}, [process])


def make_h264_high(out: Path, base_env: dict) -> tuple[bool, str]:
    """640x360, 30 frames, profile High, no B frames (strang-bildschirm.md SCR-M3, U13)."""
    ffmpeg = which("ffmpeg")
    if not ffmpeg:
        return False, "ffmpeg fehlt (Paket ffmpeg)"
    out.parent.mkdir(parents=True, exist_ok=True)
    command = [ffmpeg, "-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=30",
               "-frames:v", "30", "-c:v", "libx264", "-profile:v", "high", "-bf", "0", "-pix_fmt", "yuv420p",
               "-f", "h264", str(out)]
    completed = subprocess.run(command, env=base_env, capture_output=True, text=True, check=False, timeout=600)
    if completed.returncode != 0 or not out.is_file() or out.stat().st_size == 0:
        return False, f"ffmpeg/libx264 erzeugte keine Testdatei (Exitcode {completed.returncode}): " \
                      f"{' '.join((completed.stdout + completed.stderr).split())[:400]}"
    return True, f"{out} ({out.stat().st_size} Bytes, High ohne B-Bilder)"


# --------------------------------------------------------------------------- netem

def netem_commands() -> tuple[list[list[str]], list[str]]:
    """(setup commands, cleanup command) for sudo -n."""
    setup = [["sudo", "-n", "modprobe", "sch_netem"],
             ["sudo", "-n", "tc", "qdisc", "replace", "dev", "lo", "root", *NETEM_RULE]]
    cleanup = ["sudo", "-n", "tc", "qdisc", "del", "dev", "lo", "root"]
    return setup, cleanup


# --------------------------------------------------------------------------- session bus

def start_session_bus(root: Path, env: dict) -> Session:
    daemon = which("dbus-daemon")
    if not daemon:
        return Session(False, "dbus-daemon fehlt")
    socket = root / "session_bus_socket"
    process = spawn([daemon, "--session", "--nofork", "--nopidfile", f"--address=unix:path={socket}"], env,
                    root / "dbus-session.log")
    if not wait_for(socket.exists, 20, process):
        end_all([process])
        return Session(False, f"privater Sitzungsbus startete nicht: {tail(root / 'dbus-session.log')}")
    return Session(True, f"unix:path={socket}", {"DBUS_SESSION_BUS_ADDRESS": f"unix:path={socket}"}, [process])


def bus_name_present(env: dict, name: str) -> bool:
    sender = which("dbus-send")
    if not sender:
        return False
    probe = subprocess.run([sender, "--session", "--print-reply", "--dest=org.freedesktop.DBus",
                            "/org/freedesktop/DBus", "org.freedesktop.DBus.NameHasOwner", f"string:{name}"],
                           env=env, capture_output=True, text=True, check=False, timeout=30)
    return "boolean true" in probe.stdout


def private_runtime(root: Path, base_env: dict) -> dict:
    runtime = root / "runtime"
    runtime.mkdir(parents=True, exist_ok=True)
    runtime.chmod(0o700)
    env = dict(base_env)
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"):
        env.pop(key, None)
    for key, folder in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state"),
                        ("XDG_CACHE_HOME", "cache")):
        (root / folder).mkdir(parents=True, exist_ok=True)
        env[key] = str(root / folder)
    env["XDG_RUNTIME_DIR"] = str(runtime)
    return env


def start_pipewire(root: Path, env: dict) -> Session:
    pipewire, wireplumber = which("pipewire"), which("wireplumber")
    if not pipewire or not wireplumber:
        return Session(False, f"{'pipewire' if not pipewire else 'wireplumber'} fehlt")
    processes = [spawn([pipewire], env, root / "pipewire.log")]
    socket = Path(env["XDG_RUNTIME_DIR"]) / "pipewire-0"
    if not wait_for(socket.exists, 20, processes[0]):
        end_all(processes)
        return Session(False, f"PipeWire startete nicht: {tail(root / 'pipewire.log')}")
    processes.append(spawn([wireplumber], env, root / "wireplumber.log"))
    time.sleep(2)
    if processes[1].poll() is not None:
        end_all(processes)
        return Session(False, f"WirePlumber endete sofort: {tail(root / 'wireplumber.log')}")
    return Session(True, "PipeWire und WirePlumber", {}, processes)


# --------------------------------------------------------------------------- Sway + portal

def sway_answers(runtime: Path, env: dict) -> bool:
    """Sway answers ``get_version`` on its IPC socket; without ``swaymsg`` only the process state counts."""
    swaymsg = which("swaymsg")
    if not swaymsg:
        return True
    ipc = sorted(runtime.glob("sway-ipc.*.sock"))
    if not ipc:
        return False
    try:
        probe = subprocess.run([swaymsg, "-s", str(ipc[0]), "-t", "get_version"], env=env, capture_output=True,
                               text=True, check=False, timeout=10)
    except subprocess.TimeoutExpired:
        return False
    return probe.returncode == 0


def start_wayland_portal(root: Path, base_env: dict) -> Session:
    sway = which("sway")
    xdpw = which("xdg-desktop-portal-wlr")
    portal = which("xdg-desktop-portal")
    missing = [name for name, path in (("sway", sway), ("xdg-desktop-portal-wlr", xdpw),
                                        ("xdg-desktop-portal", portal)) if not path]
    if missing:
        return Session(False, "fehlt: " + ", ".join(missing))
    env = private_runtime(root, base_env)
    processes: list = []
    bus = start_session_bus(root, env)
    if not bus.ok:
        return bus
    processes += bus.processes
    env.update(bus.env)
    env.update(XDG_CURRENT_DESKTOP="sway", XDG_SESSION_TYPE="wayland", WLR_BACKENDS="headless",
               WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1")
    config = root / "sway.conf"
    config.write_text(f"output {HEADLESS_OUTPUT} resolution 1920x1080\n", encoding="utf-8")
    processes.append(spawn([sway, "-c", config], env, root / "sway.log"))
    compositor = processes[-1]
    runtime = Path(env["XDG_RUNTIME_DIR"])

    def sockets() -> list[Path]:
        return sorted(path for path in runtime.glob("wayland-*") if not path.name.endswith(".lock"))
    if not wait_for(lambda: bool(sockets()), 30, compositor):
        end_all(processes)
        return Session(False, f"Sway startete kopflos nicht: {tail(root / 'sway.log')}")
    env["WAYLAND_DISPLAY"] = sockets()[0].name
    # A socket file alone is no running Sway (an abnormal end leaves it behind); Sway answers on its IPC
    # socket only once its event loop runs.
    if not wait_for(lambda: sway_answers(runtime, env), 30, compositor) or compositor.poll() is not None:
        reason = f"Sway läuft nicht ({exit_text(compositor)}): {tail(root / 'sway.log')}"
        end_all(processes)
        return Session(False, reason)
    sound = start_pipewire(root, env)
    if not sound.ok:
        end_all(processes)
        return sound
    processes += sound.processes
    xdpw_config = root / "xdpw.conf"
    xdpw_config.write_text(f"[screencast]\noutput_name={HEADLESS_OUTPUT}\nchooser_type=none\n", encoding="utf-8")
    portals = Path(env["XDG_CONFIG_HOME"]) / "xdg-desktop-portal"
    portals.mkdir(parents=True, exist_ok=True)
    (portals / "sway-portals.conf").write_text(
        "[preferred]\ndefault=wlr\norg.freedesktop.impl.portal.ScreenCast=wlr\n"
        "org.freedesktop.impl.portal.Screenshot=wlr\n", encoding="utf-8")
    processes.append(spawn([xdpw, "-c", xdpw_config], dict(env, XDPW_PERSIST_MODE="permanent"), root / "xdpw.log"))
    if not wait_for(lambda: bus_name_present(env, "org.freedesktop.impl.portal.desktop.wlr"), 30, processes[-1]):
        reason = (f"xdg-desktop-portal-wlr meldete sich nicht am Bus: {tail(root / 'xdpw.log')}; "
                  f"Sway {exit_text(compositor)} auf {env['WAYLAND_DISPLAY']}: {tail(root / 'sway.log', 300)}")
        end_all(processes)
        return Session(False, reason)
    processes.append(spawn([portal], env, root / "xdg-desktop-portal.log"))
    if not wait_for(lambda: bus_name_present(env, "org.freedesktop.portal.Desktop"), 30, processes[-1]):
        end_all(processes)
        return Session(False, f"xdg-desktop-portal meldete sich nicht am Bus: {tail(root / 'xdg-desktop-portal.log')}")
    keep = {key: env[key] for key in ("WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS",
                                      "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "XDG_CONFIG_HOME")}
    return Session(True, f"Sway kopflos ({env['WAYLAND_DISPLAY']}, Ausgang {HEADLESS_OUTPUT}), PipeWire, "
                         "xdg-desktop-portal-wlr (chooser_type=none, XDPW_PERSIST_MODE=permanent)", keep, processes)


# --------------------------------------------------------------------------- KWin + KDE portal

def start_kwin(root: Path, base_env: dict, test_program: str | None) -> Session:
    kwin = which("kwin_wayland")
    if not kwin:
        return Session(False, "kwin_wayland fehlt (Paket kwin-wayland nicht installierbar oder nicht installiert; S4 K3)")
    env = private_runtime(root, base_env)
    bus = start_session_bus(root, env)
    if not bus.ok:
        return bus
    processes = list(bus.processes)
    env.update(bus.env)
    env.update(XDG_CURRENT_DESKTOP="KDE", XDG_SESSION_TYPE="wayland", KWIN_COMPOSE="Q", QT_QPA_PLATFORM="offscreen")
    if test_program:
        applications = Path(env["XDG_DATA_HOME"]) / "applications"
        applications.mkdir(parents=True, exist_ok=True)
        (applications / "smart-explorer-task.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=se task\n"
            f"Exec={test_program}\nX-KDE-Wayland-Interfaces=org_kde_plasma_window_management\n", encoding="utf-8")
    socket_name = "wayland-kwin"
    processes.append(spawn([kwin, "--virtual", "--no-lockscreen", "--socket", socket_name, "--width", "1280",
                            "--height", "800"], env, root / "kwin.log"))
    socket = Path(env["XDG_RUNTIME_DIR"]) / socket_name
    if not wait_for(socket.exists, 40, processes[-1]):
        end_all(processes)
        return Session(False, f"KWin startet kopflos nicht (S4 K3): {tail(root / 'kwin.log')}")
    env["WAYLAND_DISPLAY"] = socket_name
    keep = {key: env[key] for key in ("WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS",
                                      "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "XDG_DATA_HOME", "XDG_CONFIG_HOME")}
    # The window list (screen_task_kde_window_list) needs one ordinary window with a known title; foot keeps
    # the title it is given while its command prints nothing.
    terminal = which("foot")
    window = "kein Testfenster (foot fehlt)"
    if terminal:
        processes.append(spawn([terminal, "--title", KDE_WINDOW_TITLE, "--app-id", "se-task-window", "sleep",
                                "infinity"], env, root / "foot.log"))
        time.sleep(2)
        if processes[-1].poll() is None:
            window = f"Testfenster „{KDE_WINDOW_TITLE}“"
            keep["SE_SCREEN_TASK_KDE_TITLE"] = KDE_WINDOW_TITLE
        else:
            window = f"Testfenster endete sofort ({exit_text(processes[-1])}): {tail(root / 'foot.log')}"
    return Session(True, f"KWin kopflos auf {socket_name}; {window}", keep, processes)


def start_kde_portal(root: Path, base_env: dict, kwin_env: dict) -> Session:
    portal_kde = which("xdg-desktop-portal-kde")
    portal = which("xdg-desktop-portal")
    sender = which("dbus-send")
    missing = [name for name, path in (("xdg-desktop-portal-kde", portal_kde), ("xdg-desktop-portal", portal),
                                        ("dbus-send", sender)) if not path]
    if missing:
        return Session(False, "fehlt: " + ", ".join(missing))
    env = dict(base_env)
    env.update(kwin_env)
    sound = start_pipewire(root, env)
    if not sound.ok:
        return sound
    processes = list(sound.processes)
    processes.append(spawn([portal_kde], env, root / "xdg-desktop-portal-kde.log"))
    if not wait_for(lambda: bus_name_present(env, "org.freedesktop.impl.portal.desktop.kde"), 30, processes[-1]):
        end_all(processes)
        return Session(False, f"xdg-desktop-portal-kde meldete sich nicht am Bus: {tail(root / 'xdg-desktop-portal-kde.log')}")
    processes.append(spawn([portal], env, root / "xdg-desktop-portal.log"))
    if not wait_for(lambda: bus_name_present(env, "org.freedesktop.portal.Desktop"), 30, processes[-1]):
        end_all(processes)
        return Session(False, f"xdg-desktop-portal meldete sich nicht am Bus: {tail(root / 'xdg-desktop-portal.log')}")
    granted = subprocess.run([sender, "--session", "--print-reply", "--dest=org.freedesktop.impl.portal.PermissionStore",
                              "/org/freedesktop/impl/portal/PermissionStore",
                              "org.freedesktop.impl.portal.PermissionStore.SetPermission", "string:kde-authorized",
                              "boolean:true", "string:remote-desktop", "string:", "array:string:yes"],
                             env=env, capture_output=True, text=True, check=False, timeout=30)
    if granted.returncode != 0:
        end_all(processes)
        return Session(False, "Eintrag im Permission-Store für RemoteDesktop nicht setzbar: "
                              + " ".join((granted.stdout + granted.stderr).split())[:300])
    return Session(True, "xdg-desktop-portal-kde, PipeWire, Permission-Store-Eintrag remote-desktop", {}, processes)


def probe() -> dict:
    return {name: which(name) for name in TOOLS + ("xdg-desktop-portal", "xdg-desktop-portal-wlr",
                                                   "xdg-desktop-portal-kde")}


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "probe":
        print(json.dumps(probe(), indent=2))
        sys.exit(0)
    print(__doc__)
    sys.exit(2)
