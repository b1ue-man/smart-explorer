#!/usr/bin/env python3
"""Evaluators of the Android task suite (android/test-android-task.sh).

Sub-commands (every expected value is derived from the repository itself):
  classes          list the instrumented test classes found in the androidTest sources
  instrumentation  strict evaluation of one raw `am instrument -w -r` output
  unit-tests       strict evaluation of Gradle's JVM unit-test XML results
  coverage         api.md method coverage from the calls the instrumented tests recorded
  release-lists    G7: release asset/commit lists of build.yml against release-publication.ps1

Runs only on the remote CI runner (AGENTS.md: no local builds or test execution).
"""
import argparse
import json
import re
import shlex
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

STATUS_OK = 0
STATUS_NAMES = {1: "gestartet", 0: "ok", -1: "Fehler", -2: "fehlgeschlagen", -3: "ignoriert", -4: "Annahme verletzt"}


def source_tests(root: Path) -> dict:
    """Fully qualified test class -> test method names, from Kotlin sources under `root`."""
    tests = {}
    for path in sorted(root.rglob("*.kt")):
        text = path.read_text(encoding="utf-8")
        if re.search(r"@Ignore\b", text):
            raise SystemExit(f"{path}: @Ignore is not allowed in the task suite (a skipped test hides a gap)")
        package = re.search(r"^package\s+([\w.]+)", text, re.M)
        if not package:
            continue
        classes = [(m.start(), m.group(1)) for m in re.finditer(r"^(?:internal\s+)?class\s+(\w+)", text, re.M)]
        for match in re.finditer(r"@Test\b[^\n]*\n\s*(?:@\w+[^\n]*\n\s*)*fun\s+(\w+)", text):
            owner = [name for start, name in classes if start < match.start()]
            if not owner:
                raise SystemExit(f"{path}: @Test outside a class")
            tests.setdefault(f"{package.group(1)}.{owner[-1]}", []).append(match.group(1))
    return tests


def expected_for(tests: dict, spec: str) -> set:
    expected = set()
    for item in filter(None, (part.strip() for part in spec.split(","))):
        cls, _, method = item.partition("#")
        if cls not in tests:
            raise SystemExit(f"unknown test class {cls}; known: {sorted(tests)}")
        if method:
            if method not in tests[cls]:
                raise SystemExit(f"unknown test {cls}#{method}")
            expected.add(f"{cls}#{method}")
        else:
            expected.update(f"{cls}#{name}" for name in tests[cls])
    return expected


def cmd_classes(args) -> int:
    for cls in sorted(source_tests(Path(args.sources))):
        print(cls)
    return 0


def cmd_instrumentation(args) -> int:
    tests = source_tests(Path(args.sources))
    expected = expected_for(tests, args.classes)
    status, details, current, key = {}, {}, {}, None
    result_lines, code = [], None
    for raw in Path(args.output).read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.rstrip("\r")
        if line.startswith("INSTRUMENTATION_STATUS: "):
            key, _, value = line[len("INSTRUMENTATION_STATUS: "):].partition("=")
            current[key] = value
        elif line.startswith("INSTRUMENTATION_STATUS_CODE: "):
            value = int(line.split(":", 1)[1].strip())
            if current.get("class") and current.get("test"):
                name = f"{current['class']}#{current['test']}"
                if status.get(name) not in (-1, -2, -4):
                    status[name] = value
                if value not in (0, 1):
                    details[name] = current.get("stack", "")[:4000]
            current, key = {}, None
        elif line.startswith("INSTRUMENTATION_RESULT: ") or line.startswith("INSTRUMENTATION_FAILED"):
            result_lines.append(line)
            key = None
        elif line.startswith("INSTRUMENTATION_CODE: "):
            code = int(line.split(":", 1)[1].strip())
        elif key is not None:
            current[key] = current.get(key, "") + "\n" + line
    problems = []
    for name in sorted(expected):
        state = status.get(name)
        if state is None:
            problems.append(f"FEHLT       {name} (nicht gelaufen)")
        elif state != STATUS_OK:
            problems.append(f"{STATUS_NAMES.get(state, state):<11} {name}\n{details.get(name, '').strip()}")
    for name in sorted(set(status) - expected):
        problems.append(f"UNERWARTET  {name} ({STATUS_NAMES.get(status[name], status[name])})")
    crash = [line for line in result_lines if "shortMsg=" in line or line.startswith("INSTRUMENTATION_FAILED")]
    if crash:
        problems.append("Instrumentation abgebrochen: " + " | ".join(crash))
    if code != -1:
        problems.append(f"INSTRUMENTATION_CODE {code} statt -1 (Lauf nicht vollständig)")
    with open(args.summary, "w", encoding="utf-8") as out:
        for name in sorted(expected | set(status)):
            out.write(f"{name}\t{STATUS_NAMES.get(status.get(name), 'fehlt')}\n")
    passed = sum(1 for name in expected if status.get(name) == STATUS_OK)
    print(f"instrumentation {args.name}: {passed}/{len(expected)} erwartete Tests ok")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    return 0


