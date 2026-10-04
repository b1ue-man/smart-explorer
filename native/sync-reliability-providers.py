#!/usr/bin/env python3
"""Owned real protocol fixtures for the single remote sync reliability task.

Import fixtures(logs, env, cli, share_server); never execute on the workstation.
The yielded environment contains SE_SYNC_PROVIDER_MANIFEST and the test-only CA.
"""
from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time
import zipfile


class Owner:
    def __init__(self, root, logs, env):
        self.root, self.logs, self.env = root, logs, env
        self.names, self.cleanups = [], []
        self.prefix = "se-sync-" + secrets.token_hex(6)

    def command(self, args, *, input=None, timeout=180, check=True):
        result = subprocess.run([str(x) for x in args], env=self.env, input=input,
                                text=True, capture_output=True, timeout=timeout)
        # Passwords never appear in command diagnostics; output is retained.
        with (self.logs / "fixture-commands.log").open("a", encoding="utf-8") as log:
            output = "[owned metadata retained in memory]" if "inspect" in args else result.stdout
            log.write(f"{args[0]} exit={result.returncode}\n{output}{result.stderr}\n")
        if check and result.returncode:
            raise RuntimeError(f"fixture command {args[0]} failed; see fixture-commands.log")
        return result.stdout.strip()

    def docker(self, label, image, options, command=()):
        name = self.prefix + "-" + label
        # Record before run, so an interrupted docker client cannot orphan its container.
        self.names.append(name)
        self.command(["docker", "run", "--detach", "--name", name,
                      "--label", f"se.sync.owner={self.prefix}", *options, image, *command], timeout=600)
        return name

    def port(self, name, internal):
        rows = self.command(["docker", "port", name, f"{internal}/tcp"]).splitlines()
        if len(rows) != 1 or not rows[0].startswith("127.0.0.1:"):
            raise RuntimeError(f"unexpected owned port mapping: {rows}")
        return int(rows[0].rsplit(":", 1)[1])

    def close(self):
        failures = []
        for cleanup in reversed(self.cleanups):
            try:
                cleanup()
            except Exception as error:
                failures.append(str(error))
        for name in reversed(self.names):
            try:
                metadata = self.command(["docker", "inspect", name], check=False)
                if not metadata:
                    continue
                info = json.loads(metadata)[0]
                if info["Config"]["Labels"].get("se.sync.owner") != self.prefix:
                    raise RuntimeError(f"refusing cleanup of foreign container {name}")
                (self.logs / f"{name}.log").write_text(
                    self.command(["docker", "logs", name], check=False), encoding="utf-8")
                self.command(["docker", "rm", "--force", name], timeout=60)
            except Exception as error:
                failures.append(str(error))
        if failures:
            raise RuntimeError("owned fixture cleanup failed: " + "; ".join(failures))


def wait_port(port, prefix=None, timeout=180):
    deadline = time.monotonic() + timeout
    last = ""
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=2) as peer:
                if prefix is None or peer.recv(128).startswith(prefix):
                    return
        except OSError as error:
            last = str(error)
        time.sleep(0.25)
    raise RuntimeError(f"owned server readiness failed on port {port}: {last}")


def certificate(owner):
    root = owner.root
    owner.command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                   "-keyout", root / "ca.key", "-out", root / "ca.pem", "-days", "2",
                   "-subj", "/CN=Sync task fixture CA", "-addext", "basicConstraints=critical,CA:TRUE"])
    owner.command(["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes",
                   "-keyout", root / "server.key", "-out", root / "server.csr",
                   "-subj", "/CN=localhost"])
    ext = root / "server.ext"
    ext.write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n"
                   "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n")
    owner.command(["openssl", "x509", "-req", "-in", root / "server.csr", "-CA", root / "ca.pem",
                   "-CAkey", root / "ca.key", "-CAcreateserial", "-out", root / "server.pem",
                   "-days", "2", "-extfile", ext])
    owner.command(["openssl", "x509", "-in", root / "ca.pem", "-outform", "DER", "-out", root / "ca.der"])
    for path in root.glob("*.key"):
        path.chmod(0o600)


def connection(name, protocol, port, user, password, root, agent=False):
    return dict(name=name, protocol=protocol, host="127.0.0.1", port=port,
                user=user, password=password, root=root, agent=agent)


def passive_ports(count=4):
    # Reserve a contiguous range on loopback; advertise the same host ports in FTP.
    for _ in range(100):
        sockets = []
        try:
            first = socket.socket()
            first.bind(("127.0.0.1", 0))
            start = first.getsockname()[1]
            sockets.append(first)
            if start + count > 65535:
                continue
            for port in range(start + 1, start + count):
                peer = socket.socket()
                sockets.append(peer)
                peer.bind(("127.0.0.1", port))
            return start, start + count - 1
        except OSError:
            pass
        finally:
            for peer in sockets:
                peer.close()
    raise RuntimeError("cannot allocate owned FTP passive port range")


