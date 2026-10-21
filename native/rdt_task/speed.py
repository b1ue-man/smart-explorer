"""Stage family ``speed`` of the batch suite (U55, M-1 to M-7): throughput of Smart Explorer per way against the
capacity of the same path, on shaped links, with the reference tools ``iperf3`` and ``fio`` and the comparison
tools ``sftp``, ``curl``, ``rclone``, ``cp``, ``rsync`` and ``robocopy``.

Jobs (workflow ``rechte-desktop-tresore-task.yml``): ``linux-speed-net`` runs the stages ``speed-ref`` to
``speed-cli`` below, ``linux-speed-local`` and ``windows-speed`` run the stages of ``speed_local.py``, the job
``android-device`` runs ``speed-android-device``. Each stage sets up links and containers (``speed_net.py``), runs
``cargo test --test speed_task -- --ignored --test-threads=1 --exact <paths>`` once per link, reads the libtest
output and the report lines of ``SE_SPEED_REPORT`` (``speed_report.py``) and records one result per case.
The case names are written out in this file: the inventory (``art=skript``) looks for them here.
"""
from __future__ import annotations

import getpass
import json
import os
import re
import shutil
from pathlib import Path

from . import common, speed_local, speed_net, speed_report
from .common import FAILED, NOT_RUN, excerpt
from .speed_net import Servers, Unavailable, link
from .speed_run import (collect_tools, fetch_rclone, net_stage, not_run, prepare, record_all, run_and_record,
                        run_group, stage_env)

IPERF = ["speed_task_ref_iperf_lan", "speed_task_ref_iperf_wan", "speed_task_ref_iperf_fat",
         "speed_task_ref_iperf_loss"]
LARGE = {
    "lan": ["speed_task_sftp_large_up_lan", "speed_task_sftp_large_down_lan", "speed_task_ftp_large_up_lan",
            "speed_task_ftp_large_down_lan", "speed_task_webdav_large_up_lan", "speed_task_webdav_large_down_lan",
            "speed_task_smb_large_up_lan", "speed_task_smb_large_down_lan", "speed_task_share_large_up_lan",
            "speed_task_share_large_down_lan", "speed_task_agent_large_up_lan", "speed_task_agent_large_down_lan"],
    "wan": ["speed_task_sftp_large_up_wan", "speed_task_sftp_large_down_wan", "speed_task_ftp_large_up_wan",
            "speed_task_ftp_large_down_wan", "speed_task_webdav_large_up_wan", "speed_task_webdav_large_down_wan",
            "speed_task_smb_large_up_wan", "speed_task_smb_large_down_wan", "speed_task_share_large_up_wan",
            "speed_task_share_large_down_wan", "speed_task_agent_large_up_wan", "speed_task_agent_large_down_wan",
            "speed_task_r2r_sftp_to_ftp_large_wan", "speed_task_r2r_share_to_sftp_large_wan"],
    "fat": ["speed_task_sftp_large_up_fat", "speed_task_sftp_large_down_fat", "speed_task_share_large_up_fat",
            "speed_task_share_large_down_fat", "speed_task_agent_large_up_fat", "speed_task_agent_large_down_fat"],
    "loss": ["speed_task_sftp_large_up_loss", "speed_task_sftp_large_down_loss", "speed_task_ftp_large_up_loss",
             "speed_task_ftp_large_down_loss", "speed_task_webdav_large_up_loss", "speed_task_webdav_large_down_loss",
             "speed_task_share_large_up_loss", "speed_task_share_large_down_loss"],
}
SMALL = {
    "lan": ["speed_task_sftp_small_up_lan", "speed_task_sftp_small_down_lan", "speed_task_ftp_small_up_lan",
            "speed_task_ftp_small_down_lan", "speed_task_webdav_small_up_lan", "speed_task_webdav_small_down_lan",
            "speed_task_smb_small_up_lan", "speed_task_smb_small_down_lan", "speed_task_share_small_up_lan",
            "speed_task_share_small_down_lan", "speed_task_agent_small_up_lan", "speed_task_agent_small_down_lan",
            "speed_task_webdav_requests_per_file"],
    "wan": ["speed_task_sftp_small_up_wan", "speed_task_sftp_small_down_wan", "speed_task_ftp_small_up_wan",
            "speed_task_ftp_small_down_wan", "speed_task_webdav_small_up_wan", "speed_task_webdav_small_down_wan",
            "speed_task_smb_small_up_wan", "speed_task_smb_small_down_wan", "speed_task_share_small_up_wan",
            "speed_task_share_small_down_wan", "speed_task_agent_small_up_wan", "speed_task_agent_small_down_wan",
            "speed_task_sftp_mixed_up_wan", "speed_task_sftp_mixed_down_wan", "speed_task_share_mixed_up_wan",
            "speed_task_share_mixed_down_wan"],
}
RAW = ["speed_task_raw_sftp_large_up", "speed_task_raw_sftp_large_down", "speed_task_raw_ftp_large_up",
       "speed_task_raw_ftp_large_down", "speed_task_raw_webdav_large_up", "speed_task_raw_webdav_large_down"]
