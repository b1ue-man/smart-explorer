"""S3 (Tresore) in the batch suite: fixtures, stages and suite checks (plan D3; strang-tresore.md VLT-M14).

Fixtures (requested by expected entries with ``fixture=``; environment contract for test authors):

``interop``  builds ``native/vault-task-interop`` (MIT harness; CryptoFS 2.11.0 from Maven Central at run
             time, never uploaded as artifact or cache) and exports ``SE_VAULT_INTEROP_JAVA`` and
             ``SE_VAULT_INTEROP_JAR``. Call: ``$SE_VAULT_INTEROP_JAVA -jar $SE_VAULT_INTEROP_JAR <command>
             --vault DIR --password-file FILE ...`` with the commands ``create``, ``read``, ``write``,
             ``unlock``, ``health``, ``version`` (VaultTaskInterop.java). CryptoFS 2.11.0 is compiled for
             Java 25 (maven.compiler.release 25 in its POM), so the job installs Temurin 25.
``keep``     ``SE_VAULT_TASK_KEEP_VAULTS``: a test keeps a vault for the CryptoFS health check of the stage
             vault-interop as ``<dir>/<test name>/<case>/vault/`` plus ``password.txt`` (UTF-8, no newline)
             and optionally ``allowed-findings.txt`` (one CryptoFS result type per line, e.g. expected
             conflict cases).
``marker``   isolated HOME, XDG_* and TMPDIR under ``SE_VAULT_TASK_ROOT`` and the marker
             ``SE_VAULT_TASK_MARKER``. Tests put marker names and contents into the vault through the
             surfaces of VLT-M6 and keep their own cleartext inputs under ``$SE_VAULT_TASK_ROOT/input``;
             fallback copies made with consent are listed in ``$SE_VAULT_TASK_ROOT/allowed-marker-paths.txt``.
             The check ``vault_task_klartext_marker_scan`` then searches every byte and every name under the
             stage's fixture folders, ``/tmp`` and ``/dev/shm`` (files of this run) for the marker.

Suite checks: ``rdt_suite_interop_harness_selftest``, ``vault_task_interop_health_after_sync`` and
``..._after_compare`` (CryptoFS health of the kept vaults), ``vault_task_klartext_marker_scan`` and
``..._drive``, ``vault_task_docs_rows_t1_t22`` (VLT-M13) and three checks that are always ``not_run``
(``vault_task_screen_lock_delivery_by_os``, ``vault_task_backends_google_drive``,
``vault_task_android_arm64_compile``; D3 "nie als bestanden").
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import secrets
import shutil
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

from . import common
from .common import FAILED, LINUX, NATIVE, NOT_RUN, PASSED, REPO, WINDOWS

HARNESS_DIR = NATIVE / "vault-task-interop"
CRYPTOFS_VERSION = "2.11.0"
JAVA_MIN = 25
VAULTS_INVENTORY = REPO / "docs" / "oberflaechen" / "tresore.md"
SCAN_FILE_LIMIT = 256 * 1024 * 1024


# --------------------------------------------------------------------------- interop harness

@dataclass
class Harness:
    ok: bool
    reason: str
    tools_present: bool = False
    java: str | None = None
    jar: Path | None = None


def java_major(ctx: common.JobContext, java: str) -> int | None:
    completed = ctx.run([java, "-version"], stage=ctx.current_stage, timeout=120)
    match = re.search(r'version "(\d+)', completed.output)
    return int(match.group(1)) if match else None


def harness(ctx: common.JobContext) -> Harness:
    cached = ctx.state.get("interop_harness")
    if cached is not None:
        return cached
    java_home = os.environ.get("JAVA_HOME")
    java = None
    if java_home:
        candidate = Path(java_home) / "bin" / ("java.exe" if WINDOWS else "java")
        java = str(candidate) if candidate.is_file() else None
    java = java or shutil.which("java")
    mvn = shutil.which("mvn") or shutil.which("mvn.cmd")
    if not java or not mvn:
        result = Harness(False, f"{'java' if not java else 'mvn'} fehlt im Runner")
    else:
        major = java_major(ctx, java)
        if major is None or major < JAVA_MIN:
            result = Harness(False, f"Java {major} gefunden; CryptoFS {CRYPTOFS_VERSION} ist für Java {JAVA_MIN} "
                                    "übersetzt (maven.compiler.release 25 in seinem POM)")
        else:
            build = ctx.work / "interop-build"
            repository = ctx.work / "interop-m2"  # outside the uploaded log folder and outside every cache
            completed = ctx.run([mvn, "-B", "-ntp", "-f", HARNESS_DIR / "pom.xml",
                                 f"-Dmaven.repo.local={repository}", f"-Drdt.build.directory={build}", "package"],
                                stage=ctx.current_stage, env=ctx.base_env(), timeout=30 * 60)
            jar = build / "vault-task-interop.jar"
            if completed.ok and jar.is_file():
                result = Harness(True, f"Harness mit CryptoFS {CRYPTOFS_VERSION} gebaut (Java {major})", True, java, jar)
            else:
                result = Harness(False, f"mvn package: {completed.describe()}\n" + common.excerpt(completed.output, 2000),
                                 True)
    ctx.state["interop_harness"] = result
    return result


def harness_call(ctx: common.JobContext, tool: Harness, stage: str, args: list, timeout: float = 900):
    completed = ctx.run([tool.java, "-jar", tool.jar, *args], stage=stage, env=ctx.base_env(), timeout=timeout)
    data = None
    for line in reversed(completed.output.splitlines()):
        if line.startswith("{"):
            try:
                data = json.loads(line)
                break
            except ValueError:
                continue
    return completed, data


def harness_status(tool: Harness) -> str:
    return FAILED if tool.tools_present else NOT_RUN


# --------------------------------------------------------------------------- fixtures

def fixture_interop(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    tool = harness(fixtures.ctx)
    if not tool.ok:
        return False, tool.reason
    fixtures.provide(name, env={"SE_VAULT_INTEROP_JAVA": tool.java, "SE_VAULT_INTEROP_JAR": tool.jar})
    return True, tool.reason


def fixture_keep(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    target = fixtures.ctx.work / "kept-vaults" / fixtures.stage
    target.mkdir(parents=True, exist_ok=True)
    fixtures.provide(name, env={"SE_VAULT_TASK_KEEP_VAULTS": target})
    return True, f"SE_VAULT_TASK_KEEP_VAULTS={target}"


def fixture_marker(fixtures: common.StageFixtures, name: str) -> tuple[bool, str]:
    if not LINUX:
        return False, "die Klartext-Suche läuft nach D3 nur auf Linux-Runnern"
    root = fixtures.make_dir("klartext")
    token = "RDTMARK" + secrets.token_hex(10)
    env = {"SE_VAULT_TASK_MARKER": token, "SE_VAULT_TASK_ROOT": root, "HOME": root / "home",
           "XDG_DATA_HOME": root / "data", "XDG_CONFIG_HOME": root / "config", "XDG_CACHE_HOME": root / "cache",
           "XDG_STATE_HOME": root / "state", "TMPDIR": root / "tmp"}
    for key, value in env.items():
        if key not in ("SE_VAULT_TASK_MARKER", "SE_VAULT_TASK_ROOT"):
            Path(value).mkdir(parents=True, exist_ok=True)
    (root / "input").mkdir(exist_ok=True)
    fixtures.ctx.state.setdefault("markers", {})[fixtures.stage] = {"root": root, "token": token,
                                                                     "started": time.time()}
    fixtures.provide(name, env=env)
    return True, f"Markierung und isolierte Ordner unter {root}"


# --------------------------------------------------------------------------- checks: interop

def check_interop_selftest(ctx: common.JobContext) -> tuple[str, str]:
    """The harness itself: CryptoFS creates a vault, reads it back byte-exactly (long name, umlauts,
    empty file, file across two chunks, deep folders), its health check finds nothing, and a wrong
    password is refused. A later interop failure is then attributable to se, not to the harness."""
    tool = harness(ctx)
    if not tool.ok:
        return harness_status(tool), tool.reason
    stage = "vault-interop"
    root = ctx.work / "interop-selftest"
    shutil.rmtree(root, ignore_errors=True)
    source, vault = root / "klartext", root / "tresor"
    files = {"a.txt": b"hallo tresor\n", "Ordner/ü-ä.txt": "Umlaute".encode("utf-8"), "leer.txt": b"",
             "lang-" + "x" * 195 + ".bin": os.urandom(70000), "tief/a/b/c.txt": b"tief"}
    for relative, content in files.items():
        target = source / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
    password, wrong = root / "password.txt", root / "wrong.txt"
    password.write_text("Prüf-Passwort-2026", encoding="utf-8")
    wrong.write_text("falsch-falsch-falsch", encoding="utf-8")
    manifest = root / "manifest.json"
    manifest.write_text(json.dumps({"files": {"/" + relative: hashlib.sha256(content).hexdigest()
                                              for relative, content in files.items()}}), encoding="utf-8")
    problems = []
    created, _ = harness_call(ctx, tool, stage, ["create", "--vault", vault, "--password-file", password,
                                                 "--from", source])
    if not created.ok:
        return FAILED, f"create: {created.describe()}\n" + common.excerpt(created.output, 1500)
    read, data = harness_call(ctx, tool, stage, ["read", "--vault", vault, "--password-file", password,
                                                 "--expect", manifest])
    if not read.ok:
        problems.append(f"read --expect: {read.describe()}: {common.excerpt(read.output, 800)}")
    health, health_data = harness_call(ctx, tool, stage, ["health", "--vault", vault, "--password-file", password])
    if not health.ok:
        problems.append(f"health: {health.describe()}: {json.dumps((health_data or {}).get('findings'))[:800]}")
    refused, _ = harness_call(ctx, tool, stage, ["unlock", "--vault", vault, "--password-file", wrong])
    if refused.code != 2:
        problems.append(f"falsches Passwort nicht abgelehnt ({refused.describe()}, erwartet Exitcode 2)")
    version, version_data = harness_call(ctx, tool, stage, ["version"])
    if problems:
        return FAILED, "; ".join(problems)
    return PASSED, (f"CryptoFS {(version_data or {}).get('cryptofs', CRYPTOFS_VERSION)} unter Java "
                    f"{(version_data or {}).get('java', '?')}: anlegen, {len((data or {}).get('files', {}))} Dateien "
                    "byte-genau gelesen, Prüfung ohne Befund, falsches Passwort abgelehnt")


def kept_vaults(ctx: common.JobContext, stage: str) -> list[dict]:
    root = ctx.work / "kept-vaults" / stage
    found = []
    for password in sorted(root.glob("*/*/password.txt")):
        case = password.parent
        vault = case / "vault"
        if not (vault / "vault.cryptomator").is_file():
            continue
        allowed_file = case / "allowed-findings.txt"
        allowed = []
        if allowed_file.is_file():
            allowed = [line.strip() for line in allowed_file.read_text(encoding="utf-8").splitlines()
                       if line.strip() and not line.startswith("#")]
        found.append({"label": f"{case.parent.name}/{case.name}", "vault": vault, "password": password,
                      "allowed": allowed})
    return found


def health_after(stage_name: str):
    def check(ctx: common.JobContext) -> tuple[str, str]:
        vaults = kept_vaults(ctx, stage_name)
        if not vaults:
            return NOT_RUN, (f"keine Tresore aus der Stufe {stage_name} abgelegt (Vertrag: "
                             "SE_VAULT_TASK_KEEP_VAULTS/<test>/<fall>/vault mit password.txt; die Tests der Stufe "
                             "fehlen oder legen nichts ab)")
        tool = harness(ctx)
        if not tool.ok:
            return harness_status(tool), tool.reason
        problems = []
        for item in vaults:
            args = ["health", "--vault", item["vault"], "--password-file", item["password"]]
            for allowed in item["allowed"]:
                args += ["--allow", allowed]
            completed, data = harness_call(ctx, tool, "vault-interop", args)
            if not completed.ok:
                findings = [f"{finding.get('type')} ({finding.get('severity')})"
                            for finding in (data or {}).get("findings", [])
                            if finding.get("severity") in ("WARN", "CRITICAL")]
                problems.append(f"{item['label']}: {completed.describe()}: {', '.join(findings) or common.first_line(completed.output)}")
        if problems:
            return FAILED, f"CryptoFS-Prüfung mit Befunden in {len(problems)} von {len(vaults)} Tresoren:\n" + "\n".join(problems)
        return PASSED, f"CryptoFS {CRYPTOFS_VERSION}: {len(vaults)} Tresore aus {stage_name} ohne unerwartete Befunde"
    return check


# --------------------------------------------------------------------------- checks: plaintext boundary

def fuse_mount_points() -> set[Path]:
    """Mount points of FUSE file systems (reading through them would show decrypted content)."""
    points: set[Path] = set()
    try:
        lines = Path("/proc/self/mountinfo").read_text(encoding="utf-8").splitlines()
    except OSError:
        return points
    for line in lines:
        fields = line.split(" - ", 1)
        if len(fields) == 2 and fields[1].split(" ", 1)[0].startswith("fuse"):
            mount_point = fields[0].split()[4].replace("\\040", " ")
            points.add(Path(mount_point))
    return points


def scan_tree(base: Path, needles: list[bytes], token: str, since: float | None, excluded: list[Path]):
    scanned, hits = 0, []
    if not base.is_dir():
        return scanned, hits
    overlap = max(len(needle) for needle in needles)
    uid = os.getuid() if hasattr(os, "getuid") else None
    mounts = fuse_mount_points()
    for directory, dirnames, filenames in os.walk(base, followlinks=False):
        here = Path(directory)
        if any(here == item or item in here.parents for item in excluded):
            dirnames[:] = []
            continue
        if here in mounts:
            hits.append(f"{here}: FUSE-Laufwerk am Ende der Stufe noch eingehängt (nicht durchsucht)")
            dirnames[:] = []
            continue
        for name in dirnames + filenames:
            if token in name:
                hits.append(f"{here / name}: Markierung im Namen")
        for name in filenames:
            path = here / name
            if path in excluded:
                continue
            try:
                info = path.lstat()
            except OSError:
                continue
            if not path.is_file() or path.is_symlink() or info.st_size > SCAN_FILE_LIMIT:
                continue
            if since is not None and info.st_mtime < since:
                continue
            if uid is not None and info.st_uid != uid:
                continue
            try:
                with open(path, "rb") as handle:
                    scanned += 1
                    tail = b""
                    for block in iter(lambda: handle.read(1 << 20), b""):
                        window = tail + block
                        if any(needle in window for needle in needles):
                            hits.append(f"{path}: Markierung im Inhalt")
                            break
                        tail = window[-overlap:]
            except OSError:
                continue
    return scanned, hits


def marker_scan(stage_name: str):
    def check(ctx: common.JobContext) -> tuple[str, str]:
        info = ctx.state.get("markers", {}).get(stage_name)
        if not info:
            return NOT_RUN, ("kein Klartext-Test mit fixture=marker lief in dieser Stufe (Tests fehlen: "
                             "Abschnitt 'noch nicht umgesetzt')")
        root, token = Path(info["root"]), info["token"]
        needles = [token.encode("ascii"), token.encode("utf-16-le")]
        excluded = [root / "input"]
        allowed_file = root / "allowed-marker-paths.txt"
        if allowed_file.is_file():
            for line in allowed_file.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    path = Path(line.strip())
                    excluded.append(path if path.is_absolute() else root / path)
        scanned, hits = 0, []
        for base, since in ((ctx.work / "fixtures" / stage_name, None), (Path(tempfile.gettempdir()), info["started"]),
                            (Path("/dev/shm"), info["started"])):
            count, found = scan_tree(base, needles, token, since, excluded)
            scanned += count
            hits.extend(found)
        if hits:
            return FAILED, f"Klartext-Markierung außerhalb erlaubter Rückfallkopien ({len(hits)}):\n" + "\n".join(hits[:60])
        return PASSED, (f"{scanned} Dateien unter den Fixture-Ordnern der Stufe, {tempfile.gettempdir()} und "
                        "/dev/shm durchsucht: keine Markierung in Inhalt oder Namen")
    return check


# --------------------------------------------------------------------------- checks: documentation and never-pass

def check_docs_rows_t1_t22(ctx: common.JobContext) -> tuple[str, str]:
    """VLT-M13: every row T1-T22 in tresore.md names a Datei:Zeile reference or an exception with
    a reason, and every reference resolves."""
    if not VAULTS_INVENTORY.is_file():
        return FAILED, "docs/oberflaechen/tresore.md fehlt (VLT-M13; Paket U42)"
    text = VAULTS_INVENTORY.read_text(encoding="utf-8", errors="replace")
    rows: dict[int, tuple[int, str]] = {}
    for number, cells in common.table_rows(text):
        if not cells:
            continue
        match = re.fullmatch(r"\W*T(\d{1,2})\W*", cells[0])
        if match and 1 <= int(match.group(1)) <= 22:
            rows.setdefault(int(match.group(1)), (number, " | ".join(cells)))
    problems = [f"Zeile T{index} fehlt" for index in range(1, 23) if index not in rows]
    for index, (number, row) in sorted(rows.items()):
        exception = re.search(r"Ausnahme\w*\s*[:–-]?\s*(\S+\s+\S+)", row, re.I)
        if not common.CITATION.search(row) and not exception:
            problems.append(f"tresore.md:{number}: T{index} ohne Beleg Datei:Zeile und ohne Ausnahme mit Grund")
    count, unresolved = common.Citations().check_text(text)
    problems.extend(unresolved)
    if problems:
        return FAILED, f"{len(problems)} Befunde (Paket U42):\n" + "\n".join(problems[:60])
    return PASSED, f"T1-T22 mit Beleg oder Ausnahme, {count} Belege aufgelöst"


def check_screen_lock_delivery(ctx: common.JobContext) -> tuple[str, str]:
    return NOT_RUN, ("die Zustellung der Bildschirmsperre durch das Betriebssystem ist im Runner nicht prüfbar "
                     "(S3 K13; Handtest VAULT-SCREENLOCK-VERIFY): geprüft wird nur die Verdrahtung "
                     "(vault_task_windows_wts_window_dispatch, vault_task_linux_logind_sleep_and_locked_hint_lock)")


def check_backends_google_drive(ctx: common.JobContext) -> tuple[str, str]:
    return NOT_RUN, "keine Google-Drive-Zugangsdaten im Runner: Tresor auf Google Drive nicht live geprüft (D3, NR G18)"


def check_android_arm64_compile(ctx: common.JobContext) -> tuple[str, str]:
    return NOT_RUN, "kein plattformübergreifender Bau in der Suite (AGENTS.md); arm64 baut das Release"


# --------------------------------------------------------------------------- stage: vault-backends

SERVER_TESTS = ("vault_task_backends_provider_matrix", "vault_task_backends_share_pair")
PROVIDERS_SCRIPT = NATIVE / "sync-reliability-providers.py"
# What fixtures() of the providers script calls on the runner.
PROVIDER_TOOLS = ("docker", "openssl", "ssh-keyscan", "ssh-keygen", "curl")
# The logs of the provider fixtures that name the step that failed (written into their log folder).
PROVIDER_LOGS = ("fixture-commands.log", "share-main-commands.log", "share-remote-commands.log")


def load_providers_script():
    """native/sync-reliability-providers.py as a module (its file name is no module name)."""
    import importlib.util  # only this stage needs it

    spec = importlib.util.spec_from_file_location("sync_reliability_providers", PROVIDERS_SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"{PROVIDERS_SCRIPT} lässt sich nicht laden")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def error_chain(error: BaseException) -> str:
    """The error and what it was raised while handling (a failed cleanup hides the first error)."""
    parts: list[str] = []
    seen: set[int] = set()
    current: BaseException | None = error
    while current is not None and id(current) not in seen and len(parts) < 4:
        seen.add(id(current))
        parts.append(f"{type(current).__name__}: {current}")
        current = current.__cause__ or current.__context__
    return " <- ".join(parts)


def provider_log_tails(logs: Path) -> str:
    """The end of the provider logs that name the failing step."""
    sections = []
    for name in PROVIDER_LOGS:
        try:
            text = (logs / name).read_text(encoding="utf-8", errors="replace").strip()
        except OSError:
            continue
        if text:
            sections.append(f"Ende von {name}:\n{common.excerpt(text, 800)}")
    return "\n".join(sections)


def server_blocker(ctx: common.JobContext, stage: str) -> str | None:
    """Why the owned servers cannot start on this runner, else None."""
    if not LINUX:
        return ("die Testserver von native/sync-reliability-providers.py laufen als Docker-Container und hier nur "
                "auf Linux-Runnern; dieser Runner ist keiner")
    missing = [tool for tool in PROVIDER_TOOLS if not shutil.which(tool)]
    if missing:
        return (f"im Runner fehlt {', '.join(missing)} (native/sync-reliability-providers.py braucht "
                f"{', '.join(PROVIDER_TOOLS)})")
    info = ctx.run(["docker", "info", "--format", "{{.ServerVersion}}"], stage=stage, timeout=120)
    if not info.ok:
        return f"docker ist nicht benutzbar ({info.describe()}): {common.first_line(info.output) or 'keine Ausgabe'}"
    return None


def profile_env(ctx: common.JobContext) -> dict:
    """The isolated profile of the stage, laid out like that of the sync reliability suite: the confirmed SSH host
    keys, the saved connections and the Share daemon of the owned servers live in it, and the tests run in it too."""
    assert ctx.fixtures is not None
    root = ctx.fixtures.make_dir("backends-profile")
    folders = {"APPDATA": root, "LOCALAPPDATA": root / "local", "XDG_CONFIG_HOME": root / "config",
               "XDG_DATA_HOME": root / "data", "XDG_CACHE_HOME": root / "cache"}
    for folder in folders.values():
        folder.mkdir(parents=True, exist_ok=True)
        folder.chmod(0o700)
    token = re.sub(r"[^A-Za-z0-9]", "", ctx.candidate_sha)[:12] or "local"
    env = {key: str(folder) for key, folder in folders.items()}
    env["RUST_BACKTRACE"] = "1"
    env["SMART_EXPLORER_E2E_TEST_NAMESPACE"] = f"vault-backends-{token}-{secrets.token_hex(3)}"
    return env


def provider_names(runtime: dict) -> list[str]:
    """The provider names of the manifest (no connection data)."""
    try:
        manifest = json.loads(Path(runtime["SE_SYNC_PROVIDER_MANIFEST"]).read_text(encoding="utf-8"))
        return [str(item["name"]) for item in manifest["providers"]]
    except (KeyError, TypeError, OSError, ValueError):
        return []


def stage_vault_backends(ctx: common.JobContext, stage: str) -> None:
    """Stage ``vault-backends`` (VLT-M14, job linux-core): vaults on the remote backends.

    ``vault_task_backends_provider_matrix`` (SFTP, agent, FTP, FTPS, SMB, WebDAV) and
    ``vault_task_backends_share_pair`` (a Direct Share pair) need real servers. The stage starts the owned servers of
    ``native/sync-reliability-providers.py`` with its ``fixtures()`` (Docker containers, the candidate's Share server and
    two ``se`` daemons for the pair; nothing of it is repeated here), runs the stage with their connections and ends the
    servers afterwards, also after an error (the cleanup is registered with the stage fixtures).

    Environment of the tests (extra_env of the stage): ``SE_SYNC_PROVIDER_MANIFEST`` (providers.json: sftp, sftp-other,
    sftp-key, agent, ftp, ftps, smb, webdav, direct, room), ``SE_SYNC_FIXTURE_CA_DER``, ``SE_SHARE_RELAY_URL`` and the
    isolated profile (APPDATA, LOCALAPPDATA, XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME,
    SMART_EXPLORER_E2E_TEST_NAMESPACE) in which the SSH host keys were confirmed. The entries carry ``ignored`` and
    ``threads=1`` and no ``fixture=``: the pair comes from the providers script, not from the fixture ``share``.

    A server that does not start makes both entries ``not_run`` with the exact reason (``fixtures()`` starts all of
    them or none); a ``se`` that does not build fails them like any other environment that cannot be made."""
    assert ctx.fixtures is not None
    notes = ctx.state.setdefault("stage_notes", {})
    wanted = [entry for entry in ctx.entries(stage) if entry.name in SERVER_TESTS and not ctx.recorded(entry.name)]
    present = []
    if wanted and ctx.cargo.build_lib():
        present = [entry for entry in wanted if len(ctx.cargo.by_leaf.get(entry.name, [])) == 1]
    if not present:
        if wanted:
            notes[stage] = ("Testserver nicht gestartet: die Server-Tests fehlen im Testprogramm oder dessen Bau "
                            "scheiterte (das Ergebnis steht bei den Tests)")
        ctx.run_default(stage)
        return

    def give_up(status: str, reason: str) -> None:
        for entry in present:
            if status == FAILED:
                ctx.record(entry.name, stage, FAILED, reason)
            else:
                ctx.not_run_all(stage, reason, entries=[entry])
        notes[stage] = "Testserver nicht gestartet: " + (common.first_line(reason) or reason)
        ctx.run_default(stage)

    blocker = server_blocker(ctx, stage)
    if blocker:
        give_up(NOT_RUN, blocker)
        return
    cli = ctx.cargo.bin_path("se")
    if cli is None:
        give_up(FAILED, "Umgebung nicht herstellbar: " + ctx.cargo.bin_errors.get("se", "Bau von se gescheitert"))
        return
    share = ctx.cargo.share_server()
    if share is None:
        give_up(NOT_RUN, f"Share-Server nicht verfügbar: {ctx.cargo.server_error}")
        return
    try:
        script = load_providers_script()
    except Exception as error:  # noqa: BLE001 - a broken providers script fails the entries with its error
        give_up(FAILED, f"native/sync-reliability-providers.py nicht ladbar: {error_chain(error)}")
        return
    profile = profile_env(ctx)
    base = {**ctx.base_env(), **profile}
    logs = ctx.logs / "vault-backends-servers"
    started = time.monotonic()
    manager = script.fixtures(logs, base, cli, share)
    try:
        runtime = manager.__enter__()
    except Exception as error:  # noqa: BLE001 - the exact reason is the report of the entries
        tails = provider_log_tails(logs)
        give_up(NOT_RUN, "Testserver aus native/sync-reliability-providers.py nicht gestartet: "
                + error_chain(error) + (f"\n{tails}" if tails else ""))
        return

    closed: list[bool] = []
    problems: list[str] = []

    def shutdown() -> None:
        if closed:
            return
        closed.append(True)
        try:
            manager.__exit__(None, None, None)
        except Exception as error:  # noqa: BLE001 - reported in the stage note, the results stand
            problems.append(error_chain(error))
            ctx.log_line(stage, f"Testserver beenden gescheitert: {error_chain(error)}")

    ctx.fixtures.add_cleanup(shutdown)
    names = provider_names(runtime)
    notes[stage] = (f"Testserver aus native/sync-reliability-providers.py in {time.monotonic() - started:.0f} s "
                    f"gestartet ({', '.join(names) or 'Namen unbekannt'}); Share-Paar über {share.name}; "
                    "nach den Tests beendet")
    ctx.log_line(stage, f"Testserver bereit: {', '.join(names)}; Profil {profile['XDG_DATA_HOME']}")
    added = {key: str(value) for key, value in runtime.items() if base.get(key) != str(value)}
    try:
        ctx.run_default(stage, extra_env={**profile, **added})
    finally:
        shutdown()
    if problems:
        notes[stage] += "; Beenden der Testserver gescheitert: " + "; ".join(problems)


FIXTURES = {"interop": fixture_interop, "keep": fixture_keep, "marker": fixture_marker}
STAGES = {"vault-backends": stage_vault_backends}
CHECKS = {
    "rdt_suite_interop_harness_selftest": check_interop_selftest,
    "vault_task_interop_health_after_sync": health_after("vault-sync"),
    "vault_task_interop_health_after_compare": health_after("vault-compare"),
    "vault_task_klartext_marker_scan": marker_scan("vault-klartext"),
    "vault_task_klartext_marker_scan_drive": marker_scan("vault-klartext-drive"),
    "vault_task_docs_rows_t1_t22": check_docs_rows_t1_t22,
    "vault_task_screen_lock_delivery_by_os": check_screen_lock_delivery,
    "vault_task_backends_google_drive": check_backends_google_drive,
    "vault_task_android_arm64_compile": check_android_arm64_compile,
}