def linux_protocols(owner):
    result = []
    user, password = "synctask", secrets.token_hex(18)
    for label in ("sftp", "sftp-other"):
        name = owner.docker(label, "atmoz/sftp:latest", ["-p", "127.0.0.1::22"],
                            [f"{user}:{password}:::upload"])
        port = owner.port(name, 22)
        wait_port(port, b"SSH-")
        result.append(connection(label, "sftp", port, user, password, "/upload"))
    # The encrypted private key remains on the host; only its public half is
    # mounted into the owned SFTP-only server. Its password login is disabled.
    key = owner.root / "sftp-fixture-key"
    passphrase = secrets.token_hex(18)
    owner.command(["ssh-keygen", "-q", "-t", "ed25519", "-N", passphrase,
                   "-C", "sync-task", "-f", key])
    key.chmod(0o600)
    public_key = key.with_name(key.name + ".pub")
    name = owner.docker("sftp-key", "atmoz/sftp:latest", ["-p", "127.0.0.1::22", "-v",
                        f"{public_key}:/home/{user}/.ssh/keys/sync-task.pub:ro"],
                        [f"{user}::::upload"])
    port = owner.port(name, 22)
    wait_port(port, b"SSH-")
    keyed = connection("sftp-key", "sftp", port, user, passphrase, "/upload")
    keyed["key_path"] = str(key)
    result.append(keyed)
    name = owner.docker("agent", "lscr.io/linuxserver/openssh-server:latest",
                        ["-p", "127.0.0.1::2222", "-e", "PUID=1000", "-e", "PGID=1000",
                         "-e", "PASSWORD_ACCESS=true", "-e", f"USER_NAME={user}",
                         "-e", f"USER_PASSWORD={password}"])
    port = owner.port(name, 2222)
    wait_port(port, b"SSH-")
    result.append(connection("agent", "sftp", port, user, password, "/config", True))
    for protocol in ("ftp", "ftps"):
        low, high = passive_ports()
        options = ["-p", "127.0.0.1::21", "-p", f"127.0.0.1:{low}-{high}:{low}-{high}",
                   "-e", f"USERS={user}|{password}"]
        command = ["vsftpd", "/etc/vsftpd/vsftpd.conf", "-obackground=NO",
                   f"-opasv_min_port={low}", f"-opasv_max_port={high}", "-opasv_address=127.0.0.1"]
        if protocol == "ftps":
            options += ["-v", f"{owner.root}:/fixture:ro"]
            command += ["-ossl_enable=YES", "-oforce_local_logins_ssl=YES", "-oforce_local_data_ssl=YES",
                        "-orequire_ssl_reuse=NO", "-orsa_cert_file=/fixture/server.pem",
                        "-orsa_private_key_file=/fixture/server.key"]
        name = owner.docker(protocol, "delfer/alpine-ftp-server:latest", options, command)
        port = owner.port(name, 21)
        wait_port(port, b"220")
        result.append(connection(protocol, protocol, port, user, password, f"/ftp/{user}"))
    name = owner.docker("smb", "dockurr/samba:4.23.10", ["-p", "127.0.0.1::445",
                        "-e", "NAME=sync", "-e", f"USER={user}", "-e", f"PASS={password}", "-e", "RW=true"])
    port = owner.port(name, 445)
    wait_port(port)
    result.append(connection("smb", "smb", port, user, password, "/sync"))
    # Keep the image's established module/listener configuration; append one TLS DAV vhost.
    dav = owner.root / "dav"
    dav.mkdir()
    conf = owner.root / "dav.conf"
    conf.write_text('''LoadModule dav_module modules/mod_dav.so
LoadModule dav_fs_module modules/mod_dav_fs.so
LoadModule ssl_module modules/mod_ssl.so
LoadModule socache_shmcb_module modules/mod_socache_shmcb.so
Listen 443
DavLockDB /tmp/sync-dav-lock
<VirtualHost *:443>
ServerName localhost
SSLEngine on
SSLCertificateFile /fixture/server.pem
SSLCertificateKeyFile /fixture/server.key
DocumentRoot /usr/local/apache2/htdocs
<Directory /usr/local/apache2/htdocs>
Dav On
AuthType Basic
AuthName SyncTask
AuthUserFile /tmp/dav.passwd
Require valid-user
</Directory>
</VirtualHost>
''')
    hashed = owner.command(["openssl", "passwd", "-apr1", "-stdin"], input=password + "\n")
    (owner.root / "dav.passwd").write_text(user + ":" + hashed + "\n")
    name = owner.docker("dav", "httpd:2.4", ["-p", "127.0.0.1::443", "-v", f"{owner.root}:/fixture:ro"],
                        ["sh", "-c", "printf '\\nInclude /fixture/dav.conf\\n' >> /usr/local/apache2/conf/httpd.conf; "
                         "cp /fixture/dav.passwd /tmp/dav.passwd; chmod 644 /tmp/dav.passwd; "
                         "chown -R daemon:daemon /usr/local/apache2/htdocs; exec httpd-foreground"])
    port = owner.port(name, 443)
    wait_port(port)
    # Actual authenticated TLS/PROPFIND readiness, with the owned CA, not merely a socket.
    owner.command(["curl", "--fail", "--silent", "--cacert", owner.root / "ca.pem",
                   "--user", f"{user}:{password}", "--request", "PROPFIND", "--header", "Depth: 0",
                   f"https://127.0.0.1:{port}/"])
    result.append(connection("webdav", "webdav", port, user, password, "/"))
    return result


