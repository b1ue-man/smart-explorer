#!/usr/bin/env python3
"""One remote-only task suite for the batch of 2026-10-28 (plan
docs/plaene/2026-10-28-verbinden-ist-einrichtung/): the self-setup of remote
access (WP1 host, WP2 viewer, WP3 surfaces) and the fast transfer paths
(KP1 crash log, KP2 Share evidence, KP3 remote backends, KP4 local copy,
KP5a Share host, KP6 sync/drives/vaults/zip), with the G5 measurement frame
of analyse-plan.md (yardstick: aggregate throughput of the job from the first
to the last byte, compared with the fastest established tool on the same
runner and with v0.5.188 measured on the same runner).

Run only on GitHub Actions (.github/workflows/speed-setup-task.yml); never on
the workstation. Development builds for the functional job, release builds
only for the measurement (`--release`, never the release-artifact workflow).

Jobs (--job):
  fmt         rustfmt (stdin mode) of every Rust file changed since v0.5.188;
              a deviation fails only when it touches a changed line
  functional  library test binary, then one run of every test named
              `speed_task_`, `einrichtung_task_`, `removable_media_task_`,
              `batch_handoff_task_`, the five tests the batch moved, and the
              directly affected modules; every compiled, not ignored test of
              those prefixes must pass (the list comes from `--list`);
              (Windows) the FAT32/exFAT paste with `--nocapture`;
              RDP clients: A4 sdl-freerdp3 first (Linux, Xvfb), A5 the native
              client owns its connection (xfreerdp3 / mstsc), V2 another
              account is refused (Linux, sudo -u nobody);
              elevated: H3 xrdp on loopback as root (Linux, sudo), H4 RDP
              switched on without a firewall change (Windows runner is admin);
              `--test transfer_task` crash-log cases and the G3 cases
              (`G3_FUNCTIONAL`) against real servers (Linux: SFTP atmoz/sftp,
              FTP vsftpd, SMB dockurr/samba, WebDAV httpd mod_dav; Windows:
              FTP pyftpdlib, SFTP and G3 cases skipped); app data isolated
              per run (XDG_DATA_HOME/XDG_CONFIG_HOME, Windows APPDATA)
  measure     --part local          (Linux, Windows) user-level `se cp -r` and
                                    cross-volume `se mv` of v0.5.188 and of the
                                    candidate next to cp -r, rsync -a, mv
                                    (Linux) or robocopy /MT:16 (Windows);
                                    iperf3 on loopback; Windows also with
                                    Defender real-time on and with a folder
                                    exclusion; then every ignored
                                    `speed_task_` benchmark of the library
              --part remote-bench   (Linux) every `speed_task_remote_*`
                                    benchmark of `transfer_task` against the
                                    four servers and Nextcloud
                                    (nextcloud:30-apache, user from occ), the
                                    FTP ones again against
                                    pyftpdlib (MLST); reference tools rclone,
                                    lftp, scp, smbclient on the same data
              --part remote-cli     (Linux) `se cp` up and down of v0.5.188 and
                                    of the candidate to SFTP, FTP and SMB next
                                    to rclone, lftp, scp and smbclient; a local
                                    Share pair per version (Direct, loopback);
                                    with --netem the same at 1 ms and 20 ms
                                    delay per direction on `lo`
              --part windows-remote (Windows) the same against pyftpdlib FTP
                                    and the `speed_task_remote_ftp_*` benchmarks
              Data sets: 5000 x 4 KiB in 50 folders, 1000 x 1 MiB, 1 x 2 GiB
              (only with enough free disk). Every measurement runs
              `--repetitions` times; the report shows the median.

Outputs in --log-root: summary.md (appended to the step summary),
summary.json (checks), measurements.json (every run), bench.json (every
`BENCH` line per repetition and the medians), logs per stage.

Helpers (results, isolated `se` clients, SSH host-key confirmation) come from
test-ai-plugin-task.py; the Share pair from sync-reliability-share-fixture.py.
"""
import argparse
import base64
import contextlib
import difflib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import types
import urllib.error
import urllib.request
import zipfile

NATIVE = Path(__file__).resolve().parent
ROOT = NATIVE.parent
WINDOWS = os.name == "nt"
EXE = ".exe" if WINDOWS else ""
BASELINE_TAG = "v0.5.188"
MIB = 1024.0 * 1024.0
GIB = 1024 ** 3


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ai = _load("ai_plugin_task", NATIVE / "test-ai-plugin-task.py")
ensure = ai.ensure

# Test names of this batch (functional job). The prefixes are substring
# filters, so `local_speed_task_*` (KP4) is selected by `speed_task_` too.
# Added after G5 (packages R1-R4, D1, D3, KP5a): the worker's job service
# (`worker_job_task_`), remote-to-remote packets (`remote_packet_task_`), the
# relay start bridge (`route_start_task_`, Linux relay fixture with Iroh's
# `test-utils` dev feature from native/Cargo.toml) and its adapted B5 tests,
# the drive request gate (`mount_vault_task_`) and the rooted drive backend
# (`remote_drive_task_`). Everything else new is named `speed_task_`.
PREFIXES = ("speed_task_", "einrichtung_task_", "removable_media_task_", "batch_handoff_task_",
            "worker_job_task_", "remote_packet_task_", "route_start_task_", "batches_task_b5_",
            "mount_vault_task_", "remote_drive_task_")
# Tests the batch moved into new files unchanged (KP2: under 500 lines).
MOVED = ("batches_task_transfer_compat_reserved_share_rename_is_a_single_write_with_distinct_wire_operation",
         "remote_drive_task_only_idempotent_control_reads_are_replayable",
         "transfer_engine_task_only_batch_status_is_replayable",
         "transfer_engine_task_sized_stage_commits_only_exact_length",
         "transfer_engine_task_batch_entries_stay_below_an_export")
# Directly affected integrations: every test of the modules the batch changed
# (as the transfer-engine suite did), plus the pure core of the changed
# screen, mount and desktop code and the two desktop test files WP3 edited.
AFFECTED = ("transfer::", "copy::", "vfs::", "sync::", "bisync::", "syncjobs::", "connect::", "gdrive::",
            "ftp::", "webdav::", "sftp::", "smb::", "zipfs::", "cryptomator::", "agent::", "agent_proto::",
            "daemon::", "share::", "rdp::", "source_media::", "app::", "cli::", "mobile::", "screen::core::",
            "mount::core::", "desktop::core::", "open_target_tests", "adapter_tests", "file_recovery",
            "batches_missing_retry_compatibility", "batches_publication_compatibility",
            "batches_backend_compatibility")
# The daemon's Windows analysis bridge needs the Windows remote suite's own
# switch and isolated profile (as in test-transfer-engine-task.sh).
UNIT_SKIPS = ("windows_analysis_task_tests",)
# Prepared-runner tests (ignored by default), per platform.
RDP_CLIENT_TESTS = {
    "linux": ("einrichtung_task_sdl_freerdp_is_the_first_client",
              "einrichtung_task_rdp_client_connects_from_its_own_process",
              "einrichtung_task_other_account_is_refused"),
    "windows": ("einrichtung_task_rdp_client_connects_from_its_own_process",),
}
H3 = "einrichtung_task_h3_xrdp_on_loopback_as_root"
H4 = "einrichtung_task_h4_windows_rdp_switched_on_without_firewall_change"
CRASH_LOG_PREFIX = "transfer_task_crash_log_"
# Correctness cases of the G3 follow-ups in `transfer_task` (ignored, real
# servers), run once in the functional job: SMB compound stage (G3-07), the
# SFTP copy batch and same-account packets through the engine (G3-13/R2).
# The Nextcloud chunk upload (G3-12) is a 2 GiB case and runs in the
# remote-bench measurement next to its benchmarks.
G3_FUNCTIONAL = ("speed_task_remote_smb_compound_stage_never_replaces",
                 "speed_task_remote_sftp_server_copy_batch",
                 "speed_task_remote_sftp_same_account_packets_byte_exact_never_replace")
# Nextcloud for SE_TASK_NEXTCLOUD (G3-12): a pinned major line with Apache.
NEXTCLOUD_IMAGE = "nextcloud:30-apache"
# Ignored library tests named `speed_task_` that are no benchmark of this batch.
NOT_BENCHMARKS = ("speed_task_smb_segments_end_to_end",)

DATASETS = (("small", 5000, 50, 4096), ("medium", 1000, 1, 1 << 20), ("large", 1, 1, 2 * GIB))
# Free space needed before a 2 GiB data set or benchmark starts: the source,
# one copy and a server copy, with room to spare.
LARGE_FREE_GIB = 12
BENCH_FREE_GIB = 10
RCLONE_VERSION = "v1.75.2"  # as in rdt_task/speed_run.py (stable release of 2026-10-09)
RCLONE_SHA256 = {
    "linux-amd64": "349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702",
    "windows-amd64": "8a9db2bad1dc79a65b3348fd857069598e2aab7cf4d118b77b1df23055aff818",
}
SECRETS = set()
USER = "bench"


# --------------------------------------------------------------------------- process helpers

def redact(text):
    for secret in SECRETS:
        if secret:
            text = text.replace(secret, "***")
    return text


class Ran:
    def __init__(self, code, stdout, stderr, seconds):
        self.code, self.stdout, self.stderr, self.seconds = code, stdout, stderr, seconds

    @property
    def ok(self):
        return self.code == 0

    @property
    def output(self):
        return self.stdout + ("\n" + self.stderr if self.stderr else "")

    def tail(self, size=1500):
        return redact(self.output[-size:])


def run(command, *, cwd=None, env=None, timeout=1800, input_text=None, log=None):
    """Runs a command and never raises: a timeout or a missing program is code None."""
    started = time.perf_counter()
    try:
        completed = subprocess.run([str(part) for part in command], cwd=cwd, env=env, timeout=timeout,
                                   input=input_text.encode() if input_text is not None else None,
                                   stdin=subprocess.DEVNULL if input_text is None else None,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        result = Ran(completed.returncode, completed.stdout.decode(errors="replace"),
                     completed.stderr.decode(errors="replace"), time.perf_counter() - started)
    except subprocess.TimeoutExpired as error:
        result = Ran(None, (error.stdout or b"").decode(errors="replace"),
                     (error.stderr or b"").decode(errors="replace") + f"\n[timeout after {timeout} s]",
                     time.perf_counter() - started)
    except OSError as error:
        result = Ran(None, "", f"[cannot start {command[0]}: {error}]", time.perf_counter() - started)
    if log is not None:
        log = Path(log)
        log.parent.mkdir(parents=True, exist_ok=True)
        with open(log, "a", encoding="utf-8") as handle:
            handle.write(redact(f"$ {' '.join(map(str, command))}  ({result.seconds:.1f} s, exit {result.code})\n"
                                f"{result.stdout}\n--- stderr ---\n{result.stderr}\n"))
    return result


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def wait_tcp(port, prefix=None, timeout=180):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=2) as peer:
                if prefix is None or peer.recv(16).startswith(prefix):
                    return True
        except OSError:
            pass
        time.sleep(0.5)
    return False


def free_gib(path):
    try:
        return shutil.disk_usage(path).free / GIB
    except OSError:
        return 0.0


def remove(path):
    path = Path(path)
    if not path.exists() and not path.is_symlink():
        return

    def writable(function, target, _info):
        try:
            os.chmod(target, 0o700)
            function(target)
        except OSError:
            pass
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path, onerror=writable)
    else:
        path.unlink()