def cmd_unit_tests(args) -> int:
    expected = {f"{cls}#{name}" for cls, names in source_tests(Path(args.sources)).items() for name in names}
    seen, problems = {}, []
    files = sorted(Path(args.results).glob("TEST-*.xml"))
    if not files:
        print(f"no JVM unit-test results in {args.results}", file=sys.stderr)
        return 1
    for path in files:
        for case in ET.parse(path).getroot().iter("testcase"):
            name = f"{case.get('classname')}#{case.get('name')}"
            bad = [child.tag for child in case if child.tag in ("failure", "error", "skipped")]
            seen[name] = bad
            if bad:
                message = next((child.get("message", "") for child in case if child.tag in ("failure", "error")), "")
                problems.append(f"{'/'.join(bad)}: {name} {message}")
    for name in sorted(expected - set(seen)):
        problems.append(f"FEHLT: {name}")
    for name in sorted(set(seen) - expected):
        problems.append(f"UNERWARTET: {name}")
    print(f"JVM unit tests: {sum(1 for bad in seen.values() if not bad)}/{len(expected)} erwartete Tests ok")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    return 0


# api.md methods that the suite does not call successfully, with the reason (umsetzung.md G4).
EXCLUDED = {
    "gdrive.signIn": "echtes Google-Konto nötig (umsetzung.md G4, ausdrückliche Ausnahme)",
}
ERROR_PATH_ALLOWED = {
    "share.connect": "PIN-Pairing braucht ein zweites Gerät mit laufendem Discovery-Angebot; die Desktop-CLI bietet keins",
    "share.cancelConnect": "ohne laufenden Schlüsselaustausch nur mit unbekannter exchangeId aufrufbar",
    "share.decide": "eine eingehende Anfrage bräuchte eine Zugriffsanfrage des Desktops an das Telefon",
    "share.retry": "nur für abgelehnte/fehlgeschlagene ausgehende Anfragen möglich",
    "share.deleteRequest": "hängt davon ab, ob die ausgehende Anfrage schon entschieden wurde",
    "share.requestAccess": "hängt von der Direct-Erreichbarkeit des Desktops ab; Ergebnis wird protokolliert",
    "share.leaveRoom": "Laufzeitbefehl an den eingebetteten Worker; Ergebnis wird protokolliert",
}


def api_methods(api: Path) -> list:
    text = api.read_text(encoding="utf-8")
    pattern = r"`((?:sys|loc|fs|scan|index|trash|task|conn|gdrive|sync|bg|share|analyze|reclaim|update)\.[A-Za-z]+) \{"
    return sorted(set(re.findall(pattern, text)))