CIPHERS = {"speed_task_sftp_cipher_aes_gcm": "aes256-gcm@openssh.com",
           "speed_task_sftp_cipher_chacha": "chacha20-poly1305@openssh.com"}
RAMP = ["speed_task_ramp_sftp_small_wan", "speed_task_ramp_ftp_small_wan", "speed_task_ramp_persisted_hint_sftp_wan"]
LZ4 = [("speed_task_share_lz4_text_slow", "share"), ("speed_task_share_lz4_random_slow", "share"),
       ("speed_task_agent_lz4_text_slow", "agent")]
CC = [("speed_task_share_cc_bbr3_vs_cubic_loss", "loss"), ("speed_task_share_cc_bbr3_vs_cubic_wan", "wan")]
LIMITER = ["speed_task_limiter_link_wan_large", "speed_task_limiter_target_disk_throttled",
           "speed_task_limiter_source_disk_throttled", "speed_task_limiter_remote_server_throttled",
           "speed_task_limiter_se_cpu_quota", "speed_task_limiter_idle_when_source_slow"]
ROOM = "speed_task_share_room_parallel_downloads_no_eagain"
CLI = ["speed_task_cli_put_large_wan", "speed_task_cli_get_large_wan", "speed_task_cli_cp_small_tree_up_wan",
       "speed_task_cli_cp_small_tree_down_wan", "speed_task_cli_cp_refuses_existing_destination_unchanged"]
TOOLS = "speed_task_ref_tools_present"
GDRIVE = "speed_task_gdrive_not_run"
SMB_LIB = "speed_task_smb_segments_end_to_end"
ANDROID = ["speed_task_android_local_large", "speed_task_android_local_small",
           "speed_task_android_content_vs_file_read"]


# --------------------------------------------------------------------------- the stages of linux-speed-net

def stage_ref(ctx: common.JobContext, stage: str) -> None:
    error = prepare(ctx)
    if error:
        for name in [TOOLS, *IPERF]:
            ctx.record(name, stage, FAILED, error)
        return
    servers = Servers(ctx, stage)
    servers.start("iperf")
    env = stage_env(ctx, stage, {**servers.env, "SE_SPEED_TOOLS_REQUIRED": "iperf3,tc,curl,ssh,docker,fio,rsync"})
    with link(ctx, stage, "raw") as shaping:
        run_and_record(ctx, stage, [TOOLS], {**env, **shaping})
    collect_tools(ctx)
    for profile in ("lan", "wan", "fat", "loss"):
        name = f"speed_task_ref_iperf_{profile}"
        if "iperf" in servers.failed:
            ctx.record(name, stage, NOT_RUN, "iperf3-Server nicht gestartet: " + servers.failed["iperf"])
            continue
        try:
            with link(ctx, stage, profile) as shaping:
                run_and_record(ctx, stage, [name], {**env, **shaping})
        except Unavailable as problem:
            ctx.record(name, stage, NOT_RUN, str(problem))