def tree_stats(path):
    """(files, bytes) of every regular file below `path`."""
    files = size = 0
    for folder, _dirs, names in os.walk(path):
        for name in names:
            try:
                size += os.lstat(os.path.join(folder, name)).st_size
                files += 1
            except OSError:
                pass
    return files, size


def sudo_ok():
    return not WINDOWS and run(["sudo", "-n", "true"], timeout=30).ok


def drop_caches():
    """Linux: every timed run starts with a cold page cache (sources and servers share the kernel)."""
    if WINDOWS or not sudo_ok():
        return
    run(["sync"], timeout=600)
    run(["sudo", "-n", "sh", "-c", "echo 3 > /proc/sys/vm/drop_caches"], timeout=120)


def cargo_executable(args, target_name, log, env=None, cwd=NATIVE, timeout=4 * 3600):
    """Builds a test target with `--no-run` and returns (path of its executable, Ran)."""
    ran = run(["cargo", *args, "--no-run", "--message-format=json"], cwd=cwd, env=env, timeout=timeout, log=log)
    executable = None
    for line in ran.stdout.splitlines():
        if not line.startswith("{"):
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact" or not message.get("executable"):
            continue
        if not message.get("profile", {}).get("test"):
            continue
        if message.get("target", {}).get("name") == target_name:
            executable = message["executable"]
    return executable, ran


def list_tests(executable, ignored=False, env=None):
    ran = run([executable, "--list", *(["--ignored"] if ignored else [])], cwd=NATIVE, env=env, timeout=600)
    return [match.group(1) for match in re.finditer(r"^(\S+): test$", ran.stdout, re.MULTILINE)]


def test_outcomes(output):
    return {match.group(1): match.group(2)
            for match in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED|ignored)\b", output, re.MULTILINE)}


def failure_detail(output):
    lines = output.splitlines()
    failed = [line for line in lines if line.startswith("test ") and line.endswith("FAILED")]
    panics = [line for line in lines if "panicked at" in line]
    return redact("\n".join(failed[:40] + panics[:40]) + "\n...\n" + output[-3000:])


def leaf(name):
    return name.rsplit("::", 1)[-1]


def platform_key():
    return "windows" if WINDOWS else "linux"


# --------------------------------------------------------------------------- job fmt

def changed_lines(base):
    diff = run(["git", "diff", "-U0", "--no-color", base, "HEAD", "--", "native/src", "native/tests"],
               cwd=ROOT, timeout=300).stdout
    changed, current = {}, None
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[4:].strip()
            current = path[2:] if path.startswith("b/") else None
        elif line.startswith("@@") and current:
            match = re.search(r"\+(\d+)(?:,(\d+))?", line)
            start, count = int(match.group(1)), int(match.group(2) or 1)
            changed.setdefault(current, set()).update(range(start, start + count))
    return changed


def job_fmt(results, log_root):
    rustfmt = shutil.which("rustfmt")
    if not rustfmt:
        results.record("fmt", "rustfmt is installed", False, "rustfmt not found")
        return
    checked = 0
    for relative, lines in sorted(changed_lines(BASELINE_TAG).items()):
        path = ROOT / relative
        if not relative.endswith(".rs") or not lines or not path.is_file():
            continue
        checked += 1
        source = path.read_text(encoding="utf-8").replace("\r\n", "\n")
        formatted = run([rustfmt, "--edition", "2021", "--emit", "stdout"], input_text=source, timeout=120,
                        log=log_root / "fmt.log")
        if not formatted.ok:
            results.record("fmt", relative, False, formatted.stderr[-600:])
            continue
        before, after = source.split("\n"), formatted.stdout.replace("\r\n", "\n").split("\n")
        touched = []
        matcher = difflib.SequenceMatcher(a=before, b=after, autojunk=False)
        for tag, i1, i2, j1, j2 in matcher.get_opcodes():
            if tag == "equal":
                continue
            span = range(i1 + 1, i2 + 1) if i2 > i1 else range(max(i1, 1), i1 + 2)
            if lines.intersection(span):
                touched.append(f"lines {i1 + 1}-{max(i2, i1 + 1)}: "
                               + " | ".join(after[j1:j2])[:300])
        if touched:
            results.record("fmt", f"{relative}: not rustfmt-formatted on changed lines", False,
                           "\n".join(touched[:20]))
    results.record("fmt", f"rustfmt on the changed lines of {checked} Rust files", True)


# --------------------------------------------------------------------------- servers (Linux, Docker)

class Servers:
    """SFTP, FTP, SMB, WebDAV and Nextcloud on the runner's loopback; every failure is kept per kind."""

    def __init__(self, work, log_root):
        self.work, self.log = Path(work), Path(log_root) / "servers.log"
        self.password = secrets.token_hex(12)
        SECRETS.add(self.password)
        self.tag = f"se-speed-{os.getpid()}-{secrets.token_hex(3)}"
        self.names, self.info, self.failed = [], {}, {}

    def docker(self, label, image, options, command=()):
        name = f"{self.tag}-{label}"
        self.names.append(name)
        ran = run(["docker", "run", "-d", "--name", name, *options, image, *command], timeout=900, log=self.log)
        ensure(ran.ok, f"docker run {image}: {ran.tail(400)}")
        return name

    def port(self, name, internal):
        ran = run(["docker", "port", name, f"{internal}/tcp"], timeout=60, log=self.log)
        rows = [row for row in ran.stdout.splitlines() if row.startswith("127.0.0.1:")]
        ensure(rows, f"docker port {name}: {ran.tail(300)}")
        return int(rows[0].rsplit(":", 1)[1])

    def start(self, kinds):
        for kind in kinds:
            try:
                getattr(self, "_" + kind)()
            except Exception as error:  # noqa: BLE001 - the reason reaches the report
                self.failed[kind] = f"{type(error).__name__}: {error}"
        return self

    def _sftp(self):
        key = self.work / "sftp-key"
        if not key.exists():
            run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", key], timeout=60, log=self.log)
        name = self.docker("sftp", "atmoz/sftp:latest",
                           ["-p", "127.0.0.1::22", "-v", f"{key}.pub:/home/{USER}/.ssh/keys/bench.pub:ro"],
                           [f"{USER}:{self.password}:::upload"])
        port = self.port(name, 22)
        ensure(wait_tcp(port, b"SSH-"), f"SFTP server does not answer on {port}")
        by_type = ai.scan_ssh_host_keys("127.0.0.1", port)
        ordered = [by_type[kind] for kind in ai.SSH_HOST_KEY_ORDER if by_type.get(kind)]
        ensure(ordered, "no SFTP host key fingerprint")
        self.info["sftp"] = dict(container=name, port=port, data=f"/home/{USER}/upload", remote="/upload",
                                 fingerprints=" ".join(ordered), key=str(key))

    def _ftp(self):
        # vsftpd on the host network: data connections need no published range (TE1).
        name = self.docker("ftp", "delfer/alpine-ftp-server:latest",
                           ["--network", "host", "-e", f"USERS={USER}|{self.password}"],
                           ["vsftpd", "/etc/vsftpd/vsftpd.conf", "-obackground=NO", "-opasv_min_port=21000",
                            "-opasv_max_port=21999", "-opasv_address=127.0.0.1", "-omax_clients=0",
                            "-omax_per_ip=0"])
        ensure(wait_tcp(21, b"220"), "vsftpd does not answer on port 21")
        self.info["ftp"] = dict(container=name, port=21, data=f"/ftp/{USER}", remote=f"/ftp/{USER}")

    def _smb(self):
        name = self.docker("smb", "dockurr/samba:4.23.10",
                           ["-p", "127.0.0.1::445", "-e", f"NAME={USER}", "-e", f"USER={USER}",
                            "-e", f"PASS={self.password}", "-e", "RW=true"])
        port = self.port(name, 445)
        ensure(wait_tcp(port), f"Samba does not answer on {port}")
        time.sleep(3)
        self.info["smb"] = dict(container=name, port=port, data="/shared", remote=f"/{USER}", share=USER)

    def _webdav(self):
        root = self.work / "webdav-fixture"
        root.mkdir(parents=True, exist_ok=True)
        root.chmod(0o755)
        (root / "dav.conf").write_text(
            "LoadModule dav_module modules/mod_dav.so\nLoadModule dav_fs_module modules/mod_dav_fs.so\n"
            "DavLockDB /tmp/speed-dav-lock\n<VirtualHost *:80>\nDocumentRoot /tmp/sync-dav/data\n"
            # httpd refuses request bodies above 1 GiB by default; the large data set is bigger.
            "LimitRequestBody 0\n"
            "<Directory /tmp/sync-dav/data>\nOptions None\nAllowOverride None\nDirectoryIndex disabled\n"
            "Require all granted\n</Directory>\n<Location />\nDav On\nAuthType Basic\nAuthName Speed\n"
            "AuthUserFile /tmp/dav.passwd\nRequire valid-user\n</Location>\n</VirtualHost>\n")
        hashed = run(["openssl", "passwd", "-apr1", "-stdin"], input_text=self.password + "\n",
                     timeout=30).stdout.strip()
        ensure(hashed, "openssl passwd gave no hash")
        (root / "dav.passwd").write_text(f"{USER}:{hashed}\n")
        script = ("set -eu; printf '\\nInclude /fixture/dav.conf\\n' >> /usr/local/apache2/conf/httpd.conf; "
                  "cp /fixture/dav.passwd /tmp/dav.passwd; chmod 644 /tmp/dav.passwd; mkdir -p /tmp/sync-dav/data; "
                  "u=$(awk '$1 == \"User\" { print $2; exit }' /usr/local/apache2/conf/httpd.conf); "
                  "g=$(awk '$1 == \"Group\" { print $2; exit }' /usr/local/apache2/conf/httpd.conf); "
                  "chown -R \"$u:$g\" /tmp/sync-dav; exec httpd-foreground")
        name = self.docker("webdav", "httpd:2.4", ["-p", "127.0.0.1::80", "-v", f"{root}:/fixture:ro"],
                           ["sh", "-c", script])
        port = self.port(name, 80)
        ensure(wait_tcp(port), f"httpd does not answer on {port}")
        self.info["webdav"] = dict(container=name, port=port, data="/tmp/sync-dav/data", remote="/")

    def _nextcloud(self):
        # SQLite installation by the image's entrypoint; the bench user comes from occ.
        admin = secrets.token_hex(12)
        SECRETS.add(admin)
        name = self.docker("nextcloud", NEXTCLOUD_IMAGE,
                           ["-p", "127.0.0.1::80", "-e", "SQLITE_DATABASE=nextcloud",
                            "-e", "NEXTCLOUD_ADMIN_USER=admin", "-e", f"NEXTCLOUD_ADMIN_PASSWORD={admin}",
                            "-e", "NEXTCLOUD_TRUSTED_DOMAINS=127.0.0.1 localhost",
                            "-e", "PHP_UPLOAD_LIMIT=4G", "-e", "PHP_MEMORY_LIMIT=1G"])
        port = self.port(name, 80)
        # The entrypoint copies the code and installs before Apache listens.
        occ = ["docker", "exec", "-u", "www-data", name, "php", "occ"]
        deadline = time.monotonic() + 1200
        while True:
            status = run([*occ, "status", "--output=json"], timeout=120, log=self.log)
            if status.ok and '"installed":true' in status.stdout.replace(" ", ""):
                break
            ensure(time.monotonic() < deadline, f"Nextcloud did not finish its installation: {status.tail(400)}")
            time.sleep(5)
        ensure(wait_tcp(port, timeout=600), f"Nextcloud does not answer on {port}")
        # Many parallel requests from one address must not be slowed down as an attack.
        run([*occ, "config:system:set", "auth.bruteforce.protection.enabled", "--value=false",
             "--type=boolean"], timeout=120, log=self.log)
        added = run(["docker", "exec", "-u", "www-data", "-e", f"OC_PASS={self.password}", name, "php", "occ",
                     "user:add", "--password-from-env", USER], timeout=300, log=self.log)
        ensure(added.ok, f"occ user:add {USER}: {added.tail(400)}")
        # The case root below the user's DAV files path.
        token = base64.b64encode(f"{USER}:{self.password}".encode()).decode()
        request = urllib.request.Request(f"http://127.0.0.1:{port}/remote.php/dav/files/{USER}/bench",
                                         method="MKCOL", headers={"Authorization": f"Basic {token}"})
        created = None
        for _ in range(60):
            try:
                with urllib.request.urlopen(request, timeout=120) as answer:
                    created = answer.status
            except urllib.error.HTTPError as error:
                created = error.code
            except (urllib.error.URLError, OSError) as error:  # Apache still starting
                created = f"{type(error).__name__}: {error}"
            if created in (201, 405):
                break
            time.sleep(5)
        ensure(created in (201, 405), f"MKCOL of the Nextcloud case root: {created}")
        self.info["nextcloud"] = dict(container=name, port=port, remote=f"/remote.php/dav/files/{USER}/bench")

    def env(self):
        """The variables of native/tests/transfer_task/support.rs."""
        values = {}
        if "sftp" in self.info:
            info = self.info["sftp"]
            values["SE_TASK_SFTP"] = f"127.0.0.1:{info['port']}:{USER}:{self.password}:{info['remote']}"
            values["SE_TASK_SFTP_FINGERPRINTS"] = info["fingerprints"]
        if "ftp" in self.info:
            values["SE_TASK_FTP_URL"] = f"ftp://{USER}:{self.password}@127.0.0.1:21/ftp/{USER}"
        if "smb" in self.info:
            values["SE_TASK_SMB"] = f"127.0.0.1:{self.info['smb']['port']}:{USER}:{self.password}:/{USER}"
        if "webdav" in self.info:
            values["SE_TASK_WEBDAV"] = f"http://{USER}:{self.password}@127.0.0.1:{self.info['webdav']['port']}/"
        if "nextcloud" in self.info:
            info = self.info["nextcloud"]
            values["SE_TASK_NEXTCLOUD"] = f"http://{USER}:{self.password}@127.0.0.1:{info['port']}{info['remote']}"
        return values

    # Server-side file access through the container (seed, check, purge).
    def exec(self, kind, *command, timeout=600):
        return run(["docker", "exec", self.info[kind]["container"], *command], timeout=timeout, log=self.log)

    def seed(self, kind, local, name):
        info = self.info[kind]
        target = f"{info['data']}/{name}"
        self.exec(kind, "rm", "-rf", target)
        self.exec(kind, "mkdir", "-p", info["data"])
        copied = run(["docker", "cp", str(local), f"{info['container']}:{target}"], timeout=3600, log=self.log)
        ensure(copied.ok, f"docker cp into {kind}: {copied.tail(300)}")
        self.exec(kind, "chmod", "-R", "a+rwX", target)

    def stats(self, kind, name):
        target = f"{self.info[kind]['data']}/{name}"
        listed = self.exec(kind, "sh", "-c", 'find "$1" -type f -exec stat -c %s {} +', "sh", target,
                           timeout=1800)
        sizes = [int(value) for value in listed.stdout.split() if value.isdigit()]
        return len(sizes), sum(sizes)

    def purge(self, kind, name):
        self.exec(kind, "rm", "-rf", f"{self.info[kind]['data']}/{name}")

    def close(self):
        for name in reversed(self.names):
            logs = run(["docker", "logs", name], timeout=120)
            with open(self.log, "a", encoding="utf-8") as handle:
                handle.write(redact(f"--- docker logs {name}\n{logs.output[-20000:]}\n"))
            run(["docker", "rm", "-f", name], timeout=120, log=self.log)


