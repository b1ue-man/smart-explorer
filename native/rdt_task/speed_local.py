"""Drives and local copies of the stage family ``speed`` (U55-6, M-3 to M-5): the two volumes, the XFS loop
device for the clone case, and the stages ``speed-ref-local``, ``speed-local`` (Linux) and ``speed-windows``.

The case names are written out here because the inventory (``art=skript``) looks for them in this file.
"""
from __future__ import annotations

import os
import shutil
from pathlib import Path

from . import common
from .common import LINUX, NOT_RUN, WINDOWS, excerpt

REF_LOCAL = ["speed_task_ref_tools_present", "speed_task_ref_fio_volumes", "speed_task_ref_fio_small_files"]
LOCAL_SAME = ["speed_task_local_large_same_volume", "speed_task_local_small_same_volume"]
LOCAL_TWO = ["speed_task_local_large_two_volumes", "speed_task_local_small_two_volumes"]
LOCAL_XFS = ["speed_task_local_reflink_xfs"]
WINDOWS_CASES = ["speed_task_windows_local_large_same_volume", "speed_task_windows_local_large_two_volumes",
                 "speed_task_windows_local_small_two_volumes", "speed_task_windows_pdh_physicaldisk_counters_exist"]
XFS_IMAGE_BYTES = 20 * 1024 ** 3


def volumes(ctx: common.JobContext, stage: str) -> tuple[Path, Path, bool, str]:
    """(fast folder, second folder, both on one device, note). The fast one is on the runner's temporary disk
    (``/mnt``, ``D:``), the second on the system disk; without a second disk both are the same and the
    "two volumes" cases are not run."""
    if "speed_volumes" in ctx.state:
        return ctx.state["speed_volumes"]
    slow = ctx.work / "speed-slow"
    slow.mkdir(parents=True, exist_ok=True)
    fast, note = None, ""
    if WINDOWS:
        drive = Path("D:/")
        if drive.exists():
            fast = drive / "rdt-speed"
            fast.mkdir(parents=True, exist_ok=True)
    elif LINUX:
        fast = Path("/mnt/rdt-speed")
        if not fast.is_dir() and ctx.sudo_ok():
            ctx.run(["sudo", "-n", "mkdir", "-p", fast], stage=stage, timeout=60)
            ctx.run(["sudo", "-n", "chown", f"{os.getuid()}:{os.getgid()}", fast], stage=stage, timeout=60)
        if not (fast.is_dir() and os.access(fast, os.W_OK)):
            fast = None
    if fast is None:
        fast, note = ctx.work / "speed-fast", "kein zweites Laufwerk (/mnt oder D: fehlt oder ist nicht beschreibbar)"
        fast.mkdir(parents=True, exist_ok=True)
    same = LINUX and os.stat(fast).st_dev == os.stat(slow).st_dev
    if WINDOWS:
        same = fast.drive.upper() == slow.drive.upper()
    note = note or ("beide Ordner liegen auf demselben Laufwerk" if same else f"{fast} und {slow} auf zwei Laufwerken")
    ctx.state["speed_volumes"] = (fast, slow, bool(same), note)
    return ctx.state["speed_volumes"]


def xfs_volume(ctx: common.JobContext, stage: str, fast: Path) -> tuple[Path | None, str]:
    """An XFS file system with reflinks on a loop device (M-5 clone row); the cleanup unmounts it."""
    missing = [tool for tool in ("mkfs.xfs", "losetup", "mount", "umount", "truncate") if not shutil.which(tool)]
    if not LINUX or missing or not ctx.sudo_ok():
        return None, "XFS-Schleifengerät nicht möglich: " + (", ".join(missing) + " fehlt" if missing else
                                                              "nur Linux mit sudo -n")
    image, mount = fast / "xfs.img", fast / "xfs-mount"
    mount.mkdir(parents=True, exist_ok=True)
    ctx.run(["truncate", "-s", str(XFS_IMAGE_BYTES), image], stage=stage, timeout=60)
    loop = ctx.run(["sudo", "-n", "losetup", "--find", "--show", image], stage=stage, timeout=60)
    device = loop.output.strip().splitlines()[-1] if loop.ok and loop.output.strip() else ""
    if not device:
        return None, f"losetup scheitert: {excerpt(loop.output, 300)}"

    def cleanup() -> None:
        ctx.run(["sudo", "-n", "umount", mount], stage=stage, timeout=120)
        ctx.run(["sudo", "-n", "losetup", "-d", device], stage=stage, timeout=60)
        image.unlink(missing_ok=True)
    ctx.fixtures.add_cleanup(cleanup)
    made = ctx.run(["sudo", "-n", "mkfs.xfs", "-m", "reflink=1", "-f", device], stage=stage, timeout=300)
    mounted = ctx.run(["sudo", "-n", "mount", device, mount], stage=stage, timeout=120) if made.ok else made
    if not (made.ok and mounted.ok):
        return None, f"mkfs.xfs/mount scheitert: {excerpt(mounted.output, 300)}"
    ctx.run(["sudo", "-n", "chown", f"{os.getuid()}:{os.getgid()}", mount], stage=stage, timeout=60)
    return mount, f"XFS mit reflink=1 auf {device}"