def smb_lib_test(ctx: common.JobContext, stage: str, servers: Servers) -> None:
    """The ignored library test of the segmented SMB stage needs ``SE_SYNC_PROVIDER_MANIFEST`` with provider smb."""
    if "smb" in servers.failed:
        ctx.record(SMB_LIB, stage, NOT_RUN, "SMB-Server nicht gestartet: " + servers.failed["smb"])
        return
    if not ctx.cargo.build_lib():
        ctx.record(SMB_LIB, stage, FAILED, "Bau der Bibliothekstests gescheitert: " + ctx.cargo.lib_error)
        return
    paths = ctx.cargo.by_leaf.get(SMB_LIB, [])
    match = re.match(r"smb://([^:]+):([^@]+)@[^:]+:(\d+)/(\S+)", servers.env["SE_SPEED_SMB_URL"])
    if len(paths) != 1 or match is None:
        ctx.record(SMB_LIB, stage, FAILED, f"Pfad des Tests unklar ({paths}) oder SMB-Angabe unlesbar")
        return
    manifest = ctx.fixtures.make_dir("smb-manifest") / "providers.json"
    user, password, port, share = match.groups()
    manifest.write_text(json.dumps({"providers": [{"name": "smb", "protocol": "smb", "host": "127.0.0.1",
                        "port": int(port), "user": user, "password": password, "root": "/" + share}], "zip": ""}))
    env = stage_env(ctx, stage, {"SE_SYNC_PROVIDER_MANIFEST": manifest})
    status, reason, duration = ctx.run_libtest(stage, paths, env, True, 1, False)[paths[0]]
    ctx.record(SMB_LIB, stage, status, reason or paths[0], duration)


AGENT_RANGE_LABEL = b"range-v1"
AGENT_STALE = ("eingebettetes Agent-Bündel ist älter als der Arbeitsbaum (kein range-v1); "
               "der Release-Wrapper baut es neu")


def agent_bundle_current() -> bool:
    """The suite builds no agent bundle (AGENTS.md, D3): the committed one must already speak range-v1."""
    bundle = common.NATIVE / "agent-bin" / "se-agent-x86_64-linux-musl"
    return bundle.is_file() and AGENT_RANGE_LABEL in bundle.read_bytes()


def without_stale_agent(ctx: common.JobContext, stage: str, plan: list, ways: tuple) -> tuple[list, tuple]:
    """Agent cases are not started against an older embedded agent; each one is reported as not run."""
    if agent_bundle_current():
        return plan, ways
    kept = []
    for profile, names in plan:
        for name in names:
            if "_agent_" in name:
                ctx.record(name, stage, NOT_RUN, AGENT_STALE)
        kept.append((profile, [name for name in names if "_agent_" not in name]))
    return kept, tuple(way for way in ways if way != "agent")


def stage_net_large(ctx: common.JobContext, stage: str) -> None:
    ctx.record(GDRIVE, stage, NOT_RUN, "kein Google-Drive-Konto auf dem Runner (A53)")
    plan = [(profile, LARGE[profile]) for profile in ("lan", "wan", "fat", "loss")]
    plan, ways = without_stale_agent(ctx, stage, plan, ("sftp", "ftp", "webdav", "smb", "agent", "share"))
    servers = net_stage(ctx, stage, plan, ways)
    if servers is None:
        ctx.record(SMB_LIB, stage, FAILED, ctx.state["speed_prepared"])
    else:
        smb_lib_test(ctx, stage, servers)


def stage_net_small(ctx: common.JobContext, stage: str) -> None:
    fetch_rclone(ctx)
    plan, ways = without_stale_agent(ctx, stage, [("lan", SMALL["lan"]), ("wan", SMALL["wan"])],
                                     ("sftp", "ftp", "webdav", "smb", "agent", "share"))
    net_stage(ctx, stage, plan, ways)
    notes = ctx.state.setdefault("stage_notes", {})
    notes[stage] = (notes.get(stage, "") + "; " + ctx.state["speed_rclone"][1]).strip("; ")


def stage_raw(ctx: common.JobContext, stage: str) -> None:
    names = RAW + list(CIPHERS)
    error = prepare(ctx)
    if error:
        return not_run_failed(ctx, stage, names, error)
    net_stage(ctx, stage, [("raw", RAW)], ("sftp", "ftp", "webdav"))
    for name, cipher in CIPHERS.items():
        servers = Servers(ctx, stage)
        base = servers.mark()
        servers.start("sftp")
        servers.start("sftp", prefix="SE_SPEED_SFTP_CIPHER", ciphers=cipher)
        if servers.failed.get("sftp"):
            ctx.record(name, stage, NOT_RUN, "SFTP-Server nicht gestartet: " + servers.failed["sftp"])
        else:
            with link(ctx, stage, "raw") as shaping:
                run_and_record(ctx, stage, [name], {**stage_env(ctx, stage, servers.env), **shaping})
        servers.release(base)


