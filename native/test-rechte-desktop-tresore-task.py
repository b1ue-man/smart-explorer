#!/usr/bin/env python3
"""The one remote task suite of the batch "Rechte, se Desktop, Tresore" (plan.md D3).

Runs only on the remote runner (AGENTS.md); one call per job of
``.github/workflows/rechte-desktop-tresore-task.yml``::

    python native/test-rechte-desktop-tresore-task.py --job <JOB> --log-root "$RUNNER_TEMP/rdt-<JOB>"

Development profile only: the library tests of ``smart_explorer`` with exact name lists
(``cargo test --lib --manifest-path native/Cargo.toml -- <full paths> --exact``), ``se`` and
``smart_explorer`` where a stage needs them, the share server only for device pairs, the Android
library only for the emulator. Never ``--workspace``, ``--all-targets``, ``--release``, installer
or feed.

Jobs and stages (D3; owner module in ``native/rdt_task``; ``inventory`` closes every job):

  prepare               lock, audit, licenses, static                  (artifact rdt-prepare)
  linux-core            setup, unit, perm-pair, drive-cli, drive-devices, vault-backends,
                        vault-sync, vault-compare, vault-cli, vault-mcp, vault-interop,
                        vault-klartext, screen-pair (U41), docs-cli
  linux-root            setup, perm-linux-root, perm-linux-helper, drive-fuse, drive-vault,
                        vault-mount, vault-linux, vault-klartext-drive
  linux-2204            setup, perm-2204
  linux-desktop         setup, gui-harness, screen-x11 (U41), desktop-linux-x11 (U41), drive-desktop
  linux-wayland         setup, screen-wayland, desktop-linux-wayland, kde        (all U41)
  linux-gnome           setup, desktop-gnome                                      (U41)
  windows-core          setup, unit, perm-windows, drive-dokany-regression, drive-cli,
                        drive-vault, vault-unit, vault-windows, vault-mount
  windows-screen        setup, screen-windows                                     (U41)
  windows-desktop       setup, desktop-gui-harness, desktop-windows               (U41)
  desktop-bins          setup, desktop-bins                    (artifact rdt-desktop-bins)
  android-build         setup, apk, drive-android, vault-android, screen-jvm (U41), launcher-jvm (U41), speed-jvm (U54)
                        (artifact rdt-apks)
  android-device        device-setup, perm-android-device, drive-android-device,
                        vault-android-device, screen-android-device (U41),
                        launcher-android-device (U41), speed-android-device (U55), android-bestand
  android-device-api30  device-setup, perm-android-api30, screen-android-api30 (U41),
                        launcher-android-api30 (U41)
  linux-speed-net       setup, speed-ref, speed-net-large, speed-net-small, speed-raw, speed-ramp,
                        speed-compress, speed-limiter, speed-room, speed-cli          (U55)
  linux-speed-local     setup, speed-ref-local, speed-local                           (U55)
  windows-speed         setup, speed-windows                                          (U55)
  report                overview of every job; a job without report.json becomes "nicht ausgeführt"

The Android device jobs call this script twice: ``--phase device`` inside the emulator action,
then ``--phase verdict`` (inventory, summary, exit code) in a step that always runs.

Environment of the workflow input ``speed_scope`` (all | local | none): ``RDT_SKIP_JOB_REASON`` leaves a whole
speed job out (every entry "nicht ausgeführt" with that reason); ``RDT_SPEED_SCOPE`` is read by the stage
``speed-android-device`` of the job android-device (rdt_task/speed.py): it measures only for ``all``, otherwise its
entries are "nicht ausgeführt" with the reason of the user decision.

Each job writes ``report.json`` = ``{"job", "candidate_sha", "tests": [{"name", "stage", "status",
"reason", "duration_s"}], "stages": [...]}``, ``summary.md`` and the GitHub job summary. Statuses:
bestanden (passed), gescheitert (failed), nicht ausgeführt (not_run, always with a reason, never
counted as passed). Exit code 1 when a test or stage failed or rdt_suite_inventory_complete failed.
"""
from __future__ import annotations

import argparse
import importlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
import traceback
from pathlib import Path

NATIVE = Path(__file__).resolve().parent
sys.path.insert(0, str(NATIVE))

from rdt_task import common  # noqa: E402

C, PREPARE, LICENSES, PERM, DRIVE, VAULT = ("common",), ("prepare",), ("licenses",), ("perm",), ("drive",), ("vault",)
SCREEN, DESKTOP, LAUNCHER = ("screen",), ("desktop",), ("launcher",)
SPEED = ("common", "speed")
INVENTORY_STAGE = ("inventory", ())