class PyFtp:
    """pyftpdlib on loopback (Windows, and the MLST variant on Linux): files live in a local folder."""

    def __init__(self, work, log_root):
        self.root = Path(work) / "pyftp-root"
        self.root.mkdir(parents=True, exist_ok=True)
        self.password = secrets.token_hex(10)
        SECRETS.add(self.password)
        self.port = free_port()
        self.log_root = Path(log_root)
        self.process = None

    def start(self):
        installed = run([sys.executable, "-m", "pip", "install", "pyftpdlib>=1.5.9,<3"], timeout=600,
                        log=self.log_root / "pyftpdlib.log")
        ensure(installed.ok, f"pip install pyftpdlib: {installed.tail(400)}")
        self.process = subprocess.Popen(
            [sys.executable, "-m", "pyftpdlib", "-i", "127.0.0.1", "-p", str(self.port), "-w", "-d",
             str(self.root), "-u", USER, "-P", self.password],
            stdout=open(self.log_root / "pyftpdlib-server.log", "wb"), stderr=subprocess.STDOUT)
        ensure(wait_tcp(self.port, b"220", 60), "pyftpdlib does not answer")
        return self

    def url(self):
        return f"ftp://{USER}:{self.password}@127.0.0.1:{self.port}/"

    def close(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()


# --------------------------------------------------------------------------- job functional

def stage_unit(results, log_root, executable, env):
    listed = list_tests(executable, env=env)
    ignored = set(list_tests(executable, ignored=True, env=env))
    ensure_listed = bool(listed)
    results.record("unit", "library test list", ensure_listed, "" if ensure_listed else "empty --list output")
    filters = [*PREFIXES, *MOVED, *AFFECTED]
    skips = [argument for name in UNIT_SKIPS for argument in ("--skip", name)]
    ran = run([executable, "--test-threads=1", *skips, *filters], cwd=NATIVE, env=env, timeout=4 * 3600,
              log=log_root / "unit.log")
    outcomes = test_outcomes(ran.output)
    summary = [line for line in ran.output.splitlines() if line.startswith("test result:")]
    results.record("unit", "batch tests and the directly affected modules", ran.ok,
                   failure_detail(ran.output) if not ran.ok else "; ".join(summary))
    for prefix in PREFIXES + ("moved tests",):
        if prefix == "moved tests":
            expected = [name for name in listed if leaf(name) in MOVED and name not in ignored]
        else:
            expected = [name for name in listed if prefix in leaf(name) and name not in ignored]
        missing = [name for name in expected if outcomes.get(name) != "ok"]
        results.record("unit", f"{prefix}: every compiled, not ignored test passed", not missing,
                       f"{len(expected) - len(missing)} of {len(expected)} ok"
                       + (f"; not ok: {', '.join(leaf(name) for name in missing[:40])}" if missing else ""))
    # Source names this platform does not compile (cfg-gated): reported, not failed.
    compiled = {leaf(name) for name in listed}
    in_sources = set()
    for path in list((NATIVE / "src").rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for name in re.findall(r"\bfn (\w*(?:" + "|".join(PREFIXES) + r")\w*)\s*\(", text):
            in_sources.add(name)
    absent = sorted(in_sources - compiled)
    (log_root / "not-compiled-here.txt").write_text("\n".join(absent) + "\n", encoding="utf-8")
    results.record("unit", f"batch test functions in the sources not compiled on {platform_key()} "
                   f"(cfg-gated, informational): {len(absent)}", True)


def stage_removable(results, log_root, executable, env):
    if not os.environ.get("SE_TASK_REMOVABLE_ROOTS"):
        return
    ran = run([executable, "--test-threads=1", "--nocapture", "removable_media_task_"], cwd=NATIVE, env=env,
              timeout=3600, log=log_root / "removable.log")
    results.record("removable", "paste into the roots and a folder of FAT32 and exFAT",
                   ran.ok and "removable media roots checked: 2" in ran.output,
                   failure_detail(ran.output) if not ran.ok else ran.tail(600))


def run_named(results, stage, executable, names, env, log, *, prefix=()):
    """Runs ignored tests by name (substring filters) and requires each one to pass."""
    ran = run([*prefix, executable, "--ignored", "--test-threads=1", *names], cwd=NATIVE, env=env,
              timeout=3 * 3600, log=log)
    outcomes = test_outcomes(ran.output)
    for name in names:
        matching = [state for test, state in outcomes.items() if leaf(test) == name]
        results.record(stage, name, matching == ["ok"],
                       "" if matching == ["ok"] else failure_detail(ran.output))
    return ran


def stage_rdp_clients(results, log_root, executable, env):
    names = RDP_CLIENT_TESTS[platform_key()]
    xvfb = None
    environment = dict(env)
    if not WINDOWS:
        environment.pop("WAYLAND_DISPLAY", None)
        if shutil.which("Xvfb"):
            xvfb = subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1280x800x24", "-nolisten", "tcp"],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            environment["DISPLAY"] = ":99"
            for _ in range(100):
                if run(["xdpyinfo"], env=environment, timeout=10).ok:
                    break
                time.sleep(0.1)
    try:
        run_named(results, "rdp-clients", executable, names, environment, log_root / "rdp-clients.log")
    finally:
        if xvfb is not None:
            xvfb.terminate()
        if WINDOWS:
            run(["taskkill", "/F", "/IM", "mstsc.exe"], timeout=60)


def stage_elevated(results, log_root, executable, env):
    if WINDOWS:
        elevated = run(["net", "session"], timeout=60).ok
        if not elevated:
            results.record("elevated", H4, False, "the runner process is not elevated")
            return
        run_named(results, "elevated", executable, [H4], env, log_root / "elevated.log")
        return
    if not sudo_ok():
        results.record("elevated", H3, False, "passwordless sudo is not available")
        return
    # The test binary runs as root itself; cargo and the target folder stay the runner's.
    prefix = ["sudo", "-n", "env", "DEBIAN_FRONTEND=noninteractive", f"PATH={os.environ.get('PATH', '')}",
              "HOME=/root", "RUST_BACKTRACE=1"]
    run_named(results, "elevated", executable, [H3], env, log_root / "elevated.log", prefix=prefix)


def stage_transfer_task(results, log_root, work, env):
    executable, built = cargo_executable(["test", "--locked", "--test", "transfer_task"], "transfer_task",
                                         log_root / "transfer-task-build.log", env=env)
    results.record("transfer_task", "integration target builds", bool(executable),
                   "" if executable else built.tail(3000))
    if not executable:
        return
    listed = list_tests(executable, ignored=True, env=env)
    names = [name for name in listed if CRASH_LOG_PREFIX in name]
    if not names:
        results.record("transfer_task", f"{CRASH_LOG_PREFIX}* cases listed", False, "no ignored crash-log case")
        return
    g3 = [name for name in listed if leaf(name) in G3_FUNCTIONAL]
    results.record("transfer_task", "G3 correctness cases listed", len(g3) == len(G3_FUNCTIONAL),
                   ", ".join(sorted(set(G3_FUNCTIONAL) - {leaf(name) for name in g3})))
    environment = dict(env)
    servers = pyftp = None
    try:
        if WINDOWS:
            pyftp = PyFtp(work, log_root)
            try:
                pyftp.start()
                environment["SE_TASK_FTP_URL"] = pyftp.url()
            except Exception as error:  # noqa: BLE001
                results.record("transfer_task", "pyftpdlib server", False, error)
                return
            # No SFTP server without Linux containers; SMB and WebDAV print SKIP (optional).
            # The G3 cases need the SFTP and SMB containers.
            names = [name for name in names if "_sftp_" not in name]
            g3 = []
            expected_skips = ("SE_TASK_SMB", "SE_TASK_WEBDAV")
        else:
            servers = Servers(work, log_root).start(["sftp", "ftp", "smb", "webdav"])
            for kind, reason in servers.failed.items():
                results.record("transfer_task", f"{kind} server", False, reason)
            environment.update(servers.env())
            expected_skips = ()
        names = names + g3
        ran = run([executable, "--ignored", "--test-threads=1", "--exact", *names], cwd=NATIVE,
                  env=environment, timeout=3 * 3600, log=log_root / "transfer-task.log")
        outcomes = test_outcomes(ran.output)
        for name in names:
            results.record("transfer_task", leaf(name), outcomes.get(name) == "ok",
                           "" if outcomes.get(name) == "ok" else failure_detail(ran.output))
        skipped = sorted(set(re.findall(r"^SKIP (SE_TASK_\w+)", ran.output, re.MULTILINE)) - set(expected_skips))
        results.record("transfer_task", "every server case ran (no SKIP of a started server)", not skipped,
                       ", ".join(skipped))
    finally:
        if servers is not None:
            servers.close()
        if pyftp is not None:
            pyftp.close()


def job_functional(results, log_root):
    work = Path(tempfile.mkdtemp(prefix="se-speed-setup-"))
    # The copy-paste and GUI task constructors (`App::new_for_copy_task`, R3's
    # `speed_task_view_insert_keeps_sort_order_and_selection`) refuse to run
    # without their switch; every test then works in its own app data, never
    # the runner account's.
    env = dict(os.environ, SMART_EXPLORER_COPY_PASTE_TASK="1", SMART_EXPLORER_GUI_TASK="1", RUST_BACKTRACE="1")
    profile = work / "app-profile"
    isolated = ("APPDATA",) if WINDOWS else ("XDG_DATA_HOME", "XDG_CONFIG_HOME")
    for variable in isolated:
        (profile / variable.lower()).mkdir(parents=True, exist_ok=True)
        env[variable] = str(profile / variable.lower())
    try:
        executable, built = cargo_executable(["test", "--locked", "--lib"], "smart_explorer",
                                             log_root / "build.log", env=env)
        results.record("build", "library test binary (development profile)", bool(executable),
                       "" if executable else built.tail(4000))
        if not executable:
            return
        for stage in (stage_unit, stage_removable, stage_rdp_clients, stage_elevated):
            try:
                stage(results, log_root, executable, env)
            except Exception as error:  # noqa: BLE001 - every stage runs
                results.record(stage.__name__, "stage", False, f"{type(error).__name__}: {error}")
        try:
            stage_transfer_task(results, log_root, work, env)
        except Exception as error:  # noqa: BLE001
            results.record("transfer_task", "stage", False, f"{type(error).__name__}: {error}")
    finally:
        shutil.rmtree(work, ignore_errors=True)


# --------------------------------------------------------------------------- measurement ledger

def median_metrics(samples):
    keys = sorted({key for sample in samples for key, value in sample.items() if isinstance(value, float)})
    return {key: statistics.median([sample[key] for sample in samples if isinstance(sample.get(key), float)])
            for key in keys}


class Ledger:
    """Every measurement of the job: rows of timed runs and `BENCH` lines."""

    def __init__(self, results, log_root, part, deadline, reps):
        self.results, self.log_root, self.part = results, log_root, part
        self.deadline, self.reps = deadline, reps
        self.rows, self.bench, self.notes, self.facts = [], {}, [], {}

    def left(self):
        return self.deadline - time.monotonic()

    def row(self, path, dataset, link, mode, tool, kind):
        for row in self.rows:
            if (row["path"], row["dataset"], row["link"], row["mode"], row["tool"]) == (path, dataset, link,
                                                                                       mode, tool):
                return row
        row = dict(path=path, dataset=dataset, link=link, mode=mode, tool=tool, kind=kind, samples=[],
                   errors=[])
        self.rows.append(row)
        return row

    def sample(self, row, seconds, files, size):
        row["samples"].append(dict(seconds=seconds, mib_per_s=size / MIB / max(seconds, 1e-9),
                                   files_per_s=files / max(seconds, 1e-9)))

    def error(self, row, message):
        row["errors"].append(redact(str(message))[:600])
        if row["kind"] == "candidate" and not message.startswith("not run"):
            self.results.record("measure", f"candidate {row['tool']}: {row['path']} {row['dataset']} "
                                f"{row['link']} {row['mode']}".strip(), False, message[:1500])

    def note(self, message):
        self.notes.append(redact(message))
        print(f"[note] {redact(message)}", flush=True)

    def write(self):
        (self.log_root / "measurements.json").write_text(json.dumps(
            dict(part=self.part, facts=self.facts, notes=self.notes, rows=self.rows), indent=2), encoding="utf-8")
        bench = {case: dict(samples=samples, median=median_metrics(samples)) for case, samples in self.bench.items()}
        (self.log_root / "bench.json").write_text(json.dumps(bench, indent=2), encoding="utf-8")


def bench_lines(output):
    """`BENCH <case> key=value ...` lines: {case: {key: float or text}}."""
    found = {}
    for match in re.finditer(r"^BENCH (\S+)(.*)$", output, re.MULTILINE):
        metrics = {}
        for token in match.group(2).split():
            key, _, value = token.partition("=")
            try:
                metrics[key] = float(value)
            except ValueError:
                metrics[key] = value
        found[match.group(1)] = metrics
    return found


class Tool:
    def __init__(self, name, kind, command, env=None, ok=(0,), before=None):
        self.name, self.kind, self.command, self.env, self.ok, self.before = name, kind, command, env, ok, before


def run_group(ledger, *, path, dataset, link, mode, tools, reps, source, target_of, verify, cleanup,
              expected, timeout=3600, prepare=None):
    """Every tool `reps` times on the same data, the order rotated per repetition; one row per tool."""
    tools = [tool for tool in tools if tool is not None]
    log = ledger.log_root / "runs" / f"{re.sub(r'[^A-Za-z0-9]+', '-', path)}.log"
    for rep in range(reps):
        shift = rep % max(len(tools), 1)
        for tool in tools[shift:] + tools[:shift]:
            row = ledger.row(path, dataset, link, mode, tool.name, tool.kind)
            if ledger.left() < 900:
                ledger.error(row, "not run: time budget of the job exhausted")
                continue
            target = target_of(tool, rep)
            cleanup(target)
            run_source = source
            if prepare is not None:
                run_source = prepare(rep)
            if tool.before is not None:
                tool.before(target)
            drop_caches()
            ran = run(tool.command(run_source, target), env=tool.env, timeout=timeout, log=log)
            if ran.code not in tool.ok:
                ledger.error(row, f"exit {ran.code}: {ran.tail(800)}")
            else:
                try:
                    actual = verify(target)
                    if actual != expected:
                        ledger.error(row, f"result has {actual[0]} files / {actual[1]} bytes, "
                                     f"expected {expected[0]} / {expected[1]}")
                    else:
                        ledger.sample(row, ran.seconds, *expected)
                except Exception as error:  # noqa: BLE001
                    ledger.error(row, f"check failed: {type(error).__name__}: {error}")
            cleanup(target)
    for tool in tools:
        row = ledger.row(path, dataset, link, mode, tool.name, tool.kind)
        if row["kind"] == "candidate" and row["samples"] and not row["errors"]:
            ledger.results.record("measure", f"candidate {tool.name}: {path} {dataset} {link} {mode}".strip(),
                                  True)


def run_benchmarks(ledger, executable, names, env, reps, *, prefix="", timeout=3 * 3600, temp=None):
    """Each ignored benchmark alone, `reps` times; its BENCH lines go to the ledger by case."""
    log = ledger.log_root / "bench.log"
    for name in names:
        lines = 0
        failed = None
        for rep in range(reps):
            if ledger.left() < 1200:
                ledger.note(f"{leaf(name)}: repetition {rep + 1} not run: time budget of the job exhausted")
                break
            heavy = any(word in leaf(name) for word in ("large", "512m", "throughput", "2g"))
            if heavy and free_gib(temp or tempfile.gettempdir()) < BENCH_FREE_GIB:
                ledger.note(f"{leaf(name)}: not run: less than {BENCH_FREE_GIB} GiB free for its 2 GiB data set")
                break
            ran = run([executable, "--ignored", "--exact", name, "--nocapture", "--test-threads=1"], cwd=NATIVE,
                      env=env, timeout=timeout, log=log)
            if not ran.ok:
                failed = failure_detail(ran.output)
                break
            for case, metrics in bench_lines(ran.stdout).items():
                lines += 1
                metrics = dict(metrics, rep=rep + 1, test=leaf(name))
                ledger.bench.setdefault(prefix + case, []).append(metrics)
        ledger.results.record("bench", f"{prefix}{leaf(name)}", failed is None,
                              failed or f"{lines} BENCH lines")
        ledger.write()


# --------------------------------------------------------------------------- reference tools

def fetch_rclone(work, log):
    key = ("windows-amd64" if WINDOWS else "linux-amd64") if platform.machine().lower() in (
        "x86_64", "amd64") else None
    if key is None:
        return None, f"no rclone archive for {platform.machine()}"
    name = f"rclone-{RCLONE_VERSION}-{key}"
    folder = Path(work) / "rclone"
    folder.mkdir(parents=True, exist_ok=True)
    archive = folder / f"{name}.zip"
    try:
        url = f"https://github.com/rclone/rclone/releases/download/{RCLONE_VERSION}/{name}.zip"
        with urllib.request.urlopen(url, timeout=300) as response, open(archive, "wb") as handle:  # noqa: S310
            shutil.copyfileobj(response, handle)
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        ensure(actual == RCLONE_SHA256[key], f"rclone SHA-256 {actual} differs from {RCLONE_SHA256[key]}")
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(folder)
        binary = folder / name / f"rclone{EXE}"
        if not WINDOWS:
            binary.chmod(0o755)
        (folder / "empty.conf").write_text("")
        return binary, f"rclone {RCLONE_VERSION} (SHA-256 checked)"
    except Exception as error:  # noqa: BLE001 - the reason reaches the report
        with open(log, "a", encoding="utf-8") as handle:
            handle.write(f"rclone download failed: {error}\n")
        return None, f"rclone {RCLONE_VERSION} not available: {type(error).__name__}: {error}"


class Rclone:
    def __init__(self, binary, work):
        self.binary = binary
        self.config = Path(work) / "rclone" / "empty.conf"
        self.obscured = {}

    def obscure(self, password):
        if password not in self.obscured:
            value = run([self.binary, "obscure", password], timeout=60).stdout.strip()
            SECRETS.add(value)
            self.obscured[password] = value
        return self.obscured[password]

    def remote(self, kind, port, password, path, share=USER):
        secret = self.obscure(password)
        if kind == "sftp":
            return (f":sftp,host=127.0.0.1,port={port},user={USER},pass={secret},shell_type=none,"
                    f"md5sum_command=none,sha1sum_command=none:{path}")
        if kind == "ftp":
            return f":ftp,host=127.0.0.1,port={port},user={USER},pass={secret},concurrency=32:{path}"
        if kind == "smb":
            return f":smb,host=127.0.0.1,port={port},user={USER},pass={secret}:{share}/{path}"
        if kind == "webdav":
            return (f":webdav,url=\"http://127.0.0.1:{port}/\",vendor=other,user={USER},pass={secret}:"
                    f"{path.lstrip('/')}")
        raise ValueError(kind)

    def copy(self, source, target):
        return [self.binary, "copy", source, target, "--config", self.config, "--transfers", "16",
                "--checkers", "16", "--no-traverse", "-q"]


def make_dataset(base, name, files, folders, size):
    """`base/data-<name>/<name>`: incompressible files, `folders` folders (1 = files directly inside)."""
    root = Path(base) / f"data-{name}" / name
    if root.exists():
        return root
    root.mkdir(parents=True)
    per_folder = files // folders
    for folder_index in range(folders):
        folder = root if folders == 1 else root / f"d{folder_index:03d}"
        folder.mkdir(exist_ok=True)
        for index in range(per_folder):
            with open(folder / f"f{index:05d}.bin", "wb") as handle:
                left = size
                while left > 0:
                    chunk = min(left, 8 << 20)
                    handle.write(os.urandom(chunk))
                    left -= chunk
    return root


def datasets(ledger, work):
    made = []
    for name, files, folders, size in DATASETS:
        if size * files >= GIB and free_gib(work) < LARGE_FREE_GIB:
            ledger.note(f"data set {name}: not created, less than {LARGE_FREE_GIB} GiB free "
                        f"({free_gib(work):.1f} GiB)")
            continue
        root = make_dataset(work, name, files, folders, size)
        made.append((name, root, (files, files * size)))
    return made


# --------------------------------------------------------------------------- builds

class Bins:
    def __init__(self, work):
        self.dir = Path(work) / "bins"
        self.baseline = self.candidate = None
        self.lib_tests = self.transfer_task = None


def build_baseline(ledger, work, bins):
    log = ledger.log_root / "build-baseline.log"
    tree = Path(work) / "baseline-src"
    added = run(["git", "worktree", "add", "--detach", tree, BASELINE_TAG], cwd=ROOT, timeout=600, log=log)
    if not added.ok:
        ledger.note(f"baseline {BASELINE_TAG}: worktree failed: {added.tail(400)}")
        return
    # The candidate's target folder: the dependencies compile once. The
    # binary is copied out before the candidate overwrites target/release/se.
    env = dict(os.environ, CARGO_TARGET_DIR=str(NATIVE / "target"))
    built = run(["cargo", "build", "--locked", "--release", "--bin", "se"], cwd=tree / "native", env=env,
                timeout=4 * 3600, log=log)
    if not built.ok:
        ledger.note(f"baseline {BASELINE_TAG}: release build failed: {built.tail(600)}")
        return
    (bins.dir / "baseline").mkdir(parents=True, exist_ok=True)
    bins.baseline = bins.dir / "baseline" / f"se{EXE}"
    shutil.copy2(NATIVE / "target" / "release" / f"se{EXE}", bins.baseline)
    commit = run(["git", "rev-parse", BASELINE_TAG + "^{commit}"], cwd=ROOT).stdout.strip()
    ledger.facts["baseline"] = f"{BASELINE_TAG} ({commit})"


def build_candidate(ledger, bins, *, se=True, lib_tests=False, transfer_task=False):
    log = ledger.log_root / "build-candidate.log"
    if se:
        built = run(["cargo", "build", "--locked", "--release", "--bin", "se"], cwd=NATIVE, timeout=4 * 3600,
                    log=log)
        ledger.results.record("build", "candidate se (release)", built.ok, "" if built.ok else built.tail(3000))
        if built.ok:
            (bins.dir / "candidate").mkdir(parents=True, exist_ok=True)
            bins.candidate = bins.dir / "candidate" / f"se{EXE}"
            shutil.copy2(NATIVE / "target" / "release" / f"se{EXE}", bins.candidate)
    if lib_tests:
        bins.lib_tests, built = cargo_executable(["test", "--locked", "--release", "--lib"], "smart_explorer",
                                                 log)
        ledger.results.record("build", "candidate library benchmarks (release)", bool(bins.lib_tests),
                              "" if bins.lib_tests else built.tail(3000))
    if transfer_task:
        bins.transfer_task, built = cargo_executable(["test", "--locked", "--release", "--test", "transfer_task"],
                                                     "transfer_task", log)
        ledger.results.record("build", "candidate transfer_task benchmarks (release)", bool(bins.transfer_task),
                              "" if bins.transfer_task else built.tail(3000))
    ledger.facts["candidate"] = run(["git", "rev-parse", "HEAD"], cwd=ROOT).stdout.strip()


def se_tools(bins, clients, command):
    """The baseline and candidate `se` as tools; `command(se, source, target)` builds the argv."""
    tools = []
    for kind, binary, label in (("baseline", bins.baseline, f"se {BASELINE_TAG}"),
                                ("candidate", bins.candidate, "se candidate")):
        if binary is not None and kind in clients:
            tools.append(Tool(label, kind, (lambda b: lambda s, t: command(b, s, t))(binary),
                              env=clients[kind].env))
    return tools


# --------------------------------------------------------------------------- part local

def second_volume(ledger, work):
    """A folder on another volume than the work folder (cross-volume moves), and one on another
    volume than the temp folder (SE_BENCH_SECOND_VOLUME of the library benchmarks)."""
    if not WINDOWS:
        shm = Path("/dev/shm")
        try:
            if shm.is_dir() and os.stat(shm).st_dev != os.stat(work).st_dev:
                folder = shm / f"se-speed-{os.getpid()}"
                folder.mkdir(exist_ok=True)
                return folder, None
        except OSError:
            pass
        return None, None
    work_drive = Path(work).drive.upper()
    temp_drive = Path(tempfile.gettempdir()).drive.upper()
    if work_drive and temp_drive and work_drive != temp_drive:
        other = Path(tempfile.gettempdir()) / f"se-speed-second-{os.getpid()}"
        other.mkdir(exist_ok=True)
        lib = Path(work) / "second-for-lib"
        lib.mkdir(exist_ok=True)
        return other, lib
    letter = next((letter for letter in "STUVWX" if not os.path.exists(f"{letter}:\\")), None)
    if letter is None:
        return None, None
    vhd = Path(work) / "second.vhdx"
    script = Path(work) / "second.diskpart"
    script.write_text("\n".join([f'create vdisk file="{vhd}" maximum=16384 type=expandable',
                                 f'select vdisk file="{vhd}"', "attach vdisk", "convert mbr",
                                 "create partition primary", "format fs=ntfs quick label=SESECOND",
                                 f"assign letter={letter}"]) + "\n", encoding="ascii")
    made = run(["diskpart", "/s", script], timeout=900, log=ledger.log_root / "second-volume.log")
    if not made.ok or not os.path.exists(f"{letter}:\\"):
        ledger.note(f"second volume: diskpart failed: {made.tail(300)}")
        return None, None
    folder = Path(f"{letter}:\\se-speed")
    folder.mkdir(exist_ok=True)
    ledger.facts["second volume"] = f"NTFS VHDX {letter}: on the work disk"
    return folder, folder


def defender(ledger, action, path=None):
    if not WINDOWS:
        return None
    scripts = {
        "status": "Get-MpComputerStatus | Select-Object AMServiceEnabled,RealTimeProtectionEnabled | "
                  "ConvertTo-Json -Compress",
        "on": "Set-MpPreference -DisableRealtimeMonitoring $false",
        "off": "Set-MpPreference -DisableRealtimeMonitoring $true",
        "exclude": f"Add-MpPreference -ExclusionPath '{path}'",
        "include": f"Remove-MpPreference -ExclusionPath '{path}'",
    }
    ran = run(["powershell", "-NoProfile", "-NonInteractive", "-Command", scripts[action]], timeout=300,
              log=ledger.log_root / "defender.log")
    if action == "status":
        try:
            state = json.loads(ran.stdout.strip() or "{}")
            return "on" if state.get("RealTimeProtectionEnabled") else "off"
        except json.JSONDecodeError:
            return f"unknown ({ran.tail(120)})"
    return ran.ok


def local_tools(bins, clients, mode):
    if mode == "copy":
        tools = ([Tool("robocopy /MT:16", "reference",
                       lambda s, t: ["robocopy", s, t, "/E", "/MT:16", "/NFL", "/NDL", "/NJH", "/NJS", "/NP",
                                     "/R:0", "/W:0"], ok=tuple(range(8)))]
                 if WINDOWS else
                 [Tool("cp -r", "reference", lambda s, t: ["cp", "-r", s, t]),
                  Tool("rsync -a", "reference", lambda s, t: ["rsync", "-a", f"{s}/", f"{t}/"])
                  if shutil.which("rsync") else None])
        return tools + se_tools(bins, clients, lambda b, s, t: [b, "cp", "-r", s, t])
    tools = ([Tool("robocopy /MOVE /MT:16", "reference",
                   lambda s, t: ["robocopy", s, t, "/E", "/MOVE", "/MT:16", "/NFL", "/NDL", "/NJH", "/NJS", "/NP",
                                 "/R:0", "/W:0"], ok=tuple(range(8)))]
             if WINDOWS else [Tool("mv", "reference", lambda s, t: ["mv", s, t])])
    return tools + se_tools(bins, clients, lambda b, s, t: [b, "mv", s, t])


def untimed_copy(source, target):
    remove(target)
    if WINDOWS:
        run(["robocopy", source, target, "/E", "/MT:16", "/NFL", "/NDL", "/NJH", "/NJS", "/NP"], timeout=3600)
    else:
        run(["cp", "-r", source, target], timeout=3600)


def local_user_level(ledger, work, bins, clients, data, *, mode_label="", only=None):
    out = Path(work) / "out"
    out.mkdir(exist_ok=True)
    other, _ = ledger.facts.get("_second", (None, None))
    for name, source, expected in data:
        if only and name not in only:
            continue
        run_group(ledger, path="local copy, same volume", dataset=name, link="local", mode=mode_label,
                  tools=local_tools(bins, clients, "copy"), reps=ledger.reps, source=str(source),
                  target_of=lambda tool, rep: str(out / f"copy-{tool.kind}-{rep}"),
                  verify=tree_stats, cleanup=remove, expected=expected)
        if other is None:
            ledger.note("local move to another volume: no second volume on this runner")
            continue
        staging = Path(work) / "move-source"

        def prepare(_rep, source=source, staging=staging):
            untimed_copy(str(source), str(staging))
            return str(staging)
        run_group(ledger, path="local move, other volume", dataset=name, link="local", mode=mode_label,
                  tools=local_tools(bins, clients, "move"), reps=ledger.reps, source=str(staging),
                  target_of=lambda tool, rep: str(Path(other) / f"moved-{tool.kind}-{rep}"),
                  verify=tree_stats, cleanup=remove, expected=expected, prepare=prepare)
        remove(staging)


def iperf(ledger, link):
    if WINDOWS or not shutil.which("iperf3"):
        ledger.note(f"iperf3 ({link}): not run, iperf3 is not installed on this runner")
        return
    port = free_port()
    server = subprocess.Popen(["iperf3", "-s", "-p", str(port), "-B", "127.0.0.1"], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL)
    try:
        if not wait_tcp(port, None, 30):
            ledger.note(f"iperf3 ({link}): server did not listen")
            return
        for streams in (1, 4):
            ran = run(["iperf3", "-c", "127.0.0.1", "-p", str(port), "-t", "8", "-P", str(streams), "-J"],
                      timeout=120, log=ledger.log_root / "iperf3.log")
            row = ledger.row("loopback TCP (iperf3)", "-", link, "", f"iperf3 -P{streams}", "reference")
            try:
                bits = json.loads(ran.stdout)["end"]["sum_received"]["bits_per_second"]
                row["samples"].append(dict(seconds=8.0, mib_per_s=bits / 8 / MIB, files_per_s=0.0))
            except (ValueError, KeyError) as error:
                ledger.error(row, f"iperf3: {error}: {ran.tail(300)}")
    finally:
        server.terminate()


def lib_benchmark_env(work, lib_second):
    profile = Path(work) / "bench-profile"
    env = dict(os.environ, SMART_EXPLORER_COPY_PASTE_TASK="1", RUST_BACKTRACE="1")
    for variable, child in (("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"),
                            ("APPDATA", "appdata"), ("LOCALAPPDATA", "localappdata")):
        (profile / child).mkdir(parents=True, exist_ok=True)
        env[variable] = str(profile / child)
    if lib_second is not None:
        env["SE_BENCH_SECOND_VOLUME"] = str(lib_second)
    return env


def part_local(ledger, work):
    bins = Bins(work)
    build_baseline(ledger, work, bins)
    build_candidate(ledger, bins, se=True, lib_tests=True)
    clients = {kind: ai.Client(work, f"se-{kind}") for kind in ("baseline", "candidate")}
    other, lib_second = second_volume(ledger, work)
    ledger.facts["_second"] = (str(other) if other else None, str(lib_second) if lib_second else None)
    data = datasets(ledger, work)
    try:
        if WINDOWS:
            original = defender(ledger, "status")
            ledger.facts["defender at start"] = original
            local_user_level(ledger, work, bins, clients, data, mode_label=f"Defender real-time {original}")
            if defender(ledger, "on"):
                state = defender(ledger, "status")
                local_user_level(ledger, work, bins, clients, data, mode_label=f"Defender real-time {state}",
                                 only=("small",))
                if defender(ledger, "exclude", work):
                    local_user_level(ledger, work, bins, clients, data,
                                     mode_label=f"Defender real-time {state}, work folder excluded",
                                     only=("small",))
                    defender(ledger, "include", work)
                if original == "off":
                    defender(ledger, "off")
            else:
                ledger.note("Defender real-time protection could not be switched on; only the default state "
                            "was measured")
        else:
            local_user_level(ledger, work, bins, clients, data)
            iperf(ledger, "loopback")
        ledger.write()
        if bins.lib_tests:
            names = [name for name in list_tests(bins.lib_tests, ignored=True)
                     if "speed_task_" in leaf(name) and leaf(name) not in NOT_BENCHMARKS]
            ledger.note(f"not run here: {', '.join(NOT_BENCHMARKS)} (an older SMB fixture test, not a benchmark; "
                        "it needs SE_SYNC_PROVIDER_MANIFEST of the sync reliability suite)")
            ledger.facts["library benchmarks"] = len(names)
            env = lib_benchmark_env(work, lib_second)
            run_benchmarks(ledger, bins.lib_tests, names, env, ledger.reps)
    finally:
        ai.stop_background_daemons(list(clients.values()))


# --------------------------------------------------------------------------- remote parts

def remote_tools(kind, info, password, rclone, direction, work):
    """Reference tools of one backend: commands for (source, target) of an upload or a download."""
    tools = []
    remote_root = info["remote"].rstrip("/")

    def rclone_path(name):
        # SMB and WebDAV paths are relative to the share or the DAV root.
        return name if kind in ("smb", "webdav") else f"{remote_root}/{name}"
    if rclone is not None:
        if direction == "up":
            tools.append(Tool("rclone", "reference", lambda s, t: rclone.copy(
                s, rclone.remote(kind, info["port"], password, rclone_path(t)))))
        else:
            tools.append(Tool("rclone", "reference", lambda s, t: rclone.copy(
                rclone.remote(kind, info["port"], password, rclone_path(s)), t)))
    if kind in ("sftp", "ftp") and shutil.which("lftp"):
        scheme = "sftp" if kind == "sftp" else "ftp"
        settings = ("set sftp:auto-confirm yes; set net:max-retries 2; set net:reconnect-interval-base 1; "
                    "set ftp:passive-mode on; set ftp:prefer-epsv no; ")

        def lftp(mirror):
            return ["lftp", "-u", f"{USER},{password}", "-e", f"{settings}{mirror}; quit",
                    f"{scheme}://127.0.0.1:{info['port']}"]
        if direction == "up":
            tools.append(Tool("lftp mirror --parallel=16", "reference",
                              lambda s, t: lftp(f"mirror -R --parallel=16 --no-perms {s} {remote_root}/{t}")))
        else:
            tools.append(Tool("lftp mirror --parallel=16", "reference",
                              lambda s, t: lftp(f"mirror --parallel=16 --no-perms {remote_root}/{s} {t}")))
    if kind == "sftp" and shutil.which("scp"):
        options = ["-q", "-r", "-P", str(info["port"]), "-i", info["key"], "-o", "BatchMode=yes",
                   "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null"]
        if direction == "up":
            tools.append(Tool("scp -r", "reference",
                              lambda s, t: ["scp", *options, s, f"{USER}@127.0.0.1:{remote_root}/{t}"]))
        else:
            tools.append(Tool("scp -r", "reference",
                              lambda s, t: ["scp", *options, f"{USER}@127.0.0.1:{remote_root}/{s}", t]))
    if kind == "smb" and shutil.which("smbclient"):
        auth = Path(work) / "smb-auth.txt"
        auth.write_text(f"username = {USER}\npassword = {password}\n")
        auth.chmod(0o600)
        base = ["smbclient", f"//127.0.0.1/{info['share']}", "-p", str(info["port"]), "-A", str(auth), "-m", "SMB3"]
        if direction == "up":
            tools.append(Tool("smbclient mput", "reference", lambda s, t: [
                *base, "-c", f"mkdir {t}; cd {t}; lcd {s}; prompt OFF; recurse ON; mput *"]))
        else:
            tools.append(Tool("smbclient mget", "reference", lambda s, t: [
                *base, "-c", f"lcd {t}; cd {s}; prompt OFF; recurse ON; mget *"],
                before=lambda t: Path(t).mkdir(parents=True, exist_ok=True)))
    return tools


def add_connection(ledger, client, binary, kind, info, password, label):
    """A saved connection of one `se` version; None when it works, else the reason."""
    log = ledger.log_root / f"connections-{label}.log"
    roots = {"sftp": info.get("remote"), "ftp": info.get("remote"), "smb": info.get("remote")}
    added = client.se(binary, "connections", "add", kind, "--host", "127.0.0.1", "--port", str(info["port"]),
                      "--user", USER, "--root", roots[kind] or "/", "--label", label, "--password-stdin",
                      input_text=password + "\n", timeout=300, log=log)
    if added.returncode != 0:
        return f"connections add: {redact((added.stdout + added.stderr).decode(errors='replace'))[-400:]}"
    if kind == "sftp":
        try:
            ai.trust_ssh_host_key(client, binary, "127.0.0.1", info["port"], log)
        except Exception as error:  # noqa: BLE001
            return f"host key: {error}"
    listed = client.se(binary, "ls", f"@{label}:/", timeout=600, log=log)
    if listed.returncode != 0:
        return f"ls: {redact((listed.stdout + listed.stderr).decode(errors='replace'))[-400:]}"
    return None


def connect_clients(ledger, bins, clients, kinds, info_of, password_of):
    """`{(kind, version): label}` of every working saved connection."""
    labels = {}
    for kind in kinds:
        for version, binary in (("baseline", bins.baseline), ("candidate", bins.candidate)):
            if binary is None:
                continue
            label = f"{kind}-{version}"
            problem = add_connection(ledger, clients[version], binary, kind, info_of(kind), password_of(kind),
                                     label)
            if problem:
                ledger.note(f"{version} se: saved {kind} connection unusable: {problem}")
                if version == "candidate":
                    ledger.results.record("measure", f"candidate se: saved {kind} connection", False, problem)
            else:
                labels[(kind, version)] = label
    return labels


def remote_cli_groups(ledger, *, kind, info, password, rclone, data, bins, clients, labels, link, reps,
                      seed, stats, purge, work, names=None, references=True, path_label=None):
    """Up and down of every data set: reference tools, v0.5.188 and the candidate on one backend."""
    down_root = Path(work) / "down"
    down_root.mkdir(exist_ok=True)
    path_label = path_label or kind
    for name, source, expected in data:
        if names and name not in names:
            continue
        seed_name = f"seed-{name}"
        try:
            seed(kind, source, seed_name)
        except Exception as error:  # noqa: BLE001
            ledger.note(f"{path_label}: seeding {name} failed: {error}")
            continue
        for direction in ("up", "down"):
            tools = remote_tools(kind, info, password, rclone, direction, work) if references else []
            if not references and rclone is not None:
                tools = [tool for tool in remote_tools(kind, info, password, rclone, direction, work)
                         if tool.name == "rclone"]
            for version in ("baseline", "candidate"):
                label = labels.get((kind, version))
                if label is None:
                    continue
                binary = bins.baseline if version == "baseline" else bins.candidate
                tag = f"se {BASELINE_TAG}" if version == "baseline" else "se candidate"
                if direction == "up":
                    command = (lambda b, l: lambda s, t: [b, "cp", "-r", s, f"@{l}:/{t}"])(binary, label)
                else:
                    command = (lambda b, l: lambda s, t: [b, "cp", "-r", f"@{l}:/{s}", t])(binary, label)
                tools.append(Tool(tag, version, command, env=clients[version].env))
            if direction == "up":
                run_group(ledger, path=f"{path_label} up", dataset=name, link=link, mode="", tools=tools,
                          reps=reps, source=str(source),
                          target_of=lambda tool, rep: f"up-{re.sub(r'[^a-z0-9]+', '', tool.name.lower())}-{rep}",
                          verify=lambda t: stats(kind, t), cleanup=lambda t: purge(kind, t), expected=expected)
            else:
                run_group(ledger, path=f"{path_label} down", dataset=name, link=link, mode="", tools=tools,
                          reps=reps, source=seed_name,
                          target_of=lambda tool, rep: str(down_root / f"down-{tool.kind}-{rep}"),
                          verify=tree_stats, cleanup=remove, expected=expected)
        ledger.write()


@contextlib.contextmanager
def netem(ledger, delay_ms):
    """`delay <n>ms` on `lo` for one group of measurements (each direction); None when unavailable."""
    if WINDOWS or not sudo_ok() or not shutil.which("tc"):
        yield "sudo -n or tc is missing"
        return
    log = ledger.log_root / "netem.log"
    run(["sudo", "-n", "tc", "qdisc", "del", "dev", "lo", "root"], timeout=60, log=log)
    if not run(["sudo", "-n", "modprobe", "sch_netem"], timeout=120, log=log).ok:
        release = run(["uname", "-r"], timeout=30).stdout.strip()
        run(["sudo", "-n", "env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y",
             f"linux-modules-extra-{release}"], timeout=900, log=log)
        run(["sudo", "-n", "modprobe", "sch_netem"], timeout=120, log=log)
    added = run(["sudo", "-n", "tc", "qdisc", "add", "dev", "lo", "root", "netem", "delay", f"{delay_ms}ms",
                 "limit", "100000"], timeout=60, log=log)
    shown = run(["tc", "qdisc", "show", "dev", "lo"], timeout=30, log=log).stdout
    if not added.ok or "netem" not in shown:
        run(["sudo", "-n", "tc", "qdisc", "del", "dev", "lo", "root"], timeout=60, log=log)
        yield f"netem delay {delay_ms}ms could not be set: {added.tail(200)}"
        return
    try:
        yield None
    finally:
        run(["sudo", "-n", "tc", "qdisc", "del", "dev", "lo", "root"], timeout=60, log=log)


def share_pair(ledger, work, binary, version, server):
    """Two isolated `se` daemons of one version, paired Direct over a local share server; the
    remote device exports a writable folder. Returns a namespace with `endpoint`, `export`, `main`."""
    pair = _load(f"share_fixture_{version}", NATIVE / "sync-reliability-share-fixture.py")
    root = Path(work) / f"share-{version}"
    root.mkdir(parents=True, exist_ok=True)
    root.chmod(0o700)
    logs = ledger.log_root / f"share-{version}"
    logs.mkdir(parents=True, exist_ok=True)
    owner = types.SimpleNamespace(root=root, logs=logs, cleanups=[])
    signal_port, relay_port = pair.pair_ports()
    cert, key = root / "server.pem", root / "server.key"
    made = run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-keyout", key, "-out",
                cert, "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1", "-addext",
                "basicConstraints=critical,CA:FALSE"], timeout=120, log=logs / "openssl.log")
    ensure(made.ok, f"openssl: {made.tail(300)}")
    run(["openssl", "x509", "-in", cert, "-outform", "DER", "-out", root / "server.der"], timeout=60)
    pin = hashlib.sha256((root / "server.der").read_bytes()).hexdigest()
    process = subprocess.Popen(
        [str(server), f"127.0.0.1:{signal_port}", "--tls-cert", str(cert), "--tls-key", str(key), "--state-file",
         str(root / "state.json")], stdin=subprocess.DEVNULL, stdout=open(logs / "share-server.log", "ab"),
        stderr=subprocess.STDOUT,
        env=dict(os.environ, SE_SHARE_IDLE_KEEPALIVE_SECS="30", SE_SHARE_ALLOW_PLAINTEXT="0",
                 SE_SHARE_REQUIRE_KEY_LOGIN="0", SE_IROH_RELAY_DISABLE="0",
                 SE_IROH_RELAY_BIND=f"127.0.0.1:{relay_port}", SE_SHARE_FIXTURE_ROOT=str(root)))
    handle = types.SimpleNamespace(process=process, owner=owner, clients=[])

    def close():
        for client in reversed(handle.clients):
            try:
                client.close()
            except Exception as error:  # noqa: BLE001
                ledger.note(f"share pair {version}: cleanup: {error}")
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
    handle.close = close
    try:
        pair.wait_server(process, (signal_port, relay_port))
        relay = f"https://127.0.0.1:{relay_port}"
        base = dict(os.environ, SE_SHARE_RELAY_URL=relay)
        main = pair.Client(owner, Path(binary), pair.isolated(base, root / "main"), "main")
        handle.clients.append(main)
        remote = pair.Client(owner, Path(binary), pair.isolated(base, root / "remote"), "remote")
        handle.clients.append(remote)
        for client in (main, remote):
            client.server_process = process
            client.call("share", "configure", "--server", f"wss://127.0.0.1:{signal_port}/#sha256={pin}")
            client.wait(lambda c=client: (lambda worker: worker.get("reachable") and worker.get("running")
                                          and worker.get("connected"))(
                json.loads(c.call("share", "status", "--json").stdout).get("worker", {})), "connected worker")
        identity = json.loads(remote.call("share", "identity", "--json").stdout)
        direct = json.loads(main.call("connections", "add-peer", "--code", identity["direct_code"], "--name",
                                      "SpeedPeer", "--json").stdout)
        remote.wait(lambda: json.loads(remote.call("share", "request", "--json").stdout).get("count", 0) == 1,
                    "incoming request")
        remote.call("share", "request", "accept", "--json")
        remote.call("share", "grants", "set", pair.identity_of(main), "--write", "--json")
        export = root / "export"
        export.mkdir(exist_ok=True)
        remote.call("share", "export", "add", str(export), "--label", "SpeedExport")
        remote.call("share", "export", "set", "SpeedExport", "--write")
        remote.call("share", "worker", "refresh")
        endpoint = direct["endpoint"] + "/SpeedExport"
        main.wait(lambda: main.call("ls", endpoint, check=False).returncode == 0, "exported folder")
        handle.endpoint, handle.export, handle.main = endpoint, export, main
        return handle
    except Exception:
        close()
        raise