def not_run_failed(ctx: common.JobContext, stage: str, names: list[str], error: str) -> None:
    for name in names:
        ctx.record(name, stage, FAILED, error)


def stage_ramp(ctx: common.JobContext, stage: str) -> None:
    net_stage(ctx, stage, [("wan", RAMP)], ("sftp", "ftp"))


def agent_bundle_problem() -> str | None:
    """The embedded agent honors ``SE_TEST_NO_LZ4`` only as a development build and must know ``lz4-v1``."""
    if not agent_bundle_current():
        return AGENT_STALE
    bundle = common.NATIVE / "agent-bin" / "se-agent-x86_64-linux-musl"
    data = bundle.read_bytes() if bundle.is_file() else b""
    missing = [label for label in (b"lz4-v1", b"SE_TEST_NO_LZ4") if label not in data]
    if missing:
        return ("das eingebettete Agent-Bündel native/agent-bin enthält " + ", ".join(m.decode() for m in missing)
                + " nicht (Release-Bau ohne Messschalter oder älter als U53); die Suite baut kein Bündel (AGENTS.md)")
    return None


def stage_compress(ctx: common.JobContext, stage: str) -> None:
    names = [name for name, _ in LZ4] + [name for name, _ in CC]
    error = prepare(ctx)
    if error:
        return not_run_failed(ctx, stage, names, error)
    for name, way in LZ4:
        problem = agent_bundle_problem() if way == "agent" else None
        if problem:
            ctx.record(name, stage, NOT_RUN, problem)
            continue
        start, results = speed_report.offset(ctx), {}
        for mode in ("off", "on"):
            servers = Servers(ctx, stage)
            base = servers.mark()
            servers.start(way, no_lz4=(mode == "off"))
            if way in servers.failed:
                results = {name: (NOT_RUN, f"Wirt nicht gestartet: {servers.failed[way]}", 0.0)}
            else:
                try:
                    with link(ctx, stage, "slow") as shaping:
                        env = stage_env(ctx, stage, {**servers.env, **shaping, "SE_SPEED_LZ4_MODE": mode})
                        results = run_group(ctx, stage, [name], env)
                except Unavailable as problem_text:
                    results = {name: (NOT_RUN, str(problem_text), 0.0)}
            servers.release(base)
            if results[name][0] != "passed":
                break
        record_all(ctx, stage, results, start)
    for name, profile in CC:
        start, results = speed_report.offset(ctx), {}
        for controller in ("cubic", "bbr3"):
            servers = Servers(ctx, stage)
            base = servers.mark()
            servers.start("share", cc=controller)
            if "share" in servers.failed:
                results = {name: (NOT_RUN, f"Share-Paar nicht gestartet: {servers.failed['share']}", 0.0)}
            else:
                try:
                    with link(ctx, stage, profile) as shaping:
                        env = stage_env(ctx, stage, {**servers.env, **shaping, "SE_SPEED_CC": controller})
                        results = run_group(ctx, stage, [name], env)
                except Unavailable as problem_text:
                    results = {name: (NOT_RUN, str(problem_text), 0.0)}
            servers.release(base)
        record_all(ctx, stage, results, start)


def scoped(props: list[str]) -> list[str]:
    """Runs the command in a systemd scope with the properties and then as the runner user again."""
    flags = [item for prop in props for item in ("-p", prop)]
    return ["sudo", "-n", "systemd-run", "--scope", "--quiet", "--collect", *flags, "--", "sudo", "-n", "-E", "-u",
            getpass.getuser(), "env", f"PATH={os.environ.get('PATH', '')}"]


