"""Links and servers of the stage family ``speed`` (U55-6, M-1, M-2): the five links as netem rules on the
loopback, the containers of the six ways with the same publication path as the ``iperf3`` server of their
link, the pair of Share devices, and the throttled variants of the limiter cases.

Every server registers its cleanup with ``ctx.fixtures`` (closed at the end of the stage, also after an error).
A server that cannot start does not fail the stage: ``Servers.failed`` holds the reason per way and the cases
of that way become "nicht ausgeführt" with it.
"""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import json
import os
import secrets
import shutil
import socket
import subprocess
import time
from pathlib import Path

from . import common
from .common import NATIVE, excerpt

# name -> (netem arguments, round trip in ms, rate in Mbit/s); the delay applies per direction (M-2).
PROFILES = {
    "lan": ("delay 1ms rate 1gbit limit 5000", 2, 1000),
    "wan": ("delay 25ms rate 100mbit limit 1000", 50, 100),
    "fat": ("delay 100ms rate 500mbit limit 20000", 200, 500),
    "loss": ("delay 50ms rate 100mbit loss 0.1% limit 5000", 100, 100),
    "slow": ("delay 25ms rate 20mbit limit 1000", 50, 20),
    "raw": (None, 0, None),
}
SSH_KEY_ORDER = ("ssh-ed25519", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521", "ssh-rsa")
USER = "speed"
PROFILE_KEYS = ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                "XDG_RUNTIME_DIR", "SMART_EXPLORER_E2E_TEST_NAMESPACE")


class Unavailable(RuntimeError):
    """A server or link that could not be set up; the message becomes the reason of the cases."""


def sudo(ctx: common.JobContext, stage: str, *cmd, timeout: float = 120) -> common.Completed:
    return ctx.run(["sudo", "-n", *cmd], stage=stage, timeout=timeout)


def netem_ready(ctx: common.JobContext, stage: str) -> str | None:
    """None when netem works, else the reason. Azure kernels ship ``sch_netem`` in the extra modules (TE1)."""
    if "netem" in ctx.state:
        return ctx.state["netem"]
    reason = None
    if not ctx.sudo_ok() or not shutil.which("tc"):
        reason = "sudo -n oder tc fehlt"
    else:
        sudo(ctx, stage, "tc", "qdisc", "del", "dev", "lo", "root")
        if not sudo(ctx, stage, "modprobe", "sch_netem").ok:
            release = ctx.run(["uname", "-r"], stage=stage, timeout=30).output.strip()
            sudo(ctx, stage, "apt-get", "install", "-y", f"linux-modules-extra-{release}", timeout=900)
            if not sudo(ctx, stage, "modprobe", "sch_netem").ok:
                reason = "sch_netem lässt sich nicht laden (modprobe, auch nach linux-modules-extra)"
        if reason is None:
            added = sudo(ctx, stage, "tc", "qdisc", "add", "dev", "lo", "root", "netem", "delay", "1ms")
            sudo(ctx, stage, "tc", "qdisc", "del", "dev", "lo", "root")
            if not added.ok:
                reason = f"tc qdisc add netem scheitert: {excerpt(added.output, 300)}"
    ctx.state["netem"] = reason
    return reason