def windows_protocols(owner):
    script = owner.root / "smb.ps1"
    script.write_text('''param([string]$Action,[string]$Root,[string]$Name,[string]$Letter)
$ErrorActionPreference='Stop'
if($Action -eq 'up') {
 $identity=[System.Security.Principal.WindowsIdentity]::GetCurrent().Name
 New-SmbShare -Name $Name -Path $Root -Description $Name -FullAccess $identity -Temporary | Out-Null
 $unc="\\\\localhost\\$Name"
 $letter=([char[]](90..68) | Where-Object { !(Test-Path "$($_):\\") -and !(Get-SmbMapping -LocalPath "$($_):" -ErrorAction SilentlyContinue) } | Select-Object -First 1)
 if(!$letter) { throw 'No free drive letter' }
 New-SmbMapping -LocalPath "$($letter):" -RemotePath $unc -Persistent $false | Out-Null
 @{unc=$unc;mapped="$($letter):\\";letter="$($letter):";user=$identity} | ConvertTo-Json -Compress
} else {
 $share=Get-SmbShare -Name $Name -ErrorAction SilentlyContinue
 if($share -and ($share.Path -ne $Root -or $share.Description -ne $Name)) {
  throw 'Refusing cleanup of foreign SMB share'
 }
 if(!$Letter) {
  $expected="\\\\localhost\\$Name"
  $owned=@(Get-SmbMapping -ErrorAction SilentlyContinue | Where-Object { $_.RemotePath -eq $expected })
  foreach($mapping in $owned) { Remove-SmbMapping -LocalPath $mapping.LocalPath -Force -UpdateProfile:$false }
 }
 if($Letter) { Remove-SmbMapping -LocalPath $Letter -Force -UpdateProfile:$false -ErrorAction SilentlyContinue }
 Remove-SmbShare -Name $Name -Force -ErrorAction SilentlyContinue
 if(Get-SmbShare -Name $Name -ErrorAction SilentlyContinue) { throw 'Owned SMB share remains after cleanup' }
 $remaining=@(Get-SmbMapping -ErrorAction SilentlyContinue | Where-Object { $_.RemotePath -eq "\\\\localhost\\$Name" })
 if($remaining.Count -ne 0) { throw 'Owned SMB mapping remains after cleanup' }
}
''', encoding="utf-8")
    share = owner.prefix
    root = owner.root / "smb-root"
    root.mkdir()
    state = {}
    def cleanup():
        owner.command(["powershell", "-NoProfile", "-File", script, "down", root, share,
                       state.get("letter", "")], timeout=60)
    owner.cleanups.append(cleanup)
    state.update(json.loads(owner.command(["powershell", "-NoProfile", "-File", script,
                                          "up", root, share, ""])))
    result = connection("unc", "share", 0, state["user"], "", state["unc"])
    mapped = dict(name="mapped", endpoint=state["mapped"], protocol="local")
    return [result, mapped]


@contextmanager
def fixtures(logs: Path, env: dict, cli: Path, share_server: Path | None):
    logs = Path(logs).resolve()
    logs.mkdir(parents=True, exist_ok=True)
    # Private profiles/keys/manifest must not enter uploaded diagnostic logs.
    root = Path(tempfile.mkdtemp(prefix="sync-provider-"))
    root.chmod(0o700)
    owned = Owner(root, logs, dict(env))
    try:
        protocols = windows_protocols(owned) if os.name == "nt" else linux_protocols_with_ca(owned)
        local = root / "local"
        local.mkdir()
        protocols.insert(0, dict(name="local", protocol="local", endpoint=str(local)))
        if share_server is None or not Path(share_server).is_file():
            raise RuntimeError("C04 requires the existing development Share server binary")
        module_path = Path(__file__).with_name("sync-reliability-share-fixture.py")
        spec = importlib.util.spec_from_file_location("sync_share_fixture", module_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with module.peers(owned, Path(cli), Path(share_server)) as shares:
            protocols.extend(shares)
            archive = root / "readonly.zip"
            with zipfile.ZipFile(archive, "w") as zipped:
                zipped.writestr(".obsidian/preferences.json", b'{"zip":"readonly source"}\n')
            manifest = root / "providers.json"
            manifest.write_text(json.dumps(dict(providers=protocols, zip=str(archive))), encoding="utf-8")
            manifest.chmod(0o600)
            runtime = dict(env, SE_SYNC_PROVIDER_MANIFEST=str(manifest))
            if os.name != "nt":
                runtime["SE_SYNC_FIXTURE_CA_DER"] = str(root / "ca.der")
            yield runtime
    finally:
        owned.close()
        shutil.rmtree(root)


def linux_protocols_with_ca(owner):
    certificate(owner)
    return linux_protocols(owner)
