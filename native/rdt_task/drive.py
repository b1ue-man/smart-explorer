"""S2 (Laufwerke) in the batch suite: stages and suite checks (plan D3; strang-laufwerke.md D, DRV-M10).

Stages (library tests by exact full path; options and fixtures come from expected_signals.txt):

linux-core:drive-cli, drive-devices     DRV-M6 pair tests with the candidate share server (fixture ``share``)
windows-core:drive-cli                  DRV-M3 and DRV-M6 on Windows
windows-core:drive-dokany-regression    the five existing Windows harnesses by module path (art=modul), every
                                        ``drive_task_windows_*`` and the three ``#[ignore]`` tests with real Dokany
                                        (``--include-ignored --exact``, handoff U02). Without an installed Dokany
                                        the ignored tests are ``not_run`` (fixture ``dokany``); the stage names
                                        the Dokany state
linux-root:drive-fuse                   DRV-M4 with fuse3 (``fuse``), an OpenSSH container (``sftp``) and the
                                        kernel request timeout (``fuse-timeout``, ``not_run`` before Linux 6.14);
                                        the stage names kernel and fusermount3
linux-root:drive-vault, windows-core:drive-vault   DRV-M7
linux-desktop:drive-desktop             DRV-M5 with a temporary HOME (``home``)
android-build:drive-android             DRV-M8 Robolectric results (JUnit reports of the Android build)
android-device:drive-android-device     DRV-M8 on the emulator; SFTP container reachable as 10.0.2.2 (``sftp``)

Suite checks (prepare:static): ``drive_task_docs_surfaces_complete`` (DRV-M9, C6) and
``drive_task_install_linux_dry_run_with_fuse`` (DRV-M9, C5).
"""
from __future__ import annotations

import platform
import re
import shutil

from . import common
from .common import FAILED, LINUX, NOT_RUN, PASSED, REPO, WINDOWS

DRIVES_INVENTORY = REPO / "docs" / "oberflaechen" / "laufwerke.md"
SETTINGS_INVENTORY = REPO / "docs" / "oberflaechen" / "einstellungen.md"
INVENTORY_INDEX = REPO / "docs" / "oberflaechen" / "INDEX.md"
INSTALL_LINUX = REPO / "install-linux.sh"
# Sections of laufwerke.md (C6): number, a word every matching heading contains.
SECTIONS = (("1", "Oberflächen"), ("2", "einhängen"), ("3", "Optionen"), ("4", "Einhängeort"),
            ("5", "Gerätebaum"), ("6", "Tresor"), ("7", "Dokumentanbieter"), ("8", "Laufzeit"),
            ("9", "Ausnahmen"), ("10", "Offene Punkte"))
ROW_EXEMPT_HEADINGS = ("oberflächen", "ausnahmen", "offene punkte")


def stage_drive_fuse(ctx: common.JobContext, stage: str) -> None:
    fusermount = shutil.which("fusermount3") if LINUX else None
    ctx.state.setdefault("stage_notes", {})[stage] = (
        f"Kernel {platform.release()}; fusermount3 {'= ' + fusermount if fusermount else 'fehlt'}")
    ctx.run_default(stage)


def stage_drive_dokany_regression(ctx: common.JobContext, stage: str) -> None:
    ctx.run_default(stage)
    # After the run: the fixture ``dokany`` installs the driver from the pinned MSI during it.
    installed = WINDOWS and common.dokany_installed()
    ctx.state.setdefault("stage_notes", {})[stage] = (
        "Dokany-Treiber installiert" if installed else
        "Dokany nicht installiert: #[ignore]-Tests mit echtem Dokany laufen nicht")


# --------------------------------------------------------------------------- static checks

