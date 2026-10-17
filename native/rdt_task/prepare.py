"""Job ``prepare`` of the batch suite: stages lock -> audit -> licenses -> static (plan D3, L1, L2).

lock      ``rdt_suite_lock_committed_current``: resolves ``native/Cargo.lock`` without
          ``--locked`` (``cargo fetch`` writes the minimal update the manifests need)
          and compares it with the committed file. In the first run the batch's new
          crates are missing from the committed lock, so the check fails as planned
          (L1) until the main agent commits the lock from the artifact rdt-prepare.
audit     ``rdt_suite_cargo_audit``: ``cargo audit`` of the resolved lock with the two
          ignores of D3.
licenses  ``rdt_task/licenses.py`` (the finished script native/licenses_task_checks.py).
static    every ``art=check`` entry of prepare:static from the CHECKS registers of all
          rdt_task modules, among them ``rdt_suite_workflow_static`` and
          ``rdt_suite_docs_stale_search`` defined here.

``export_artifact`` copies the resolved lock and the (generated) license file into the
folder the workflow uploads as artifact rdt-prepare, together with ``rdt-prepare.json``
(SHA-256 of both files, candidate SHA); every later job places both files before it
builds and checks them with ``rdt_suite_prepare_files_placed``.
"""
from __future__ import annotations

import datetime
import json
import re
import shutil
import subprocess
import sys

from . import common
from .common import FAILED, LICENSE_FILE, LOCK_FILE, NATIVE, NOT_RUN, PASSED, PREPARE_MANIFEST, REPO

AUDIT_IGNORES = ("RUSTSEC-2026-0194", "RUSTSEC-2026-0195")
WORKFLOW = REPO / ".github" / "workflows" / "rechte-desktop-tresore-task.yml"
ENTRYPOINT = "native/test-rechte-desktop-tresore-task.py"
BATCH_BASE = "da2646d2"  # code state at the start of the plan (plan.md, head)

# job -> (runs-on, minimum timeout-minutes from D3, needs, places the artifact rdt-prepare)
JOB_EXPECTATIONS = {
    "prepare": ("ubuntu-24.04", 60, (), False),
    "linux-core": ("ubuntu-24.04", 360, ("prepare",), True),
    "linux-root": ("ubuntu-24.04", 240, ("prepare",), True),
    "linux-2204": ("ubuntu-22.04", 90, ("prepare",), True),
    "linux-desktop": ("ubuntu-24.04", 300, ("prepare",), True),
    "linux-wayland": ("ubuntu-24.04", 240, ("prepare",), True),
    "linux-gnome": ("ubuntu-24.04", 180, ("prepare",), True),
    "windows-core": ("windows-2025", 360, ("prepare",), True),
    "windows-screen": ("windows-2025", 240, ("prepare",), True),
    "windows-desktop": ("windows-2025", 240, ("prepare",), True),
    "desktop-bins": ("ubuntu-24.04", 90, ("prepare",), True),
    "android-build": ("ubuntu-24.04", 180, ("prepare",), True),
    "android-device": ("ubuntu-24.04", 240, ("android-build", "desktop-bins"), False),
    "android-device-api30": ("ubuntu-24.04", 150, ("android-build", "desktop-bins"), False),
    "report": ("ubuntu-24.04", 30, (), False),
}

STALE_PATTERNS = (
    (re.compile(r"\bWIP\b"), "WIP"),
    (re.compile(r"needs release", re.I), "needs release"),
    (re.compile(r"prefetch next", re.I), "prefetch next"),
    (re.compile(r"Current release"), "Current release"),
    (re.compile(r"nur unter Windows", re.I), "nur unter Windows (DRV-M9)"),
)
STALE_EXCLUDED = ("docs/plaene/", "docs/lesungen/", "docs/refs/", "docs/cfapi_review/", "docs/sync_research/",
                  "docs/vfs_research/", "docs/SESSION_STATE.md")
STALE_INCLUDED_PLAN_DOCS = ("docs/superpowers/plans/2026-09-25-android-apk/api.md",)


# --------------------------------------------------------------------------- lock

def lock_packages(text: str) -> set[tuple[str, str]]:
    packages = set()
    for block in text.split("[[package]]")[1:]:
        name = re.search(r'^name = "([^"]+)"', block, re.M)
        version = re.search(r'^version = "([^"]+)"', block, re.M)
        if name and version:
            packages.add((name.group(1), version.group(1)))
    return packages


