"""S4 (Bildschirm) in the batch suite: stages, fixtures and suite checks (plan.md D3; strang-bildschirm.md
SCR-M12; package U41).

Stages (``STAGES``; the entrypoint assigns them):
  screen-pair            linux-core: two real iroh nodes on loopback (SCR-M2), relay only through the share
                         server fixture, ``screen_task_netem_recovery`` last (netem stays on ``lo`` until the
                         stage ends).
  screen-x11             linux-desktop: Xvfb (common fixture), OpenH264 and the ffmpeg test file (SCR-M3),
                         X11 capture and input (SCR-M5, SCR-M6); the window list runs twice in one result
                         (openbox = "ewmh", no window manager = "none").
  screen-wayland         linux-wayland: headless Sway with xdg-desktop-portal-wlr (SCR-M5).
  kde                    linux-wayland (also the S5 entry ``desktop_task_kwin_layer_namespace_desktop``): headless
                         KWin; when it does not start, all three entries are "nicht ausgeführt" (S4 K3).
  screen-windows         windows-screen: everything with an interactive desktop, Media Foundation only when present
                         (K6), ``screen_task_windows_lock_reports_secure_desktop`` as the very last step.
  screen-android-device  android-device: ``ScreenTaskTest`` methods one by one (not the whole class).
  screen-android-api30   android-device-api30: the same runner for the API 30 entries.
  screen-jvm             android-build (proposed registration): JVM tests of the app's screen classes; the
                         generic stage (``ctx.run_default``) reads the Gradle reports.

Fixtures (``FIXTURES``; environment contract for test authors):
  netem               ``tc qdisc replace dev lo root netem loss 2% delay 12.5ms`` via ``sudo -n`` (after
                      ``modprobe sch_netem``), removed when the stage ends; without both: not run (S4 A17).
  openh264            ``SE_SCREEN_TASK_OPENH264`` = the pinned Cisco ``.bz2`` of this platform, downloaded from
                      the pinned URL (no redirects) and checked against size and SHA-256 of
                      ``native/src/screen/os/native_ffi/openh264_pins.rs``. A pin mismatch fails the entries.
  h264-high           ``SE_SCREEN_TASK_H264_HIGH`` = H.264 High without B frames (ffmpeg/x264).
  gpu-adapter         ``SE_SCREEN_TASK_GPU=1`` when a Vulkan driver (lavapipe from mesa-vulkan-drivers) is
                      installed; otherwise not run.
  wayland-portal      ``WAYLAND_DISPLAY``, ``XDG_RUNTIME_DIR``, ``DBUS_SESSION_BUS_ADDRESS``, ``XDG_CURRENT_DESKTOP``,
                      ``XDG_SESSION_TYPE``, ``XDG_CONFIG_HOME`` of headless Sway + PipeWire + portals,
                      ``SE_SCREEN_TASK_WAYLAND=1``; ``DISPLAY`` removed.
  kwin                ``WAYLAND_DISPLAY`` and bus of headless KWin, ``SE_SCREEN_TASK_KDE=1``, ``SE_DESKTOP_TASK_KWIN=1``.
  kde-portal          on the bus of ``kwin``: xdg-desktop-portal-kde, PipeWire, permission store entry.
  interactive-desktop ``SE_SCREEN_TASK_WINDOWS=1`` when the job has an input desktop (Windows).
  media-foundation    present only when the H.264 coders of Media Foundation exist (Windows, K6).
  elevated-window     never available on a runner (NR N7); the reason says why.

Suite checks (``CHECKS``): ``screen_task_installer_lists_screen_licenses`` and
``screen_task_install_linux_kde_interfaces_line`` (prepare: static), ``screen_task_docs_inventory_lists_every_cli_command``
(linux-core: docs-cli).

Shared with ``desktop.py`` and ``launcher.py``: ``run_entries`` (exact names, then a test's own "nicht ausgeführt: <Grund>"
panic becomes not_run), ``run_entry_once`` (a library test run without recording), ``helper`` (loads the
``native/*-task-*.py`` helper scripts) and ``cli_docs_check``.
"""
from __future__ import annotations

import hashlib
import importlib.util
import os
import re
import secrets
import shutil
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

from . import common
from .common import FAILED, LINUX, NATIVE, NOT_RUN, PASSED, REPO, WINDOWS