def check_docs_surfaces_complete(ctx: common.JobContext) -> tuple[str, str]:
    """DRV-M9/C6: laufwerke.md has the ten sections, every table row outside the sections
    'Oberflächen', 'Ausnahmen' and 'Offene Punkte' names a Datei:Zeile reference or an exception,
    every reference resolves; einstellungen.md and INDEX.md carry the batch's lines."""
    if not DRIVES_INVENTORY.is_file():
        return FAILED, "docs/oberflaechen/laufwerke.md fehlt (C6; angelegt von U06, vervollständigt von U42)"
    text = DRIVES_INVENTORY.read_text(encoding="utf-8", errors="replace")
    lines = text.splitlines()
    problems = []
    headings = [line.lstrip("#").strip() for line in lines if line.startswith("#")]
    for number, word in SECTIONS:
        if not any(word.lower() in heading.lower() for heading in headings):
            problems.append(f"Abschnitt {number} ('{word}') fehlt")
    heading, rows = "", 0
    for index, line in enumerate(lines):
        if line.startswith("#"):
            heading = line.lstrip("#").strip().lower()
            continue
        stripped = line.strip()
        if not stripped.startswith("|") or re.fullmatch(r"\|[\s:|-]+\|?", stripped):
            continue
        following = lines[index + 1].strip() if index + 1 < len(lines) else ""
        if re.fullmatch(r"\|[\s:|-]+\|?", following):
            continue  # header row of a table
        if any(word in heading for word in ROW_EXEMPT_HEADINGS):
            continue
        rows += 1
        if not common.CITATION.search(stripped) and not re.search(r"ausnahme|ausgenommen", stripped, re.I):
            problems.append(f"laufwerke.md:{index + 1}: Zeile ohne Beleg Datei:Zeile und ohne Ausnahme mit Grund")
    count, unresolved = common.Citations().check_text(text)
    problems.extend(unresolved)
    settings = SETTINGS_INVENTORY.read_text(encoding="utf-8", errors="replace") if SETTINGS_INVENTORY.is_file() else ""
    if not re.search(r"Laufwerke:\s*Windows und Linux", settings):
        problems.append("einstellungen.md ohne 'Laufwerke: Windows und Linux; Android: Dokumentanbieter' (C6)")
    index_text = INVENTORY_INDEX.read_text(encoding="utf-8", errors="replace") if INVENTORY_INDEX.is_file() else ""
    if "laufwerke.md" not in index_text:
        problems.append("INDEX.md verweist nicht auf laufwerke.md (C6)")
    if problems:
        return FAILED, f"{len(problems)} Befunde (Paket U42):\n" + "\n".join(problems[:60])
    return PASSED, f"10 Abschnitte, {rows} Tabellenzeilen mit Beleg oder Ausnahme, {count} Belege aufgelöst"


def check_install_linux_dry_run_with_fuse(ctx: common.JobContext) -> tuple[str, str]:
    """DRV-M9/C5: install-linux.sh knows --with-fuse (usage line) and a dry run with --with-fuse
    exits 0, names fuse3 and writes nothing into the temporary HOME."""
    if not INSTALL_LINUX.is_file():
        return FAILED, "install-linux.sh fehlt"
    script = INSTALL_LINUX.read_text(encoding="utf-8", errors="replace")
    if "--with-fuse" not in script:
        return FAILED, "install-linux.sh kennt --with-fuse nicht (C5, DRV-M9; Paket U36)"
    if "--dry-run" not in script:
        return FAILED, ("install-linux.sh kennt kein --dry-run; das Signal verlangt einen Probelauf mit "
                        "--with-fuse (Paket U36)")
    if not LINUX:
        return NOT_RUN, "Probelauf nur auf einem Linux-Runner"
    root = ctx.work / "install-linux-dry-run"
    home = root / "home"
    home.mkdir(parents=True, exist_ok=True)
    env = dict(ctx.base_env(), HOME=str(home), XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"),
               XDG_BIN_HOME=str(root / "bin"))
    usage = ctx.run(["bash", INSTALL_LINUX, "--help"], stage="static", env=env, timeout=120)
    dry = ctx.run(["bash", INSTALL_LINUX, "--dry-run", "--with-fuse"], stage="static", env=env, timeout=300)
    problems = []
    if "--with-fuse" not in usage.output:
        problems.append("die Usage-Zeile (--help) nennt --with-fuse nicht")
    if not dry.ok:
        problems.append(f"Probelauf --dry-run --with-fuse: {dry.describe()}")
    if "fuse3" not in dry.output:
        problems.append("der Probelauf nennt fuse3 nicht")
    written = [str(path.relative_to(home)) for path in home.rglob("*")]
    if written:
        problems.append("der Probelauf schrieb in HOME: " + ", ".join(written[:10]))
    if problems:
        return FAILED, "; ".join(problems) + "\n" + common.excerpt(dry.output, 1500)
    return PASSED, "install-linux.sh --dry-run --with-fuse endet mit 0, nennt fuse3 und schreibt nichts"


STAGES = {"drive-fuse": stage_drive_fuse, "drive-dokany-regression": stage_drive_dokany_regression}
CHECKS = {
    "drive_task_docs_surfaces_complete": check_docs_surfaces_complete,
    "drive_task_install_linux_dry_run_with_fuse": check_install_linux_dry_run_with_fuse,
}
