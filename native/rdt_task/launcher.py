"""se Launcher (S5 Android) in the batch suite: device stages and the shared method-level instrumentation
runner (plan.md D3; strang-desktop.md DSK-M9 to DSK-M13, DSK-M15; package U41).

Stages (``STAGES``):
  launcher-jvm             android-build: no function here; the generic stage reads the Gradle JUnit reports
                           (``LayoutModelTest``, ``GridGeometryTest``, ``LauncherSearchTest``, ``WidgetPreviewTest``,
                           ``launcher_task_*`` JVM methods ...).
  launcher-android-device  android-device (API 36): every ``launcher_task_`` instrumentation method of the list,
                           phased methods with their host steps (``native/launcher-task-android.py`` ``PHASES``),
                           the helper app for ``launcher_task_package_removed_clears_items`` when the artifact
                           carries it; at the end the image's HOME role holder is restored.
  launcher-android-api30   android-device-api30: the same for the API 30 entries.

``run_instr_methods`` runs instrumentation entries method by method (``-e class a.B#m1,a.B#m2``) instead of
whole classes, so an API 30 job runs only its entries and a stage never repeats another stage's methods.
A method whose assumption fails (``assumeTrue``, status -4) is "nicht ausgeführt" with the test's reason;
a missing emulator, a missing APK or a missing fixture is "nicht ausgeführt" with that reason.
"""
from __future__ import annotations

import re
import shlex
import time
from pathlib import Path

from . import common, screen
from .common import FAILED, NOT_RUN, PASSED

PACKAGE = "app.smartexplorer.android"
GROUP_TIMEOUT_S = 90 * 60
PHASE_TIMEOUT_S = 20 * 60


def tool():
    return screen.helper("launcher-task-android")


def instrument(ctx: common.JobContext, stage: str, targets: list[str], args: list[tuple[str, str]],
               timeout: float) -> tuple[dict, common.Completed, float]:
    """One ``am instrument -w -r`` over ``targets`` (``a.B`` or ``a.B#method``).

    ``adb shell`` joins its words with spaces and the device's shell parses the result again, so every word is quoted: a phase value
    like ``(kein Text)`` is a syntax error for ``/system/bin/sh`` otherwise (suite run 7, ``launcher_task_no_bind_when_main_process_absent``).
    """
    device = ctx.device
    assert device is not None and device.component
    command = ["shell", "am", "instrument", "-w", "-r"]
    for key, value in args:
        command += ["-e", shlex.quote(key), shlex.quote(value)]
    command += ["-e", "class", shlex.quote(",".join(targets)), shlex.quote(device.component)]
    started = time.monotonic()
    completed = ctx.run([device.adb, *command], stage=stage, timeout=timeout)
    return common.parse_instrumentation(completed.output), completed, time.monotonic() - started


def outcome(parsed: dict, fqn: str, method: str | None, completed: common.Completed) -> tuple[str, str]:
    if method is None:
        items = [(f"{cls}#{name}", status, detail, 0.0) for (cls, name), (status, detail) in parsed.items()
                 if cls == fqn]
        if not items:
            return FAILED, f"keine Ergebnisse für {fqn}: {completed.describe()}: " + common.excerpt(completed.output, 1200)
        status, reason, _ = common.aggregate(items)
        return status, reason
    result = parsed.get((fqn, method))
    if result is None:
        return FAILED, (f"kein Ergebnis für {fqn}#{method}: {completed.describe()}: "
                        + common.excerpt(completed.output, 1200))
    status, detail = result
    return status, detail or f"{fqn}#{method}"


def resolve(ctx: common.JobContext, entry) -> tuple[str | None, str | None]:
    index = ctx.device.index if ctx.device else common.kotlin_index(common.ANDROID_TEST_PACKAGE_ROOTS)
    if entry.is_class:
        return index.classes.get(entry.name), None
    owners = index.methods.get(entry.name)
    return (owners[0], entry.name) if owners else (None, None)