def stage_limiter(ctx: common.JobContext, stage: str) -> None:
    error = prepare(ctx)
    if error:
        return not_run_failed(ctx, stage, LIMITER, error)
    fast, _, _, _ = speed_local.volumes(ctx, stage)
    folder = fast / "limiter"
    folder.mkdir(parents=True, exist_ok=True)
    folder.chmod(0o777)
    device = speed_net.device_of(fast)
    can_scope = bool(shutil.which("systemd-run")) and ctx.sudo_ok()
    # name -> (link, server options, systemd properties, environment, needs a block device)
    plan = {
        LIMITER[0]: ("wan", {}, [], {}, False),
        LIMITER[1]: ("raw", {}, [f"IOWriteBandwidthMax={folder} 50M"], {"SE_SPEED_THROTTLE_BPS": "50000000"}, True),
        LIMITER[2]: ("raw", {}, [f"IOReadBandwidthMax={folder} 50M"], {"SE_SPEED_THROTTLE_BPS": "50000000"}, True),
        LIMITER[3]: ("raw", {"extra_options": ["--device-write-bps", f"{device}:50mb"], "mount": folder / "server"},
                     [], {"SE_SPEED_THROTTLE_BPS": "50000000"}, True),
        LIMITER[4]: ("raw", {}, ["CPUQuota=25%"], {}, False),
        LIMITER[5]: ("raw", {}, [], {"SE_SPEED_THROTTLE_BPS": "625000"}, False),
    }
    for name, (profile, options, props, extra, needs_device) in plan.items():
        if (props and not can_scope) or (needs_device and not device):
            ctx.record(name, stage, NOT_RUN, "systemd-run/sudo -n fehlt" if props and not can_scope else
                       f"kein Blockgerät unter {fast} (findmnt/lsblk)")
            continue
        (folder / "server").mkdir(exist_ok=True)
        (folder / "server").chmod(0o777)
        servers = Servers(ctx, stage)
        base = servers.mark()
        if not servers.start("sftp", **options):
            ctx.record(name, stage, NOT_RUN, "SFTP-Server nicht gestartet: " + servers.failed["sftp"])
            continue
        if name == LIMITER[5]:
            container = servers.env["SE_SPEED_SFTP_CONTAINER"].split(":", 1)[0]
            slowed = speed_net.slow_container_rate(ctx, stage, container)
            if slowed:
                ctx.record(name, stage, NOT_RUN, slowed)
                servers.release(base)
                continue
        try:
            with link(ctx, stage, profile) as shaping:
                env = stage_env(ctx, stage, {**servers.env, **shaping, **extra, "SE_SPEED_LIMITER_DIR": folder})
                run_and_record(ctx, stage, [name], env, scoped(props) if props else ())
        except Unavailable as problem:
            ctx.record(name, stage, NOT_RUN, str(problem))
        servers.release(base)


def stage_room(ctx: common.JobContext, stage: str) -> None:
    error = prepare(ctx)
    if error:
        return not_run_failed(ctx, stage, [ROOM], error)
    servers = Servers(ctx, stage)
    if not servers.start("share", room=True):
        ctx.record(ROOM, stage, NOT_RUN, "Share-Raum nicht eingerichtet: " + servers.failed["share"])
        return
    with link(ctx, stage, "raw") as shaping:
        run_and_record(ctx, stage, [ROOM], stage_env(ctx, stage, {**servers.env, **shaping}))