def cmd_coverage(args) -> int:
    methods = api_methods(Path(args.api))
    if len(methods) < 50:
        print(f"api.md yielded only {len(methods)} methods; extraction pattern broken?", file=sys.stderr)
        return 1
    outcomes = {}
    for calls in args.calls:
        path = Path(calls)
        if not path.is_file():
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            method, _, outcome = line.partition("\t")
            outcomes.setdefault(method, set()).add(outcome)
    rows, problems = [], []
    for method in methods:
        seen = outcomes.get(method, set())
        if "ok" in seen:
            rows.append((method, "aufgerufen", ", ".join(sorted(seen))))
        elif method in EXCLUDED:
            rows.append((method, "ausgenommen", EXCLUDED[method]))
        elif seen and method in ERROR_PATH_ALLOWED:
            rows.append((method, "nur Fehlerpfad", f"{', '.join(sorted(seen))} – {ERROR_PATH_ALLOWED[method]}"))
        elif seen:
            rows.append((method, "NUR FEHLER", ", ".join(sorted(seen))))
            problems.append(f"{method}: nur Fehler ({', '.join(sorted(seen))})")
        else:
            rows.append((method, "FEHLT", ""))
            problems.append(f"{method}: nicht aufgerufen und nicht ausgenommen")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    with open(out / "api-coverage.tsv", "w", encoding="utf-8") as tsv:
        tsv.write("method\tstatus\tdetail\n")
        for row in rows:
            tsv.write("\t".join(row) + "\n")
    with open(out / "api-coverage.md", "w", encoding="utf-8") as md:
        md.write("# api.md coverage of the Android task suite\n\n| Methode | Status | Details |\n|---|---|---|\n")
        for method, state, detail in rows:
            md.write(f"| `{method}` | {state} | {detail.replace('|', '/')} |\n")
    called = sum(1 for row in rows if row[1] == "aufgerufen")
    print(f"api.md coverage: {called}/{len(methods)} aufgerufen, "
          f"{sum(1 for row in rows if row[1] == 'nur Fehlerpfad')} nur Fehlerpfad (erlaubt), "
          f"{sum(1 for row in rows if row[1] == 'ausgenommen')} ausgenommen")
    unknown = sorted(set(outcomes) - set(methods))
    if unknown:
        problems.append(f"Aufrufe außerhalb von api.md: {unknown}")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    return 0


def shell_tokens(block: str, version: str) -> list:
    return [token.replace("$ver", version).replace("${ver}", version) for token in shlex.split(block.replace("\\\n", " "))]