def share_groups(ledger, work, bins, data):
    """`se cp` up and down over a Direct pair on loopback, separately for v0.5.188 and the candidate."""
    log = ledger.log_root / "build-share-server.log"
    server_dir = ROOT / "share-server"
    built = run(["cargo", "build", "--locked", "--bin", "se-share-server"], cwd=server_dir,
                env=dict(os.environ, CARGO_TARGET_DIR=str(server_dir / "target")), timeout=3600, log=log)
    server = server_dir / "target" / "debug" / f"se-share-server{EXE}"
    if not built.ok or not server.exists():
        ledger.note(f"Share pair: the development share server did not build: {built.tail(400)}")
        return
    down_root = Path(work) / "share-down"
    down_root.mkdir(exist_ok=True)
    for version, binary in (("baseline", bins.baseline), ("candidate", bins.candidate)):
        if binary is None or ledger.left() < 1800:
            continue
        tag = f"se {BASELINE_TAG}" if version == "baseline" else "se candidate"
        try:
            pair = share_pair(ledger, work, binary, version, server)
        except Exception as error:  # noqa: BLE001
            ledger.note(f"Share pair {version}: setup failed: {type(error).__name__}: {error}")
            if version == "candidate":
                ledger.results.record("measure", "candidate Share pair (Direct, loopback)", False, str(error))
            continue
        try:
            for name, source, expected in data:
                export = Path(pair.export)
                up = Tool(tag, version, lambda s, t: [pair.main.cli, "cp", "-r", s, f"{pair.endpoint}/{t}"],
                          env=pair.main.env)
                run_group(ledger, path="Share Direct up", dataset=name, link="loopback", mode="", tools=[up],
                          reps=ledger.reps, source=str(source),
                          target_of=lambda tool, rep: f"up-{rep}",
                          verify=lambda t: tree_stats(export / t), cleanup=lambda t: remove(export / t),
                          expected=expected)
                seed = export / f"seed-{name}"
                untimed_copy(str(source), str(seed))
                down = Tool(tag, version, lambda s, t: [pair.main.cli, "cp", "-r", f"{pair.endpoint}/{s}", t],
                            env=pair.main.env)
                run_group(ledger, path="Share Direct down", dataset=name, link="loopback", mode="", tools=[down],
                          reps=ledger.reps, source=f"seed-{name}",
                          target_of=lambda tool, rep: str(down_root / f"down-{rep}"),
                          verify=tree_stats, cleanup=remove, expected=expected)
                remove(seed)
                ledger.write()
        finally:
            pair.close()


