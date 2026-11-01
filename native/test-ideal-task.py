#!/usr/bin/env python3
"""Task-level checks of the 2026-11-01 batch (.github/workflows/ideal-task.yml).

--job clippy   cargo clippy on library, binaries and tests; a finding on a line this
               batch changed since --since fails (older debt elsewhere does not).
--job measure  `se sync compare` of two copies of one tree whose times differ, with no
               stored state: every file pair has equal size and must be decided by
               content. The released --old-tag binary (its release asset, checked
               against the published .sha256) and the candidate --new run on the same
               trees in fresh configuration folders; their results must be equal.
               Wall time and CPU use per run go to summary.md (median of 3).
"""
import argparse, hashlib, json, os, re, shutil, statistics, subprocess, sys, tempfile, time
from pathlib import Path

WINDOWS = os.name == "nt"
REPO = Path(__file__).resolve().parent.parent
SMALL_FILES, SMALL_SIZE = 4000, 64 << 10
LARGE_FILES, LARGE_SIZE = 8, 32 << 20
SHIFT = 4000  # seconds between the copies' times: not an FAT hour, so content decides
REPEAT = 3


def clippy(since):
    diff = subprocess.run(["git", "diff", "-U0", "--no-color", since, "HEAD", "--", "native/src"],
                          cwd=REPO, capture_output=True, text=True, check=True).stdout
    changed, current = {}, None
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[4:].strip()
            current = path[len("b/native/"):] if path.startswith("b/native/") else None
        elif line.startswith("@@") and current:
            m = re.search(r"\+(\d+)(?:,(\d+))?", line)
            start, count = int(m.group(1)), int(m.group(2) or 1)
            changed.setdefault(current, set()).update(range(start, start + count))
    ran = subprocess.run(["cargo", "clippy", "--lib", "--bins", "--tests", "--message-format", "short"],
                         cwd=REPO / "native", capture_output=True, text=True)
    out = (ran.stdout + ran.stderr).replace("\\", "/")
    print(out[-20000:])
    if re.search(r"(^|: )error\[E\d+\]", out, re.M) or ran.returncode != 0 and "error: could not compile" in out:
        print("::error::compile errors in clippy")
        return 1
    findings = sorted({line for line in out.splitlines()
                       if (m := re.match(r"^((?:src|tests)/[^:]+):(\d+):\d+: (error|warning)", line))
                       and int(m.group(2)) in changed.get(m.group(1), ())})
    if findings:
        print("::error::clippy findings on changed lines")
        print("\n".join(findings))
        return 1
    print("clippy clean on changed lines")
    return 0