def ui_dump(device) -> str:
    """The UI hierarchy of the screen. ``uiautomator dump`` answers "null root node returned by UiTestAutomationBridge" while no window
    is ready to be read (right after the Home key, with the launcher starting cold), so it is asked again a few times."""
    last = ""
    for _ in range(6):
        dump = device.adb_run("exec-out", "uiautomator", "dump", "/dev/tty", timeout=120)
        last = dump.output
        if "<hierarchy" in last:
            return last
        time.sleep(3)
    return last


def observe(ctx: common.JobContext, kind: str) -> tuple[bool, str, dict]:
    """Host steps between two phases; returns (ok, note, observations)."""
    device = ctx.device
    assert device is not None
    if kind == "pair-down":
        root = ctx.state.get("se_pair_root")
        if not root:
            return False, "kein Share-Paar aktiv", {}
        down = ctx.run(["bash", common.REPO / tool().SHARE_HELPER, "down", root, ctx.logs / "se-pair-down"],
                       stage=ctx.current_stage, timeout=300)
        ctx.state["se_pair_root"] = None
        return down.ok, f"Share des Runners abgemeldet ({down.describe()})", {}
    device.adb_run("shell", "am", "kill", PACKAGE, timeout=60)
    time.sleep(2)
    observed: dict[str, str] = {}
    if kind == "kill-home-observe":
        device.adb_run("shell", "input", "keyevent", "KEYCODE_HOME", timeout=60)
        time.sleep(5)
        observed["tile"] = tool().tile_text(ui_dump(device))
    elif kind == "kill-widget-update":
        widget = device.adb_run("shell", "run-as", PACKAGE, "cat", tool().WIDGET_ID_FILE, timeout=60)
        widget_id = widget.output.strip().splitlines()[-1].strip() if widget.ok and widget.output.strip() else ""
        if not widget_id.isdigit():
            return False, f"Widget-Kennung nicht lesbar ({widget.describe()}): {common.excerpt(widget.output, 200)}", {}
        device.adb_run("shell", "am", "broadcast", "-a", "android.appwidget.action.APPWIDGET_UPDATE", "-n",
                       PACKAGE + "/" + tool().WIDGET_PROVIDER, "--eia", "appWidgetIds", widget_id, timeout=60)
        time.sleep(5)
    else:
        return False, f"unbekannter Schritt {kind}", {}
    pid = device.adb_run("shell", "pidof", PACKAGE, timeout=60).output.strip()
    observed["pid"] = pid if pid.isdigit() else "none"
    return True, f"{kind}: " + ", ".join(f"{key}={value}" for key, value in observed.items()), observed


def run_phased(ctx: common.JobContext, stage: str, entry, fqn: str, base_args: list[tuple[str, str]]) -> None:
    started = time.monotonic()
    steps = tool().PHASES[entry.name]
    notes: list[str] = []
    observed: dict[str, str] = {}
    for kind, args in steps:
        if kind == "reinstall-app":
            apk = tool().app_apk(Path(ctx.args.apks) if ctx.args.apks else None)
            if apk is None:
                ctx.record(entry.name, stage, NOT_RUN, "App-APK fehlt im Artefakt rdt-apks; Ersetzen nicht möglich")
                return
            assert ctx.device is not None
            installed = ctx.device.adb_run("install", "-r", "-t", "-g", apk, timeout=900)
            if not installed.ok or "Success" not in installed.output:
                ctx.record(entry.name, stage, FAILED, f"adb install -r {apk.name}: {installed.describe()}: "
                           + common.excerpt(installed.output, 600), time.monotonic() - started)
                return
            notes.append(f"adb install -r {apk.name}: Success")
            continue
        if kind != "run":
            ok, note, values = observe(ctx, kind)
            if not ok:
                ctx.record(entry.name, stage, FAILED, f"Wirtsschritt {kind}: {note}" +
                           (f" ({'; '.join(notes)})" if notes else ""), time.monotonic() - started)
                return
            observed.update(values)
            notes.append(note)
            continue
        filled = [(key, value.format(**observed) if "{" in value else value) for key, value in args]
        parsed, completed, _ = instrument(ctx, stage, [f"{fqn}#{entry.name}"], base_args + filled, PHASE_TIMEOUT_S)
        status, reason = outcome(parsed, fqn, entry.name, completed)
        label = ", ".join(f"{key}={value}" for key, value in filled) or "ohne Argumente"
        if status != PASSED:
            ctx.record(entry.name, stage, status, f"Phase {label}: {reason}" + (f" ({'; '.join(notes)})" if notes else ""),
                       time.monotonic() - started)
            return
        notes.append(f"Phase {label}: bestanden")
    ctx.record(entry.name, stage, PASSED, "; ".join(notes), time.monotonic() - started)