def part_remote_cli(ledger, work, args):
    bins = Bins(work)
    build_baseline(ledger, work, bins)
    build_candidate(ledger, bins, se=True)
    rclone_binary, rclone_note = fetch_rclone(work, ledger.log_root / "rclone.log")
    ledger.facts["rclone"] = rclone_note
    rclone = Rclone(rclone_binary, work) if rclone_binary else None
    servers = Servers(work, ledger.log_root).start(["sftp", "ftp", "smb"])
    for kind, reason in servers.failed.items():
        ledger.note(f"{kind} server: {reason}")
        ledger.results.record("measure", f"{kind} server", False, reason)
    clients = {kind: ai.Client(work, f"se-{kind}") for kind in ("baseline", "candidate")}
    try:
        kinds = [kind for kind in ("sftp", "ftp", "smb") if kind in servers.info]
        labels = connect_clients(ledger, bins, clients, kinds, lambda kind: servers.info[kind],
                                 lambda kind: servers.password)
        data = datasets(ledger, work)
        iperf(ledger, "loopback")
        for kind in kinds:
            remote_cli_groups(ledger, kind=kind, info=servers.info[kind], password=servers.password, rclone=rclone,
                              data=data, bins=bins, clients=clients, labels=labels, link="loopback",
                              reps=ledger.reps, seed=servers.seed, stats=servers.stats, purge=servers.purge,
                              work=work)
        share_groups(ledger, work, bins, data)
        if args.netem:
            for delay in (1, 20):
                with netem(ledger, delay) as problem:
                    if problem:
                        ledger.note(f"netem {delay} ms: not measured: {problem}")
                        continue
                    link = f"netem {delay} ms each way"
                    iperf(ledger, link)
                    for kind in kinds:
                        remote_cli_groups(ledger, kind=kind, info=servers.info[kind], password=servers.password,
                                          rclone=rclone, data=data, bins=bins, clients=clients, labels=labels,
                                          link=link, reps=args.netem_repetitions, seed=servers.seed,
                                          stats=servers.stats, purge=servers.purge, work=work,
                                          names=("small", "medium"), references=False)
    finally:
        ai.stop_background_daemons(list(clients.values()))
        servers.close()