def check_lock_committed_current(ctx: common.JobContext) -> tuple[str, str]:
    committed = subprocess.run(["git", "show", f"HEAD:{LOCK_FILE}"], cwd=REPO, capture_output=True, check=False)
    resolved = ctx.run(["cargo", "fetch", "--manifest-path", NATIVE / "Cargo.toml"], stage="lock",
                       cwd=REPO, env=ctx.base_env(), timeout=40 * 60)
    lock = REPO / LOCK_FILE
    if not resolved.ok:
        return FAILED, f"cargo fetch (Auflösung ohne --locked): {resolved.describe()}\n" + common.excerpt(resolved.output)
    if not lock.is_file():
        return FAILED, f"{LOCK_FILE} fehlt nach der Auflösung"
    if committed.returncode != 0:
        return FAILED, f"{LOCK_FILE} fehlt im Commit; die Auflösung erzeugte ihn (Artefakt rdt-prepare)"
    after = lock.read_bytes()
    if after == committed.stdout:
        return PASSED, f"{LOCK_FILE} aus dem Commit ist aktuell ({len(lock_packages(after.decode()))} Pakete)"
    before_packages = lock_packages(committed.stdout.decode("utf-8", "replace"))
    after_packages = lock_packages(after.decode("utf-8", "replace"))
    added = sorted(after_packages - before_packages)
    removed = sorted(before_packages - after_packages)
    detail = "; ".join(filter(None, [
        ("neu: " + ", ".join(f"{name} {version}" for name, version in added[:60])) if added else "",
        ("entfernt: " + ", ".join(f"{name} {version}" for name, version in removed[:60])) if removed else "",
    ])) or "nur Prüfsummen oder Reihenfolge geändert"
    return FAILED, (f"Die Auflösung ändert {LOCK_FILE} ({detail}). Erwarteter Befund im ersten Lauf (L1): "
                    "der Hauptagent committet die Datei aus dem Artefakt rdt-prepare.")


# --------------------------------------------------------------------------- audit

def check_cargo_audit(ctx: common.JobContext) -> tuple[str, str]:
    env = ctx.base_env()
    if not ctx.run(["cargo", "audit", "--version"], stage="audit", env=env, timeout=120).ok:
        installed = ctx.run(["cargo", "install", "cargo-audit", "--locked"], stage="audit", env=env,
                            timeout=40 * 60)
        if not installed.ok:
            return FAILED, f"cargo-audit nicht installierbar: {installed.describe()}\n" + common.excerpt(installed.output)
    command = ["cargo", "audit", "--file", REPO / LOCK_FILE]
    for advisory in AUDIT_IGNORES:
        command += ["--ignore", advisory]
    audited = ctx.run(command, stage="audit", cwd=REPO, env=env, timeout=20 * 60)
    if audited.ok:
        return PASSED, f"cargo audit ohne Befund (ignoriert nach D3: {', '.join(AUDIT_IGNORES)})"
    return FAILED, f"cargo audit: {audited.describe()}\n" + common.excerpt(audited.output)


# --------------------------------------------------------------------------- workflow static

def load_yaml(ctx: common.JobContext, path):
    try:
        import yaml  # noqa: PLC0415 - only this check needs PyYAML
    except ImportError:
        installed = ctx.run([sys.executable, "-m", "pip", "install", "--quiet", "pyyaml"], stage="static", timeout=600)
        if not installed.ok:
            raise common.StageError("PyYAML fehlt und ist nicht installierbar")
        import yaml  # noqa: PLC0415
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def step_text(step: dict) -> str:
    parts = [str(step.get("run") or "")]
    with_block = step.get("with") or {}
    parts.append(str(with_block.get("script") or ""))
    return "\n".join(parts)