def content(seed, size):
    block = hashlib.sha256(str(seed).encode()).digest() * 2048  # 64 KiB of seed-dependent bytes
    return (block * (size // len(block) + 1))[:size]


def put(path, data, mtime):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    os.utime(path, (mtime, mtime))


def make_trees(root):
    a, b = root / "a", root / "b"
    base = int(time.time()) - 86400
    for index in range(SMALL_FILES + LARGE_FILES):
        size = LARGE_SIZE if index < LARGE_FILES else SMALL_SIZE
        data = content(index, size)
        rel = Path(f"d{index % 40:02}") / f"f{index:05}.bin"
        put(a / rel, data, base)
        put(b / rel, data, base + SHIFT)
    put(a / "differs.bin", content("a", SMALL_SIZE), base)
    put(b / "differs.bin", content("b", SMALL_SIZE), base + SHIFT)
    return a, b


def env_for(config):
    env = dict(os.environ)
    if WINDOWS:
        for name in ("APPDATA", "LOCALAPPDATA"):
            env[name] = str(config / name)
            (config / name).mkdir(parents=True, exist_ok=True)
    else:
        env["HOME"] = str(config)
        env["XDG_CONFIG_HOME"] = str(config / ".config")
        env["XDG_DATA_HOME"] = str(config / ".local" / "share")
    return env


def compare(binary, a, b, scratch, log):
    config = Path(tempfile.mkdtemp(dir=scratch, prefix="cfg-"))
    env = env_for(config)
    added = subprocess.run([binary, "sync", "add", "--name", "T", "--source", str(a), "--target", str(b),
                            "--trigger", "manual", "--enabled", "false"], env=env, capture_output=True, text=True)
    if added.returncode != 0:
        raise SystemExit(f"sync add failed: {added.stderr[-500:]}")
    cpu_before = os.times()
    started = time.monotonic()
    ran = subprocess.run([binary, "sync", "compare", "T", "--json"], env=env, capture_output=True, text=True)
    wall = time.monotonic() - started
    cpu_after = os.times()
    cpu = (cpu_after.children_user - cpu_before.children_user) + (cpu_after.children_system - cpu_before.children_system)
    log.write(f"{binary} rc={ran.returncode} wall={wall:.2f}s cpu={cpu:.2f}s\n{ran.stderr[-2000:]}\n")
    if ran.returncode != 0:
        raise SystemExit(f"sync compare failed: {ran.stderr[-800:]}")
    shutil.rmtree(config, ignore_errors=True)
    data = json.loads(ran.stdout)
    decided = sorted(json.dumps(item, sort_keys=True) for key in ("actions", "conflicts") for item in data.get(key, []))
    return wall, cpu, decided


def released(tag, scratch):
    name = "se.exe" if WINDOWS else "se"
    folder = Path(scratch) / "old"
    folder.mkdir(exist_ok=True)
    for asset in (name, name + ".sha256"):
        subprocess.run(["gh", "release", "download", tag, "-R", os.environ.get("GITHUB_REPOSITORY", "b1ue-man/smart-explorer"),
                        "-p", asset, "-D", str(folder), "--clobber"], check=True)
    expected = (folder / (name + ".sha256")).read_text().split()[0].lower()
    actual = hashlib.sha256((folder / name).read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"{tag} {name}: SHA-256 {actual} differs from the published {expected}")
    if not WINDOWS:
        (folder / name).chmod(0o755)
    return str(folder / name)


def drop_caches():
    if WINDOWS:
        return False
    return subprocess.run(["sudo", "-n", "sh", "-c", "sync; echo 3 > /proc/sys/vm/drop_caches"]).returncode == 0


def measure(old_tag, new, log_root):
    log_root = Path(log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="ideal-"))
    old = released(old_tag, scratch)
    new = str(Path(new).resolve()) + (".exe" if WINDOWS and not new.endswith(".exe") else "")
    a, b = make_trees(scratch)
    cores = os.cpu_count()
    rows, failures = [], []
    with open(log_root / "runs.log", "w", encoding="utf-8") as log:
        for state in (("warm", "cold") if not WINDOWS else ("warm",)):
            results = {"old": [], "new": []}
            for _ in range(REPEAT):
                for label, binary in (("old", old), ("new", new)):
                    if state == "cold" and not drop_caches():
                        continue
                    results[label].append(compare(binary, a, b, scratch, log))
            if not results["old"] or not results["new"]:
                continue
            if results["old"][0][2] != results["new"][0][2]:
                failures.append(f"{state}: the candidate decides differently than {old_tag}")
            if [entry for entry in results["new"][0][2] if "differs.bin" not in entry] or not results["new"][0][2]:
                failures.append(f"{state}: expected exactly differs.bin to be reported, got {results['new'][0][2][:5]}")
            for label in ("old", "new"):
                walls = [r[0] for r in results[label]]
                cpus = [r[1] for r in results[label]]
                wall, cpu = statistics.median(walls), statistics.median(cpus)
                rows.append((state, old_tag if label == "old" else "candidate", wall, cpu, cpu / wall / cores * 100))
    files = SMALL_FILES + LARGE_FILES + 1
    size = (SMALL_FILES * SMALL_SIZE + LARGE_FILES * LARGE_SIZE + SMALL_SIZE) / (1 << 20)
    lines = [f"### First compare of two copies with other times ({'Windows' if WINDOWS else 'Linux'}, {cores} cores)",
             "", f"{files} file pairs per side, {size:.0f} MiB per side; median of {REPEAT}.", "",
             "| cache | binary | wall s | CPU s | CPU share of all cores |", "|---|---|---|---|---|"]
    # Windows reports no CPU time of finished children (os.times), so the share is n/a there.
    lines += [f"| {s} | {b} | {w:.2f} | {'n/a' if WINDOWS else f'{c:.2f}'} | {'n/a' if WINDOWS else f'{p:.0f} %'} |"
              for s, b, w, c, p in rows]
    if failures:
        lines += ["", "**Failures:**"] + [f"- {failure}" for failure in failures]
    (log_root / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    shutil.rmtree(scratch, ignore_errors=True)
    return 1 if failures else 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--job", required=True, choices=["clippy", "measure"])
    parser.add_argument("--since", default="v0.5.190")
    parser.add_argument("--old-tag", default="v0.5.190")
    parser.add_argument("--new", default="native/target/release/se")
    parser.add_argument("--log-root", default="ideal-measure")
    args = parser.parse_args()
    if args.job == "clippy":
        return clippy(args.since)
    return measure(args.old_tag, args.new, args.log_root)


if __name__ == "__main__":
    sys.exit(main())