def bench_rows(ledger, link, prefix=""):
    """`BENCH <backend>_<case>_<up|down|same_server>_<plain|crash_log>` of transfer_task as rows next to
    the reference tools of the same path and data set."""
    cases = {"small_50_folders": "small", "medium": "medium", "large": "large"}
    for case, samples in ledger.bench.items():
        if prefix and not case.startswith(prefix):
            continue
        match = re.match(r"^(sftp|ftp|smb|webdav)_(small_50_folders|small_one_folder|medium|large)_"
                         r"(up|down|same_server)_(plain|crash_log)$", case[len(prefix):])
        if not match or match.group(2) not in cases or match.group(3) == "same_server":
            continue
        backend, dataset, direction, mode = match.group(1), cases[match.group(2)], match.group(3), match.group(4)
        row = ledger.row(f"{backend} {direction}", dataset, link, mode.replace("_", " "),
                         f"engine ({mode.replace('_', ' ')})", "candidate")
        row["samples"] = [dict(seconds=0.0, mib_per_s=sample.get("mib_per_s", 0.0),
                               files_per_s=sample.get("files_per_s", 0.0)) for sample in samples
                          if isinstance(sample.get("mib_per_s"), float)]


def part_remote_bench(ledger, work, args):
    bins = Bins(work)
    build_candidate(ledger, bins, se=False, transfer_task=True)
    rclone_binary, rclone_note = fetch_rclone(work, ledger.log_root / "rclone.log")
    ledger.facts["rclone"] = rclone_note
    rclone = Rclone(rclone_binary, work) if rclone_binary else None
    servers = Servers(work, ledger.log_root).start(["sftp", "ftp", "smb", "webdav", "nextcloud"])
    for kind, reason in servers.failed.items():
        ledger.note(f"{kind} server: {reason}")
        ledger.results.record("measure", f"{kind} server", False, reason)
    pyftp = None
    try:
        data = datasets(ledger, work)
        iperf(ledger, "loopback")
        kinds = [kind for kind in ("sftp", "ftp", "smb", "webdav") if kind in servers.info]
        for kind in kinds:
            remote_cli_groups(ledger, kind=kind, info=servers.info[kind], password=servers.password, rclone=rclone,
                              data=data, bins=bins, clients={}, labels={}, link="loopback", reps=ledger.reps,
                              seed=servers.seed, stats=servers.stats, purge=servers.purge, work=work)
        if bins.transfer_task:
            env = dict(os.environ, RUST_BACKTRACE="1", **servers.env())
            # Nextcloud has no `se cp` path (saved WebDAV connections are HTTPS only):
            # its chunk-upload case and benchmarks run here only.
            bench_kinds = kinds + [kind for kind in ("nextcloud",) if kind in servers.info]
            names = [name for name in list_tests(bins.transfer_task, ignored=True, env=env)
                     if "speed_task_remote_" in leaf(name)
                     and any(f"_{kind}_" in leaf(name) for kind in bench_kinds)]
            ledger.facts["transfer_task benchmarks"] = len(names)
            run_benchmarks(ledger, bins.transfer_task, names, env, ledger.reps)
            bench_rows(ledger, "loopback")
            # The cached FTP path needs MLST (KP3); vsftpd has none, pyftpdlib does.
            try:
                pyftp = PyFtp(work, ledger.log_root).start()
                env_mlst = dict(env, SE_TASK_FTP_URL=pyftp.url())
                ftp_names = [name for name in names if "_ftp_" in leaf(name)] or [
                    name for name in list_tests(bins.transfer_task, ignored=True, env=env_mlst)
                    if "speed_task_remote_ftp_" in leaf(name)]
                run_benchmarks(ledger, bins.transfer_task, ftp_names, env_mlst, 1, prefix="mlst_")
                bench_rows(ledger, "pyftpdlib (MLST)", prefix="mlst_")
            except Exception as error:  # noqa: BLE001
                ledger.note(f"FTP with MLST (pyftpdlib): not measured: {error}")
    finally:
        if pyftp is not None:
            pyftp.close()
        servers.close()