def check_workflow_static(ctx: common.JobContext) -> tuple[str, str]:
    """D1 U39: every job has timeout-minutes >= 30 (and at least the D3 value), the workflow has
    exactly one input candidate_sha, and every job binds its checkout to that SHA."""
    if not WORKFLOW.is_file():
        return FAILED, f"{WORKFLOW.relative_to(REPO)} fehlt"
    document = load_yaml(ctx, WORKFLOW)
    problems: list[str] = []
    triggers = document.get("on", document.get(True))  # YAML 1.1 reads the key `on` as True
    if not isinstance(triggers, dict) or set(triggers) != {"workflow_dispatch"}:
        problems.append(f"Auslöser sind {sorted(triggers) if isinstance(triggers, dict) else triggers}, "
                        "erwartet nur workflow_dispatch")
    else:
        inputs = (triggers["workflow_dispatch"] or {}).get("inputs") or {}
        if set(inputs) != {"candidate_sha"}:
            problems.append(f"Eingänge {sorted(inputs)}, erwartet genau candidate_sha")
        else:
            definition = inputs["candidate_sha"] or {}
            if definition.get("required") is not True or definition.get("type") != "string":
                problems.append("candidate_sha muss required: true und type: string sein")
    if document.get("permissions") != {"contents": "read"}:
        problems.append(f"permissions = {document.get('permissions')}, erwartet contents: read")
    concurrency = document.get("concurrency") or {}
    if concurrency.get("group") != "rechte-desktop-tresore-task-${{ github.repository }}" or \
            concurrency.get("cancel-in-progress") is not False:
        problems.append("concurrency muss die Gruppe rechte-desktop-tresore-task-${{ github.repository }} "
                        "mit cancel-in-progress: false haben")
    jobs = document.get("jobs") or {}
    if set(jobs) != set(JOB_EXPECTATIONS):
        problems.append(f"Jobs {sorted(jobs)} weichen von {sorted(JOB_EXPECTATIONS)} ab")
    for name, (runner, minimum, needs, places) in JOB_EXPECTATIONS.items():
        job = jobs.get(name)
        if not isinstance(job, dict):
            continue
        prefix = f"Job {name}: "
        if job.get("runs-on") != runner:
            problems.append(prefix + f"runs-on {job.get('runs-on')}, erwartet {runner}")
        timeout = job.get("timeout-minutes")
        if not isinstance(timeout, int) or timeout < max(30, minimum):
            problems.append(prefix + f"timeout-minutes {timeout}, erwartet mindestens {max(30, minimum)}")
        declared = job.get("needs") or []
        declared = [declared] if isinstance(declared, str) else list(declared)
        for required in needs:
            if required not in declared:
                problems.append(prefix + f"needs ohne {required}")
        env = dict(job.get("env") or {})
        steps = job.get("steps") or []
        if not steps or not str(steps[0].get("uses", "")).startswith("actions/checkout@"):
            problems.append(prefix + "der erste Schritt ist nicht actions/checkout")
        else:
            checkout = steps[0].get("with") or {}
            if checkout.get("ref") != "${{ github.sha }}" or checkout.get("persist-credentials") is not False:
                problems.append(prefix + "Checkout ohne ref: ${{ github.sha }} oder persist-credentials: false")
        binding = steps[1] if len(steps) > 1 else {}
        binding_env = dict(env, **(binding.get("env") or {}))
        text = str(binding.get("run") or "")
        if not ("CANDIDATE_SHA" in text and "{40}" in text and "GITHUB_SHA" in text and "git rev-parse HEAD" in text
                and binding_env.get("CANDIDATE_SHA") == "${{ inputs.candidate_sha }}"):
            problems.append(prefix + "zweiter Schritt bindet den Checkout nicht an candidate_sha "
                                     "(SHA-Format, CANDIDATE_SHA = GITHUB_SHA = HEAD)")
        entry_steps = [index for index, step in enumerate(steps) if ENTRYPOINT in step_text(step)]
        if not entry_steps:
            problems.append(prefix + f"ruft {ENTRYPOINT} nicht auf")
        for index in entry_steps:
            command = step_text(steps[index])
            if f"--job {name}" not in command or f"rdt-{name}" not in command or "--log-root" not in command:
                problems.append(prefix + f"Schritt {index + 1} ruft den Einstieg nicht mit --job {name} und "
                                         f"--log-root .../rdt-{name} auf")
            step_timeout = steps[index].get("timeout-minutes")
            if step_timeout is not None and (not isinstance(step_timeout, int) or step_timeout < 30):
                problems.append(prefix + f"Schritt {index + 1} hat timeout-minutes {step_timeout} < 30")
        if places:
            downloads = [index for index, step in enumerate(steps)
                         if str(step.get("uses", "")).startswith("actions/download-artifact@")
                         and (step.get("with") or {}).get("name") == "rdt-prepare"]
            if not downloads or not entry_steps or downloads[0] > entry_steps[0]:
                problems.append(prefix + "legt das Artefakt rdt-prepare nicht vor dem Einstieg in den Checkout")
        uploads = [step for step in steps if str(step.get("uses", "")).startswith("actions/upload-artifact@")
                   and f"rdt-{name}" in str((step.get("with") or {}).get("path", ""))
                   and "always()" in str(step.get("if", ""))]
        if not uploads or not any((step.get("with") or {}).get("retention-days") == 14 for step in uploads):
            problems.append(prefix + "kein Belegschritt mit if: always() und retention-days: 14")
    if problems:
        return FAILED, f"{len(problems)} Befunde:\n" + "\n".join(problems)
    return PASSED, (f"{len(jobs)} Jobs mit Zeitlimit >= 30 min (D3-Werte), ein Eingang candidate_sha, "
                    "Bindungsschritt und Belege in jedem Job")


