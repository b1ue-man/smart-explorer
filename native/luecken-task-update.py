"""Update stage (Linux): a real update from the published `se` 0.5.180.

Called only by native/test-luecken-task.py (`lib` is that module); no entry
point of its own. Plan: docs/plaene/2026-10-16-luecken-schliessen/plan.md (E11,
SP1, SP2, SP6, SP9).

The published Linux `se` is taken from the update feed of the base commit
(`git show <base>:release-native/update-feed/se`) and checked against the
`se.sha256` of the same revision. With it an isolated host client "B" gets the
state a user of 0.5.180 has:

  * a Share pair with a counterpart "A" (the candidate `se`, so the pair is
    a mixed old/new one) and an Exec grant for A, running on B's old worker;
  * a saved SFTP connection with a password to an OpenSSH container that has
    several host-key types, which 0.5.180 pinned silently (no confirmation);
  * plaintext (`SESEC01`) secret records: the Share identity and the password.

Then B is updated the way the updater does it: the binary is replaced in
place (rename), the updater's `daemon.stop` request ends the old worker (the
test kills nothing), and the first candidate CLI call starts the candidate
worker. Afterwards, without any secret access of the CLI, every old record is
sealed, none of the old secret bytes is left anywhere, and identity, pin,
saved connection and Exec grant (enabled, revision, Standardnutzer, full
shell) work as before. Last, a restricted grant written by the candidate is
read with the 0.5.180 binary: it must appear switched off (D5/E8).

Why `daemon.stop` instead of the version-bound handoff: the suite's candidate
is built before the release's version bump, so it reports the same version as
the published `se` (0.5.180), and `worker_version_is_current` (version
equality) would keep the old worker. The real updater ends the worker with
`daemon.stop` as well.
"""
import hashlib
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from types import SimpleNamespace

OLD_REVISION = "01546a6307887f14d6e0b9a700d86a4c476bdb67"
OLD_VERSION = "0.5.180"
FEED = "release-native/update-feed"
LEGACY_MAGIC = b"SESEC01\0"
SEALED_MAGIC = b"SESEC02\0"
LEGACY_HEADER_BYTES = 8 + 1 + 32 + 4
MIN_NEEDLE_BYTES = 12
PROTECTORS = ("systemd-creds-system", "systemd-creds-user", "app-key-file")


def feed_blob(lib, name):
    done = subprocess.run(["git", "show", f"{OLD_REVISION}:{FEED}/{name}"], cwd=lib.ai.ROOT,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=300)
    lib.ensure(done.returncode == 0, f"git show {OLD_REVISION[:8]}:{FEED}/{name} failed (shallow checkout?): "
                                     f"{done.stderr.decode(errors='replace')[:400]}")
    return done.stdout


def fetch_published_se(lib, directory):
    """The published 0.5.180 `se`, verified against its published SHA-256."""
    version = feed_blob(lib, "version.txt").decode().strip()
    lib.ensure(version == OLD_VERSION, f"the feed of the base commit says {version}, expected {OLD_VERSION}")
    blob = feed_blob(lib, "se")
    sidecar = feed_blob(lib, "se.sha256").decode().split()[0]
    digest = hashlib.sha256(blob).hexdigest()
    lib.ensure(digest == sidecar, f"published se has SHA-256 {digest}, its sidecar says {sidecar}")
    lib.ensure(blob[:4] == b"\x7fELF", "the published se is not a Linux executable")
    path = directory / "se"
    directory.mkdir(parents=True, exist_ok=True)
    path.write_bytes(blob)
    path.chmod(0o755)
    shown = lib.ai.run([path, "--version"], timeout=60)
    lib.ensure(shown.returncode == 0 and OLD_VERSION in lib.text(shown), lib.text(shown))
    return path, digest


def marker_pids(client):
    """Processes whose environment carries this client's data directory."""
    marker = f"XDG_DATA_HOME={client.root / 'data'}".encode()
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if marker in (entry / "environ").read_bytes().split(b"\0"):
                found.append(int(entry.name))
        except OSError:
            continue
    return sorted(found)