def part_windows_remote(ledger, work, args):
    bins = Bins(work)
    build_baseline(ledger, work, bins)
    build_candidate(ledger, bins, se=True, transfer_task=True)
    rclone_binary, rclone_note = fetch_rclone(work, ledger.log_root / "rclone.log")
    ledger.facts["rclone"] = rclone_note
    rclone = Rclone(rclone_binary, work) if rclone_binary else None
    clients = {kind: ai.Client(work, f"se-{kind}") for kind in ("baseline", "candidate")}
    pyftp = PyFtp(work, ledger.log_root)
    try:
        try:
            pyftp.start()
        except Exception as error:  # noqa: BLE001
            ledger.results.record("measure", "pyftpdlib server", False, error)
            return
        info = dict(port=pyftp.port, remote="/")
        labels = connect_clients(ledger, bins, clients, ["ftp"], lambda kind: info, lambda kind: pyftp.password)
        data = datasets(ledger, work)
        root = pyftp.root

        def seed(_kind, source, name):
            untimed_copy(str(source), str(root / name))

        remote_cli_groups(ledger, kind="ftp", info=info, password=pyftp.password, rclone=rclone, data=data,
                          bins=bins, clients=clients, labels=labels, link="loopback (pyftpdlib)",
                          reps=ledger.reps, seed=seed, stats=lambda _kind, name: tree_stats(root / name),
                          purge=lambda _kind, name: remove(root / name), work=work)
        if bins.transfer_task:
            env = dict(os.environ, RUST_BACKTRACE="1", SE_TASK_FTP_URL=pyftp.url())
            names = [name for name in list_tests(bins.transfer_task, ignored=True, env=env)
                     if "speed_task_remote_ftp_" in leaf(name)]
            ledger.note("speed_task_remote_sftp_/_smb_/_webdav_ on Windows: not run, their servers need Linux "
                        "containers (measured in the Linux remote-bench job)")
            run_benchmarks(ledger, bins.transfer_task, names, env, ledger.reps)
            bench_rows(ledger, "loopback (pyftpdlib)")
    finally:
        ai.stop_background_daemons(list(clients.values()))
        pyftp.close()