PINS_FILE = NATIVE / "src" / "screen" / "os" / "native_ffi" / "openh264_pins.rs"
CISCO_LICENSE = REPO / "third-party" / "openh264" / "BINARY_LICENSE.txt"
INSTALL_LINUX = REPO / "install-linux.sh"
SCREEN_DOC = REPO / "docs" / "oberflaechen" / "bildschirm.md"
KDE_LINE = "X-KDE-Wayland-Interfaces=org_kde_plasma_window_management"
WINDOW_LIST = "screen_task_x11_window_list_ewmh_and_querytree"
NETEM = "screen_task_netem_recovery"
LOCK_LAST = "screen_task_windows_lock_reports_secure_desktop"
NOT_RUN_PANIC = re.compile(r"nicht ausgeführt:\s*([^\n]+)")
# W6: these license files were dropped; the installer must not ship them.
DROPPED_LICENSES = ("openh264-rs", "ashpd\\license", "wide\\license", "bzip2-rs")


# --------------------------------------------------------------------------- helper scripts

def helper(name: str):
    """Loads ``native/<name>.py`` (file names with hyphens) once."""
    key = "rdt_helper_" + name.replace("-", "_")
    if key in sys.modules:
        return sys.modules[key]
    spec = importlib.util.spec_from_file_location(key, NATIVE / f"{name}.py")
    if spec is None or spec.loader is None:
        raise ImportError(f"native/{name}.py fehlt")
    module = importlib.util.module_from_spec(spec)
    sys.modules[key] = module
    spec.loader.exec_module(module)
    return module


# --------------------------------------------------------------------------- shared library-test runners

def settle_not_run(ctx: common.JobContext, stage: str, names: set[str]) -> None:
    """A test that ends with the panic "nicht ausgeführt: <Grund>" (U34, U37: missing tool, no adapter,
    environment does not start) is reported as not run with that reason, never as failed or passed."""
    changed = False
    for test in ctx.report.tests:
        if test["stage"] != stage or test["name"] not in names or test["status"] != FAILED:
            continue
        match = NOT_RUN_PANIC.search(test["reason"] or "")
        if match:
            test["status"] = NOT_RUN
            test["reason"] = f"vom Test gemeldet: nicht ausgeführt: {match.group(1).strip()}"
            changed = True
    if changed:
        ctx.report.write()


def run_entries(ctx: common.JobContext, stage: str, entries: list, extra_env: dict | None = None) -> None:
    """Library tests (exact full paths, ``--exact``), harness modules and suite checks of ``entries``."""
    todo = [entry for entry in entries if not ctx.recorded(entry.name)]
    libs = [entry for entry in todo if entry.kind in ("lib", "modul")]
    if libs:
        ctx.run_lib(stage, libs, extra_env or {})
        settle_not_run(ctx, stage, {entry.name for entry in libs})
    for entry in todo:
        if entry.kind == "check" and entry.name != common.INVENTORY and not ctx.recorded(entry.name):
            ctx.run_check(stage, entry)
    junit = [entry for entry in todo if entry.kind == "junit" and not ctx.recorded(entry.name)]
    if junit:
        ctx.run_junit(stage, junit)


def run_entry_once(ctx: common.JobContext, stage: str, entry, extra_env: dict) -> tuple[str, str, float]:
    """One run of a single library test without recording it (for tests that run twice, or that a stage
    function sequences itself). Returns (status, reason, duration)."""
    if not ctx.cargo.build_lib():
        return FAILED, "Bau der Bibliothekstests gescheitert: " + ctx.cargo.lib_error, 0.0
    paths = ctx.cargo.by_leaf.get(entry.name, [])
    if len(paths) != 1:
        if not paths and entry.pending:
            return NOT_RUN, entry.pending_reason(), 0.0
        return FAILED, ("fehlt im Testprogramm dieser Plattform" if not paths
                        else "Name mehrdeutig im Testprogramm: " + ", ".join(paths)), 0.0
    assert ctx.fixtures is not None
    ready, why = ctx.fixtures.ensure(entry.fixtures)
    if not ready:
        return NOT_RUN, why, 0.0
    tmp = ctx.work / "groups" / f"{stage}-{secrets.token_hex(4)}"
    tmp.mkdir(parents=True, exist_ok=True)
    env = ctx.base_env()
    fixture_env, removed = ctx.fixtures.env_for(entry.fixtures)
    env.update(fixture_env)
    env.update(extra_env)
    for key in removed:
        env.pop(key, None)
    try:
        for key, value in entry.env:
            env[key] = ctx.expand(value, tmp)
    except common.StageError as error:
        return FAILED, f"Umgebung nicht herstellbar: {error}", 0.0
    results = ctx.run_libtest(stage, paths, env, entry.ignored, entry.threads, entry.root)
    status, reason, duration = results.get(paths[0], [FAILED, "kein Ergebnis", 0.0])
    if status == FAILED:
        match = NOT_RUN_PANIC.search(reason or "")
        if match:
            return NOT_RUN, f"vom Test gemeldet: nicht ausgeführt: {match.group(1).strip()}", duration
    return status, reason or paths[0], duration


