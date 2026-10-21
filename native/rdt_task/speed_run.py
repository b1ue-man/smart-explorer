"""Runner of the stage family ``speed`` (U55-6): build of the test program ``speed_task``, the environment of the
cases, one ``cargo test --exact`` run per group, the verdict per case, the fixed rclone release, and the generic
network stage (servers of some ways, then one run per link). The stages themselves are in ``speed.py`` and
``speed_local.py``.
"""
from __future__ import annotations

import hashlib
import json
import platform
import shutil
import time
import urllib.request
import zipfile
from pathlib import Path

from . import common, speed_local, speed_report
from .common import FAILED, NOT_RUN, WINDOWS, excerpt
from .speed_net import Servers, Unavailable, link

RCLONE_VERSION = "v1.75.2"  # stable release of 2026-10-09 (github.com/rclone/rclone/releases)
RCLONE_SHA256 = {  # SHA256SUMS of that release
    "linux-amd64": "349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702",
    "windows-amd64": "8a9db2bad1dc79a65b3348fd857069598e2aab7cf4d118b77b1df23055aff818",
}
WAYS = ("sftp", "ftp", "webdav", "smb", "share", "agent")


def ways_of(name: str) -> list[str]:
    return [token for token in name.split("_") if token in WAYS]


def build(ctx: common.JobContext) -> str | None:
    """``cargo test --profile speed --test speed_task --no-run`` and the list of its ignored tests. Throughput is
    measured on release optimization (``common.SPEED_PROFILE``), never on the development profile."""
    done = ctx.run(["cargo", "test", "--locked", "--profile", common.SPEED_PROFILE, "--test", "speed_task", "--no-run",
                    "--manifest-path", common.Cargo.manifest(), "--message-format=json-render-diagnostics"],
                   stage="build-speed", cwd=common.REPO, env=ctx.base_env(), timeout=common.BUILD_TIMEOUT_S)
    exe = None
    for message in common.json_lines(done.output):
        target = message.get("target") or {}
        if (message.get("reason") == "compiler-artifact" and (message.get("profile") or {}).get("test")
                and target.get("name") == "speed_task" and message.get("executable")):
            exe = Path(message["executable"])
    if not (done.ok and exe and exe.is_file()):
        return "Bau von speed_task gescheitert: " + done.describe() + "\n" + excerpt(
            "\n".join(line for line in done.output.splitlines() if not line.startswith("{")), 2500)
    listing = ctx.run([exe, "--list", "--ignored"], stage="build-speed", cwd=common.NATIVE, env=ctx.test_env(),
                      timeout=600)
    if not listing.ok:
        return "Testliste von speed_task nicht lesbar: " + excerpt(listing.output, 1500)
    ctx.state["speed_paths"] = {common.leaf(path): path for path in common.LIST_LINE.findall(listing.output)}
    return None


def fetch_rclone(ctx: common.JobContext) -> None:
    """The fixed rclone release as a zip with a verified SHA-256 (U55-3); on any problem the comparison
    requirements are "nicht ausgeführt" with the reason."""
    if "speed_rclone" in ctx.state:
        return
    key = "windows-amd64" if WINDOWS else ("linux-amd64" if platform.machine() in ("x86_64", "AMD64") else None)
    if key is None:
        ctx.state["speed_rclone"] = (None, f"kein rclone-Archiv für {platform.machine()}")
        return
    name = f"rclone-{RCLONE_VERSION}-{key}"
    folder = ctx.work / "rclone"
    folder.mkdir(parents=True, exist_ok=True)
    archive = folder / f"{name}.zip"
    try:
        url = f"https://github.com/rclone/rclone/releases/download/{RCLONE_VERSION}/{name}.zip"
        with urllib.request.urlopen(url, timeout=120) as response, open(archive, "wb") as handle:  # noqa: S310
            shutil.copyfileobj(response, handle)
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        if actual != RCLONE_SHA256[key]:
            raise RuntimeError(f"SHA-256 weicht ab: {actual} statt {RCLONE_SHA256[key]}")
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(folder)
        binary = folder / name / ("rclone.exe" if WINDOWS else "rclone")
        if not WINDOWS:
            binary.chmod(0o755)
        ctx.state["speed_rclone"] = (binary, f"rclone {RCLONE_VERSION}, SHA-256 {RCLONE_SHA256[key]} geprüft")
    except Exception as error:  # noqa: BLE001 - the reason reaches the cases
        ctx.state["speed_rclone"] = (None, f"rclone {RCLONE_VERSION} nicht verfügbar: {type(error).__name__}: {error}")


def prepare(ctx: common.JobContext) -> str | None:
    """Builds once per job and attaches the table to the summary; returns the error text, if any."""
    if "speed_prepared" not in ctx.state:
        speed_report.attach_summary(ctx)
        bins = Path(ctx.args.bins) if getattr(ctx.args, "bins", None) else None
        if bins and bins.is_dir() and not WINDOWS:
            for path in bins.iterdir():
                if path.is_file():
                    path.chmod(0o755)  # artifacts do not keep the executable bit
            if (bins / "se-share-server").is_file():
                ctx.state["desktop_bins_dir"] = str(bins)
        ctx.state["speed_prepared"] = build(ctx)
    return ctx.state["speed_prepared"]