def alive(client, pid):
    return pid in marker_pids(client)


def parse_legacy(data):
    """Secret bytes of a `SESEC01` record (magic, version, account digest, length, secret, SHA-256)."""
    length = int.from_bytes(data[LEGACY_HEADER_BYTES - 4:LEGACY_HEADER_BYTES], "big")
    if len(data) != LEGACY_HEADER_BYTES + length + 32:
        raise AssertionError("damaged plaintext record")
    return data[LEGACY_HEADER_BYTES:LEGACY_HEADER_BYTES + length]


def pair(lib, a, b, a_bin, b_bin, signal, relay, log):
    """Pairs A with B (Direct); returns A's endpoint of B and the readiness probe."""
    ai, ensure, text = lib.ai, lib.ensure, lib.text

    def worker_ready(client, binary):
        status = json.loads(client.se(binary, "share", "status", "--json", log=log).stdout)
        worker = status["worker"]
        return bool(worker["reachable"] and worker["running"] and worker["connected"]
                    and (worker.get("relay_url") or "").rstrip("/") == relay)

    code = json.loads(b.se(b_bin, "share", "identity", "--json", log=log).stdout)["direct_code"]
    for client, binary in ((b, b_bin), (a, a_bin)):
        configured = client.se(binary, "share", "configure", "--server", f"tcp://{signal}", "--allow-plaintext",
                               log=log)
        ensure(configured.returncode == 0, text(configured))
        ai.poll(lambda client=client, binary=binary: worker_ready(client, binary), 90,
                "Share worker did not connect")
    added = json.loads(a.se(a_bin, "connections", "add-peer", "--code", code, "--name", "Target", "--json",
                            log=log).stdout)
    endpoint, request_id = added["endpoint"], added["request_id"]

    def accepted():
        inbox = json.loads(b.se(b_bin, "share", "request", "--json", log=log).stdout)
        if any(entry.get("decision", {}).get("state") == "pending" for entry in inbox.get("requests", [])):
            b.se(b_bin, "share", "request", "accept", "--json", log=log)
        shown = json.loads(a.se(a_bin, "share", "request", "show", request_id, "--json", log=log).stdout)
        return shown["decision"]["state"] == "accepted" and shown["authorization"]["active"]
    ai.poll(accepted, 90, "the access request was never accepted")
    return endpoint, worker_ready