@contextlib.contextmanager
def link(ctx: common.JobContext, stage: str, profile: str):
    """The link of M-2 on ``lo`` for one group of cases; checks the rule with ``tc qdisc show`` and removes it
    afterwards. Yields the environment of the cases. Raises ``Unavailable`` when the rule cannot be set."""
    arguments, rtt, rate = PROFILES[profile]
    env = {"SE_SPEED_PROFILE": profile, "SE_SPEED_RTT_MS": str(rtt), "SE_SPEED_RATE_MBIT": str(rate or "")}
    if arguments is None:
        if netem_ready(ctx, stage) is None:
            sudo(ctx, stage, "tc", "qdisc", "del", "dev", "lo", "root")
        yield env
        return
    reason = netem_ready(ctx, stage)
    if reason:
        raise Unavailable(f"Strecke {profile}: {reason}")
    sudo(ctx, stage, "tc", "qdisc", "del", "dev", "lo", "root")
    added = sudo(ctx, stage, "tc", "qdisc", "add", "dev", "lo", "root", "netem", *arguments.split())
    shown = ctx.run(["tc", "qdisc", "show", "dev", "lo"], stage=stage, timeout=30).output.lower()
    values = [token for token in arguments.lower().split() if token not in ("delay", "rate", "limit", "loss")]
    missing = [token for token in values if token not in shown]
    if not added.ok or "netem" not in shown or missing:
        sudo(ctx, stage, "tc", "qdisc", "del", "dev", "lo", "root")
        raise Unavailable(f"Strecke {profile}: Regel netem {arguments} nicht gesetzt: "
                          f"{excerpt(added.output + shown, 300)}")
    try:
        yield env
    finally:
        sudo(ctx, stage, "tc", "qdisc", "del", "dev", "lo", "root")


def banner(port: int, prefix: bytes | None, timeout: float = 180) -> bool:
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