def stage_cli(ctx: common.JobContext, stage: str) -> None:
    error = prepare(ctx)
    if error:
        return not_run_failed(ctx, stage, CLI, error)
    servers = Servers(ctx, stage)
    handed = Path(ctx.args.bins) / "se" if ctx.args.bins else None
    binary = handed if handed and handed.is_file() else ctx.cargo.bin_path("se")
    if not servers.start("sftp") or binary is None:
        reason = servers.failed.get("sftp") or ctx.cargo.bin_errors.get("se", "se fehlt")
        return not_run(ctx, stage, CLI, "SFTP-Server oder se fehlt: " + reason)
    root = ctx.fixtures.make_dir("cli-client")
    envs = {"SE_SPEED_CLI_ROOT": str(root), "SE_SPEED_CLI_LABEL": "speed", "SE_SPEED_SE_BIN": str(binary)}
    child = {key: str(root / sub) for key, sub in (("HOME", "home"), ("USERPROFILE", "home"), ("XDG_DATA_HOME", "data"),
             ("XDG_CONFIG_HOME", "config"), ("XDG_RUNTIME_DIR", "runtime"), ("APPDATA", "data"), ("LOCALAPPDATA", "data"))}
    for sub in ("home", "data", "config", "runtime"):
        (root / sub).mkdir(parents=True, exist_ok=True)
        (root / sub).chmod(0o700)
    spec = servers.env["SE_SPEED_SFTP"].split(":", 4)
    shell = {**os.environ, **child}
    added = ctx.run([binary, "connections", "add", "sftp", "--host", "127.0.0.1", "--port", spec[1], "--user", spec[2],
                     "--root", spec[4], "--label", "speed", "--password-stdin"], stage=stage, env=shell, timeout=120,
                    input_text=spec[3] + "\n", redact=(spec[3],))
    trusted = any(ctx.run([binary, "connections", "trust-host-key", "speed", "--fingerprint", fingerprint], stage=stage,
                          env=shell, timeout=120).ok for fingerprint in servers.env["SE_SPEED_SFTP_FINGERPRINTS"].split())
    if not (added.ok and trusted):
        return not_run(ctx, stage, CLI, f"gespeicherte Verbindung für se nicht einrichtbar: {added.describe()}, "
                       f"Host-Schlüssel bestätigt: {trusted}")
    with link(ctx, stage, "wan") as shaping:
        run_and_record(ctx, stage, CLI[:4], {**stage_env(ctx, stage, {**servers.env, **envs}), **shaping})
    with link(ctx, stage, "raw") as shaping:
        run_and_record(ctx, stage, CLI[4:], {**stage_env(ctx, stage, {**servers.env, **envs}), **shaping})


# --------------------------------------------------------------------------- Android

DD = re.compile(r"(\d+) bytes.*?([\d.]+)\s*(?:s|sec|secs|seconds)\b", re.S)


def fixture_dd(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    """M-3 Android: the emulator's own ``dd`` write and read rate (A-S9), given to the instrumentation."""
    ctx = fixtures.ctx
    if ctx.device is None:
        return False, "Emulator nicht bereit: " + ctx.state.get("device_reason", "")
    adb = ctx.device.adb_run
    dropped = adb("shell", "su", "0", "sh", "-c", "echo 3 > /proc/sys/vm/drop_caches").ok
    rates = {}
    for key, command in (("write", ["dd", "if=/dev/zero", "of=/sdcard/Download/speed.bin", "bs=1M", "count=2048", "conv=fsync"]),
                         ("read", ["dd", "if=/sdcard/Download/speed.bin", "of=/dev/null", "bs=1M"])):
        done = adb("shell", *command, timeout=1800)
        found = DD.search(done.output)
        seconds = float(found.group(2)) if found else done.duration_s
        if not done.ok or seconds <= 0:
            return False, f"dd ({key}) auf dem Emulator scheitert: {excerpt(done.output, 300)}"
        rates[key] = (int(found.group(1)) if found else 2048 * 1024 * 1024) / seconds
    adb("shell", "rm", "-f", "/sdcard/Download/speed.bin")
    note = "Seitencache geleert" if dropped else "Seitencache nicht geleert (kein root)"
    fixtures.provide(name, args=[("speed_dd_write_bps", str(int(rates["write"]))),
                                 ("speed_dd_read_bps", str(int(rates["read"]))), ("speed_dd_note", note)])
    return True, f"dd schreiben {rates['write'] / 1e6:.0f} MB/s, lesen {rates['read'] / 1e6:.0f} MB/s; {note}"


def stage_android(ctx: common.JobContext, stage: str) -> None:
    speed_report.attach_summary(ctx)
    ctx.run_default(stage)
    if ctx.device is not None:
        logged = ctx.device.adb_run("logcat", "-d", "-s", "SpeedTask:I", timeout=300).output
        with open(speed_report.report_file(ctx), "a", encoding="utf-8") as handle:
            for found in re.finditer(r"SpeedTask[^{\n]*(\{.*\})\s*$", logged, re.M):
                handle.write(found.group(1) + "\n")
        speed_report.write_json(ctx)


STAGES = {
    "speed-ref": stage_ref, "speed-net-large": stage_net_large, "speed-net-small": stage_net_small,
    "speed-raw": stage_raw, "speed-ramp": stage_ramp, "speed-compress": stage_compress,
    "speed-limiter": stage_limiter, "speed-room": stage_room, "speed-cli": stage_cli,
    "speed-android-device": stage_android, **speed_local.STAGES,
}
FIXTURES = {"speed-dd": fixture_dd}
