"""Real Direct/Room peers with normal saved profiles and bounded owned processes."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import socket
import shutil
import signal
import subprocess
import time


def pair_ports():
    # An adjacent port may be reserved/excluded on Windows even when a bind
    # probe succeeds. Probe actual exclusive listeners, each on an OS port.
    with socket.socket() as signaling, socket.socket() as relay:
        for listener in (signaling, relay):
            if os.name == "nt":
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
        return signaling.getsockname()[1], relay.getsockname()[1]


def wait_server(process, ports, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("owned Share server exited before TLS listeners; see share-server.log")
        try:
            for port in ports:
                with socket.create_connection(("127.0.0.1", port), timeout=1):
                    pass
            if process.poll() is None:
                return
        except OSError:
            pass
        time.sleep(0.1)
    raise RuntimeError("owned Share server did not publish both TLS listeners")


def isolated(base, root):
    result = dict(base)
    for variable, child in (("HOME", "home"), ("USERPROFILE", "home"), ("APPDATA", "data"),
                            ("LOCALAPPDATA", "data"), ("XDG_DATA_HOME", "data"),
                            ("XDG_CONFIG_HOME", "config"), ("XDG_RUNTIME_DIR", "runtime")):
        path = root / child
        path.mkdir(parents=True, exist_ok=True)
        path.chmod(0o700)
        result[variable] = str(path)
    seed = base.get("SMART_EXPLORER_E2E_TEST_NAMESPACE", "") + "\0" + str(root.resolve())
    result["SMART_EXPLORER_E2E_TEST_NAMESPACE"] = "sync_peer_" + hashlib.sha256(seed.encode()).hexdigest()[:32]
    return result


def owned_processes(executable, stop=False):
    """Identify only this client's private executable; never executable names."""
    executable = executable.resolve()
    if os.name == "nt":
        script = executable.parent / "owned-processes.ps1"
        script.write_text(r'''param([string]$Executable,[string]$Stop)
$ErrorActionPreference='Stop'
$rows=@()
foreach($entry in @(Get-CimInstance Win32_Process)) {
 if(!$entry.ExecutablePath -or ![StringComparer]::OrdinalIgnoreCase.Equals($entry.ExecutablePath,$Executable)) { continue }
 $process=$null
 try {
  $process=[System.Diagnostics.Process]::GetProcessById([int]$entry.ProcessId)
  $handle=$process.Handle
  if($process.HasExited) { continue }
  if(![StringComparer]::OrdinalIgnoreCase.Equals($process.MainModule.FileName,$Executable)) { continue }
  $stamp=$process.StartTime.ToUniversalTime().Ticks.ToString()
  if($Stop -eq 'stop') {
   $process.Kill()
   if(!$process.WaitForExit(10000)) { throw 'Owned process did not exit after Kill' }
  } else {
   $rows+=@{pid=[int]$entry.ProcessId;stamp=$stamp}
  }
 } catch {
  if($process -and $process.HasExited) { continue }
  if(!(Get-Process -Id ([int]$entry.ProcessId) -ErrorAction SilentlyContinue)) { continue }
  throw
 } finally {
  if($process) { $process.Dispose() }
 }
}
ConvertTo-Json -InputObject $rows -Compress
''', encoding="utf-8")
        result = subprocess.run(["powershell", "-NoProfile", "-File", str(script), str(executable),
                                 "stop" if stop else "inspect"], text=True, capture_output=True,
                                check=True, timeout=45)
        return json.loads(result.stdout.strip() or "[]")
    rows = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            if (entry / "exe").resolve(strict=True) != executable:
                continue
            stat = (entry / "stat").read_text()
            stamp = stat.rsplit(")", 1)[1].split()[19]
            rows.append(dict(pid=int(entry.name), stamp=stamp))
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return rows