def by_name(entries: list, name: str):
    return next((entry for entry in entries if entry.name == name), None)


# --------------------------------------------------------------------------- OpenH264 (pinned download)

def openh264_pin() -> dict | None:
    """The pin of this runner's platform, read from the Rust source (single source of truth)."""
    if not PINS_FILE.is_file():
        return None
    text = PINS_FILE.read_text(encoding="utf-8", errors="replace")
    marker = r"all\(windows, target_arch = \"x86_64\"\)" if WINDOWS else \
        r"all\(target_os = \"linux\", target_arch = \"x86_64\"\)"
    match = re.search(r"#\[cfg\(" + marker + r"\)\]\s*pub\(crate\) const PIN: Option<OpenH264Pin> = Some\(OpenH264Pin \{(.*?)\}\);",
                      text, re.S)
    if not match:
        return None
    body = match.group(1)
    fields = dict(re.findall(r"(\w+):\s*\"?([^\",\n]+)\"?,", body))
    try:
        return {"url": fields["url"], "size": int(fields["compressed_size"].replace("_", "")),
                "sha256": fields["compressed_sha256"].lower()}
    except (KeyError, ValueError):
        return None


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D401 - urllib hook
        raise urllib.error.HTTPError(req.full_url, code, f"Weiterleitung nach {newurl} abgelehnt", headers, fp)


def openh264_state(ctx: common.JobContext) -> tuple[str, str, Path | None]:
    """("ok" | "mismatch" | "unavailable", reason, path) once per job."""
    cached = ctx.state.get("openh264")
    if cached is not None:
        return cached
    pin = openh264_pin()
    if pin is None:
        result = ("unavailable", f"kein Pin für diese Plattform in {PINS_FILE.relative_to(REPO)}", None)
    else:
        target = ctx.work / "openh264" / pin["url"].rsplit("/", 1)[-1]
        target.parent.mkdir(parents=True, exist_ok=True)
        data = None
        if not target.is_file():
            try:
                opener = urllib.request.build_opener(NoRedirect)
                with opener.open(pin["url"], timeout=120) as response:
                    data = response.read(pin["size"] + 1)
                target.write_bytes(data)
            except (urllib.error.URLError, OSError, TimeoutError) as error:
                result = ("unavailable", f"Download {pin['url']} gescheitert: {error}", None)
                ctx.state["openh264"] = result
                return result
        data = target.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        if len(data) != pin["size"] or digest != pin["sha256"]:
            result = ("mismatch", f"{pin['url']}: {len(data)} Bytes, SHA-256 {digest}; Pin {pin['size']} Bytes, "
                                  f"{pin['sha256']} (openh264_pins.rs)", None)
        else:
            result = ("ok", f"{target.name} ({len(data)} Bytes, SHA-256 wie gepinnt)", target)
    ctx.state["openh264"] = result
    return result


def fail_on_pin_mismatch(ctx: common.JobContext, stage: str, entries: list) -> None:
    state, reason, _ = openh264_state(ctx)
    if state != "mismatch":
        return
    for entry in entries:
        if "openh264" in entry.fixtures and not ctx.recorded(entry.name):
            ctx.record(entry.name, stage, FAILED, "OpenH264-Pin weicht ab: " + reason)


# --------------------------------------------------------------------------- fixtures

def add_session(fixtures: common.StageFixtures, session) -> None:
    for process in session.processes:
        fixtures.add_process(process)