def load_sync_module(file: str, name: str):
    spec = importlib.util.spec_from_file_location(name, NATIVE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Servers:
    """The servers one stage needs, started on demand."""

    def __init__(self, ctx: common.JobContext, stage: str):
        self.ctx, self.stage, self.fx = ctx, stage, ctx.fixtures
        self.env: dict[str, str] = {}
        self.failed: dict[str, str] = {}
        self.password = secrets.token_hex(12)
        self.tag = f"rdt-speed-{os.getpid()}-{secrets.token_hex(3)}"
        self.count = 0

    def mark(self) -> tuple[int, int]:
        return len(self.fx.cleanups), len(self.fx.processes)

    def release(self, mark: tuple[int, int]) -> None:
        """Stops what was started since ``mark`` (cases that need a different server variant one after another)."""
        cleanups, processes = mark
        for process in reversed(self.fx.processes[processes:]):
            common.kill_tree(process)
        for cleanup in reversed(self.fx.cleanups[cleanups:]):
            try:
                cleanup()
            except Exception as error:  # noqa: BLE001
                self.ctx.log_line(self.stage, f"cleanup failed: {error}")
        del self.fx.cleanups[cleanups:]
        del self.fx.processes[processes:]

    def start(self, way: str, **options) -> bool:
        try:
            getattr(self, "_" + way)(**options)
            return True
        except (Unavailable, OSError, subprocess.SubprocessError, KeyError, RuntimeError) as error:
            self.failed[way] = f"{type(error).__name__}: {error}"
            return False

    # ---- docker
    def docker(self, label: str, image: str, options: list, command: list = (), timeout: float = 900) -> str:
        if not shutil.which("docker"):
            raise Unavailable("Docker fehlt")
        self.count += 1
        name = f"{self.tag}-{label}-{self.count}"
        done = self.ctx.run(["docker", "run", "-d", "--name", name, *options, image, *command], stage=self.stage,
                            timeout=timeout, redact=(self.password,))
        if not done.ok:
            raise Unavailable(f"docker run {image}: {done.describe()}: {excerpt(done.output, 300)}")
        self.fx.add_cleanup(lambda: self.ctx.run(["docker", "rm", "-f", name], stage=self.stage, timeout=120))
        return name

    def port(self, name: str, internal: int) -> int:
        done = self.ctx.run(["docker", "port", name, f"{internal}/tcp"], stage=self.stage, timeout=60)
        if not done.ok or ":" not in done.output:
            raise Unavailable(f"docker port {name}: {done.describe()}")
        return int(done.output.strip().splitlines()[0].rsplit(":", 1)[1])

    def fingerprints(self, port: int) -> str:
        scanned = ""
        for _ in range(10):
            scanned = self.ctx.run(["ssh-keyscan", "-p", str(port), "127.0.0.1"], stage=self.stage, timeout=60).output
            if any(line.strip() and not line.startswith("#") for line in scanned.splitlines()):
                break
            time.sleep(1)
        found = {}
        for line in scanned.splitlines():
            if line.strip() and not line.startswith("#") and len(line.split()) > 1:
                listed = self.ctx.run(["ssh-keygen", "-l", "-E", "sha256", "-f", "-"], stage=self.stage,
                                      timeout=30, input_text=line + "\n").output.split()
                found[line.split()[1]] = next((part for part in listed if part.startswith("SHA256:")), "")
        ordered = [found[kind] for kind in SSH_KEY_ORDER if found.get(kind)]
        if not ordered:
            raise Unavailable(f"keine Host-Schlüssel von 127.0.0.1:{port} gelesen")
        return " ".join(ordered)

    def ssh_server(self, label: str, image: str, internal: int, root: str, options: list, command: list,
                   config: str, extra: list = ()) -> dict:
        name = self.docker(label, image, ["-p", f"127.0.0.1::{internal}", *options], command)
        port = self.port(name, internal)
        if not banner(port, b"SSH-"):
            raise Unavailable(f"{label}: der SSH-Server antwortet nicht auf Port {port}")
        if extra:
            line = "; ".join(f'echo "{item}" >> {config}' for item in extra)
            self.ctx.run(["docker", "exec", name, "sh", "-c", line], stage=self.stage, timeout=60)
            self.ctx.run(["docker", "restart", name], stage=self.stage, timeout=180)
            port = self.port(name, internal)  # a restart assigns a new ephemeral host port
            if not banner(port, b"SSH-"):
                raise Unavailable(f"{label}: der SSH-Server antwortet nach dem Neustart nicht")
        return {"port": port, "name": name, "fingerprints": self.fingerprints(port),
                "spec": f"127.0.0.1:{port}:{USER}:{self.password}:{root}"}

    # ---- the six ways
    def _sftp(self, prefix: str = "SE_SPEED_SFTP", ciphers: str | None = None, extra_options: list = (),
              mount: Path | None = None) -> None:
        key = self.fx.make_dir("sftp-key") / "id"
        if not key.exists():
            self.ctx.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", key], stage=self.stage, timeout=60)
        options = ["-v", f"{key}.pub:/home/{USER}/.ssh/keys/speed.pub:ro", *extra_options]
        if mount:
            options += ["-v", f"{mount}:/home/{USER}/upload"]
        server = self.ssh_server("sftp", "atmoz/sftp:latest", 22, "/upload", options,
                                 [f"{USER}:{self.password}:::upload"], "/etc/ssh/sshd_config",
                                 [f"Ciphers {ciphers}"] if ciphers else [])
        self.env.update({prefix: server["spec"], prefix + "_FINGERPRINTS": server["fingerprints"],
                         prefix + "_CONTAINER": f"{server['name']}:/home/{USER}/upload"})
        if prefix == "SE_SPEED_SFTP":
            self.env["SE_SPEED_SFTP_KEY"] = str(key)

    def _agent(self, no_lz4: bool = False) -> None:
        options = ["-e", f"PUID={os.getuid()}", "-e", f"PGID={os.getgid()}", "-e", "PASSWORD_ACCESS=true",
                   "-e", f"USER_NAME={USER}", "-e", f"USER_PASSWORD={self.password}"]
        server = self.ssh_server("agent", common.SFTP_IMAGE, 2222, "/config", options, [], "/config/sshd/sshd_config",
                                 ["PerSourcePenalties no"] + (["SetEnv SE_TEST_NO_LZ4=1"] if no_lz4 else []))
        self.env.update({"SE_SPEED_AGENT": server["spec"], "SE_SPEED_AGENT_FINGERPRINTS": server["fingerprints"],
                         "SE_SPEED_AGENT_CONTAINER": f"{server['name']}:/config"})

    def _ftp(self) -> None:
        # vsftpd on the host network: the data connections need no published port range and cross the shaped
        # loopback directly (TE1).
        name = self.docker("ftp", "delfer/alpine-ftp-server:latest",
                           ["--network", "host", "-e", f"USERS={USER}|{self.password}"],
                           ["vsftpd", "/etc/vsftpd/vsftpd.conf", "-obackground=NO", "-opasv_min_port=21000",
                            "-opasv_max_port=21999", "-opasv_address=127.0.0.1", "-omax_clients=0", "-omax_per_ip=0"])
        if not banner(21, b"220"):
            raise Unavailable("FTP: der Server antwortet nicht auf Port 21")
        self.env.update({"SE_SPEED_FTP_URL": f"ftp://{USER}:{self.password}@127.0.0.1:21/ftp/{USER}",
                         "SE_SPEED_FTP_CONTAINER": f"{name}:/ftp/{USER}"})

    def _webdav(self) -> None:
        root = self.fx.make_dir("webdav")
        root.chmod(0o777)
        (root / "dav.conf").write_text(
            'LoadModule dav_module modules/mod_dav.so\nLoadModule dav_fs_module modules/mod_dav_fs.so\n'
            'LoadModule logio_module modules/mod_logio.so\nDavLockDB /tmp/speed-dav-lock\n'
            'LogFormat "%h %l %u %t \\"%r\\" %>s %I %O" speed\n<VirtualHost *:80>\n'
            'DocumentRoot /tmp/sync-dav/data\nCustomLog /fixture/access.log speed\n'
            '<Directory /tmp/sync-dav/data>\nOptions None\nAllowOverride None\nDirectoryIndex disabled\n'
            'Require all granted\n</Directory>\n<Location />\nDav On\nAuthType Basic\nAuthName Speed\n'
            'AuthUserFile /tmp/dav.passwd\nRequire valid-user\n</Location>\n</VirtualHost>\n')
        hashed = self.ctx.run(["openssl", "passwd", "-apr1", "-stdin"], stage=self.stage, timeout=30,
                              input_text=self.password + "\n", redact=(self.password,)).output.strip()
        (root / "dav.passwd").write_text(f"{USER}:{hashed}\n")
        script = ("set -eu; printf '\\nInclude /fixture/dav.conf\\n' >> /usr/local/apache2/conf/httpd.conf; "
                  "cp /fixture/dav.passwd /tmp/dav.passwd; chmod 644 /tmp/dav.passwd; mkdir -p /tmp/sync-dav/data; "
                  "u=$(awk '$1 == \"User\" { print $2; exit }' /usr/local/apache2/conf/httpd.conf); "
                  "g=$(awk '$1 == \"Group\" { print $2; exit }' /usr/local/apache2/conf/httpd.conf); "
                  "chown -R \"$u:$g\" /tmp/sync-dav; exec httpd-foreground")
        name = self.docker("webdav", "httpd:2.4", ["-p", "127.0.0.1::80", "-v", f"{root}:/fixture"],
                           ["sh", "-c", script])
        port = self.port(name, 80)
        if not banner(port, None):
            raise Unavailable("WebDAV: httpd antwortet nicht")
        self.env.update({"SE_SPEED_WEBDAV_URL": f"http://{USER}:{self.password}@127.0.0.1:{port}/",
                         "SE_SPEED_WEBDAV_CONTAINER": f"{name}:/tmp/sync-dav/data",
                         "SE_SPEED_WEBDAV_ACCESS_LOG": str(root / "access.log")})

    def _smb(self) -> None:
        name = self.docker("smb", "dockurr/samba:4.23.10", ["-p", "127.0.0.1::445", "-e", "NAME=sync", "-e",
                           f"USER={USER}", "-e", f"PASS={self.password}", "-e", "RW=true"])
        port = self.port(name, 445)
        if not banner(port, None):
            raise Unavailable("SMB: samba antwortet nicht")
        self.env.update({"SE_SPEED_SMB_URL": f"smb://{USER}:{self.password}@127.0.0.1:{port}/sync",
                         "SE_SPEED_SMB_CONTAINER": f"{name}:/storage"})

    def _iperf(self) -> None:
        """The iperf3 server of a link on the same publication path as the protocol servers (M-1): once on the
        loopback itself (FTP, Share) and once behind Docker's published port (SFTP, WebDAV, SMB, agent)."""
        if not shutil.which("iperf3"):
            raise Unavailable("iperf3 fehlt")
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            host = probe.getsockname()[1]
        log = open(self.fx.make_dir("iperf") / "iperf3-server.log", "ab")
        process = subprocess.Popen(["iperf3", "-s", "-p", str(host), "-B", "127.0.0.1"], stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        self.fx.add_process(process)
        if not common.wait_tcp(host, 30, process):
            raise Unavailable("iperf3 -s lauscht nicht auf der Schleife")
        self.env["SE_SPEED_IPERF_HOST_PORT"] = str(host)
        try:
            name = self.docker("iperf", "networkstatic/iperf3", ["-p", "127.0.0.1::5201"], ["-s"])
            proxy = self.port(name, 5201)
            if banner(proxy, None, 60):
                self.env["SE_SPEED_IPERF_PROXY_PORT"] = str(proxy)
        except Unavailable as error:
            self.failed["iperf-proxy"] = str(error)

    # ---- Share pair
    def _share(self, no_lz4: bool = False, cc: str | None = None, room: bool = False) -> None:
        """Two devices of the Share (main = the profile of the test program, remote = the host that exports a
        folder). No relay-only switch: the path between them is what the platform chooses on the loopback."""
        ctx = self.ctx
        handed = Path(ctx.args.bins) / "se" if ctx.args.bins else None
        se = handed if handed and handed.is_file() else ctx.cargo.bin_path("se")
        server = ctx.cargo.share_server()
        if se is None or server is None:
            raise Unavailable(f"se oder se-share-server fehlt: {ctx.cargo.bin_errors.get('se') or ctx.cargo.server_error}")
        providers = load_sync_module("sync-reliability-providers.py", "speed_sync_providers")
        pair = load_sync_module("sync-reliability-share-fixture.py", "speed_sync_share")
        root = self.fx.make_dir("share")
        root.chmod(0o700)
        owner = providers.Owner(root, ctx.logs, dict(os.environ))
        owner.cli = Path(se)
        self.fx.add_cleanup(lambda: [cleanup() for cleanup in reversed(owner.cleanups)])
        signal_port, relay_port = pair.pair_ports()
        cert, key = root / "server.pem", root / "server.key"
        owner.command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-keyout", key,
                       "-out", cert, "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1",
                       "-addext", "basicConstraints=critical,CA:FALSE"])
        owner.command(["openssl", "x509", "-in", cert, "-outform", "DER", "-out", root / "server.der"])
        pin = hashlib.sha256((root / "server.der").read_bytes()).hexdigest()
        process = subprocess.Popen(
            [str(server), f"127.0.0.1:{signal_port}", "--tls-cert", str(cert), "--tls-key", str(key), "--state-file",
             str(root / "state.json")], stdin=subprocess.DEVNULL, start_new_session=True,
            stdout=open(ctx.logs / "speed-share-server.log", "ab"), stderr=subprocess.STDOUT,
            env=dict(os.environ, SE_SHARE_IDLE_KEEPALIVE_SECS="30", SE_SHARE_ALLOW_PLAINTEXT="0",
                     SE_SHARE_REQUIRE_KEY_LOGIN="0", SE_IROH_RELAY_DISABLE="0",
                     SE_IROH_RELAY_BIND=f"127.0.0.1:{relay_port}", SE_SHARE_FIXTURE_ROOT=str(root)))
        self.fx.add_process(process)
        pair.wait_server(process, (signal_port, relay_port))
        relay = f"https://127.0.0.1:{relay_port}"
        shared = {"SE_SHARE_RELAY_URL": relay, **({"SE_SHARE_QUIC_CC": cc} if cc else {})}
        main_env = {**pair.isolated(dict(os.environ), root / "share-main"), **shared}
        remote_env = {**pair.isolated(dict(os.environ), root / "share-remote"), **shared,
                      **({"SE_TEST_NO_LZ4": "1"} if no_lz4 else {})}
        clients = [pair.Client(owner, Path(se), env, label) for env, label in ((main_env, "main"), (remote_env, "remote"))]
        main, remote = clients
        for client in clients:
            client.server_process = process
            client.call("share", "configure", "--server", f"wss://127.0.0.1:{signal_port}/#sha256={pin}")
            client.wait(lambda c=client: (lambda worker: worker.get("reachable") and worker.get("running") and
                                          worker.get("connected") and worker.get("relay_url", "").rstrip("/") == relay)(
                json.loads(c.call("share", "status", "--json").stdout).get("worker", {})), "verbundener Share-Worker")
        identity = json.loads(remote.call("share", "identity", "--json").stdout)
        direct = json.loads(main.call("connections", "add-peer", "--code", identity["direct_code"], "--name",
                                      "SpeedPeer", "--json").stdout)
        remote.wait(lambda: json.loads(remote.call("share", "request", "--json").stdout).get("count", 0) == 1,
                    "eingehende Anfrage")
        remote.call("share", "request", "accept", "--json")
        remote.call("share", "grants", "set", pair.identity_of(main), "--write", "--json")
        export = root / "direct-export"
        export.mkdir()
        remote.call("share", "export", "add", str(export), "--label", "DirectSpeed")
        remote.call("share", "export", "set", "DirectSpeed", "--write")
        remote.call("share", "worker", "refresh")
        endpoint = direct["endpoint"] + "/DirectSpeed"
        main.wait(lambda: main.call("ls", endpoint, check=False).returncode == 0, "freigegebener Export")
        # The test program talks to the daemon of "main": it runs in that profile.
        self.env.update({key: main_env[key] for key in PROFILE_KEYS + tuple(shared)})
        self.env.update({"SE_SPEED_SHARE_ENDPOINT": endpoint, "SE_SPEED_SHARE_DIR": str(export)})
        if room:
            made = dict(line.split("\t", 1) for line in remote.call("share", "room", "create", "--name",
                                                                    "SpeedRoom").stdout.splitlines() if "\t" in line)
            main.call("connections", "add-room", "--code", made["room_code"], "--name", "SpeedRoom")
            room_dir = root / "room-export"
            room_dir.mkdir()
            remote.call("share", "export", "add", str(room_dir), "--label", "RoomSpeed", "--room", "SpeedRoom")
            remote.call("share", "export", "set", "RoomSpeed", "--write", "--room", "SpeedRoom")
            for client in clients:
                client.call("share", "worker", "refresh")
            relation = made["room_code"].removeprefix("SE-R3-").rsplit("-", 1)[0]
            room_endpoint = f"share://room/{relation}/{identity['device_id']}/RoomSpeed"
            main.wait(lambda: main.call("ls", room_endpoint, check=False).returncode == 0, "Raum-Export")
            pids = [row["pid"] for client in clients for row in pair.owned_processes(client.cli)]
            self.env.update({"SE_SPEED_ROOM_ENDPOINT": room_endpoint, "SE_SPEED_ROOM_DIR": str(room_dir),
                             "SE_SPEED_ROOM_PIDS": ",".join(str(pid) for pid in pids)})


def device_of(path: Path) -> str | None:
    """The whole disk under ``path`` (``/dev/sdb``) for the throttles of the limiter cases."""
    source = subprocess.run(["findmnt", "-no", "SOURCE", "--target", str(path)], capture_output=True, text=True)
    device = source.stdout.strip()
    if not device.startswith("/dev/"):
        return None
    parent = subprocess.run(["lsblk", "-no", "PKNAME", device], capture_output=True, text=True).stdout.strip()
    return f"/dev/{parent.splitlines()[0]}" if parent else device


def slow_container_rate(ctx: common.JobContext, stage: str, container: str, mbit: int = 5) -> str | None:
    """Limits the egress of a container's network namespace (the slow source of the idle case); None on success."""
    pid = ctx.run(["docker", "inspect", "-f", "{{.State.Pid}}", container], stage=stage, timeout=60).output.strip()
    done = sudo(ctx, stage, "nsenter", "-t", pid, "-n", "tc", "qdisc", "add", "dev", "eth0", "root", "tbf", "rate",
                f"{mbit}mbit", "burst", "32kbit", "latency", "400ms")
    return None if done.ok else f"tc im Netzraum des Containers scheitert: {excerpt(done.output, 300)}"