JOBS: dict[str, list[tuple[str, tuple[str, ...]]]] = {
    "prepare": [("lock", PREPARE), ("audit", PREPARE), ("licenses", LICENSES), ("static", PREPARE), INVENTORY_STAGE],
    "linux-core": [("setup", C), ("unit", C), ("perm-pair", PERM), ("drive-cli", DRIVE), ("drive-devices", DRIVE),
                   ("vault-backends", VAULT), ("vault-sync", VAULT), ("vault-compare", VAULT), ("vault-cli", VAULT),
                   ("vault-mcp", VAULT), ("vault-interop", VAULT), ("vault-klartext", VAULT),
                   ("screen-pair", SCREEN), ("docs-cli", C), INVENTORY_STAGE],
    "linux-root": [("setup", C), ("perm-linux-root", PERM), ("perm-linux-helper", PERM), ("drive-fuse", DRIVE),
                   ("drive-vault", DRIVE), ("vault-mount", VAULT), ("vault-linux", VAULT),
                   ("vault-klartext-drive", VAULT), INVENTORY_STAGE],
    "linux-2204": [("setup", C), ("perm-2204", PERM), INVENTORY_STAGE],
    "linux-desktop": [("setup", C), ("gui-harness", C), ("screen-x11", SCREEN), ("desktop-linux-x11", DESKTOP),
                      ("drive-desktop", DRIVE), INVENTORY_STAGE],
    "linux-wayland": [("setup", C), ("screen-wayland", SCREEN), ("desktop-linux-wayland", DESKTOP),
                      ("kde", SCREEN + DESKTOP), INVENTORY_STAGE],
    "linux-gnome": [("setup", C), ("desktop-gnome", DESKTOP), INVENTORY_STAGE],
    "windows-core": [("setup", C), ("unit", C), ("perm-windows", PERM), ("drive-dokany-regression", DRIVE),
                     ("drive-cli", DRIVE), ("drive-vault", DRIVE), ("vault-unit", VAULT), ("vault-windows", VAULT),
                     ("vault-mount", VAULT), INVENTORY_STAGE],
    "windows-screen": [("setup", C), ("screen-windows", SCREEN), INVENTORY_STAGE],
    "windows-desktop": [("setup", C), ("desktop-gui-harness", DESKTOP), ("desktop-windows", DESKTOP),
                        INVENTORY_STAGE],
    "desktop-bins": [("setup", C), ("desktop-bins", C), INVENTORY_STAGE],
    "android-build": [("setup", C), ("apk", C), ("drive-android", DRIVE), ("vault-android", VAULT),
                      ("screen-jvm", SCREEN), ("launcher-jvm", LAUNCHER),
                      ("speed-jvm", SPEED), INVENTORY_STAGE],
    "android-device": [("device-setup", C), ("perm-android-device", PERM), ("drive-android-device", DRIVE),
                       ("vault-android-device", VAULT), ("screen-android-device", SCREEN),
                       ("launcher-android-device", LAUNCHER), ("speed-android-device", SPEED), ("android-bestand", C),
                       INVENTORY_STAGE],
    "android-device-api30": [("device-setup", C), ("perm-android-api30", PERM), ("screen-android-api30", SCREEN),
                             ("launcher-android-api30", LAUNCHER), INVENTORY_STAGE],
    "linux-speed-net": [("setup", C), ("speed-ref", SPEED), ("speed-net-large", SPEED), ("speed-net-small", SPEED),
                        ("speed-raw", SPEED), ("speed-ramp", SPEED), ("speed-compress", SPEED), ("speed-limiter", SPEED),
                        ("speed-room", SPEED), ("speed-cli", SPEED), INVENTORY_STAGE],
    "linux-speed-local": [("setup", C), ("speed-ref-local", SPEED), ("speed-local", SPEED), INVENTORY_STAGE],
    "windows-speed": [("setup", C), ("speed-windows", SPEED), INVENTORY_STAGE],
}
STAGE_TABLE = {job: [stage for stage, _ in plan] for job, plan in JOBS.items()}
EXPECTED_API = {"android-device": 36, "android-device-api30": 30}
MODULE_NAMES = ("common", "prepare", "licenses", "perm", "drive", "vault", "screen", "desktop", "launcher", "speed")
EMULATOR_MARKER = ".rdt-emulator-started"


class BrokenModule:
    """An rdt_task module that exists but cannot be imported; its stages fail with the error."""

    def __init__(self, name: str, error: str):
        self.name = name
        self.load_error = error


