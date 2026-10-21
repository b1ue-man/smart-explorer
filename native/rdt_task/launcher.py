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
    """One ``am instrument -w -r`` over ``targets`` (``a.B`` or ``a.B#method``)."""
    device = ctx.device
    assert device is not None and device.component
    command = ["shell", "am", "instrument", "-w", "-r"]
    for key, value in args:
        command += ["-e", key, value]
    command += ["-e", "class", ",".join(targets), device.component]
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


def run_phased(ctx: common.JobContext, stage: str, entry, fqn: str, base_args: list[tuple[str, str]]) -> None:
    started = time.monotonic()
    steps = tool().PHASES[entry.name]
    notes = []
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
        parsed, completed, _ = instrument(ctx, stage, [f"{fqn}#{entry.name}"], base_args + args, PHASE_TIMEOUT_S)
        status, reason = outcome(parsed, fqn, entry.name, completed)
        label = ", ".join(f"{key}={value}" for key, value in args) or "ohne Argumente"
        if status != PASSED:
            ctx.record(entry.name, stage, status, f"Phase {label}: {reason}" + (f" ({'; '.join(notes)})" if notes else ""),
                       time.monotonic() - started)
            return
        notes.append(f"Phase {label}: bestanden")
    ctx.record(entry.name, stage, PASSED, "; ".join(notes), time.monotonic() - started)


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


def device_setup(ctx: common.JobContext, device) -> None:
    holders = device.adb_run(*tool().home_holder_args(), timeout=120)
    holder = tool().parse_holder(holders.output) if holders.ok else None
    ctx.state["launcher_home_holder"] = holder
    ctx.log_line("device-setup", f"launcher: HOME-Rolle vor den Launcher-Tests: {holder or 'unbekannt'} "
                                 f"({holders.describe()})")


STAGES = {
    "launcher-android-device": stage_launcher_device,
    "launcher-android-api30": stage_launcher_device,
}
CHECKS: dict = {}
FIXTURES: dict = {}
DEVICE_SETUP = device_setup
