#!/usr/bin/env python3
"""One remote-only task suite for the 2026-10-14 batch (analysis job id, plugin priority).

Plan and expected results: docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md (D).
Builds only the development `se` and the library test binary of this host (no
release build), then checks every milestone once:

  fmt     rustfmt of the batch's Rust files (stdin mode, no crate-wide debt)
  build   `se` plus the lib tests; compiler warnings inside batch files fail
  unit    M1: `analysis_id_task_` (id format; remote analysis and duplicate
          search between two real loopback Iroh nodes with this client's id,
          with the base64url id of clients 0.5.170 to 0.5.177, and the refusal
          of a path-like id), the directly affected wire and host analysis
          tests; M2: `mcp_task_` (catalog limits and `_meta`, skill frontmatter)
  mcp     M2 over the wire: `se mcp` under both lifecycles delivers the
          priority paragraph within the first 512 characters of its
          instructions and `_meta["anthropic/alwaysLoad"]` on exactly the file
          tools
  plugin  M2 as installed: `se plugin install claude` writes a skill and
          manifest that state the priority, and Claude Code validates the plugin

Helpers (results, process runner, isolated clients, MCP client) come from
test-ai-plugin-task.py. Every stage runs even when an earlier one failed; the
summary lists them all.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile

NATIVE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("ai_plugin_task", NATIVE / "test-ai-plugin-task.py")
ai = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ai)
# The shared fmt/build helpers read these module globals.
ai.BATCH_BASE = "fa6043efebeff6ad80971206d89e4162085e7f75"
ai.NEW_FILE_PREFIXES = ("src/share/core/analysis_request_id", "src/share/core/peer_storage_analysis.rs",
                        "src/share/core/peer_duplicates.rs", "src/share/os/shared/analysis_tasks.rs",
                        "src/cli/mcp/catalog.rs", "src/cli/plugin")
WINDOWS = ai.WINDOWS
ensure = ai.ensure

UNIT_FILTERS = ["analysis_id_task_", "remote_drive_task_wire_tests", "storage_analysis_host", "mcp_task_"]
UPFRONT = {"connections", "list", "stat", "read", "write", "edit", "mkdir", "remove", "copy", "move",
           "search", "job_status"}
# Words of the priority paragraph an agent must receive: the order (before)
# and the host tools it ranks below Smart Explorer.
PRIORITY_WORDS = ("first", "before", "Read", "Write", "Edit", "Glob", "ls", "cat", "find")


def stage_unit(results, log_root):
    completed = ai.run(["cargo", "test", "--locked", "--lib", "--", *UNIT_FILTERS],
                       cwd=ai.NATIVE, log=log_root / "unit.log", timeout=5400)
    output = (completed.stdout + completed.stderr).decode(errors="replace")
    summary = [line for line in output.splitlines() if line.startswith("test result:")]
    results.record("unit", " + ".join(UNIT_FILTERS), completed.returncode == 0,
                   output[-4000:] if completed.returncode else "; ".join(summary))
    for prefix, expected in (("analysis_id_task_", 3), ("mcp_task_", 15)):
        ran = [line for line in output.splitlines()
               if line.startswith("test ") and prefix in line and line.rstrip().endswith("ok")]
        results.record("unit", f"{prefix} tests passed", len(ran) >= expected, f"{len(ran)} passed")


def stage_mcp(results, se_binary, work, log_root):
    stage = "mcp"
    client = ai.Client(work, "mcp")
    logs = log_root / "mcp"
    logs.mkdir(parents=True, exist_ok=True)

    def catalog(lifecycle):
        def check():
            session = ai.McpSession(se_binary, client, lifecycle, logs, f"catalog-{lifecycle}")
            try:
                instructions = session.info.get("instructions", "")
                first = instructions.split("\n\n")[0]
                ensure(len(instructions) <= 2048, f"instructions {len(instructions)} characters")
                ensure(len(first) <= 512, f"first paragraph {len(first)} characters")
                missing = [word for word in PRIORITY_WORDS if word not in first]
                ensure(not missing, f"priority paragraph lacks {missing}: {first}")
                tools = session.tools()
                ensure([tool["name"] for tool in tools] == ai.EXPECTED_TOOLS, [tool["name"] for tool in tools])
                for tool in tools:
                    upfront = tool.get("_meta", {}).get("anthropic/alwaysLoad")
                    expected = True if tool["name"] in UPFRONT else None
                    ensure(upfront is expected, f"{tool['name']}: alwaysLoad {upfront!r}")
                    ensure(len(tool.get("description", "")) <= 2048, f"{tool['name']} description length")
            finally:
                ensure(session.close() == 0, "server exit code")
            return f"{len(first)}-character priority paragraph, {len(UPFRONT)} upfront tools"
        return check

    results.check(stage, "instructions and alwaysLoad over initialize 2025-06-18 (Codex)", catalog("initialize"))
    results.check(stage, "instructions and alwaysLoad over server/discover 2026-07-28 (Claude Code)",
                  catalog("modern"))
    ai.stop_background_daemons([client])


def stage_plugin(results, se_binary, work, log_root):
    stage = "plugin"
    logs = log_root / "plugin"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / "commands.log"
    client = ai.Client(work, "plugin")
    claude = shutil.which("claude")
    results.record(stage, "claude CLI on PATH", bool(claude), f"claude={claude}")
    if not claude:
        return
    plugin = client.app_data / "ai-plugins" / "claude" / "smart-explorer"

    def install():
        installed = client.se(se_binary, "plugin", "install", "claude", "--json", timeout=600, log=log)
        ensure(installed.returncode == 0, installed.stdout.decode() + installed.stderr.decode())
        skill = (plugin / "skills" / "smart-explorer" / "SKILL.md").read_text(encoding="utf-8")
        description = next(line for line in skill.splitlines() if line.startswith("description: "))
        missing = [word for word in PRIORITY_WORDS if word not in description]
        ensure(not missing, f"skill description lacks {missing}: {description}")
        ensure("## Priority" in skill, "skill has no priority section")
        manifest = json.loads((plugin / ".claude-plugin" / "plugin.json").read_text(encoding="utf-8"))
        ensure("before the built-in file tools" in manifest["description"], manifest)
        validated = ai.run([claude, "plugin", "validate", plugin], env=client.env, log=log, timeout=180)
        ensure(validated.returncode == 0, validated.stdout.decode() + validated.stderr.decode())
    results.check(stage, "se plugin install claude: skill and manifest state the priority, plugin validates",
                  install)

    def uninstall():
        removed = client.se(se_binary, "plugin", "uninstall", "claude", "--json", timeout=600, log=log)
        ensure(removed.returncode == 0, removed.stdout.decode() + removed.stderr.decode())
        ensure(not plugin.exists(), "generated files left")
    results.check(stage, "se plugin uninstall claude removes the generated plugin", uninstall)
    ai.stop_background_daemons([client])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-root", required=True)
    args = parser.parse_args()
    log_root = Path(args.log_root)
    log_root.mkdir(parents=True, exist_ok=True)
    results = ai.Results(log_root)
    ai.stage_fmt(results, log_root)
    built, _ = ai.stage_build(results, log_root)
    stage_unit(results, log_root)
    se_binary = ai.NATIVE / "target" / "debug" / ("se.exe" if WINDOWS else "se")
    if not built or not se_binary.exists():
        results.record("e2e", "development se binary", False, se_binary)
        return results.finish()
    work = Path(tempfile.mkdtemp(prefix="se-analysis-plugin-"))
    try:
        stage_mcp(results, se_binary, work, log_root)
        stage_plugin(results, se_binary, work, log_root)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return results.finish()


if __name__ == "__main__":
    sys.exit(main())