def stage_update(lib, results, candidate, work, log_root, server_binary):
    stage = "update"
    logs = log_root / "update"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "cli.log"
    ai, ensure, text = lib.ai, lib.ensure, lib.text
    state = SimpleNamespace(published=None, old=None, container=None, server=None)
    clients = []

    def need(*names):
        missing = [name for name in names if getattr(state, name, None) is None]
        ensure(not missing, f"an earlier update check did not establish: {', '.join(missing)}")

    published_dir = work / "update-published"
    try:
        state.published, digest = fetch_published_se(lib, published_dir)
        results.record(stage, f"published se {OLD_VERSION} from the feed of {OLD_REVISION[:8]}: SHA-256, ELF, version",
                       True, digest)
    except Exception as error:  # noqa: BLE001
        results.record(stage, f"published se {OLD_VERSION} from the feed", False, f"{type(error).__name__}: {error}")
        return
    if server_binary is None:
        results.record(stage, "candidate share server fixture", False, "the server stage did not build it")
        return
    install = work / "update-install"
    install.mkdir(parents=True, exist_ok=True)
    installed = install / "se"
    shutil.copyfile(state.published, installed)
    installed.chmod(0o755)
    try:
        agent_key = work / "update_agent_key"
        generated = ai.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "se-luecken-update", "-f",
                            agent_key], log=log)
        ensure(generated.returncode == 0, text(generated))
        container, port, user, password = lib.start_openssh(logs, (work / "update_agent_key.pub").read_text().strip())
        state.container = container
        lines, by_type = lib.scan_host_keys(port, work, log)
        ensure(len(by_type) >= 2, f"the SSH container presents only {sorted(by_type)}")
        server, signal, relay = lib.start_share_server(server_binary, logs, "update")
        state.server = server
    except Exception as error:  # noqa: BLE001
        results.record(stage, "fixtures (OpenSSH container with several host-key types, share server)", False,
                       f"{type(error).__name__}: {error}")
        cleanup(lib, state, clients)
        return
    env = {"SE_SHARE_RELAY_ONLY": "1"}
    a = lib.Client(work, "update-a", env)
    b = lib.Client(work, "update-b", env)
    clients.extend([a, b])
    account = f"sftp://{user}@127.0.0.1:{port}/config"
    digest_name = hashlib.sha256(b"smart_explorer\0" + account.encode()).hexdigest() + ".secret"
    store = b.app_data / "secrets-v1"
    pins = b.app_data / "known_hosts_sftp.txt"
    profile = b.app_data / "share_profiles.json"
    try:
        import pwd
        worker_user = pwd.getpwuid(os.getuid()).pw_name

        # ------------------------------------------------------------------ the state of 0.5.180
        def old_state():
            lib.add_password_connection(b, installed, port, user, password, "box", log)
            listed = b.se(installed, "ls", "@box:/", log=log)
            ensure(listed.returncode == 0, f"0.5.180 could not list the container: {text(listed)}")
            ensure(pins.exists(), "0.5.180 stored no host-key pin")
            lines_ = [line for line in pins.read_text().splitlines() if line.startswith(f"127.0.0.1:{port} ")]
            ensure(len(lines_) == 1, f"pins: {pins.read_text()}")
            fingerprint = lines_[0].split()[1]
            ensure(fingerprint in by_type.values(), f"{fingerprint} is none of the server's keys {by_type}")
            state.pin_bytes = pins.read_bytes()
            state.pinned_type = next(kind for kind, value in by_type.items() if value == fingerprint)
            identity = json.loads(b.se(installed, "share", "identity", "--json", log=log).stdout)
            ensure(all(identity.get(field) for field in ("device_id", "node_id", "fingerprint", "direct_code")),
                   identity)
            state.identity = identity
            return f"pinned {state.pinned_type} of {sorted(by_type)}"
        results.check(stage, "0.5.180 state: saved SFTP password connection silently pinned, Share identity",
                      old_state)

        def mixed_pair():
            need("identity")
            endpoint, ready = pair(lib, a, b, candidate, installed, signal, relay, log)
            state.endpoint, state.ready = endpoint, ready
            # Exec is off for every device until its owner enables it locally (V3).
            off = lib.grants(b, installed, log)
            ensure(off["enabled"] is False, off)
            denied = a.se(candidate, "exec", endpoint, "--", "id", "-un", log=log, timeout=120)
            ensure(denied.returncode != 0 and worker_user not in denied.stdout.decode(), text(denied))
            unconfirmed = b.se(installed, "share", "grants", "exec", "enable", log=log)
            ensure(unconfirmed.returncode != 0, "enabling Exec without confirmation succeeded")
            ensure(lib.grants(b, installed, log)["enabled"] is False, "Exec was enabled without confirmation")
            enabled = json.loads(b.se(installed, "share", "grants", "exec", "enable", "--yes", "--json",
                                      log=log).stdout)
            ensure(enabled.get("persisted") and enabled.get("applied"), enabled)
            return endpoint
        results.check(stage, "pair of the candidate with the 0.5.180 worker; Exec off by default, enabled locally",
                      mixed_pair)

        def mixed_exec():
            need("endpoint")
            ran = lib.exec_until_ok(a, candidate, state.endpoint, log, "--", "id", "-un")
            ensure(ran.stdout.decode().strip() == worker_user, ran.stdout)
            shell = lib.exec_out(a, candidate, state.endpoint, log, "--shell", "printf LUECKEN_OLD_%s 1")
            ensure(shell.returncode == 0 and shell.stdout.decode() == "LUECKEN_OLD_1", text(shell))
            document = json.loads(profile.read_text())
            grants = lib.exec_grants_in(document)
            ensure(grants and all(grant["enabled"] is True for grant in grants), grants)
            state.grants_before = grants
            state.old_pids = marker_pids(b)
            state.a_pids = marker_pids(a)
            ensure(state.old_pids and state.a_pids, f"workers: B {state.old_pids}, A {state.a_pids}")
            return f"grant revision {[grant['policy_revision'] for grant in grants]}"
        results.check(stage, "mixed pair (candidate to 0.5.180): full shell as the worker's account", mixed_exec)

        def legacy_store():
            need("identity")
            ensure(store.is_dir(), "0.5.180 created no secret store")
            records = {}
            for path in sorted(store.iterdir()):
                if path.name.endswith(".secret"):
                    data = path.read_bytes()
                    ensure(data.startswith(LEGACY_MAGIC), f"{path.name} is not a plaintext record of 0.5.180")
                    records[path.name] = parse_legacy(data)
            ensure(len(records) >= 2, f"records of 0.5.180: {sorted(records)}")
            ensure(not (store / "keyring-v2").exists() and not store.with_suffix(".kek").exists(),
                   "0.5.180 left a key descriptor or key file")
            ensure(records.get(digest_name) == password.encode(), "the saved password is not the plaintext record")
            state.record_names = sorted(records)
            state.needles = {name: secret for name, secret in records.items() if len(secret) >= MIN_NEEDLE_BYTES}
            ensure(digest_name in state.needles, "the password is too short to be searched for")
            found = lib.files_containing(b.root, password.encode())
            ensure(any(path.endswith(digest_name) for path in found), f"the scan does not find the password: {found}")
            # Where else the old secrets already stand in plaintext before the update: the update is only
            # judged on what it was to seal, but such a copy is reported (it would be a finding of its own).
            state.preexisting = {}
            for name, secret in state.needles.items():
                outside = [path for path in lib.files_containing(b.root, secret) if not path.startswith(str(store))]
                state.preexisting[name] = set(outside)
                if outside:
                    print(f"[INFO] update: the secret of {name} is also in plaintext in {outside} before the update",
                          flush=True)
            return f"{len(records)} plaintext records, {len(state.needles)} searchable secrets"
        results.check(stage, "0.5.180 secret store (identity, password, pairing secrets) is plaintext: SESEC01, no key "
                      "descriptor", legacy_store)

        # ------------------------------------------------------------------ the update
        def update_in_place():
            need("old_pids", "a_pids")
            staged = install / "se.update"
            shutil.copyfile(candidate, staged)
            staged.chmod(0o755)
            os.replace(staged, installed)
            digest_new = hashlib.sha256(installed.read_bytes()).hexdigest()
            ensure(digest_new == hashlib.sha256(Path(candidate).read_bytes()).hexdigest(), "replacement differs")
            ensure(digest_new != hashlib.sha256(state.published.read_bytes()).hexdigest(),
                   "the candidate is the published binary")
            sync = b.app_data / "sync"
            ensure(sync.is_dir(), "no sync control directory")
            stage_file = sync / "daemon.stop.luecken"
            stage_file.write_text("stop")
            os.replace(stage_file, sync / "daemon.stop")      # the updater's request to the worker
            deadline = time.time() + 90
            while time.time() < deadline and any(alive(b, pid) for pid in state.old_pids):
                time.sleep(0.5)
            survivors = [pid for pid in state.old_pids if alive(b, pid)]
            ensure(not survivors, f"the 0.5.180 worker did not end on the updater's stop request: {survivors}")
            ensure(all(alive(a, pid) for pid in state.a_pids), "the update ended a process of the counterpart")
            started = b.se(installed, "share", "status", "--json", log=log)
            ensure(started.returncode == 0, text(started))
            # A key protector probe may take up to 90 s (three 30 s limits) before the worker starts Share.
            ai.poll(lambda: state.ready(b, installed), 240, "the candidate worker did not connect")
            fresh = marker_pids(b)
            ensure(fresh and not set(fresh) & set(state.old_pids), f"workers after the update: {fresh}")
            return f"old worker {state.old_pids} ended by itself, new {fresh}"
        results.check(stage, "update in place: binary replaced, old worker retired by daemon.stop (nothing killed), "
                      "candidate worker started", update_in_place)

        # ------------------------------------------------------------------ nothing may need a secret read
        def sealed_by_the_worker():
            need("record_names", "needles", "preexisting")

            def sealed():
                entries = list(store.iterdir())
                records = [path for path in entries if path.name.endswith(".secret")]
                return bool(records) and all(path.read_bytes().startswith(SEALED_MAGIC) for path in records) \
                    and not [path for path in entries if path.name.startswith(".stage")]
            ai.poll(sealed, 240, "the candidate worker left plaintext records (it must seal them at its start)")
            names = sorted(path.name for path in store.iterdir() if path.name.endswith(".secret"))
            ensure(set(state.record_names) <= set(names), f"records lost: {set(state.record_names) - set(names)}")
            ensure(not any(path.name.startswith(".stage") for path in store.iterdir()), "stage files left")
            descriptor = json.loads((store / "keyring-v2").read_text())
            ensure(descriptor["protector"] in PROTECTORS and len(descriptor["key_id"]) == 32, descriptor)
            for name, secret in state.needles.items():
                found = [path for path in lib.files_containing(b.root, secret)
                         if path not in state.preexisting.get(name, set())]
                ensure(not found, f"plaintext of {name} is still in {found}")
            ensure(oct(store.stat().st_mode & 0o777) == oct(0o700), oct(store.stat().st_mode))
            for path in list(store.iterdir()):
                if path.name.endswith(".secret") or path.name == "keyring-v2":
                    ensure(oct(path.stat().st_mode & 0o777) == oct(0o600),
                           f"{path.name}: {oct(path.stat().st_mode)}")
            key_file = store.with_suffix(".kek")
            if key_file.exists():
                ensure(oct(key_file.stat().st_mode & 0o777) == oct(0o600), oct(key_file.stat().st_mode))
            # The password record is read by no worker path: only the start-time port can have sealed it.
            ensure((store / digest_name).read_bytes().startswith(SEALED_MAGIC), "the password record is plaintext")
            return f"protector {descriptor['protector']}, {len(names)} sealed records"
        results.check(stage, "worker start alone sealed every old record (no secret access by any command), "
                      "no old secret byte left, modes 0700/0600", sealed_by_the_worker)

        def doctor_after():
            report = b.se(installed, "doctor", "--json", log=log)
            status = json.loads(report.stdout)["credential_store"]
            ensure(status["state"] == "ok", status)
            detail = status["detail"]
            ensure("plaintext_records=0" in detail and "stage_files=0" in detail
                   and "unreadable_records=0" in detail and "none-yet" not in detail, detail)
            return detail
        results.check(stage, "se doctor: plaintext_records=0, no stage or unreadable record", doctor_after)

        # ------------------------------------------------------------------ everything keeps working
        def identity_same():
            need("identity")
            shown = json.loads(b.se(installed, "share", "identity", "--json", log=log).stdout)
            for field in ("device_id", "node_id", "fingerprint", "direct_code"):
                ensure(shown[field] == state.identity[field], f"{field} changed: {state.identity[field]} -> {shown[field]}")
            return state.identity["node_id"]
        results.check(stage, "Share identity (NodeId, device id, fingerprint, direct code) unchanged", identity_same)

        def sftp_still_connects():
            need("pin_bytes")
            listed = b.se(installed, "ls", "@box:/", log=log)
            ensure(listed.returncode == 0, f"the saved password or the 0.5.180 pin no longer works: {text(listed)}")
            ensure(not lib.MARKER.search(text(listed)), "the update asked for a host-key confirmation again")
            ensure(pins.read_bytes() == state.pin_bytes, "the pin file changed")
            return f"connected with the {state.pinned_type} pin of 0.5.180"
        results.check(stage, "saved SFTP connection connects without a question (password read, 0.5.180 pin accepted, "
                      "server with several key types)", sftp_still_connects)

        def grant_kept():
            need("grants_before", "endpoint")
            grant = lib.grants(b, installed, log)
            ensure(grant["enabled"] is True and grant["run_as"] == "standard" and grant["programs"] == [], grant)
            after = lib.exec_grants_in(json.loads(profile.read_text()))
            ensure(len(after) == len(state.grants_before), after)
            for old_grant, new_grant in zip(state.grants_before, after):
                changed = {key: (value, new_grant.get(key)) for key, value in old_grant.items()
                           if new_grant.get(key) != value}
                ensure(not changed, f"the update changed the grant: {changed}")
            ran = lib.exec_until_ok(a, candidate, state.endpoint, log, "--", "id", "-un")
            ensure(ran.stdout.decode().strip() == worker_user, ran.stdout)
            shell = lib.exec_out(a, candidate, state.endpoint, log, "--shell", "printf LUECKEN_NEW_%s 2")
            ensure(shell.returncode == 0 and shell.stdout.decode() == "LUECKEN_NEW_2", text(shell))
            return "enabled, same revision, Standardnutzer, full shell"
        results.check(stage, "Exec grant: still enabled, same policy_revision, run_as standard, full shell as the "
                      "worker's account, pair works without re-pairing", grant_kept)

        def nothing_else_ended():
            need("a_pids")
            ensure(all(alive(a, pid) for pid in state.a_pids), "a process of the counterpart ended")
            running = ai.run(["docker", "inspect", "-f", "{{.State.Running}}", state.container]).stdout
            ensure(running.decode().strip() == "true", "the SSH container stopped")
            ensure(state.server.poll() is None, "the share server stopped")
            return "counterpart workers, SSH container and share server still running"
        results.check(stage, "the update ended nothing but the worker it replaced", nothing_else_ended)

        # ------------------------------------------------------------------ downgrade reading (D5/E8)
        def downgrade_reads_restricted_as_off():
            need("endpoint")
            reader = lib.Client(work, "update-old-reader", env)
            clients.append(reader)
            reader.app_data.mkdir(parents=True, exist_ok=True)
            os.chmod(reader.app_data, 0o700)

            def read_with_published(label):
                lib.write_private(reader.app_data / "share_profiles.json", profile.read_bytes())
                shown = reader.se(state.published, "share", "grants", "exec", "--json", log=log)
                ensure(shown.returncode == 0, f"0.5.180 cannot read the {label} profile: {text(shown)}")
                values = json.loads(shown.stdout)
                ensure(len(values) == 1, values)
                return values[0]["enabled"]
            ensure(read_with_published("unrestricted") is True, "control: 0.5.180 reads the full grant as enabled")
            restricted = lib.set_grant(b, installed, log, "--allow-program", "/usr/bin/id")
            ensure(restricted.returncode == 0, text(restricted))
            ensure(lib.grants(b, installed, log)["programs"] == ["/usr/bin/id"], "the program list was not stored")
            ensure(read_with_published("restricted") is False,
                   "0.5.180 reads a restricted grant as ENABLED: a downgrade would open a full shell")
            full = lib.set_grant(b, installed, log, "--full")
            ensure(full.returncode == 0, text(full))
            ensure(read_with_published("restored") is True, "control: the restored grant reads as enabled")
        results.check(stage, "downgrade: 0.5.180 reads a grant restricted by the candidate as switched off",
                      downgrade_reads_restricted_as_off)
    finally:
        cleanup(lib, state, clients)


def cleanup(lib, state, clients):
    lib.ai.stop_background_daemons(clients)
    if state.server is not None:
        state.server.terminate()
        try:
            state.server.wait(timeout=30)
        except subprocess.TimeoutExpired:
            state.server.kill()
    if state.container:
        lib.ai.run(["docker", "rm", "-f", state.container], timeout=120)