def kill_owned(executable):
    if os.name == "nt":
        owned_processes(executable, stop=True)
        return
    # A pidfd stays bound to this process even if its numerical PID is reused.
    for row in owned_processes(executable):
        try:
            handle = os.pidfd_open(row["pid"])
        except ProcessLookupError:
            continue
        try:
            if row not in owned_processes(executable):
                continue
            signal.pidfd_send_signal(handle, signal.SIGKILL)
        except ProcessLookupError:
            pass
        finally:
            os.close(handle)


class Client:
    def __init__(self, owner, cli, env, label):
        self.owner, self.env, self.label = owner, env, label
        namespace = env.get("SMART_EXPLORER_E2E_TEST_NAMESPACE", "")
        if not namespace or len(namespace) > 48 or not namespace.isascii() or not namespace[0].isalnum() or any(
                not (letter.isalnum() or letter in "-_") for letter in namespace):
            raise RuntimeError("Share peer requires its valid isolated test namespace")
        binary_root = owner.root / ("share-binary-" + label)
        binary_root.mkdir()
        self.cli = binary_root / ("se.exe" if os.name == "nt" else "se")
        shutil.copy2(cli, self.cli)
        if hashlib.sha256(cli.read_bytes()).digest() != hashlib.sha256(self.cli.read_bytes()).digest():
            raise RuntimeError("Owned CLI copy does not match development binary")
        self._closed = False
        self.log = (owner.logs / f"share-{label}-daemon.log").open("w", encoding="utf-8")
        try:
            self.process = subprocess.Popen([str(self.cli), "--sync-daemon"], env=env,
                                            stdin=subprocess.DEVNULL, stdout=self.log, stderr=subprocess.STDOUT)
        except BaseException:
            self.log.close()
            raise
        owner.cleanups.append(self.close)
        (owner.logs / f"share-{label}-owner.json").write_text(
            json.dumps(dict(pid=self.process.pid, started=time.time(), profile=env.get("APPDATA"),
                            executable=str(self.cli), namespace=namespace)))
        # Await the explicit worker's published IPC file before CLI can consider spawning one.
        candidates = [Path(env[key]) / "smart_explorer" / "sync" / "daemon.ipc"
                      for key in ("APPDATA", "XDG_DATA_HOME") if env.get(key)]
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                self.log.close()
                raise RuntimeError(f"owned Share daemon {label} exited before IPC readiness")
            for path in candidates:
                if path.is_file():
                    address = path.read_text().strip()
                    try:
                        host, port = address.rsplit(":", 1)
                        with socket.create_connection((host, int(port)), timeout=1):
                            self.sync = path.parent
                            return
                    except (OSError, ValueError):
                        pass
            time.sleep(0.1)
        self.close()
        raise RuntimeError(f"owned Share daemon {label} did not publish IPC")

    def call(self, *args, check=True):
        result = subprocess.run([str(self.cli), *args], env=self.env, text=True,
                                capture_output=True, timeout=90)
        with (self.owner.logs / f"share-{self.label}-commands.log").open("a", encoding="utf-8") as log:
            log.write(f"{' '.join(args)} exit={result.returncode}\n{result.stdout}{result.stderr}\n")
        if check and result.returncode:
            raise RuntimeError(f"Share command failed for {self.label}; see command log")
        return result

    def wait(self, callback, description, seconds=180):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"owned daemon {self.label} exited during {description}")
            if hasattr(self, "server_process") and self.server_process.poll() is not None:
                raise RuntimeError(f"owned Share server exited during {description}; see share-server.log")
            try:
                value = callback()
                if value:
                    return value
            except (json.JSONDecodeError, RuntimeError, KeyError):
                pass
            time.sleep(0.25)
        raise RuntimeError(f"{self.label} readiness failed: {description}")

    def close(self):
        if self._closed:
            return
        roots = [self.sync] if hasattr(self, "sync") else [
            Path(self.env[key]) / "smart_explorer" / "sync"
            for key in ("APPDATA", "XDG_DATA_HOME") if self.env.get(key)]
        for root in set(roots):
            if root.is_dir():
                stage = root / ("daemon.stop." + self.label + ".tmp")
                with stage.open("w", encoding="ascii") as stream:
                    stream.write("stop")
                    stream.flush()
                    os.fsync(stream.fileno())
                stage.replace(root / "daemon.stop")
        # The actual worker consumes daemon.stop and exits successfully; its
        # guardian consequently exits instead of supervising an orphan child.
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            self.process.poll()  # Reap the direct guardian before scanning /proc.
            if not owned_processes(self.cli):
                break
            time.sleep(0.25)
        else:
            # Include detached children/replacements using the private exe path,
            # not just the original Popen or a generic process-name match.
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=10)
            kill_owned(self.cli)
        self.process.wait(timeout=15)
        deadline = time.monotonic() + 15
        remaining = owned_processes(self.cli)
        while remaining and time.monotonic() < deadline:
            time.sleep(0.1)
            remaining = owned_processes(self.cli)
        if remaining:
            raise RuntimeError(f"{self.label} guardian/worker cleanup incomplete: {remaining}")
        for root in set(roots):
            address = root / "daemon.ipc"
            if address.is_file():
                value = address.read_text().strip()
                try:
                    host, port = value.rsplit(":", 1)
                    with socket.create_connection((host, int(port)), timeout=1):
                        raise RuntimeError(f"{self.label} IPC still accepts connections after cleanup")
                except (OSError, ValueError):
                    pass
        self.log.close()
        (self.owner.logs / f"share-{self.label}-cleanup.json").write_text(
            json.dumps(dict(namespace=self.env["SMART_EXPLORER_E2E_TEST_NAMESPACE"],
                            executable=str(self.cli), guardian_exit=self.process.returncode,
                            owned_processes_remaining=remaining, ipc_closed=True)))
        self._closed = True