# --------------------------------------------------------------------------- stale search

def in_stale_scope(path: str) -> bool:
    if path in ("README.md", "native/README.md", "AGENTS.md", "DISCLAIMER.txt"):
        return True
    if path in STALE_INCLUDED_PLAN_DOCS:
        return True
    if not path.startswith("docs/") or not path.endswith(".md"):
        return False
    if path.startswith("docs/superpowers/") or "_research/" in path:
        return False
    return not path.startswith(STALE_EXCLUDED)


def check_docs_stale_search(ctx: common.JobContext) -> tuple[str, str]:
    """AGENTS.md stale search over the lines the batch added to first-party documentation."""
    base = subprocess.run(["git", "rev-parse", "--verify", f"{BATCH_BASE}^{{commit}}"], cwd=REPO,
                          capture_output=True, text=True, check=False)
    if base.returncode != 0:
        return NOT_RUN, (f"Basis-Commit {BATCH_BASE} fehlt im Checkout (der Job prepare braucht fetch-depth: 0); "
                         "die Veraltetsuche lief nicht")
    diff = subprocess.run(["git", "diff", "-U0", "--no-color", "--no-ext-diff", f"{base.stdout.strip()}..HEAD",
                           "--", "README.md", "native/README.md", "AGENTS.md", "DISCLAIMER.txt", "docs"],
                          cwd=REPO, capture_output=True, text=True, encoding="utf-8", errors="replace", check=False)
    hits, files, current, line_number = [], set(), None, 0
    for line in diff.stdout.splitlines():
        if line.startswith("+++ "):
            current = line[6:] if line.startswith("+++ b/") else None
            continue
        if line.startswith("@@"):
            match = re.search(r"\+(\d+)", line)
            line_number = int(match.group(1)) if match else 0
            continue
        if current is None or not in_stale_scope(current):
            continue
        if line.startswith("+") and not line.startswith("+++"):
            files.add(current)
            for pattern, label in STALE_PATTERNS:
                if pattern.search(line[1:]):
                    hits.append(f"{current}:{line_number}: {label}: {line[1:].strip()[:160]}")
            line_number += 1
    if hits:
        return FAILED, f"{len(hits)} veraltete Stellen in neu hinzugefügten Zeilen:\n" + "\n".join(hits[:60])
    return PASSED, (f"{len(files)} geänderte Doku-Dateien seit {BATCH_BASE}, keine Treffer für "
                    + ", ".join(label for _, label in STALE_PATTERNS)
                    + "; alte Versionsnummern prüft erst das Release (Version 0.5.183 entsteht dort)")


# --------------------------------------------------------------------------- artifact rdt-prepare

def export_artifact(ctx: common.JobContext, out_dir) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {}
    for relative in (LOCK_FILE, LICENSE_FILE):
        source = REPO / relative
        if source.is_file():
            target = out_dir / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            files[relative] = common.sha256_file(target)
    generated = False
    report = ctx.log_root / "licenses" / "licenses-report.json"
    if report.is_file():
        try:
            generated = bool(json.loads(report.read_text(encoding="utf-8")).get("generated"))
        except ValueError:
            generated = False
    manifest = {"candidate_sha": ctx.candidate_sha, "files": files, "license_file_generated_in_run": generated,
                "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")}
    text = json.dumps(manifest, indent=2) + "\n"
    (out_dir / PREPARE_MANIFEST).write_text(text, encoding="utf-8")
    (ctx.log_root / PREPARE_MANIFEST).write_text(text, encoding="utf-8")
    ctx.log_line("static", f"artifact rdt-prepare: {json.dumps(manifest)}")


STAGES: dict = {}
CHECKS = {
    "rdt_suite_lock_committed_current": check_lock_committed_current,
    "rdt_suite_cargo_audit": check_cargo_audit,
    "rdt_suite_workflow_static": check_workflow_static,
    "rdt_suite_docs_stale_search": check_docs_stale_search,
}
