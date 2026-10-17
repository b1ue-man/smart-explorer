"""INT-LIC in the batch suite (plan C4, D3; handoff from U33).

Stage ``prepare:licenses`` calls the finished script ``native/licenses_task_checks.py`` exactly
as the handoff names it::

    python native/licenses_task_checks.py --log-root "$RUNNER_TEMP/rdt-prepare/licenses" --place-generated

and copies every result of its ``licenses-report.json`` into the job report (the five names
``licenses_task_generated_file_current``, ``licenses_task_every_shipped_crate_has_text``,
``licenses_task_new_batch_crates_listed``, ``licenses_task_winit_notice_included`` and
``licenses_task_release_wrapper_checks_licenses``). ``--place-generated`` puts the generated
``third-party/rust-crates/THIRD-PARTY-LICENSES.txt`` into the checkout, so the artifact rdt-prepare
carries it (L2). In the first run ``licenses_task_generated_file_current`` fails as planned until
the main agent commits that file.

The script needs ``pwsh``; when the runner lacks it, the stage installs it with
``sudo apt-get install -y powershell`` from the runner's Microsoft package source (C4).

``licenses_task_installer_installs_file`` (prepare:static) checks ``native/installer.nsi``
statically: the installer writes the file to ``$INSTDIR\\licenses\\Rust-Crates`` and the
uninstaller removes it and that folder before ``$INSTDIR\\licenses`` (C4, C5; U36).
"""
from __future__ import annotations

import json
import re
import shutil
import sys

from . import common
from .common import FAILED, LINUX, NATIVE, PASSED, REPO, STATUSES

SCRIPT = NATIVE / "licenses_task_checks.py"
SCRIPT_TIMEOUT_S = 55 * 60


def ensure_pwsh(ctx: common.JobContext, stage: str) -> tuple[bool, str]:
    if shutil.which("pwsh"):
        return True, "pwsh vorhanden"
    if not LINUX or not ctx.sudo_ok():
        return False, "pwsh fehlt und ist ohne sudo -n nicht installierbar"
    installed = ctx.run(["sudo", "-n", "apt-get", "install", "-y", "powershell"], stage=stage, timeout=900)
    if not installed.ok:
        ctx.run(["sudo", "-n", "apt-get", "update"], stage=stage, timeout=900)
        installed = ctx.run(["sudo", "-n", "apt-get", "install", "-y", "powershell"], stage=stage, timeout=900)
    if shutil.which("pwsh"):
        return True, "pwsh mit sudo apt-get install -y powershell installiert"
    return False, f"pwsh nicht installierbar ({installed.describe()})"


def stage_licenses(ctx: common.JobContext, stage: str) -> None:
    pwsh_ready, pwsh_reason = ensure_pwsh(ctx, stage)
    ctx.log_line(stage, f"pwsh: {pwsh_reason}")
    log_root = ctx.log_root / "licenses"
    completed = ctx.run([sys.executable, SCRIPT, "--log-root", log_root, "--place-generated"], stage=stage,
                        cwd=REPO, env=ctx.base_env(), timeout=SCRIPT_TIMEOUT_S)
    report_path = log_root / "licenses-report.json"
    tests: list[dict] = []
    if report_path.is_file():
        try:
            tests = list(json.loads(report_path.read_text(encoding="utf-8")).get("tests", []))
        except ValueError:
            tests = []
    reported = set()
    for test in tests:
        name = str(test.get("name") or "")
        if not name or name in reported:
            continue
        reported.add(name)
        status = test.get("status") if test.get("status") in STATUSES else FAILED
        reason = str(test.get("reason") or "")
        if status == FAILED and not pwsh_ready:
            reason += f" (pwsh: {pwsh_reason})"
        # Every reported name is recorded; one without an entry is flagged by the inventory.
        ctx.record(name, stage, status, reason, float(test.get("duration_s") or 0.0))
    for entry in ctx.entries(stage):
        if entry.kind == "skript" and entry.name not in reported:
            ctx.record(entry.name, stage, FAILED,
                       f"{SCRIPT.name} meldete diesen Namen nicht ({completed.describe()}; Bericht "
                       f"{'vorhanden' if report_path.is_file() else 'fehlt'}):\n" + common.excerpt(completed.output, 2000))
    note = (f"{SCRIPT.name}: {completed.describe()}; erzeugte Datei "
            f"{'in den Checkout gelegt' if (log_root / 'THIRD-PARTY-LICENSES.txt').is_file() else 'fehlt'}")
    if any(test.get("name") == "licenses_task_generated_file_current" and test.get("status") == FAILED
           for test in tests):
        note += ("; licenses_task_generated_file_current scheitert planmäßig, bis der Hauptagent "
                 "third-party/rust-crates/THIRD-PARTY-LICENSES.txt aus dem Artefakt rdt-prepare eincheckt (L2)")
    ctx.state.setdefault("stage_notes", {})[stage] = note
    ctx.run_default(stage)


def check_installer_installs_file(ctx: common.JobContext) -> tuple[str, str]:
    if not common.INSTALLER.is_file():
        return FAILED, "native/installer.nsi fehlt"
    text = common.INSTALLER.read_text(encoding="utf-8", errors="replace")
    lines = [(number, common.nsis_normal(line.strip())) for number, line in enumerate(text.splitlines(), 1)]
    folder = r"$instdir\licenses\rust-crates"
    problems = []
    outpaths = [index for index, (_, line) in enumerate(lines)
                if line.startswith("setoutpath") and line.rstrip("\\").endswith(folder)]
    installs = [index for index in outpaths
                if any(line.startswith("file") and "third-party\\rust-crates\\third-party-licenses.txt" in line
                       for _, line in lines[index + 1:index + 8])]
    if not installs:
        problems.append('kein SetOutPath "$INSTDIR\\licenses\\Rust-Crates" mit File '
                        '"...\\third-party\\rust-crates\\THIRD-PARTY-LICENSES.txt" danach')
    uninstall = [(number, common.nsis_normal(line)) for number, line in common.nsis_uninstall_lines(text)]
    delete = [number for number, line in uninstall
              if line.startswith("delete") and line.endswith(folder + "\\third-party-licenses.txt")]
    rmdir_crates = [number for number, line in uninstall
                    if line.startswith("rmdir") and line.rstrip("\\").endswith(folder)]
    rmdir_licenses = [number for number, line in uninstall
                      if line.startswith("rmdir") and re.search(r"\$instdir\\licenses\\?$", line)]
    if not delete:
        problems.append('Deinstallation ohne Delete "$INSTDIR\\licenses\\Rust-Crates\\THIRD-PARTY-LICENSES.txt"')
    if not rmdir_crates:
        problems.append('Deinstallation ohne RMDir "$INSTDIR\\licenses\\Rust-Crates"')
    elif rmdir_licenses and min(rmdir_licenses) < min(rmdir_crates):
        problems.append('RMDir "$INSTDIR\\licenses" steht vor RMDir "$INSTDIR\\licenses\\Rust-Crates"')
    if problems:
        return FAILED, "; ".join(problems) + " (C4, C5; Paket U36)"
    line_of = lines[installs[0]][0]
    return PASSED, (f"installer.nsi:{line_of} legt THIRD-PARTY-LICENSES.txt unter licenses\\Rust-Crates ab; "
                    f"Deinstallation entfernt Datei (Z. {delete[0]}) und Ordner (Z. {rmdir_crates[0]})")


STAGES = {"licenses": stage_licenses}
CHECKS = {"licenses_task_installer_installs_file": check_installer_installs_file}