def load_modules() -> dict:
    modules: dict = {}
    for name in MODULE_NAMES:
        if importlib.util.find_spec(f"rdt_task.{name}") is None:
            modules[name] = None
            continue
        try:
            modules[name] = importlib.import_module(f"rdt_task.{name}")
        except Exception as error:  # noqa: BLE001 - reported by the stages the module owns
            modules[name] = BrokenModule(name, f"{type(error).__name__}: {error}\n{traceback.format_exc(limit=4)}")
    return modules


def stage_hook(ctx: common.JobContext, stage: str, owners: tuple[str, ...]):
    """(hook, missing owners, broken module) for a stage."""
    modules = [ctx.modules.get(owner) for owner in owners]
    present = [module for module in modules if module is not None and not isinstance(module, BrokenModule)]
    broken = [module for module in modules if isinstance(module, BrokenModule)]
    if not present:
        return None, (owners if not broken else ()), (broken[0] if broken else None)
    for module in present:
        hook = getattr(module, "STAGES", {}).get(stage)
        if hook:
            return hook, (), None
    return None, (), None


def execute_stage(ctx: common.JobContext, stage: str, owners: tuple[str, ...]) -> None:
    started = time.monotonic()
    ctx.current_stage = stage
    ctx.fixtures = common.StageFixtures(ctx, stage)
    error = None
    print(f"\n==== Stufe {stage} ({', '.join(owners) or '-'})", flush=True)
    try:
        hook, missing, broken = stage_hook(ctx, stage, owners)
        if broken is not None:
            raise RuntimeError(f"rdt_task/{broken.name}.py ist nicht ladbar: {broken.load_error}")
        if missing:
            ctx.stage_not_implemented(stage, missing)
        elif hook is not None:
            hook(ctx, stage)
        else:
            ctx.run_default(stage)
    except Exception:  # noqa: BLE001 - the stage fails, the job goes on with the next stage
        error = traceback.format_exc()
        print(error, flush=True)
    finally:
        try:
            ctx.fixtures.close()
        except Exception as close_error:  # noqa: BLE001
            ctx.log_line(stage, f"fixture close failed: {close_error}")
        finish_stage(ctx, stage, started, error)
        ctx.current_stage = None


def finish_stage(ctx: common.JobContext, stage: str, started: float, error: str | None) -> None:
    for entry in ctx.entries(stage):
        if entry.name != common.INVENTORY and not ctx.recorded(entry.name):
            reason = (f"Stufe abgebrochen: {common.first_line(error.strip().splitlines()[-1])}" if error
                      else "kein Ergebnis aus der Stufe (Stufenfunktion hat den Eintrag nicht ausgeführt)")
            ctx.record(entry.name, stage, common.FAILED, reason + (f"\n{error[-2500:]}" if error else ""))
    tests = [test for test in ctx.report.tests if test["stage"] == stage]
    counts = ctx.report.counts(stage)
    note = ctx.state.get("stage_notes", {}).get(stage, "")
    if error:
        status, reason = common.FAILED, "Abbruch: " + common.first_line(error.strip().splitlines()[-1])
    elif counts[common.FAILED]:
        status, reason = common.FAILED, f"{counts[common.FAILED]} gescheitert"
    elif counts[common.PASSED]:
        status = common.PASSED
        reason = f"{counts[common.PASSED]} bestanden" + (
            f", {counts[common.NOT_RUN]} nicht ausgeführt (Gründe je Eintrag)" if counts[common.NOT_RUN] else "")
    elif tests:
        reasons = []
        for test in tests:
            line = common.first_line(test["reason"])
            if line not in reasons:
                reasons.append(line)
        status, reason = common.NOT_RUN, "alle Einträge nicht ausgeführt: " + " | ".join(reasons[:3])
    else:
        status, reason = common.NOT_RUN, "keine Einträge für diese Stufe in expected_signals.txt"
    if note:
        reason = f"{reason}; {note}" if reason else note
    ctx.report.stage_done(stage, status, reason, time.monotonic() - started)


def run_inventory(ctx: common.JobContext) -> None:
    started = time.monotonic()
    ctx.current_stage = "inventory"
    try:
        problems, notes = common.inventory_job(ctx)
        if ctx.job == "prepare":
            more, more_notes = common.inventory_sources(ctx)
            problems += more
            notes += more_notes
    except Exception:  # noqa: BLE001 - an inventory that cannot run has failed
        problems, notes = [traceback.format_exc()], []
    expected = [entry for entry, _ in ctx.signals.for_job(ctx.job) if entry.name != common.INVENTORY]
    status = common.FAILED if problems else common.PASSED
    if problems:
        reason = f"{len(problems)} Befunde:\n" + "\n".join(problems[:80])
    else:
        reason = f"alle {len(expected)} erwarteten Einträge des Jobs mit genau einem Ergebnis, keine unerwarteten Namen"
    if notes:
        reason += "\nHinweise:\n" + "\n".join(notes[:40])
    duration = time.monotonic() - started
    if any(entry.name == common.INVENTORY for entry, _ in ctx.signals.for_job(ctx.job)):
        ctx.record(common.INVENTORY, "inventory", status, reason, duration)
    ctx.report.stage_done("inventory", status, common.first_line(reason), duration)
    ctx.current_stage = None


