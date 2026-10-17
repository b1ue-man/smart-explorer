#!/usr/bin/env python3
"""INT-LIC checks of the suite job ``prepare``, stage ``licenses`` (plan C4, D3).

They need ``pwsh``, ``cargo`` and network access for crate sources, but no
compilation of the Smart Explorer crates. The stage runs after the stage
``lock`` resolved ``native/Cargo.lock`` (L1) and before anything replaces the
checked-in license file:

- licenses_task_generated_file_current: a fresh generation equals the
  checked-in ``third-party/rust-crates/THIRD-PARTY-LICENSES.txt`` (line ends
  and the "Generated:" date line excluded).
- licenses_task_every_shipped_crate_has_text: every crate that a shipped
  program builds as a normal dependency for one of the targets of
  ``about.toml`` (``cargo metadata --filter-platform``) is listed under its
  manifest, and the license texts attached to each listed crate satisfy its
  license expression.
- licenses_task_new_batch_crates_listed: the crates this batch adds (plan C4
  table) are listed with the license that ``cargo metadata`` reports.
- licenses_task_winit_notice_included: every NOTICE file of ``winit`` appears
  verbatim; without one the check passes and reports that as a finding.
- licenses_task_release_wrapper_checks_licenses (static):
  ``native/publish-release-local.ps1`` runs ``rust-licenses.ps1 -Mode Check``
  before its first build call.

Usage::

    python native/licenses_task_checks.py --log-root DIR [--only NAME ...] [--place-generated]

Writes DIR/THIRD-PARTY-LICENSES.txt (generated), DIR/generate.log and
DIR/licenses-report.json ``{"stage", "generated", "generator_exit", "tests":
[{"name", "stage", "status", "reason", "duration_s"}]}`` and exits 1 when a
check failed. ``--place-generated`` afterwards copies the generated file over
the checked-in path, so the job artifact and the later jobs build with it (L2).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

REPO_ROOT = Path(__file__).resolve().parent.parent
LICENSE_FILE = REPO_ROOT / "third-party" / "rust-crates" / "THIRD-PARTY-LICENSES.txt"
ABOUT_TOML = REPO_ROOT / "third-party" / "rust-crates" / "about.toml"
GENERATOR = REPO_ROOT / "native" / "rust-licenses.ps1"
WRAPPER = REPO_ROOT / "native" / "publish-release-local.ps1"
MANIFESTS = ("native/Cargo.toml", "native/explorer-command/Cargo.toml", "share-server/Cargo.toml")
STAGE = "licenses"
PASSED, FAILED = "passed", "failed"
GENERATE_TIMEOUT_S = 45 * 60
METADATA_TIMEOUT_S = 15 * 60

# Layout written by native/rust-licenses.ps1.
RULE = "=" * 80
THIN_RULE = "-" * 80
CRATES_HEADING = "Crates by manifest (name version license)"
TEXTS_HEADING = "License texts"
NOTICES_HEADING = "NOTICE files"
GENERATED_PREFIX = "Generated: "
LICENSE_HEAD = re.compile(r"^License (\d+) of (\d+): (.*) \[([^\]]+)\]$")
NOTICE_HEAD = re.compile(r"^NOTICE (\d+) of (\d+): (\S+) (\S+) \((.+)\)$")
NOTICE_NAME = re.compile(r"^notice(\.(txt|md|markdown|rst))?$", re.IGNORECASE)
SPDX_TOKEN = re.compile(r"\(|\)|[^\s()]+")

# Plan C4, "Neue Crates dieses Stapels": (name, planned version, planned
# license). cargo metadata is the reference; a differing plan value is a note.
NEW_BATCH_CRATES: tuple[tuple[str, str | None, str | None], ...] = (
    ("fuser", "0.18.0", "MIT"),
    ("aes-siv", "0.8.0", "Apache-2.0 OR MIT"),
    ("aes-kw", "0.3.1", "Apache-2.0 OR MIT"),
    ("zune-jpeg", None, "MIT OR Apache-2.0 OR Zlib"),
    ("zune-core", None, "MIT OR Apache-2.0 OR Zlib"),
    ("image-webp", None, "MIT OR Apache-2.0"),
    ("quick-error", None, "MIT/Apache-2.0"),
    ("gif", None, "MIT OR Apache-2.0"),
    ("weezl", None, "MIT OR Apache-2.0"),
    ("color_quant", None, "MIT"),
    ("image", "0.25.10", "MIT OR Apache-2.0"),
    ("smithay-client-toolkit", "0.19.2", None),
    ("wayland-client", "0.31.14", None),
    ("wayland-protocols-wlr", "0.3.12", None),
    ("calloop", "0.13.0", None),
    ("xkbcommon-dl", "0.4.2", None),
    ("x11rb", "0.13.2", None),
    ("egui-wgpu", "0.29.1", None),
    ("raw-window-handle", "0.6.2", None),
    ("winit", "0.30.13", "Apache-2.0"),
    ("ashpd", "0.13.13", "MIT"),
    ("openh264", "0.9.9", "BSD-2-Clause"),
    ("openh264-sys2", "0.9.9", "BSD-2-Clause"),
    ("wide", None, "Zlib OR Apache-2.0 OR MIT"),
    ("bzip2", "0.6.1", "MIT OR Apache-2.0"),
    ("libbz2-rs-sys", None, "bzip2-1.0.6"),
    ("lz4_flex", "0.14.0", "MIT"),
    ("libloading", "0.8.9", "ISC"),
    ("png", "0.18.1", "MIT OR Apache-2.0"),
    ("wayland-protocols", "0.32.12", None),
    ("wayland-protocols-plasma", "0.3.12", None),
)


class CheckError(Exception):
    """A precondition of a check failed; the message is the report reason."""


@dataclass
class Generation:
    exit_code: int | None
    output: str
    error: str | None

    def problem_lines(self) -> list[str]:
        lines = self.output.splitlines()
        for index, line in enumerate(lines):
            if line.startswith("Written, but incomplete"):
                return [entry.strip() for entry in lines[index + 1:] if entry.startswith("  ")]
        return []


@dataclass
class Block:
    license_id: str
    name: str
    users: list[str]
    text: str

    def base_id(self) -> str:
        return self.license_id.split()[0].rstrip("+").lower()


@dataclass
class Notice:
    crate: str
    file: str
    text: str


@dataclass
class LicenseFile:
    sections: dict[str, list[tuple[str, str, str]]]
    blocks: list[Block]
    notices: list[Notice]

    def listed(self) -> dict[tuple[str, str], str]:
        result: dict[tuple[str, str], str] = {}
        for entries in self.sections.values():
            for name, version, expression in entries:
                result.setdefault((name, version), expression)
        return result


@dataclass
class Graph:
    first_party: set[str]
    shipped: dict[str, set[tuple[str, str]]]
    packages: dict[tuple[str, str], dict[str, Any]]
    by_name: dict[str, list[dict[str, Any]]]

    def shipped_all(self) -> set[tuple[str, str]]:
        result: set[tuple[str, str]] = set()
        for crates in self.shipped.values():
            result |= crates
        return result


def tail(text: str, count: int = 25) -> str:
    return "\n".join(text.strip().splitlines()[-count:])


def rel(path: Path) -> str:
    try:
        return path.relative_to(REPO_ROOT).as_posix()
    except ValueError:
        return str(path)


def split_lines(text: str) -> list[str]:
    return text.replace("\r\n", "\n").replace("\r", "\n").split("\n")


def normalize_text(text: str) -> str:
    """Same rule as Get-NormalizedText in native/rust-licenses.ps1."""
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    return text.lstrip("\ufeff").lstrip("\n").rstrip(" \t\n")


def comparable_lines(text: str) -> list[str]:
    """Same rule as Get-ComparableLines in native/rust-licenses.ps1."""
    result: list[str] = []
    skipped = False
    for index, line in enumerate(split_lines(text)):
        if not skipped and index < 12 and line.startswith(GENERATED_PREFIX):
            skipped = True
            continue
        result.append(line)
    return result


def configured_targets() -> list[str]:
    toml = ABOUT_TOML.read_text(encoding="utf-8")
    block = re.search(r"(?ms)^targets\s*=\s*\[(.*?)\]", toml)
    targets = re.findall(r'"([^"]+)"', block.group(1)) if block else []
    if not targets:
        raise CheckError(f"{rel(ABOUT_TOML)} nennt keine targets")
    return targets


def parse_license_file(text: str) -> LicenseFile:
    lines = split_lines(text)
    try:
        crates_at = lines.index(CRATES_HEADING)
        texts_at = lines.index(TEXTS_HEADING, crates_at + 1)
    except ValueError:
        raise CheckError("Überschrift der Crate-Liste oder der Lizenztexte fehlt") from None
    sections: dict[str, list[tuple[str, str, str]]] = {}
    current: str | None = None
    for line in lines[crates_at + 1 : texts_at]:
        if not line or set(line) == {"-"}:
            continue
        if line.startswith("[") and line.endswith("]"):
            current = line[1:-1]
            sections[current] = []
            continue
        parts = line.split(" ", 2)
        if current is None or len(parts) != 3:
            raise CheckError(f"unerwartete Zeile in der Crate-Liste: {line!r}")
        sections[current].append((parts[0], parts[1], parts[2]))

    license_heads: list[tuple[int, re.Match[str]]] = []
    for index in range(texts_at + 1, len(lines) - 1):
        match = LICENSE_HEAD.match(lines[index + 1]) if lines[index] == RULE else None
        if match and int(match.group(1)) == len(license_heads) + 1:
            license_heads.append((index, match))
    search_from = license_heads[-1][0] + 2 if license_heads else texts_at + 1
    notices_at = next(
        (
            index
            for index in range(search_from, len(lines) - 1)
            if lines[index] == NOTICES_HEADING and lines[index + 1] == "-" * len(NOTICES_HEADING)
        ),
        None,
    )
    if notices_at is None:
        raise CheckError(f"Abschnitt {NOTICES_HEADING!r} fehlt")
    notice_heads: list[tuple[int, re.Match[str]]] = []
    for index in range(notices_at + 2, len(lines) - 1):
        match = NOTICE_HEAD.match(lines[index + 1]) if lines[index] == RULE else None
        if match and int(match.group(1)) == len(notice_heads) + 1:
            notice_heads.append((index, match))

    blocks: list[Block] = []
    for position, (start, match) in enumerate(license_heads):
        end = license_heads[position + 1][0] if position + 1 < len(license_heads) else notices_at
        if lines[start + 2] != "Used by:":
            raise CheckError(f"Lizenzblock {match.group(1)} ohne 'Used by:'")
        cursor = start + 3
        users: list[str] = []
        while cursor < end and lines[cursor].startswith("  "):
            users.append(lines[cursor][2:])
            cursor += 1
        if cursor >= end or lines[cursor] != THIN_RULE:
            raise CheckError(f"Lizenzblock {match.group(1)} ohne Trennlinie vor dem Text")
        blocks.append(Block(match.group(4), match.group(3), users, _block_text(lines[cursor + 1 : end])))
    notices: list[Notice] = []
    for position, (start, match) in enumerate(notice_heads):
        end = notice_heads[position + 1][0] if position + 1 < len(notice_heads) else len(lines)
        if lines[start + 2] != THIN_RULE:
            raise CheckError(f"NOTICE-Block {match.group(1)} ohne Trennlinie vor dem Text")
        notices.append(Notice(f"{match.group(3)} {match.group(4)}", match.group(5), _block_text(lines[start + 3 : end])))
    for kind, heads in (("Lizenzblöcke", license_heads), ("NOTICE-Blöcke", notice_heads)):
        if heads and int(heads[-1][1].group(2)) != len(heads):
            raise CheckError(f"{kind}: {len(heads)} gefunden, Kopf nennt {heads[-1][1].group(2)}")
    return LicenseFile(sections, blocks, notices)


def _block_text(lines: list[str]) -> str:
    body = list(lines)
    while body and body[-1] == "":
        body.pop()
    return "\n".join(body)


class _Expression:
    """Minimal SPDX expression evaluation: OR, AND, WITH, parentheses and the
    legacy '/' separator."""

    def __init__(self, expression: str) -> None:
        self.tokens = SPDX_TOKEN.findall(expression.replace("/", " OR "))
        self.position = 0

    def evaluate(self, accept: Callable[[str], bool]) -> bool:
        value = self._any(accept)
        if self.position != len(self.tokens):
            raise ValueError(f"unerwartetes Token {self.tokens[self.position]!r}")
        return value

    def ids(self) -> set[str]:
        found: set[str] = set()
        self.evaluate(lambda license_id: found.add(license_id.lower()) or True)
        return found

    def _peek(self) -> str | None:
        return self.tokens[self.position] if self.position < len(self.tokens) else None

    def _take(self) -> str:
        token = self._peek()
        if token is None:
            raise ValueError("Ausdruck endet unerwartet")
        self.position += 1
        return token

    def _any(self, accept: Callable[[str], bool]) -> bool:
        value = self._all(accept)
        while (self._peek() or "").upper() == "OR":
            self.position += 1
            right = self._all(accept)
            value = value or right
        return value

    def _all(self, accept: Callable[[str], bool]) -> bool:
        value = self._atom(accept)
        while (self._peek() or "").upper() == "AND":
            self.position += 1
            right = self._atom(accept)
            value = value and right
        return value

    def _atom(self, accept: Callable[[str], bool]) -> bool:
        token = self._take()
        if token == "(":
            value = self._any(accept)
            if self._take() != ")":
                raise ValueError("fehlende schließende Klammer")
            return value
        if (self._peek() or "").upper() == "WITH":
            self.position += 1
            self._take()
        return accept(token.rstrip("+"))


def expression_satisfied(expression: str, available: set[str]) -> bool:
    return _Expression(expression).evaluate(lambda license_id: license_id.lower() in available)


def license_ids(expression: str) -> set[str]:
    return _Expression(expression).ids()


def path_key(path: str) -> str:
    return os.path.normcase(os.path.abspath(path))


def cargo_metadata(manifest: str, target: str | None) -> dict[str, Any]:
    command = ["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", str(REPO_ROOT / manifest)]
    if target:
        command += ["--filter-platform", target]
    completed = subprocess.run(
        command,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=METADATA_TIMEOUT_S,
        check=False,
    )
    if completed.returncode != 0:
        raise CheckError(f"{' '.join(command[1:])} endete mit {completed.returncode}: {tail(completed.stderr)}")
    return json.loads(completed.stdout)


def shipped_crates(metadata: dict[str, Any], first_party: set[str]) -> set[tuple[str, str]]:
    """Packages reachable from the workspace members over normal dependencies."""
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    packages = {package["id"]: package for package in metadata["packages"]}
    stack = list(metadata["workspace_members"])
    seen = set(stack)
    while stack:
        node = nodes.get(stack.pop())
        if node is None:
            continue
        for dependency in node.get("deps", []):
            kinds = dependency.get("dep_kinds") or [{"kind": None}]
            if not any(kind.get("kind") is None for kind in kinds):
                continue
            if dependency["pkg"] not in seen:
                seen.add(dependency["pkg"])
                stack.append(dependency["pkg"])
    result: set[tuple[str, str]] = set()
    for package_id in seen:
        package = packages.get(package_id)
        if package is not None and path_key(package["manifest_path"]) not in first_party:
            result.add((package["name"], package["version"]))
    return result


def load_graph(targets: list[str]) -> Graph:
    unfiltered = {manifest: cargo_metadata(manifest, None) for manifest in MANIFESTS}
    first_party: set[str] = set()
    for metadata in unfiltered.values():
        members = set(metadata["workspace_members"])
        for package in metadata["packages"]:
            if package["id"] in members:
                first_party.add(path_key(package["manifest_path"]))
    packages: dict[tuple[str, str], dict[str, Any]] = {}
    by_name: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for metadata in unfiltered.values():
        for package in metadata["packages"]:
            key = (package["name"], package["version"])
            if path_key(package["manifest_path"]) in first_party or key in packages:
                continue
            packages[key] = package
            by_name[package["name"]].append(package)
    shipped: dict[str, set[tuple[str, str]]] = {}
    for manifest in MANIFESTS:
        reachable: set[tuple[str, str]] = set()
        for target in targets:
            reachable |= shipped_crates(cargo_metadata(manifest, target), first_party)
        shipped[manifest] = reachable
    return Graph(first_party, shipped, packages, dict(by_name))


class Context:
    def __init__(self, log_root: Path) -> None:
        self.log_root = log_root
        self.generated_path = log_root / "THIRD-PARTY-LICENSES.txt"
        self._cache: dict[str, tuple[bool, Any]] = {}

    def _memo(self, name: str, factory: Callable[[], Any]) -> Any:
        if name not in self._cache:
            try:
                self._cache[name] = (True, factory())
            except Exception as error:  # every failure is reported by each dependent check
                self._cache[name] = (False, error)
        ok, value = self._cache[name]
        if ok:
            return value
        raise value

    def generation(self) -> Generation:
        return self._memo("generation", self._generate)

    def generated_text(self) -> str:
        generation = self.generation()
        if generation.error:
            raise CheckError(generation.error)
        return self.generated_path.read_text(encoding="utf-8")

    def parsed(self) -> LicenseFile:
        return self._memo("parsed", lambda: parse_license_file(self.generated_text()))

    def graph(self) -> Graph:
        return self._memo("graph", lambda: load_graph(configured_targets()))

    def _generate(self) -> Generation:
        pwsh = shutil.which("pwsh")
        if pwsh is None:
            return Generation(None, "", "pwsh fehlt; der Job prepare installiert PowerShell 7 (Plan C4)")
        self.generated_path.unlink(missing_ok=True)
        command = [
            pwsh, "-NoProfile", "-NonInteractive", "-File", str(GENERATOR),
            "-Mode", "Generate", "-Out", str(self.generated_path), "-EnsureTool",
        ]
        try:
            completed = subprocess.run(
                command,
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=GENERATE_TIMEOUT_S,
                check=False,
            )
        except subprocess.TimeoutExpired as error:
            output = error.stdout if isinstance(error.stdout, str) else ""
            (self.log_root / "generate.log").write_text(output, encoding="utf-8")
            return Generation(None, output, f"rust-licenses.ps1 -Mode Generate nach {GENERATE_TIMEOUT_S} s abgebrochen")
        (self.log_root / "generate.log").write_text(completed.stdout, encoding="utf-8")
        if completed.returncode in (0, 2) and self.generated_path.is_file():
            return Generation(completed.returncode, completed.stdout, None)
        return Generation(
            completed.returncode,
            completed.stdout,
            f"rust-licenses.ps1 -Mode Generate endete mit {completed.returncode}: {tail(completed.stdout)}",
        )


def describe_difference(expected: list[str], actual: list[str]) -> str:
    first = next(
        (index for index, (left, right) in enumerate(zip(expected, actual)) if left != right),
        min(len(expected), len(actual)),
    )
    only_checked_in = list((Counter(expected) - Counter(actual)).elements())
    only_generated = list((Counter(actual) - Counter(expected)).elements())
    return (
        f"{rel(LICENSE_FILE)} weicht ab: erste Abweichung in Zeile {first + 1}; "
        f"{len(only_checked_in)} Zeilen nur eingecheckt (z. B. {only_checked_in[:3]!r}), "
        f"{len(only_generated)} Zeilen nur erzeugt (z. B. {only_generated[:3]!r})"
    )


def short_list(items: list[str], limit: int = 30) -> str:
    shown = ", ".join(items[:limit])
    return shown + (f" … (+{len(items) - limit})" if len(items) > limit else "")


def check_generated_file_current(context: Context) -> tuple[str, str]:
    generated = context.generated_text()
    if not LICENSE_FILE.is_file():
        return FAILED, (
            f"eingecheckte Datei fehlt: {rel(LICENSE_FILE)}; die erzeugte Datei liegt unter "
            f"{context.generated_path} (erster Lauf, L2: der Hauptagent checkt sie ein)"
        )
    expected = comparable_lines(LICENSE_FILE.read_text(encoding="utf-8"))
    actual = comparable_lines(generated)
    if expected == actual:
        return PASSED, f"{rel(LICENSE_FILE)} entspricht der frischen Erzeugung ({len(actual)} Zeilen ohne Datumszeile)"
    return FAILED, describe_difference(expected, actual)


def check_every_shipped_crate_has_text(context: Context) -> tuple[str, str]:
    parsed = context.parsed()
    graph = context.graph()
    problems: list[str] = []
    generation = context.generation()
    if generation.exit_code == 2:
        problems.append("Generator meldet Lücken (Exitcode 2): " + short_list(generation.problem_lines(), 20))
    for manifest in MANIFESTS:
        listed = {(name, version) for name, version, _ in parsed.sections.get(manifest, [])}
        missing = sorted(graph.shipped[manifest] - listed)
        if missing:
            problems.append(f"{manifest}: nicht gelistet: " + short_list([f"{name} {version}" for name, version in missing]))
    ids_by_crate: dict[str, set[str]] = defaultdict(set)
    for block in parsed.blocks:
        for user in block.users:
            ids_by_crate[user].add(block.base_id())
    unsatisfied: list[str] = []
    for (name, version), expression in sorted(parsed.listed().items()):
        crate = f"{name} {version}"
        available = ids_by_crate.get(crate, set())
        try:
            satisfied = expression_satisfied(expression, available)
        except ValueError as error:
            unsatisfied.append(f"{crate} ({expression}): {error}")
            continue
        if not satisfied:
            texts = ", ".join(sorted(available)) or "keine"
            unsatisfied.append(f"{crate} ({expression}; Texte: {texts})")
    if unsatisfied:
        problems.append("Lizenztexte decken den Ausdruck nicht: " + short_list(unsatisfied))
    if problems:
        return FAILED, " | ".join(problems)
    return PASSED, (
        f"{len(graph.shipped_all())} ausgelieferte Crates aus {len(MANIFESTS)} Manifesten gelistet; "
        f"jeder der {len(parsed.listed())} Ausdrücke ist durch Lizenztexte gedeckt"
    )


def check_new_batch_crates_listed(context: Context) -> tuple[str, str]:
    parsed = context.parsed()
    graph = context.graph()
    listed = parsed.listed()
    shipped = graph.shipped_all()
    failures: list[str] = []
    notes: list[str] = []
    checked = 0
    for name, planned_version, planned_license in NEW_BATCH_CRATES:
        if name not in graph.by_name:
            failures.append(f"{name}: nicht im Abhängigkeitsgraphen (cargo metadata)")
            continue
        versions = sorted({package["version"] for package in graph.by_name[name] if (name, package["version"]) in shipped})
        if not versions:
            failures.append(f"{name}: im Lock, aber für kein Ziel als normale Abhängigkeit gebaut")
            continue
        if planned_version and planned_version not in versions:
            notes.append(f"{name}: Plan {planned_version}, gebaut {', '.join(versions)}")
        for version in versions:
            checked += 1
            expression = listed.get((name, version))
            if expression is None:
                failures.append(f"{name} {version}: fehlt in der Datei")
                continue
            metadata_license = graph.packages[(name, version)].get("license")
            if not metadata_license:
                notes.append(f"{name} {version}: cargo metadata ohne Lizenzfeld; Datei {expression!r}")
                continue
            if license_ids(expression) != license_ids(metadata_license):
                failures.append(f"{name} {version}: Datei {expression!r}, cargo metadata {metadata_license!r}")
            if planned_license and license_ids(planned_license) != license_ids(metadata_license):
                notes.append(f"{name} {version}: Plan {planned_license!r}, cargo metadata {metadata_license!r}")
    note_text = f"; Hinweise: {short_list(notes, 20)}" if notes else ""
    if failures:
        return FAILED, short_list(failures) + note_text
    return PASSED, f"{checked} Crate-Versionen aus {len(NEW_BATCH_CRATES)} Plannamen gelistet{note_text}"


def check_winit_notice_included(context: Context) -> tuple[str, str]:
    parsed = context.parsed()
    graph = context.graph()
    versions = sorted(version for name, version in graph.shipped_all() if name == "winit")
    if not versions:
        return FAILED, "winit wird für kein Ziel gebaut (Plan C4: Windows und Linux)"
    failures: list[str] = []
    findings: list[str] = []
    for version in versions:
        crate = f"winit {version}"
        directory = Path(graph.packages[("winit", version)]["manifest_path"]).parent
        if not any(block.base_id() == "apache-2.0" and crate in block.users for block in parsed.blocks):
            failures.append(f"{crate}: kein Apache-2.0-Text mit {crate} unter 'Used by'")
        notice_files = sorted(entry.name for entry in directory.iterdir() if entry.is_file() and NOTICE_NAME.match(entry.name))
        if not notice_files:
            findings.append(f"Befund: {crate} bringt keine NOTICE-Datei mit ({directory})")
            continue
        for file_name in notice_files:
            expected = normalize_text((directory / file_name).read_text(encoding="utf-8", errors="replace"))
            found = [notice for notice in parsed.notices if notice.crate == crate and notice.file == file_name]
            if not found:
                failures.append(f"{crate}: {file_name} fehlt im Abschnitt {NOTICES_HEADING!r}")
            elif found[0].text != expected:
                failures.append(f"{crate}: {file_name} weicht vom Original ab")
            else:
                findings.append(f"{crate}: {file_name} wörtlich enthalten")
    if failures:
        return FAILED, "; ".join(failures + findings)
    return PASSED, "; ".join(findings)


def check_release_wrapper_checks_licenses(_context: Context) -> tuple[str, str]:
    lines = split_lines(WRAPPER.read_text(encoding="utf-8"))
    bodies: dict[str, list[str]] = defaultdict(list)
    top_level: list[tuple[int, str]] = []
    function: str | None = None
    for number, line in enumerate(lines, 1):
        if function is not None:
            if line.rstrip() == "}":
                function = None
            else:
                bodies[function].append(line)
            continue
        definition = re.match(r"^function\s+([\w-]+)", line)
        if definition:
            function = definition.group(1)
            continue
        top_level.append((number, line))
    body = bodies.get("Assert-RustCrateLicenses")
    if body is None:
        return FAILED, f"{rel(WRAPPER)}: Funktion Assert-RustCrateLicenses fehlt"
    check_lines = [line for line in body if "rust-licenses.ps1" in line and "-Mode Check" in line]
    joined = "\n".join(body)
    missing = [part for part in ("-EnsureTool", "$LASTEXITCODE -ne 0", "throw") if part not in joined]
    if not check_lines or missing:
        return FAILED, f"Assert-RustCrateLicenses ohne 'rust-licenses.ps1 ... -Mode Check' oder ohne {missing}"
    code = [(number, line) for number, line in top_level if not line.lstrip().startswith("#")]
    call = next((number for number, line in code if re.match(r"^\s*Assert-RustCrateLicenses\s*$", line)), None)
    build = next(
        (number for number, line in code if re.search(r"\bInvoke-(WindowsReleaseBuild|LinuxCompleteReleaseBuild)\b", line)),
        None,
    )
    if call is None:
        return FAILED, f"{rel(WRAPPER)}: kein Aufruf von Assert-RustCrateLicenses außerhalb von Funktionen"
    if build is None:
        return FAILED, f"{rel(WRAPPER)}: kein Bauaufruf außerhalb von Funktionen gefunden"
    if call > build:
        return FAILED, f"Lizenzprüfung in Zeile {call} steht nach dem ersten Bauaufruf in Zeile {build}"
    return PASSED, f"rust-licenses.ps1 -Mode Check über Assert-RustCrateLicenses in Zeile {call}, erster Bauaufruf in Zeile {build}"


CHECKS: tuple[tuple[str, Callable[[Context], tuple[str, str]]], ...] = (
    ("licenses_task_generated_file_current", check_generated_file_current),
    ("licenses_task_every_shipped_crate_has_text", check_every_shipped_crate_has_text),
    ("licenses_task_new_batch_crates_listed", check_new_batch_crates_listed),
    ("licenses_task_winit_notice_included", check_winit_notice_included),
    ("licenses_task_release_wrapper_checks_licenses", check_release_wrapper_checks_licenses),
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--log-root", required=True, type=Path)
    parser.add_argument("--only", action="append", choices=[name for name, _ in CHECKS])
    parser.add_argument("--place-generated", action="store_true")
    args = parser.parse_args(argv)
    log_root = args.log_root.resolve()
    log_root.mkdir(parents=True, exist_ok=True)
    context = Context(log_root)
    results: list[dict[str, Any]] = []
    for name, check in CHECKS:
        if args.only and name not in args.only:
            continue
        started = time.monotonic()
        try:
            status, reason = check(context)
        except CheckError as error:
            status, reason = FAILED, str(error)
        except Exception as error:  # an unexpected error fails exactly this check
            status, reason = FAILED, f"{type(error).__name__}: {error}"
        duration = round(time.monotonic() - started, 3)
        results.append({"name": name, "stage": STAGE, "status": status, "reason": reason, "duration_s": duration})
        print(f"{status:7} {name}: {reason}", flush=True)
    generation = context._cache.get("generation")
    generator_exit = generation[1].exit_code if generation and generation[0] else None
    generated = context.generated_path if context.generated_path.is_file() else None
    if args.place_generated and generated is not None:
        LICENSE_FILE.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(generated, LICENSE_FILE)
        print(f"placed {generated} -> {rel(LICENSE_FILE)}", flush=True)
    report = {
        "stage": STAGE,
        "generated": str(generated) if generated else None,
        "generator_exit": generator_exit,
        "tests": results,
    }
    (log_root / "licenses-report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return 1 if any(result["status"] == FAILED for result in results) else 0


if __name__ == "__main__":
    sys.exit(main())