@contextmanager
def peers(owner, cli, server):
    clients = []
    process = None
    server_log = None
    try:
        signal, relay = pair_ports()
        cert, key = owner.root / "server.pem", owner.root / "server.key"
        # Certificate creation is shared with the FTPS/HTTPS fixture on Linux;
        # Windows needs the same actual TLS endpoint for Share.
        if not cert.is_file():
            owner.command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
                           "-keyout", key, "-out", cert, "-subj", "/CN=localhost",
                           "-addext", "subjectAltName=IP:127.0.0.1", "-addext", "basicConstraints=critical,CA:FALSE"])
        owner.command(["openssl", "x509", "-in", cert, "-outform", "DER", "-out", owner.root / "share-server.der"])
        pin = hashlib.sha256((owner.root / "share-server.der").read_bytes()).hexdigest()
        server_env = dict(owner.env, SE_SHARE_IDLE_KEEPALIVE_SECS="30", SE_SHARE_ALLOW_PLAINTEXT="0",
                          SE_SHARE_REQUIRE_KEY_LOGIN="0", SE_IROH_RELAY_DISABLE="0",
                          SE_IROH_RELAY_BIND=f"127.0.0.1:{relay}", SE_SHARE_FIXTURE_ROOT=str(owner.root))
        server_log = (owner.logs / "share-server.log").open("w", encoding="utf-8")
        process = subprocess.Popen([str(server), f"127.0.0.1:{signal}", "--tls-cert", str(cert),
                                    "--tls-key", str(key), "--state-file", str(owner.root / "share-state.json")],
                                   env=server_env, stdin=subprocess.DEVNULL,
                                   stdout=server_log, stderr=subprocess.STDOUT)
        (owner.logs / "share-server-owner.json").write_text(json.dumps(dict(pid=process.pid, started=time.time())))
        wait_server(process, (signal, relay))
        address = f"wss://127.0.0.1:{signal}/#sha256={pin}"
        # The normal transport defaults to signaling-port + 1. Both peers and
        # the later resolver testhost must use the actually owned relay port.
        owner.env["SE_SHARE_RELAY_URL"] = f"https://127.0.0.1:{relay}"
        # Both identities/configurations use ordinary production commands.
        main = Client(owner, cli, dict(owner.env, SE_SHARE_RELAY_ONLY="1"), "main")
        clients.append(main)
        remote_env = dict(isolated(owner.env, owner.root / "share-peer"), SE_SHARE_RELAY_ONLY="1")
        if remote_env["SMART_EXPLORER_E2E_TEST_NAMESPACE"] == main.env["SMART_EXPLORER_E2E_TEST_NAMESPACE"]:
            raise RuntimeError("Main and remote Share peers must have different test namespaces")
        remote = Client(owner, cli, remote_env, "remote")
        clients.append(remote)
        def connected(client):
            worker = json.loads(client.call("share", "status", "--json").stdout).get("worker", {})
            return (worker.get("reachable") and worker.get("running") and worker.get("connected")
                    and worker.get("relay_url", "").rstrip("/") == f"https://127.0.0.1:{relay}")
        for client in clients:
            client.server_process = process
            client.call("share", "configure", "--server", address)
            client.wait(lambda c=client: connected(c), "connected TLS Share worker and owned relay")
        identity = json.loads(remote.call("share", "identity", "--json").stdout)
        direct = json.loads(main.call("connections", "add-peer", "--code", identity["direct_code"],
                                     "--name", "SyncTaskPeer", "--json").stdout)
        remote.wait(lambda: json.loads(remote.call("share", "request", "--json").stdout)
                    .get("count", 0) == 1, "incoming direct request")
        remote.call("share", "request", "accept", "--json")
        remote.call("share", "grants", "set", identity_of(main), "--write", "--json")
        direct_root = owner.root / "direct-export"
        direct_root.mkdir()
        remote.call("share", "export", "add", str(direct_root), "--label", "DirectSync")
        remote.call("share", "export", "set", "DirectSync", "--write")
        remote.call("share", "worker", "refresh")
        direct_endpoint = direct["endpoint"] + "/DirectSync"
        main.wait(lambda: main.call("ls", direct_endpoint, check=False).returncode == 0,
                  "authorized direct export")
        room_output = remote.call("share", "room", "create", "--name", "SyncTaskRoom").stdout
        room = dict(line.split("\t", 1) for line in room_output.splitlines() if "\t" in line)
        main.call("connections", "add-room", "--code", room["room_code"], "--name", "SyncTaskRoom")
        relation = room["room_code"].removeprefix("SE-R3-").rsplit("-", 1)[0]
        room_root = owner.root / "room-export"
        room_root.mkdir()
        remote.call("share", "export", "add", str(room_root), "--label", "RoomSync", "--room", "SyncTaskRoom")
        remote.call("share", "export", "set", "RoomSync", "--write", "--room", "SyncTaskRoom")
        remote.call("share", "export", "policy", "--room", "SyncTaskRoom", "--write", "--confirm-new-members", "false")
        for client in clients:
            client.call("share", "worker", "refresh")
        room_endpoint = f"share://room/{relation}/{identity['device_id']}/RoomSync"
        main.wait(lambda: main.call("ls", room_endpoint, check=False).returncode == 0, "joined room export")
        yield [dict(name="direct", protocol="peer", endpoint=direct_endpoint),
               dict(name="room", protocol="peer", endpoint=room_endpoint)]
    finally:
        failures = []
        for client in reversed(clients):
            try:
                client.close()
            except Exception as error:
                failures.append(str(error))
        if process is not None:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
        if server_log:
            server_log.close()
        if failures:
            raise RuntimeError("Share owned cleanup failed: " + "; ".join(failures))


def identity_of(client):
    return json.loads(client.call("share", "identity", "--json").stdout)["device_id"]