def candidate_sha() -> str:
    value = os.environ.get("CANDIDATE_SHA", "").strip()
    if value:
        return value.lower()
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=common.REPO, capture_output=True, text=True, check=False)
    return head.stdout.strip()


def main() -> int:
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except AttributeError:
        pass
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--job", required=True, choices=sorted(list(JOBS) + ["report"]))
    parser.add_argument("--log-root", required=True)
    parser.add_argument("--phase", choices=("all", "device", "verdict"), default="all",
                        help="Android device jobs: 'device' inside the emulator action, then 'verdict'")
    parser.add_argument("--out", help="prepare: folder of the artifact rdt-prepare; desktop-bins: binaries; "
                                      "android-build: APKs")
    parser.add_argument("--apks", help="device jobs: folder of the downloaded artifact rdt-apks")
    parser.add_argument("--bins", help="device jobs: folder of the downloaded artifact rdt-desktop-bins")
    parser.add_argument("--api-level", type=int, help="device jobs: expected emulator API level")
    parser.add_argument("--reports", help="report: folder with the downloaded rdt-report-* artifacts")
    parser.add_argument("--work", help="working folder outside the uploaded log folder")
    args = parser.parse_args()
    if args.api_level is None:
        args.api_level = EXPECTED_API.get(args.job)
    log_root = Path(args.log_root).resolve()
    log_root.mkdir(parents=True, exist_ok=True)
    work = Path(args.work) if args.work else Path(os.environ.get("RUNNER_TEMP") or tempfile.gettempdir()) / f"rdt-work-{args.job}"
    signals = common.Signals.load(STAGE_TABLE)
    modules = load_modules()
    plan = JOBS.get(args.job, [])
    ctx = common.JobContext(args.job, plan, log_root, work.resolve(), candidate_sha(), args, signals, modules,
                            resume=(args.phase == "verdict"))
    print(f"rechte-desktop-tresore-task · Job {args.job} · Phase {args.phase} · Kandidat {ctx.candidate_sha}", flush=True)

    if args.job == "report":
        needs = json.loads(os.environ.get("RDT_NEEDS_JSON") or "{}")
        reports = Path(args.reports) if args.reports else log_root / "reports"
        common.aggregate_reports(ctx, reports, [job for job in JOBS], needs)
        return 1 if ctx.state.get("overall_failing") else 0

    stages = [(stage, owners) for stage, owners in plan if stage != "inventory"]
    if args.phase == "device":
        marker = Path(os.environ.get("GITHUB_WORKSPACE") or common.REPO) / EMULATOR_MARKER
        marker.write_text(ctx.candidate_sha + "\n", encoding="utf-8")
        for stage, owners in stages:
            execute_stage(ctx, stage, owners)
        return 0
    # A job the dispatch left out (input speed_scope): every entry is "nicht ausgeführt" with the reason.
    skip = (os.environ.get("RDT_SKIP_JOB_REASON") or "").strip()
    if skip and args.job != "prepare":
        for stage, _ in stages:
            ctx.not_run_all(stage, skip)
            ctx.report.stage_done(stage, common.NOT_RUN, skip, 0.0)
    elif args.phase == "verdict":
        if not any(stage["name"] == "device-setup" for stage in ctx.report.stages):
            reason = (f"Emulator startete nicht (reactivecircus/android-emulator-runner, API {args.api_level}"
                      + (" und Rückfall API 35" if args.job == "android-device" else "") + ")")
            for stage, _ in stages:
                ctx.not_run_all(stage, reason)
                ctx.report.stage_done(stage, common.NOT_RUN, reason, 0.0)
    else:
        try:
            for stage, owners in stages:
                execute_stage(ctx, stage, owners)
        finally:
            if args.job == "prepare":
                from rdt_task import prepare  # noqa: PLC0415
                prepare.export_artifact(ctx, Path(args.out) if args.out else ctx.work / "rdt-prepare-out")
    run_inventory(ctx)
    ctx.report.write()
    ctx.report.publish_summary()
    return 1 if ctx.report.failed() else 0


if __name__ == "__main__":
    sys.exit(main())