def keep_logs(fixtures: common.StageFixtures, folder: Path) -> Path:
    """The logs of a fixture's programs (compositor, portals, PipeWire, bus) go into the job's evidence when the
    stage ends, after its processes ended (``StageFixtures.close``)."""
    def copy() -> None:
        for path in sorted(folder.glob("*.log")):
            shutil.copyfile(path, fixtures.ctx.logs / f"fixture-{folder.name}-{path.name}")
    fixtures.add_cleanup(copy)
    return folder


def fixture_netem(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "netem nur unter Linux"
    if not fixtures.ctx.sudo_ok():
        return False, "sudo -n ist nicht verfügbar (tc braucht root)"
    if not shutil.which("tc"):
        return False, "tc fehlt (Paket iproute2)"
    setup, cleanup = helper("screen-task-linux").netem_commands()
    for command in setup:
        completed = fixtures.ctx.run(command, stage=fixtures.stage, timeout=120)
        if not completed.ok:
            return False, (f"{' '.join(command[2:])}: {completed.describe()} – sch_netem ist auf diesem Kernel nicht "
                           f"ladbar (S4 A17): {common.excerpt(completed.output, 300)}")
    fixtures.add_cleanup(lambda: fixtures.ctx.run(cleanup, stage=fixtures.stage, timeout=120))
    return True, "netem loss 2% delay 12.5ms auf lo"


def fixture_openh264(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    state, reason, path = openh264_state(fixtures.ctx)
    if state != "ok":
        return False, reason
    fixtures.provide(name, env={"SE_SCREEN_TASK_OPENH264": path})
    return True, reason


def fixture_h264_high(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "die Testdatei wird nur auf dem Linux-Runner erzeugt (SCR-M3)"
    out = fixtures.make_dir("h264-high") / "high.h264"
    ok, reason = helper("screen-task-linux").make_h264_high(out, fixtures.ctx.base_env())
    if ok:
        fixtures.provide(name, env={"SE_SCREEN_TASK_H264_HIGH": out})
    return ok, reason


def fixture_gpu_adapter(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "nur unter Linux geprüft"
    drivers = [path for base in ("/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d")
               for path in sorted(Path(base).glob("*.json")) if Path(base).is_dir()]
    if not drivers:
        return False, "kein Vulkan-Treiber im Runner (mesa-vulkan-drivers mit lavapipe fehlt)"
    fixtures.provide(name, env={"SE_SCREEN_TASK_GPU": "1"})
    return True, "Vulkan-Treiber: " + ", ".join(path.name for path in drivers)


def fixture_wayland_portal(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "nur unter Linux"
    folder = keep_logs(fixtures, fixtures.make_dir("wayland"))
    session = helper("screen-task-linux").start_wayland_portal(folder, fixtures.ctx.base_env())
    add_session(fixtures, session)
    if session.ok:
        fixtures.provide(name, env=dict(session.env, SE_SCREEN_TASK_WAYLAND="1"), remove=("DISPLAY",))
    return session.ok, session.reason


def fixture_kwin(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "nur unter Linux"
    ctx = fixtures.ctx
    program = str(ctx.cargo.test_exe) if ctx.cargo.build_lib() and ctx.cargo.test_exe else None
    session = helper("screen-task-linux").start_kwin(keep_logs(fixtures, fixtures.make_dir("kwin")), ctx.base_env(),
                                                     program)
    add_session(fixtures, session)
    if session.ok:
        fixtures.provide(name, env=dict(session.env, SE_SCREEN_TASK_KDE="1", SE_DESKTOP_TASK_KWIN="1"),
                         remove=("DISPLAY",))
    return session.ok, session.reason


def fixture_kde_portal(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    kwin = fixtures.state.get("kwin")
    if not kwin or not kwin[0]:
        return False, "KWin läuft nicht (Fixture kwin)"
    session = helper("screen-task-linux").start_kde_portal(keep_logs(fixtures, fixtures.make_dir("kde-portal")),
                                                           fixtures.ctx.base_env(), fixtures.envs.get("kwin", {}))
    add_session(fixtures, session)
    if session.ok:
        fixtures.provide(name)
    return session.ok, session.reason


def fixture_interactive_desktop(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    ok, reason = helper("screen-task-windows").interactive_desktop()
    if ok:
        fixtures.provide(name, env={"SE_SCREEN_TASK_WINDOWS": "1"})
    return ok, reason


def fixture_media_foundation(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    ctx = fixtures.ctx

    def run(command: list) -> tuple[int, str]:
        completed = ctx.run(command, stage=fixtures.stage, timeout=45 * 60)
        return (completed.code if completed.code is not None else -1), completed.output
    ok, reason = helper("screen-task-windows").media_foundation(run)
    if ok:
        fixtures.provide(name)
    return ok, reason


def fixture_elevated_window(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    ok, reason = helper("screen-task-windows").elevated_window()
    if ok:
        fixtures.provide(name, env={"SE_SCREEN_ELEVATED_TASK": "1"})
    return ok, reason


# --------------------------------------------------------------------------- stages

def stage_screen_pair(ctx: common.JobContext, stage: str) -> None:
    entries = ctx.entries(stage)
    netem = [entry for entry in entries if entry.name == NETEM]
    run_entries(ctx, stage, [entry for entry in entries if entry.name != NETEM])
    if netem:
        run_entries(ctx, stage, netem)  # last: the netem rule stays on lo until the stage ends


def window_list_twice(ctx: common.JobContext, stage: str, entry) -> None:
    """SCR-M5: the window list with an EWMH manager (openbox) and without any window manager."""
    started = time.monotonic()
    assert ctx.fixtures is not None
    ready, why = ctx.fixtures.ensure(entry.fixtures)
    if not ready:
        ctx.record(entry.name, stage, NOT_RUN, why)
        return
    display = ctx.fixtures.envs.get("xvfb", {}).get("DISPLAY", "")
    session = helper("screen-task-linux").start_openbox(display, ctx.fixtures.make_dir("openbox"), ctx.base_env())
    if session.ok:
        try:
            first = run_entry_once(ctx, stage, entry, {"SE_SCREEN_TASK_X11_WM": "ewmh"})
        finally:
            for process in session.processes:
                common.kill_tree(process)
    else:
        first = (NOT_RUN, session.reason, 0.0)
    second = run_entry_once(ctx, stage, entry, {"SE_SCREEN_TASK_X11_WM": "none"})
    parts = [f"ewmh (openbox): {common.WORDS[first[0]]}: {common.first_line(first[1])}",
             f"none (ohne Fenstermanager): {common.WORDS[second[0]]}: {common.first_line(second[1])}"]
    statuses = {first[0], second[0]}
    if FAILED in statuses:
        status = FAILED
    elif statuses == {PASSED}:
        status = PASSED
    else:
        status = NOT_RUN
    ctx.record(entry.name, stage, status, "; ".join(parts), time.monotonic() - started)


def stage_screen_x11(ctx: common.JobContext, stage: str) -> None:
    entries = ctx.entries(stage)
    fail_on_pin_mismatch(ctx, stage, entries)
    window = by_name(entries, WINDOW_LIST)
    run_entries(ctx, stage, [entry for entry in entries if entry is not window])
    if window is not None and not ctx.recorded(window.name):
        window_list_twice(ctx, stage, window)


def stage_screen_wayland(ctx: common.JobContext, stage: str) -> None:
    run_entries(ctx, stage, ctx.entries(stage))


def stage_kde(ctx: common.JobContext, stage: str) -> None:
    run_entries(ctx, stage, ctx.entries(stage))


def stage_screen_windows(ctx: common.JobContext, stage: str) -> None:
    entries = ctx.entries(stage)
    fail_on_pin_mismatch(ctx, stage, entries)
    lock = by_name(entries, LOCK_LAST)
    run_entries(ctx, stage, [entry for entry in entries if entry is not lock])
    if lock is not None:
        run_entries(ctx, stage, [lock])  # locks the workstation: always the last step of the job (D3)


def stage_screen_android(ctx: common.JobContext, stage: str) -> None:
    from . import launcher  # noqa: PLC0415 - shared instrumentation runner
    launcher.run_instr_methods(ctx, stage, ctx.entries(stage))


def device_setup(ctx: common.JobContext, device) -> None:
    lines = []
    for description, args in helper("screen-task-android").setup_commands():
        completed = device.adb_run(*args, timeout=120)
        lines.append(f"{description}: {completed.describe()}")
    ctx.log_line("device-setup", "screen: " + "; ".join(lines))


# --------------------------------------------------------------------------- checks

def check_installer_lists_screen_licenses(ctx: common.JobContext) -> tuple[str, str]:
    """SCR-M10 after W6: the installer ships the Cisco text and the Rust crate license file under
    ``licenses\\OpenH264`` and ``licenses\\Rust-Crates``, removes both on uninstall, and ships none of the
    license files W6 dropped."""
    if not common.INSTALLER.is_file():
        return FAILED, "native/installer.nsi fehlt"
    text = common.INSTALLER.read_text(encoding="utf-8", errors="replace")
    lines = [(number, line.strip()) for number, line in enumerate(text.splitlines(), 1)]
    uninstall = {number for number, _ in common.nsis_uninstall_lines(text)}
    problems = []
    if not CISCO_LICENSE.is_file():
        problems.append("third-party/openh264/BINARY_LICENSE.txt fehlt")
    for folder, source in (("openh264", "third-party\\openh264\\binary_license.txt"),
                           ("rust-crates", "third-party\\rust-crates\\third-party-licenses.txt")):
        out_dir = [number for number, line in lines if common.nsis_normal(line).startswith("setoutpath")
                   and f"\\licenses\\{folder}" in common.nsis_normal(line) and number not in uninstall]
        if not out_dir:
            problems.append(f"kein SetOutPath $INSTDIR\\licenses\\{folder} im Installationsteil")
            continue
        following = [line for number, line in lines if out_dir[0] < number <= out_dir[0] + 4]
        if not any(common.nsis_normal(line).startswith("file") and source in common.nsis_normal(line)
                   for line in following):
            problems.append(f"installer.nsi:{out_dir[0]}: File {source} fehlt nach SetOutPath")
        removed = [line for number, line in lines if number in uninstall and f"\\licenses\\{folder}" in common.nsis_normal(line)]
        if not any(common.nsis_normal(line).startswith("delete") for line in removed):
            problems.append(f"Deinstallation: kein Delete unter licenses\\{folder}")
        if not any(common.nsis_normal(line).startswith("rmdir") for line in removed):
            problems.append(f"Deinstallation: kein RMDir licenses\\{folder}")
    for number, line in lines:
        normal = common.nsis_normal(line)
        if any(dropped in normal for dropped in DROPPED_LICENSES) and not normal.startswith(";"):
            problems.append(f"installer.nsi:{number}: Lizenzdatei, die W6 gestrichen hat: {line[:120]}")
    if problems:
        return FAILED, "; ".join(problems) + " (C5, SCR-M10/W6; Paket U36)"
    return PASSED, ("Cisco-Text und THIRD-PARTY-LICENSES.txt werden installiert und bei der Deinstallation entfernt; "
                    "keine gestrichene Lizenzdatei")


def check_install_linux_kde_interfaces_line(ctx: common.JobContext) -> tuple[str, str]:
    """SCR-M5/C5: the heredoc that writes ``smart-explorer.desktop`` carries the KDE interfaces line."""
    if not INSTALL_LINUX.is_file():
        return FAILED, "install-linux.sh fehlt"
    lines = INSTALL_LINUX.read_text(encoding="utf-8", errors="replace").splitlines()
    for number, line in enumerate(lines):
        match = re.search(r"cat\s*>\s*\"?[^\"]*smart-explorer\.desktop\"?\s*<<-?\s*'?(\w+)'?", line)
        if not match:
            continue
        end = next((index for index in range(number + 1, len(lines)) if lines[index].strip() == match.group(1)), None)
        if end is None:
            return FAILED, f"install-linux.sh:{number + 1}: Heredoc {match.group(1)} endet nicht"
        body = [item.strip() for item in lines[number + 1:end]]
        if KDE_LINE in body:
            return PASSED, f"install-linux.sh:{number + 2 + body.index(KDE_LINE)}: {KDE_LINE} im Heredoc von smart-explorer.desktop"
        return FAILED, f"install-linux.sh:{number + 1}-{end + 1}: {KDE_LINE} fehlt im Heredoc (C5, SCR-M5; Paket U36)"
    return FAILED, "install-linux.sh schreibt smart-explorer.desktop nicht über einen Heredoc (C5)"


HELP_COMMANDS = re.compile(r"^\s{2}([a-z][a-z0-9-]*)\s{2,}", re.M)


def cli_commands(ctx: common.JobContext, se: Path, path: list[str], depth: int = 4) -> tuple[list[list[str]], list[str]]:
    """Every subcommand path below ``se <path>`` from clap's ``--help`` (``help`` excluded)."""
    env = ctx.base_env()
    home = ctx.work / "cli-help-home"
    home.mkdir(parents=True, exist_ok=True)
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"))
    completed = ctx.run([se, *path, "--help"], stage="docs-cli", env=env, timeout=120)
    if not completed.ok:
        return [], [f"se {' '.join(path)} --help: {completed.describe()}: {common.excerpt(completed.output, 300)}"]
    text = completed.output
    section = text.split("Commands:", 1)[1].split("\n\n", 1)[0] if "Commands:" in text else ""
    names = [name for name in HELP_COMMANDS.findall("\n" + section.strip("\n").replace("\r", "") + "\n")
             if name != "help"]
    if not names or depth == 0:
        return [path], []
    found, problems = [], []
    for name in names:
        more, issues = cli_commands(ctx, se, path + [name], depth - 1)
        found += more
        problems += issues
    return found, problems


def command_listed(doc: str, path: list[str]) -> bool:
    """Listed when ``se <path>`` stands in the document, or when a line naming ``se <top>`` carries every
    further word of the path inside its code spans (tables write ``se screen devices``, ``probe``, ...)."""
    flat = " ".join(doc.replace("`", " ").split())
    if f"se {' '.join(path)}" in flat:
        return True
    for line in doc.splitlines():
        if f"se {path[0]}" not in line.replace("`", ""):
            continue
        spans = " ".join(re.findall(r"`([^`]*)`", line))
        words = set(re.findall(r"[a-z0-9-]+", spans))
        if all(word in words for word in path[1:]):
            return True
    return False


def cli_docs_check(ctx: common.JobContext, top: str, doc_path: Path, package: str) -> tuple[str, str]:
    """``se <top> --help`` (all levels) against the subdocument: every command path is named there."""
    if not doc_path.is_file():
        return FAILED, f"{doc_path.relative_to(REPO)} fehlt (Paket {package})"
    se = ctx.cargo.bin_path("se")
    if se is None:
        return FAILED, ctx.cargo.bin_errors.get("se", "Bau von se gescheitert")
    commands, problems = cli_commands(ctx, se, [top])
    if problems:
        return FAILED, "; ".join(problems)
    doc = doc_path.read_text(encoding="utf-8", errors="replace")
    missing = [" ".join(path) for path in commands if not command_listed(doc, path)]
    if missing:
        return FAILED, (f"{len(missing)} von {len(commands)} Befehlen fehlen in {doc_path.relative_to(REPO)} "
                        f"(Paket {package}): " + ", ".join(f"se {item}" for item in missing))
    return PASSED, f"alle {len(commands)} Befehle von se {top} stehen in {doc_path.relative_to(REPO)}"


def check_docs_inventory_screen(ctx: common.JobContext) -> tuple[str, str]:
    """SCR-M11: ``se screen --help`` against docs/oberflaechen/bildschirm.md."""
    return cli_docs_check(ctx, "screen", SCREEN_DOC, "U42")


FIXTURES = {
    "netem": fixture_netem,
    "openh264": fixture_openh264,
    "h264-high": fixture_h264_high,
    "gpu-adapter": fixture_gpu_adapter,
    "wayland-portal": fixture_wayland_portal,
    "kwin": fixture_kwin,
    "kde-portal": fixture_kde_portal,
    "interactive-desktop": fixture_interactive_desktop,
    "media-foundation": fixture_media_foundation,
    "elevated-window": fixture_elevated_window,
}
STAGES = {
    "screen-pair": stage_screen_pair,
    "screen-x11": stage_screen_x11,
    "screen-wayland": stage_screen_wayland,
    "kde": stage_kde,
    "screen-windows": stage_screen_windows,
    "screen-android-device": stage_screen_android,
    "screen-android-api30": stage_screen_android,
}
CHECKS = {
    "screen_task_installer_lists_screen_licenses": check_installer_lists_screen_licenses,
    "screen_task_install_linux_kde_interfaces_line": check_install_linux_kde_interfaces_line,
    "screen_task_docs_inventory_lists_every_cli_command": check_docs_inventory_screen,
}
DEVICE_SETUP = device_setup