def stage_env(ctx: common.JobContext, stage: str, extra: dict | None = None) -> dict:
    assert ctx.fixtures is not None
    ctx.fixtures.ensure(("home",))
    env = ctx.base_env()
    env.update(ctx.fixtures.env_for(("home",))[0])
    work = ctx.work / "speed"
    work.mkdir(parents=True, exist_ok=True)
    env.update({"SE_SPEED_REPORT": str(speed_report.report_file(ctx)), "SE_SPEED_CAPACITY": str(work / "capacity.json"),
                "SE_SPEED_STATE": str(work / "state.json"), "SE_SPEED_TOOLS_JSON": str(work / "tools.json"),
                "SE_SPEED_CASE_TIMEOUT_S": "600"})
    fast, slow, _, _ = speed_local.volumes(ctx, stage)
    env.update({"SE_SPEED_DIR_FAST": str(fast), "SE_SPEED_DIR_SLOW": str(slow)})
    binary = (ctx.state.get("speed_rclone") or (None, ""))[0]
    if binary:
        env["SE_SPEED_RCLONE"] = str(binary)
    env.update({key: str(value) for key, value in (extra or {}).items()})
    return env


def run_group(ctx: common.JobContext, stage: str, names: list[str], env: dict, wrap: list | tuple = ()) -> dict:
    """One ``cargo test`` run of exact paths; ``{name: (status, reason, duration)}`` from the libtest output."""
    paths = ctx.state["speed_paths"]
    results: dict[str, tuple] = {}
    known = []
    for name in names:
        if name in paths:
            known.append(paths[name])
        else:
            results[name] = (FAILED, f"{name} fehlt im Testprogramm speed_task", 0.0)
    limit = int(env.get("SE_SPEED_CASE_TIMEOUT_S", "600"))
    for part in common.chunks(known):
        parser = common.LibtestParser(time.monotonic())
        command = [*wrap, "cargo", "test", "--locked", "--profile", common.SPEED_PROFILE, "--test", "speed_task",
                   "--manifest-path", common.Cargo.manifest(), "--", "--ignored", "--test-threads=1", "--exact", *part]
        done = ctx.run(command, stage=stage, cwd=common.REPO, env=env, timeout=limit * len(part) + 600,
                       on_line=parser.feed)
        blocks = {match.group(1): match.group(2).strip() for match in common.FAILURE_BLOCK.finditer(done.output)}
        for path in part:
            found = parser.results.get(path)
            if found is None:
                results[common.leaf(path)] = (FAILED, f"kein Ergebnis ({done.describe()}): " + excerpt(done.output, 1200), 0.0)
                continue
            status, reason, duration = found
            if status == FAILED:
                reason = blocks.get(path) or excerpt(done.output, 1200)
            results[common.leaf(path)] = (status, reason, duration)
    return results


def record_all(ctx: common.JobContext, stage: str, results: dict, start: int) -> None:
    lines = speed_report.read_lines(ctx, start)
    for name, libtest in results.items():
        status, reason = speed_report.verdict(name, libtest, lines)
        ctx.record(name, stage, status, reason, libtest[2])
    speed_report.write_json(ctx)


def run_and_record(ctx: common.JobContext, stage: str, names: list[str], env: dict, wrap: list | tuple = ()) -> None:
    start = speed_report.offset(ctx)
    record_all(ctx, stage, run_group(ctx, stage, names, env, wrap), start)


def not_run(ctx: common.JobContext, stage: str, names: list[str], reason: str) -> None:
    for name in names:
        if not ctx.recorded(name):
            ctx.record(name, stage, NOT_RUN, reason)


def collect_tools(ctx: common.JobContext) -> None:
    path = ctx.work / "speed" / "tools.json"
    if path.is_file():
        try:
            ctx.state["speed_tools"] = json.loads(path.read_text(encoding="utf-8"))
        except ValueError:
            pass


def net_stage(ctx: common.JobContext, stage: str, plan: list[tuple[str, list[str]]], ways: tuple,
              **options) -> Servers | None:
    """Servers of ``ways``, then per link the cases whose servers run; the others are "nicht ausgeführt"."""
    every = [name for _, names in plan for name in names]
    error = prepare(ctx)
    if error:
        for name in every:
            ctx.record(name, stage, FAILED, error)
        return None
    servers = Servers(ctx, stage)
    for way in ways:
        servers.start(way, **options.get(way, {}))
    ctx.state.setdefault("stage_notes", {})[stage] = "; ".join(f"{way}: {why}" for way, why in servers.failed.items())
    env = stage_env(ctx, stage, servers.env)
    for profile, names in plan:
        run, blocked = [], {}
        for name in names:
            reasons = [servers.failed[way] for way in ways_of(name) if way in servers.failed]
            if reasons:
                blocked[name] = "Server nicht gestartet: " + "; ".join(reasons)
            else:
                run.append(name)
        for name, reason in blocked.items():
            ctx.record(name, stage, NOT_RUN, reason)
        if not run:
            continue
        try:
            with link(ctx, stage, profile) as shaping:
                run_and_record(ctx, stage, run, {**env, **shaping})
        except Unavailable as problem:
            not_run(ctx, stage, run, str(problem))
    return servers
