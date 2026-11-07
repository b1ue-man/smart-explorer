#!/usr/bin/env python3
"""The test register (docs/tests/register.tsv) names every test of the repository.

Each row names the requirement the test protects, its verdict and the suite that runs it. With the
register, a failing test can be told apart: either it is outdated, or it shows a real problem
(TESTS-BEWERTUNG, docs/tests/README.md).

This check fails when:
- a test exists in the code without a row;
- a row names a test that no longer exists;
- a row is malformed;
- a row names a suite that has no workflow.

It reads source text only: it compiles nothing and starts nothing. crates-task runs it on Linux;
`python3 native/test-register.py` runs it anywhere.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REGISTER = REPO / "docs" / "tests" / "register.tsv"
HEADER = ["Datei", "Test", "Urteil", "Kategorie", "Schützt", "Suite"]
VERDICTS = {"behalten", "unklar"}
CATEGORY = re.compile(r"^(K[1-5](/K[1-5])*|U)$")
RUST_ROOTS = ("native/crates", "native/src", "native/tests", "share-server", "se-agent")
KOTLIN_ROOTS = ("android/app/src/test", "android/app/src/androidTest")
# A Rust test: #[test] or #[tokio::test(...)], further attributes, then the function.
RUST_TEST = re.compile(r"#\[(?:tokio::)?test\b[^\]]*\]((?:\s*#\[[^\]]*\])*)\s*"
                       r"(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)")
KOTLIN_TEST = re.compile(r"@Test\b(?:\([^)]*\))?((?:\s*@\w+(?:\([^)]*\))?)*)\s*"
                         r"(?:(?:public|internal|private)\s+)?fun\s+`?([^`(]+)`?\s*\(")
# "keine (<reason>)" marks a test no suite runs, with the reason; every other value is a workflow name.
NO_SUITE = re.compile(r"^keine \(.+\)$")


def tests_in_code() -> set[tuple[str, str]]:
    found: set[tuple[str, str]] = set()
    for roots, pattern, suffix in ((RUST_ROOTS, RUST_TEST, "*.rs"), (KOTLIN_ROOTS, KOTLIN_TEST, "*.kt")):
        for root in roots:
            base = REPO / root
            if not base.is_dir():
                continue
            for path in sorted(base.rglob(suffix)):
                if "/target/" in path.as_posix():
                    continue
                text = path.read_text(encoding="utf-8", errors="replace")
                relative = path.relative_to(REPO).as_posix()
                for match in pattern.finditer(text):
                    found.add((relative, match.group(2).strip()))
    return found


def workflows() -> set[str]:
    return {path.stem for path in (REPO / ".github" / "workflows").glob("*.yml")}


def main() -> int:
    if not REGISTER.is_file():
        print(f"register missing: {REGISTER.relative_to(REPO)}")
        return 1
    lines = REGISTER.read_text(encoding="utf-8").splitlines()
    problems: list[str] = []
    if not lines or lines[0].split("\t") != HEADER:
        problems.append(f"line 1: header must be {chr(9).join(HEADER)!r}")
    known = workflows()
    rows: dict[tuple[str, str], int] = {}
    for number, line in enumerate(lines[1:], 2):
        cells = line.split("\t")
        if len(cells) != len(HEADER):
            problems.append(f"line {number}: {len(cells)} columns instead of {len(HEADER)}")
            continue
        path, name, verdict, category, protects, suite = cells
        key = (path, name)
        if key in rows:
            problems.append(f"line {number}: {path} {name} also in line {rows[key]}")
        rows[key] = number
        if verdict not in VERDICTS:
            problems.append(f"line {number}: verdict {verdict!r} (allowed: {', '.join(sorted(VERDICTS))})")
        if not CATEGORY.match(category):
            problems.append(f"line {number}: category {category!r} (K1 to K5, joined with '/', or U)")
        if (verdict == "unklar") != (category == "U"):
            problems.append(f"line {number}: verdict {verdict} with category {category}")
        if not protects.strip():
            problems.append(f"line {number}: no requirement in column Schützt")
        for value in (part.strip() for part in suite.split(",")):
            if not (value in known or NO_SUITE.match(value)):
                problems.append(f"line {number}: suite {value!r} is no workflow under .github/workflows "
                                "and not 'keine (<reason>)'")
    code = tests_in_code()
    for path, name in sorted(code - rows.keys()):
        problems.append(f"{path}: test {name} has no row in docs/tests/register.tsv")
    for path, name in sorted(rows.keys() - code):
        problems.append(f"line {rows[(path, name)]}: test {name} no longer exists in {path}")
    if problems:
        print(f"test register: {len(problems)} problems")
        for problem in problems[:200]:
            print(f"- {problem}")
        if len(problems) > 200:
            print(f"- ... {len(problems) - 200} more")
        return 1
    print(f"test register: {len(rows)} tests, every one in the code and every one registered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
