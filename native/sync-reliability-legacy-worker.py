"""C08 helper for the single remote sync task; no independent suite entrypoint."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
OLD_TAG = "v0.5.169"
JOB_ID = "c08_legacy_worker"


def _sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def _wait(label, check, seconds=150):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.25)
    raise RuntimeError(f"C08 timed out waiting for {label}")


def _command(argv, log, env, seconds=120):
    flags = {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == "nt" else {"start_new_session": True}
    with log.open("wb") as out, log.with_suffix(".stderr.log").open("wb") as err:
        child = subprocess.Popen([str(arg) for arg in argv], cwd=ROOT, env=env,
                                 stdin=subprocess.DEVNULL, stdout=out, stderr=err, **flags)
        try:
            code = child.wait(timeout=seconds)
        except BaseException as error:
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"],
                               stdout=err, stderr=err, timeout=45, check=False)
            else:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            child.wait(timeout=45)
            if isinstance(error, subprocess.TimeoutExpired):
                raise RuntimeError(f"C08 command timed out: {argv[0]} {argv[1:]}") from error
            raise
    if code:
        raise RuntimeError(f"C08 command exited {code}; inspect {log}")
    return log.read_text(encoding="utf-8").strip()


def _owned_pids(paths):
    if os.name == "nt":
        script = "Get-CimInstance Win32_Process | Select-Object ProcessId,ExecutablePath | ConvertTo-Json -Compress"
        answer = subprocess.check_output(["pwsh", "-NoProfile", "-Command", script],
                                         text=True, encoding="utf-8", timeout=30).strip()
        rows = json.loads(answer) if answer else []
        if isinstance(rows, dict):
            rows = [rows]
        return {int(row["ProcessId"]) for row in rows
                if row.get("ExecutablePath") and Path(row["ExecutablePath"]).resolve() in paths}
    pids = set()
    for entry in Path("/proc").iterdir():
        if entry.name.isdecimal():
            try:
                if (entry / "exe").resolve(strict=True) in paths:
                    pids.add(int(entry.name))
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                pass
    return pids


def _stop(sync, paths, force):
    control_error = None
    if sync.is_dir():
        try:
            _atomic_text(sync / "daemon.stop", "stop")
        except OSError as error:
            if not force:
                raise
            control_error = error
    deadline = time.monotonic() + 45
    while _owned_pids(paths) and time.monotonic() < deadline:
        time.sleep(0.5)
    remaining = _owned_pids(paths)
    if remaining and force:
        deadline = time.monotonic() + 45
        while remaining and time.monotonic() < deadline:
            for pid in remaining:
                # Recheck ownership; rescan catches a last guardian restart.
                if pid not in _owned_pids(paths):
                    continue
                if os.name == "nt":
                    subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"],
                                   timeout=45, check=False, stdout=subprocess.DEVNULL)
                else:
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            remaining = _owned_pids(paths)
        if remaining:
            raise RuntimeError(f"C08 owned cleanup left processes: {sorted(remaining)}")
    elif remaining:
        raise RuntimeError(f"C08 worker did not stop normally: {sorted(remaining)}")
    if control_error:
        raise RuntimeError("C08 stop request failed; owned processes were closed through fallback cleanup") from control_error


def _atomic_text(path, body):
    stage = path.with_name(path.name + ".c08.tmp")
    with stage.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write(body)
        stream.flush()
        os.fsync(stream.fileno())
    stage.replace(path)


class _Activation:
    """Use the historical activation contract and restore exactly our value."""
    def __init__(self, config, cli):
        self.config = config
        self.cli = cli
        self.previous = None
        self.key_created = False

    def enable(self):
        if os.name != "nt":
            entry = self.config / "autostart/smart-explorer-sync-daemon.desktop"
            entry.parent.mkdir(parents=True)
            quoted = str(self.cli).replace("\\", "\\\\\\\\").replace("%", "%%")
            for char in ('"', '`', '$'):
                quoted = quoted.replace(char, "\\\\" + char)
            entry.write_text("[Desktop Entry]\nType=Application\nName=Smart Explorer Sync Daemon\n"
                             f'Exec="{quoted}" --sync-daemon\nTerminal=false\n'
                             "X-GNOME-Autostart-enabled=true\n", encoding="utf-8")
            return
        import winreg
        self.key_name = r"Software\Microsoft\Windows\CurrentVersion\Run"
        try:
            key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, self.key_name, 0, winreg.KEY_READ | winreg.KEY_SET_VALUE)
        except FileNotFoundError:
            key = winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, self.key_name, 0, winreg.KEY_READ | winreg.KEY_SET_VALUE)
            self.key_created = True
        with key:
            try:
                self.previous = winreg.QueryValueEx(key, "SmartExplorerSync")
            except FileNotFoundError:
                pass
            self.value = f'"{self.cli}" --sync-daemon'
            winreg.SetValueEx(key, "SmartExplorerSync", 0, winreg.REG_SZ, self.value)

    def restore(self):
        if os.name != "nt" or not hasattr(self, "value"):
            return
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, self.key_name, 0, winreg.KEY_READ | winreg.KEY_SET_VALUE) as key:
            if winreg.QueryValueEx(key, "SmartExplorerSync") != (self.value, winreg.REG_SZ):
                raise RuntimeError("C08 activation changed externally; refusing to replace another writer")
            if self.previous is None:
                winreg.DeleteValue(key, "SmartExplorerSync")
            else:
                winreg.SetValueEx(key, "SmartExplorerSync", 0, self.previous[1], self.previous[0])
        if self.key_created:
            try:
                winreg.DeleteKey(winreg.HKEY_CURRENT_USER, self.key_name)
            except OSError:
                # Another value may have been added; never remove that value.
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, self.key_name) as key:
                    if winreg.QueryInfoKey(key)[:2] == (0, 0):
                        raise


def _settings(path):
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if "=" not in line or line.startswith("#"):
            continue
        key, value = line.split("=", 1)
        if key in ("source_json", "target_json"):
            key, value = key.removesuffix("_json"), json.loads(value)
        result.setdefault(key, []).append(value)
    result.pop("last_run", None)
    return result


def _baseline(path):
    data = path.read_bytes()
    if not data.startswith(b"SEBL\x02"):
        result = {}
        for line in data.decode("utf-8").splitlines():
            rel, a, b = line.split("\t")
            result[rel] = tuple(None if sig == "-" else tuple(map(int, sig.split(":"))) for sig in (a, b))
        return result
    offset, result = 5, {}
    while offset < len(data):
        size = struct.unpack_from(">I", data, offset)[0]
        offset += 4
        rel = data[offset:offset + size].decode("utf-8")
        offset += size
        sides = []
        for _ in range(2):
            tag = data[offset]
            offset += 1
            if tag == 1:
                sides.append(struct.unpack_from(">QqQ", data, offset))
                offset += 24
            elif tag == 0:
                sides.append(None)
            else:
                raise RuntimeError("C08 unreadable baseline signature")
        if rel in result:
            raise RuntimeError("C08 duplicate baseline path")
        result[rel] = tuple(sides)
    return result


def _generation(sync):
    value = (sync / "daemon.generation").read_text().strip()
    if not re.fullmatch(r"[a-fA-F0-9]{32}", value):
        raise RuntimeError("C08 worker published an invalid generation")
    return value


def _state(sync):
    path = sync / "job-state" / f"{JOB_ID}.json"
    return json.loads(path.read_text(encoding="utf-8")) if path.is_file() else None


def _success(sync, after):
    state = _state(sync)
    if not state or state.get("running"):
        return None
    if state.get("blocked") or state.get("last_error"):
        raise RuntimeError("C08 stored job failed: " + json.dumps(state, ensure_ascii=False))
    result = state.get("last_result") or {}
    when = result.get("when", 0)
    success = state.get("last_success") or 0
    started = state.get("last_attempt") or 0
    if when > after and started <= when <= success and state.get("last_runner") == "daemon":
        if result.get("errors") or state.get("consecutive_failures"):
            raise RuntimeError("C08 success contains unconfirmed errors")
        return state
    return None


def _owned_baseline(sync):
    matches = list(sync.glob(f"pairs/*/job-{JOB_ID}.*.sebl"))
    if len(matches) != 1:
        raise RuntimeError(f"C08 expected one job-owned baseline, discovered {matches}")
    return matches[0]


def _same_bytes(source, target, relatives):
    for rel in relatives:
        if (source / rel).read_bytes() != (target / rel).read_bytes():
            raise RuntimeError(f"C08 endpoints differ at {rel}")


def _hidden_file(path):
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetFileAttributesW.argtypes = [wintypes.LPCWSTR]
        kernel.GetFileAttributesW.restype = wintypes.DWORD
        kernel.SetFileAttributesW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
        kernel.SetFileAttributesW.restype = wintypes.BOOL
        attributes = kernel.GetFileAttributesW(str(path))
        if attributes == 0xFFFFFFFF or not kernel.SetFileAttributesW(str(path), attributes | 0x2):
            raise ctypes.WinError(ctypes.get_last_error())


def _state_key(baseline):
    history = json.loads((baseline.parent / f"job-{JOB_ID}.replicas.json").read_text(encoding="utf-8"))
    return {"pairId": baseline.parent.name, "owner": f"job-{JOB_ID}",
            "replicaA": history["replica_a"], "replicaB": history["replica_b"]}


def _preserved_payload(root, payload, exclude):
    return [str(path) for path in root.rglob("*")
            if path.is_file() and path not in exclude and path.stat().st_size == len(payload)
            and path.read_bytes() == payload]


def run(candidate_cli: Path, logs: Path, env: dict, candidate_sha: str) -> dict:
    """Run C08 on Linux/Windows remote CI and return discovered runtimeInputs."""
    if env.get("GITHUB_ACTIONS") != "true" or sys.platform not in ("linux", "win32"):
        raise RuntimeError("C08 requires the configured Linux or Windows remote CI runner")
    if env.get("SMART_EXPLORER_E2E_TEST_NAMESPACE"):
        raise RuntimeError("C08 legacy worker requires the normal IPC profile without the debug fixture namespace")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=30).strip()
    if not re.fullmatch(r"[a-f0-9]{40}", candidate_sha) or head != candidate_sha or env.get("GITHUB_SHA") != head:
        raise RuntimeError("C08 checkout, workflow and requested candidate must match")
    logs.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="c08-legacy-worker-", dir=logs)).resolve()
    name = "se.exe" if os.name == "nt" else "se"
    previous, current = work / "previous" / name, work / "candidate" / name
    previous.parent.mkdir()
    current.parent.mkdir()
    shutil.copy2(candidate_cli, current)
    blob = f"{OLD_TAG}:release-native/update-feed/{name}"
    with previous.open("wb") as stream:
        subprocess.run(["git", "show", blob], cwd=ROOT, stdout=stream, check=True, timeout=120)
    sidecar = subprocess.check_output(["git", "show", blob + ".sha256"], cwd=ROOT, text=True, timeout=30).split()[0]
    if _sha(previous) != sidecar:
        raise RuntimeError("C08 historical payload differs from its published SHA-256")
    if os.name != "nt":
        previous.chmod(0o700)
        current.chmod(0o700)
    data, config = work / "data", work / "config"
    data.mkdir(mode=0o700)
    config.mkdir(mode=0o700)
    runtime_env = dict(env, APPDATA=str(data), LOCALAPPDATA=str(work / "local"),
                       XDG_DATA_HOME=str(data), XDG_CONFIG_HOME=str(config), XDG_CACHE_HOME=str(work / "cache"))
    sync = data / "smart_explorer/sync"
    jobs = sync / "jobs"
    jobs.mkdir(parents=True)
    source, target = work / "source folder", work / "target folder"
    source.mkdir()
    target.mkdir()
    initial = {f"stable-{i}.txt": f"old-stable-{i}".encode() for i in range(8)}
    initial.update({"change.txt": b"old-change", "delete.txt": b"old-delete",
                    "conflict.txt": b"old-conflict", "resume.bin": b"old-resume"})
    for rel, payload in initial.items():
        (source / rel).write_bytes(payload)
    (source / ".hidden").write_bytes(b"filtered hidden bytes")
    _hidden_file(source / ".hidden")
    (source / "ignored.skip").write_bytes(b"filtered ignored bytes")
    job = jobs / f"{JOB_ID}.conf"
    body = {"id": JOB_ID, "name": "C08 published legacy job", "source": source.as_posix(),
            "target": target.as_posix(), "direction": "both", "conflict": "keepboth", "retain_days": "31",
            "interval_min": "1", "include_hidden": "0", "ignore": "*.skip", "last_run": "0",
            "enabled": "1", "trigger": "interval", "catch_up": "1", "delete_policy": "propagate",
            "compare": "checksum", "versioning_scheme": "days", "max_delete_pct": "20",
            "bwlimit_kbps": "512", "atomic_copy": "1", "verify": "1", "retries": "1", "retry_delay_secs": "0"}
    job.write_text("# historical normal job\n" + "".join(f"{key}={value}\n" for key, value in body.items()), encoding="utf-8")
    (sync / "cadence.txt").write_text("2", encoding="utf-8")
    activation = _Activation(config, previous)
    paths = {previous.resolve(), current.resolve()}
    report = {"case": "C08", "candidate": candidate_sha, "runtimeInputs": {
        "platform": sys.platform, "profile": str(data), "configProfile": str(config),
        "sourceRoot": str(source), "targetRoot": str(target), "jobFile": str(job), "jobId": JOB_ID,
        "initialByteSha256": {rel: hashlib.sha256(payload).hexdigest() for rel, payload in initial.items()},
        "oldTag": OLD_TAG, "oldTagCommit": subprocess.check_output(["git", "rev-parse", OLD_TAG + "^{commit}"], cwd=ROOT, text=True, timeout=30).strip(),
        "oldCli": str(previous), "oldCliSha256": sidecar, "candidateCli": str(current), "candidateCliSha256": _sha(current)}}
    try:
        activation.enable()
        old_version = _command([previous, "--version"], work / "old-version.log", runtime_env).split()[-1]
        version = _command([current, "--version"], work / "candidate-version.log", runtime_env).split()[-1]
        if old_version != OLD_TAG.removeprefix("v"):
            raise RuntimeError("C08 extracted CLI reports another historical version")
        report["runtimeInputs"].update(oldVersion=old_version, candidateVersion=version)
        before = json.loads(_command([previous, "share", "status", "--json"], work / "old-worker.json", runtime_env))
        if not before["worker"]["reachable"] or not _owned_pids({previous.resolve()}):
            raise RuntimeError("C08 normal old CLI did not start its own reachable worker")
        old_generation = _generation(sync)
        report["runtimeInputs"]["oldWorkerPids"] = sorted(_owned_pids({previous.resolve()}))

        def old_run():
            result = sync / "results.tsv"
            if not result.is_file():
                return None
            for line in result.read_text(encoding="utf-8").splitlines():
                fields = line.split("\t")
                if fields[0] == JOB_ID and len(fields) >= 8:
                    if int(fields[6]):
                        raise RuntimeError("C08 old worker reported an actual job error: " + line)
                    if int(fields[2]) >= len(initial) and int(fields[1]) > 0:
                        return {"when": int(fields[1]), "a_to_b": int(fields[2]), "b_to_a": int(fields[3]),
                                "deleted": int(fields[4]), "conflicts": int(fields[5]), "errors": int(fields[6]),
                                "note": "\t".join(fields[7:])}
            return None

        old_result = _wait("successful published old job", old_run)
        old_when = old_result["when"]
        _same_bytes(source, target, initial)
        if (target / ".hidden").exists() or (target / "ignored.skip").exists():
            raise RuntimeError("C08 old worker did not apply the saved hidden/ignore filters")
        old_baselines = list(sync.glob("baseline_*.sebl"))
        if len(old_baselines) != 1 or not set(initial).issubset(_baseline(old_baselines[0])):
            raise RuntimeError("C08 old worker did not create its actual pair-wide baseline")
        protected = _settings(job)
        report["oldRun"] = dict(old_result, baseline=str(old_baselines[0]), baselineSha256=_sha(old_baselines[0]))
        completion = json.loads(_command([current, "update", "--complete-install", version], work / "handoff.json", runtime_env))
        if completion != {"version": version, "worker": "replaced", "worker_error": None}:
            raise RuntimeError("C08 normal version-bound worker replacement failed: " + json.dumps(completion))
        status = json.loads(_command([current, "share", "status", "--json"], work / "candidate-worker.json", runtime_env))
        new_generation = _generation(sync)
        if not status["worker"]["reachable"] or new_generation == old_generation:
            raise RuntimeError("C08 replacement did not publish a reachable new generation")
        _wait("retiring old worker exit", lambda: not _owned_pids({previous.resolve()}), seconds=45)
        state = _wait("candidate adoption of the original job", lambda: _success(sync, old_when))
        baseline = _owned_baseline(sync)
        adopted_key = _state_key(baseline)
        if not set(initial).issubset(_baseline(baseline)):
            raise RuntimeError("C08 migration lost old baseline entries")
        if any(_settings(job).get(key) != value for key, value in protected.items()):
            raise RuntimeError("C08 worker update changed saved endpoints or options")
        report["runtimeInputs"].update(oldGeneration=old_generation, candidateGeneration=new_generation,
                                      baseline=str(baseline), ownerToken=f"job-{JOB_ID}",
                                      candidateWorkerPids=sorted(_owned_pids({current.resolve()})),
                                      savedSettings=protected, stateKey=adopted_key)
        _atomic_text(sync / "pause.until", str(2**63 - 1))
        (source / "change.txt").write_bytes(b"candidate changed existing file")
        (source / "added.txt").write_bytes(b"candidate added file")
        (source / "delete.txt").unlink()
        (sync / "pause.until").unlink()
        changed = _wait("candidate changed-file and delete run", lambda: _success(sync, state["last_success"]))
        _same_bytes(source, target, ["change.txt", "added.txt"])
        if (target / "delete.txt").exists() or changed["last_result"]["deleted"] < 1:
            raise RuntimeError("C08 stored delete policy did not propagate the deletion")
        for rel in (".hidden", "ignored.skip"):
            if (target / rel).exists():
                raise RuntimeError("C08 stored filters changed meaning after update")
        _atomic_text(sync / "pause.until", str(2**63 - 1))
        winner, loser = b"candidate conflict winner", b"candidate conflict losing bytes"
        (source / "conflict.txt").write_bytes(winner)
        (target / "conflict.txt").write_bytes(loser)
        stamp = time.time()
        os.utime(source / "conflict.txt", (stamp + 10, stamp + 10))
        os.utime(target / "conflict.txt", (stamp + 5, stamp + 5))
        (sync / "pause.until").unlink()
        conflict = _wait("stored keep-both conflict choice", lambda: _success(sync, changed["last_success"]))
        _same_bytes(source, target, ["conflict.txt"])
        if (target / "conflict.txt").read_bytes() != winner:
            raise RuntimeError("C08 stored conflict choice selected unexpected bytes")
        preserved = _preserved_payload(work, loser, {source / "conflict.txt", target / "conflict.txt"})
        deleted_backup = _preserved_payload(work, initial["delete.txt"], set())
        if not preserved or not deleted_backup:
            raise RuntimeError("C08 overwritten/deleted bytes are not recoverable")
        baseline_before = _baseline(baseline)
        (work / "baseline-before-interruption.sebl").write_bytes(baseline.read_bytes())
        _atomic_text(sync / "pause.until", str(2**63 - 1))
        chunk = b"C08 interrupted atomically\x00"
        payload = chunk * (32 * 1024 * 1024 // len(chunk))
        (source / "resume.bin").write_bytes(payload)
        (sync / "pause.until").unlink()

        def transferring():
            running = _state(sync)
            if not running or not running.get("running"):
                return None
            # apply_stage::stage -> unique_staging_path(..., "bisync");
            # LocalBackend streams directly into this exclusive private stage.
            for path in target.iterdir():
                if not re.fullmatch(r"resume\.bin\.se-bisync-[0-9a-f]{16}", path.name):
                    continue
                try:
                    size = path.stat().st_size
                except FileNotFoundError:
                    continue
                if 0 < size < len(payload):
                    return {"path": str(path), "partialSize": size, "expectedSize": len(payload),
                            "jobStarted": running["running"]["started"],
                            "payloadSha256": hashlib.sha256(payload).hexdigest()}
            return None

        report["interruptionInput"] = _wait("actual in-flight staged transfer", transferring)
        stopping_pids = sorted(_owned_pids(paths))
        _stop(sync, paths, force=False)
        report["normalStop"] = {"ownedWorkerAndGuardianPids": stopping_pids,
                                "remainingOwnedPids": sorted(_owned_pids(paths))}
        if not stopping_pids or report["normalStop"]["remainingOwnedPids"]:
            raise RuntimeError("C08 normal stop did not close all owned worker/guardian processes")
        cancelled = _state(sync)
        if not cancelled or cancelled.get("running") or cancelled.get("last_success") != conflict["last_success"] or not cancelled.get("pending_trigger"):
            raise RuntimeError("C08 interruption lost pending trigger or falsely confirmed success")
        if not (cancelled.get("last_result") or {}).get("note", "").startswith("abgebrochen"):
            raise RuntimeError("C08 stopped worker did not record its actual cancellation")
        if (target / "resume.bin").read_bytes() != initial["resume.bin"] or _baseline(baseline)["resume.bin"] != baseline_before["resume.bin"]:
            raise RuntimeError("C08 interrupted replacement lost original bytes/baseline")
        restarted = json.loads(_command([current, "share", "status", "--json"], work / "restarted-worker.json", runtime_env))
        restart_generation = _generation(sync)
        if not restarted["worker"]["reachable"] or restart_generation == new_generation:
            raise RuntimeError("C08 same profile did not restart with a new reachable generation")
        resumed = _wait("same saved job convergence after interruption", lambda: _success(sync, conflict["last_success"]), seconds=240)
        if (target / "resume.bin").read_bytes() != payload or _owned_baseline(sync) != baseline or _state_key(baseline) != adopted_key:
            raise RuntimeError("C08 retry changed owner/replicas or failed to publish complete bytes")
        before_noop = _sha(baseline)
        noop = _wait("unchanged saved-job follow-up", lambda: _success(sync, resumed["last_success"]))
        if any(noop["last_result"][key] for key in ("a_to_b", "b_to_a", "deleted", "conflicts", "errors")) or _sha(baseline) != before_noop:
            raise RuntimeError("C08 follow-up did not converge to a baseline-preserving no-op")
        if any(_settings(job).get(key) != value for key, value in protected.items()):
            raise RuntimeError("C08 restart changed the original job settings")
        report["runtimeInputs"]["restartGeneration"] = restart_generation
        report.update(changedRun=changed, conflictRun=conflict, cancelledRun=cancelled, resumedRun=resumed,
                      noopRun=noop, preservedConflictBytes=preserved, deletedBackupBytes=deleted_backup,
                      finalBaselineSha256=before_noop)
    finally:
        try:
            try:
                _stop(sync, paths, force=True)
            finally:
                activation.restore()
                report["activationRestored"] = True
        finally:
            report["ownedProcessesClosed"] = not _owned_pids(paths)
            (work / "runtime.json").write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    return report