def prepare(ctx: common.JobContext, stage: str, names: list[str]):
    """Build once, attach the table; returns the module of the runner or records every name as failed."""
    from . import speed_run as speed  # noqa: PLC0415 - speed_run imports this module
    error = speed.prepare(ctx)
    if error:
        for name in names:
            ctx.record(name, stage, common.FAILED, error)
        return None
    return speed


def stage_ref_local(ctx: common.JobContext, stage: str) -> None:
    fast, slow, same, note = volumes(ctx, stage)
    ctx.state.setdefault("stage_notes", {})[stage] = note
    speed = prepare(ctx, stage, REF_LOCAL)
    if speed is None:
        return
    env = speed.stage_env(ctx, stage, {"SE_SPEED_DIR_FAST": str(fast), "SE_SPEED_DIR_SLOW": str(slow),
                                       "SE_SPEED_TOOLS_REQUIRED": "fio,rsync,cp,sudo"})
    speed.run_and_record(ctx, stage, REF_LOCAL, env)
    speed.collect_tools(ctx)


def stage_local(ctx: common.JobContext, stage: str) -> None:
    fast, slow, same, note = volumes(ctx, stage)
    ctx.state.setdefault("stage_notes", {})[stage] = note
    speed = prepare(ctx, stage, LOCAL_SAME + LOCAL_TWO + LOCAL_XFS)
    if speed is None:
        return
    names = list(LOCAL_SAME)
    if same:
        for name in LOCAL_TWO:
            ctx.record(name, stage, NOT_RUN, f"zwei Laufwerke nicht vorhanden: {note}")
    else:
        names += LOCAL_TWO
    extra = {"SE_SPEED_DIR_FAST": str(fast), "SE_SPEED_DIR_SLOW": str(slow)}
    xfs, why = xfs_volume(ctx, stage, fast)
    if xfs:
        names += LOCAL_XFS
        extra["SE_SPEED_XFS_DIR"] = str(xfs)
    else:
        ctx.record(LOCAL_XFS[0], stage, NOT_RUN, why)
    speed.run_and_record(ctx, stage, names, speed.stage_env(ctx, stage, extra))


def stage_windows(ctx: common.JobContext, stage: str) -> None:
    """Reference cases, then the Windows cases, in one run (``ref`` sorts before ``windows``)."""
    fast, slow, same, note = volumes(ctx, stage)
    ctx.state.setdefault("stage_notes", {})[stage] = note + ("; fio fehlt" if not shutil.which("fio") else "")
    names = REF_LOCAL + WINDOWS_CASES
    speed = prepare(ctx, stage, names)
    if speed is None:
        return
    if same:
        ctx.record(WINDOWS_CASES[1], stage, NOT_RUN, f"zwei Laufwerke nicht vorhanden: {note}")
        ctx.record(WINDOWS_CASES[2], stage, NOT_RUN, f"zwei Laufwerke nicht vorhanden: {note}")
        names = [name for name in names if name not in WINDOWS_CASES[1:3]]
    env = speed.stage_env(ctx, stage, {"SE_SPEED_DIR_FAST": str(fast), "SE_SPEED_DIR_SLOW": str(slow),
                                       "SE_SPEED_TOOLS_REQUIRED": "fio,robocopy"})
    speed.run_and_record(ctx, stage, names, env)
    speed.collect_tools(ctx)


STAGES = {"speed-ref-local": stage_ref_local, "speed-local": stage_local, "speed-windows": stage_windows}