def fixture_se_pair(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    """D 682: runner ↔ emulator Share pair through android/test-servers/share-desktop.sh (Linux)."""
    ctx = fixtures.ctx
    if not common.LINUX:
        return False, "das Share-Paar läuft nur auf dem Linux-Runner"
    bins = Path(ctx.state.get("desktop_bins_dir") or "")
    se, server = bins / "se", bins / "se-share-server"
    if not se.is_file() or not server.is_file():
        return False, f"se oder se-share-server fehlt im Artefakt rdt-desktop-bins ({bins})"
    helper = common.REPO / tool().SHARE_HELPER
    root = fixtures.make_dir("se-pair")
    up = ctx.run(["bash", helper, "up", root, se, server], stage=fixtures.stage, timeout=900)
    if not up.ok:
        return False, f"share-desktop.sh up: {up.describe()}: {common.excerpt(up.output, 400)}"
    ctx.state["se_pair_root"] = str(root)

    def down() -> None:
        if ctx.state.get("se_pair_root"):
            ctx.run(["bash", helper, "down", root, ctx.logs / "se-pair-down"], stage=fixtures.stage, timeout=300)
            ctx.state["se_pair_root"] = None
    fixtures.add_cleanup(down)
    listed = ctx.run(["bash", helper, "args", root], stage=fixtures.stage, timeout=120)
    words = listed.output.strip().splitlines()
    if not listed.ok or len(words) % 3 or any(words[index] != "-e" for index in range(0, len(words), 3)):
        return False, f"share-desktop.sh args lieferte keine Argumente ({listed.describe()})"
    args = [(words[index + 1], words[index + 2]) for index in range(0, len(words), 3)]
    fixtures.provide(name, args=args)
    return True, "Share-Paar: " + ", ".join(key for key, _ in args)


def method_args(ctx: common.JobContext) -> dict[str, list[tuple[str, str]]]:
    """Per-method instrumentation arguments the host provides (helper app of DSK-M10)."""
    cached = ctx.state.get("launcher_method_args")
    if cached is not None:
        return cached
    result: dict[str, list[tuple[str, str]]] = {}
    found = tool().helper_apk(Path(ctx.args.apks) if ctx.args.apks else None)
    if found and ctx.device is not None:
        apk, package = found
        target = f"{tool().DEVICE_TMP}/{tool().HELPER_APK}"
        pushed = ctx.device.adb_run("push", apk, target, timeout=300)
        if pushed.ok:
            result["launcher_task_package_removed_clears_items"] = [("launcher_helper_apk", target),
                                                                    ("launcher_helper_package", package)]
        else:
            ctx.log_line("device-setup", f"launcher: adb push {apk.name}: {pushed.describe()}")
    ctx.state["launcher_method_args"] = result
    return result


def run_instr_methods(ctx: common.JobContext, stage: str, entries: list) -> None:
    """Instrumentation entries of a stage, method by method; non-instrumentation entries via run_entries."""
    other = [entry for entry in entries if entry.kind != "instr"]
    if other:
        screen.run_entries(ctx, stage, other)
    plain: dict[tuple, list[tuple]] = {}
    phased: list[tuple] = []
    extra = method_args(ctx) if ctx.device is not None else {}
    for entry in entries:
        if entry.kind != "instr" or ctx.recorded(entry.name):
            continue
        fqn, method = resolve(ctx, entry)
        if not fqn:
            if entry.pending:
                ctx.record(entry.name, stage, NOT_RUN, entry.pending_reason())
            else:
                ctx.record(entry.name, stage, FAILED, "fehlt in android/app/src/androidTest")
            continue
        if ctx.device is None:
            ctx.record(entry.name, stage, NOT_RUN, "Emulator nicht bereit: " + ctx.state.get("device_reason", "Stufe device-setup lief nicht"))
            continue
        assert ctx.fixtures is not None
        ready, why = ctx.fixtures.ensure(entry.fixtures)
        if not ready:
            ctx.record(entry.name, stage, NOT_RUN, why)
            continue
        args = ctx.fixtures.args_for(entry.fixtures) + extra.get(entry.name, [])
        if method and method in tool().PHASES:
            phased.append((entry, fqn, args))
        else:
            plain.setdefault(tuple(args), []).append((entry, fqn, method))
    for args, items in plain.items():
        targets = list(dict.fromkeys(fqn if method is None else f"{fqn}#{method}" for _, fqn, method in items))
        parsed, completed, duration = instrument(ctx, stage, targets, list(args), GROUP_TIMEOUT_S)
        share = duration / max(len(items), 1)
        for entry, fqn, method in items:
            status, reason = outcome(parsed, fqn, method, completed)
            ctx.record(entry.name, stage, status, reason, share)
    order = list(tool().PHASES)
    phased.sort(key=lambda item: order.index(item[0].name))  # pair-down last
    for entry, fqn, args in phased:
        run_phased(ctx, stage, entry, fqn, args)


def restore_home(ctx: common.JobContext, stage: str) -> None:
    holder = ctx.state.get("launcher_home_holder")
    if ctx.device is None or not holder or holder.startswith(PACKAGE):
        return
    restored = ctx.device.adb_run(*tool().restore_home_args(holder), timeout=120)
    ctx.device.adb_run("shell", "input", "keyevent", "KEYCODE_HOME", timeout=60)
    note = f"HOME-Rolle an {holder} zurückgegeben ({restored.describe()})"
    notes = ctx.state.setdefault("stage_notes", {})
    notes[stage] = f"{notes[stage]}; {note}" if notes.get(stage) else note


def stage_launcher_device(ctx: common.JobContext, stage: str) -> None:
    try:
        run_instr_methods(ctx, stage, ctx.entries(stage))
    finally:
        restore_home(ctx, stage)


def holder_from_dumpsys(text: str) -> str | None:
    """The holder of the HOME role from ``dumpsys role``. Android 11 has no ``cmd role get-role-holders`` (it answers "Unknown command";
    ``RoleManagerShellCommand`` knows only add-role-holder, remove-role-holder and clear-role-holders); the dump lists the roles as
    ``name=<role>`` followed by ``holders=<package>`` (one holder) or ``holders=[`` and the packages (several)."""
    lines = [line.strip() for line in text.splitlines()]
    for index, line in enumerate(lines):
        if line != "name=android.app.role.HOME":
            continue
        for offset, follow in enumerate(lines[index + 1:], start=index + 1):
            if follow.startswith("}") or follow.startswith("name="):
                return None
            single = re.fullmatch(r"holders=([A-Za-z0-9_.]+)", follow)
            if single:
                return single.group(1)
            if follow == "holders=[":
                first = lines[offset + 1] if offset + 1 < len(lines) else ""
                return first if re.fullmatch(r"[A-Za-z0-9_.]+", first) else None
    return None


def device_setup(ctx: common.JobContext, device) -> None:
    holders = device.adb_run(*tool().home_holder_args(), timeout=120)
    holder = tool().parse_holder(holders.output) if holders.ok else None
    how = holders.describe()
    if holder is None and not holders.ok:
        dumped = device.adb_run("shell", "dumpsys", "role", timeout=120)
        holder = holder_from_dumpsys(dumped.output) if dumped.ok else None
        how = f"{holders.describe()}; dumpsys role: {dumped.describe()}"
    ctx.state["launcher_home_holder"] = holder
    ctx.log_line("device-setup", f"launcher: HOME-Rolle vor den Launcher-Tests: {holder or 'unbekannt'} ({how})")


STAGES = {
    "launcher-android-device": stage_launcher_device,
    "launcher-android-api30": stage_launcher_device,
}
CHECKS: dict = {}
FIXTURES = {"se-pair": fixture_se_pair}
DEVICE_SETUP = device_setup