# --------------------------------------------------------------------------- report

def cell(row):
    if row is None:
        return "–"
    samples = row["samples"]
    if not samples:
        return "not run" if any(error.startswith("not run") for error in row["errors"]) else "failed"
    medians = median_metrics(samples)
    text = f"{medians.get('mib_per_s', 0.0):.1f} MiB/s"
    if medians.get("files_per_s"):
        text += f" · {medians['files_per_s']:.0f} files/s"
    return text + f" (n={len(samples)}{', errors ' + str(len(row['errors'])) if row['errors'] else ''})"


def speed(row):
    if row is None or not row["samples"]:
        return None
    return median_metrics(row["samples"]).get("mib_per_s")


def ratio(a, b):
    return f"{a / b:.2f}" if a and b else "–"


def write_summary(ledger, results, title):
    lines = [f"## {title}", ""]
    for key, value in ledger.facts.items():
        if not key.startswith("_"):
            lines.append(f"- {key}: {value}")
    lines.append(f"- repetitions: {ledger.reps} (median shown); runner: {platform.platform()}, "
                 f"{os.cpu_count()} CPUs")
    lines += ["", "### Per path: reference | baseline | candidate", "",
              f"| Path | Data | Link | Variant | Reference (fastest) | {BASELINE_TAG} | Candidate | "
              "Candidate ÷ reference | Candidate ÷ baseline |",
              "|---|---|---|---|---|---|---|---|---|"]
    groups = []
    for row in ledger.rows:
        key = (row["path"], row["dataset"], row["link"], row["mode"])
        if key not in groups and row["kind"] in ("candidate", "baseline"):
            groups.append(key)
    for path, dataset, link, mode in groups:
        rows = [row for row in ledger.rows if (row["path"], row["dataset"], row["link"]) == (path, dataset, link)]
        references = [row for row in rows if row["kind"] == "reference" and row["mode"] in ("", mode)]
        best = max(references, key=lambda row: speed(row) or 0.0, default=None)
        baseline = next((row for row in rows if row["kind"] == "baseline" and row["mode"] == mode), None)
        candidate = next((row for row in rows if row["kind"] == "candidate" and row["mode"] == mode), None)
        reference_text = f"{best['tool']}: {cell(best)}" if best else "–"
        lines.append(f"| {path} | {dataset} | {link} | {mode or '–'} | {reference_text} | {cell(baseline)} | "
                     f"{cell(candidate)} | {ratio(speed(candidate), speed(best))} | "
                     f"{ratio(speed(candidate), speed(baseline))} |")
    references = [row for row in ledger.rows if row["kind"] == "reference"]
    if references:
        lines += ["", "### Every reference tool", "", "| Path | Data | Link | Variant | Tool | Result |",
                  "|---|---|---|---|---|---|"]
        for row in references:
            lines.append(f"| {row['path']} | {row['dataset']} | {row['link']} | {row['mode'] or '–'} | "
                         f"{row['tool']} | {cell(row)} |")
    if ledger.bench:
        lines += ["", "### BENCH lines (median of the repetitions)", "",
                  "| Case | n | Medians | ÷ reference line |", "|---|---|---|---|"]
        reference_cases = {case.split("_reference_")[0]: median_metrics(samples)
                           for case, samples in ledger.bench.items() if "_reference_" in case}
        for case in sorted(ledger.bench):
            samples = ledger.bench[case]
            medians = median_metrics(samples)
            skipped = sorted({str(sample["skipped"]) for sample in samples if "skipped" in sample})
            values = ", ".join(f"{key}={value:.2f}" for key, value in medians.items() if key != "rep")
            if skipped:
                values = (values + "; " if values else "") + "skipped=" + ",".join(skipped)
            group = max((prefix for prefix in reference_cases
                         if case.startswith(prefix + "_") and "_reference_" not in case), key=len, default=None)
            versus = "–"
            if group:
                own, other = medians.get("mib_per_s"), reference_cases[group].get("mib_per_s")
                versus = ratio(own, other)
            lines.append(f"| {case} | {len(samples)} | {values or '–'} | {versus} |")
    failed = [item for item in results.items if not item["ok"]]
    lines += ["", f"### Checks: {len(results.items) - len(failed)} passed, {len(failed)} failed", ""]
    for item in failed:
        lines.append(f"- FAILED {item['stage']}: {item['check']}")
    if ledger.notes:
        lines += ["", "### Not measured / notes", ""] + [f"- {note}" for note in ledger.notes]
    errors = [(row, error) for row in ledger.rows for error in row["errors"] if not error.startswith("not run")]
    if errors:
        lines += ["", "### Run errors (baseline and reference errors do not fail the job)", ""]
        for row, error in errors[:80]:
            lines.append(f"- {row['kind']} {row['tool']} · {row['path']} · {row['dataset']} · {row['link']}: "
                         f"{error.splitlines()[0][:300] if error else ''}")
    (ledger.log_root / "summary.md").write_text(redact("\n".join(lines)) + "\n", encoding="utf-8")


def write_functional_summary(results, log_root, title):
    failed = [item for item in results.items if not item["ok"]]
    lines = [f"## {title}", "", f"{len(results.items) - len(failed)} passed, {len(failed)} failed", "",
             "| Stage | Check | Result |", "|---|---|---|"]
    for item in results.items:
        lines.append(f"| {item['stage']} | {item['check']} | {'pass' if item['ok'] else 'FAIL'} |")
    (log_root / "summary.md").write_text(redact("\n".join(lines)) + "\n", encoding="utf-8")


# --------------------------------------------------------------------------- main

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    parser.add_argument("--job", choices=("fmt", "functional", "measure"), required=True)
    parser.add_argument("--part", choices=("local", "remote-bench", "remote-cli", "windows-remote"))
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--netem", action="store_true", help="remote-cli: also 1 ms and 20 ms delay on lo")
    parser.add_argument("--netem-repetitions", type=int, default=1)
    parser.add_argument("--budget-minutes", type=float, default=320)
    args = parser.parse_args()
    started = time.monotonic()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = ai.Results(log_root)
    if args.job == "fmt":
        job_fmt(results, log_root)
        write_functional_summary(results, log_root, "speed-setup: rustfmt on changed lines")
        return results.finish()
    if args.job == "functional":
        try:
            job_functional(results, log_root)
        finally:
            write_functional_summary(results, log_root, f"speed-setup: functional ({platform_key()})")
        return results.finish()
    if not args.part:
        parser.error("--job measure requires --part")
    if (args.part == "windows-remote") != WINDOWS and args.part != "local":
        parser.error(f"--part {args.part} does not run on {platform_key()}")
    reps = max(1, args.repetitions)
    ledger = Ledger(results, log_root, args.part, started + args.budget_minutes * 60, reps)
    ledger.facts["job"] = f"measure {args.part} on {platform_key()}"
    work = Path(os.environ.get("RUNNER_TEMP") or tempfile.gettempdir()) / f"se-speed-{args.part}"
    remove(work)
    work.mkdir(parents=True)
    ledger.facts["free disk at start"] = f"{free_gib(work):.1f} GiB in {work}"
    try:
        {"local": part_local, "remote-bench": part_remote_bench, "remote-cli": part_remote_cli,
         "windows-remote": part_windows_remote}[args.part](*((ledger, work) if args.part == "local"
                                                              else (ledger, work, args)))
    except Exception as error:  # noqa: BLE001 - the report is written in every case
        results.record("measure", f"part {args.part}", False, f"{type(error).__name__}: {error}")
    finally:
        ledger.write()
        write_summary(ledger, results, f"speed-setup: measure {args.part} ({platform_key()})")
        if not WINDOWS:
            run(["sudo", "-n", "tc", "qdisc", "del", "dev", "lo", "root"], timeout=60)
        run(["git", "worktree", "remove", "--force", str(work / "baseline-src")], cwd=ROOT, timeout=300)
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