def cmd_release_lists(args) -> int:
    repo = Path(args.repo)
    data = json.loads(Path(args.plan).read_text(encoding="utf-8-sig"))
    version = data["version"]
    assets = data["assets"]
    commit_paths = set(data["commitPaths"])
    problems = []
    local = {asset["local"] for asset in assets}
    names = {Path(asset["local"]).name for asset in assets}
    if len(assets) != 20 or len(local) != 20 or len(names) != 20:
        problems.append(f"Get-PublicationReleaseAssetMap: {len(assets)} Assets / {len(local)} Pfade / {len(names)} Namen statt 20")
    if len(commit_paths) != 27:
        problems.append(f"Get-PublicationReleaseCommitPaths: {len(commit_paths)} statt 27 Pfade")
    missing_commit = sorted(p for p in local if p.startswith("release-native/") and p not in commit_paths)
    if missing_commit:
        problems.append(f"Assets ohne Commit-Pfad: {missing_commit}")
    for required in ("release-native/update-feed/smart-explorer-android.apk", "release-native/update-feed/smart-explorer-android.apk.sha256"):
        if required not in local or required not in commit_paths:
            problems.append(f"{required} fehlt in Asset-Map oder Commit-Pfaden")

    workflow = (repo / ".github/workflows/build.yml").read_text(encoding="utf-8")
    version_expr = "${{ needs.release-candidate.outputs.version }}"
    files = re.search(r"\n(\s+)files: \|\n((?:\1\s+\S.*\n)+)", workflow)
    if not files:
        problems.append("build.yml: `files:`-Liste der Veröffentlichung nicht gefunden")
    else:
        published = [line.strip().replace(version_expr, version) for line in files.group(2).splitlines() if line.strip()]
        published_names = {entry.removeprefix("native/out/") for entry in published}
        if len(published) != 20 or published_names != names:
            problems.append(f"build.yml files: {len(published)} Einträge; Abweichung {sorted(published_names ^ names)}")

    mappings = re.findall(r"asset_mappings=\(\n(.*?)\n\s*\)", workflow, re.S)
    if not mappings:
        problems.append("build.yml: asset_mappings nicht gefunden")
    for block in mappings:
        pairs = [token.split("|", 1) for token in shell_tokens(block, version)]
        sources = {pair[0] for pair in pairs}
        targets = {Path(pair[1]).name for pair in pairs if len(pair) == 2}
        if len(pairs) != 20 or sources != local or targets != names:
            problems.append(f"build.yml asset_mappings: {len(pairs)} Einträge; Abweichung {sorted(sources ^ local)}")

    allowed = re.findall(r"for path in \\\n(.*?); do\s*\n\s*allowed_release_change\[", workflow, re.S)
    if not allowed:
        problems.append("build.yml: allowed_release_change-Liste nicht gefunden")
    for block in allowed:
        paths = set(shell_tokens(block, version))
        if paths != commit_paths:
            problems.append(f"build.yml allowed_release_change: Abweichung {sorted(paths ^ commit_paths)}")

    feed_assets = {Path(p).name for p in local if p.startswith("release-native/update-feed/")}
    staged = re.findall(r"for asset in \\\n(.*?); do\s*\n\s*cp \"\.\./release-native/update-feed/\$asset\"", workflow, re.S)
    if not staged:
        problems.append("build.yml: Feed-Staging-Liste nicht gefunden")
    for block in staged:
        tokens = set(shell_tokens(block, version))
        if tokens != feed_assets:
            problems.append(f"build.yml Feed-Staging: Abweichung {sorted(tokens ^ feed_assets)}")

    windows = re.findall(r'Source = "\.\\([^"]+)"; Staged = "\.\\release-candidate\\([^"]+)"', workflow)
    windows_sources = {source.replace("\\", "/").replace("$version", version) for source, _ in windows}
    if len(windows) != 20 or windows_sources != local:
        problems.append(f"build.yml Windows-Staging: {len(windows)} Einträge; Abweichung {sorted(windows_sources ^ local)}")

    # release-version.ps1 is dot-sourced the same way by the Android job and the wrapper: first
    # release-publication.ps1, then release-version.ps1.
    wrapper = (repo / "native/publish-release-local.ps1").read_text(encoding="utf-8")
    sourcing = {
        "build.yml": (workflow, r'(?m)^\s*\.\s+\(Join-Path \$scriptRoot "release-publication\.ps1"\)',
                      r'(?m)^\s*\.\s+\(Join-Path \$scriptRoot "release-version\.ps1"\)'),
        "publish-release-local.ps1": (wrapper, r'(?m)^\s*\.\s+\$publicationHelper\s*$', r'(?m)^\s*\.\s+\$releaseVersionHelper\s*$'),
    }
    for label, (text, first_pattern, second_pattern) in sourcing.items():
        first = re.search(first_pattern, text)
        second = re.search(second_pattern, text)
        if not first or not second or first.start() > second.start():
            problems.append(f"{label}: release-publication.ps1 muss vor release-version.ps1 dot-gesourct werden")
    if 'Join-Path $scriptRoot "release-publication.ps1"' not in wrapper or 'Join-Path $scriptRoot "release-version.ps1"' not in wrapper:
        problems.append("publish-release-local.ps1: Helfer-Pfade für release-publication.ps1/release-version.ps1 fehlen")
    agents = (repo / "AGENTS.md").read_text(encoding="utf-8")
    if "smart-explorer-android.apk" not in agents:
        problems.append("AGENTS.md: Android-APK fehlt in der Liste der Release-Assets")
    print(f"release lists: {len(assets)} Assets, {len(commit_paths)} Commit-Pfade geprüft")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("classes")
    p.add_argument("--sources", required=True)
    p = sub.add_parser("instrumentation")
    p.add_argument("--name", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--sources", required=True)
    p.add_argument("--classes", required=True)
    p.add_argument("--summary", required=True)
    p = sub.add_parser("unit-tests")
    p.add_argument("--results", required=True)
    p.add_argument("--sources", required=True)
    p = sub.add_parser("coverage")
    p.add_argument("--api", required=True)
    p.add_argument("--calls", nargs="+", required=True)
    p.add_argument("--out", required=True)
    p = sub.add_parser("release-lists")
    p.add_argument("--repo", required=True)
    p.add_argument("--plan", required=True)
    args = parser.parse_args()
    handlers = {
        "classes": cmd_classes,
        "instrumentation": cmd_instrumentation,
        "unit-tests": cmd_unit_tests,
        "coverage": cmd_coverage,
        "release-lists": cmd_release_lists,
    }
    return handlers[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
