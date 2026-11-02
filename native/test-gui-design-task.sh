#!/usr/bin/env bash
# The single remote suite for the desktop GUI correction and Drive follow-up.
# Codex must not invoke this entrypoint on a workstation.
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
candidate="$(git rev-parse HEAD)"
log_root="${SMART_EXPLORER_TASK_LOG_ROOT:-$(mktemp -d)}"
mkdir -p "$log_root/visuals" "$log_root/data"
log_root="$(realpath "$log_root")"
export SMART_EXPLORER_GUI_TASK=1
export SMART_EXPLORER_GUI_VISUALS="$log_root/visuals"
export XDG_DATA_HOME="$log_root/data"
export APPDATA="$log_root/data"
export LOCALAPPDATA="$log_root/data"
export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_TERM_COLOR=never

finish() {
    local status=$?
    trap - EXIT
    /usr/bin/python3 native/gui-task-render.py "$log_root/visuals" || status=1
    echo "GUI / Drive task evidence: $log_root"
    exit "$status"
}
trap finish EXIT

# The library tests of the workspace crates run through cargo: no supplied or discovered program,
# and the root package and the Android bridge are excluded (no release or cross-platform build).
cargo test --manifest-path native/Cargo.toml --locked --no-fail-fast --workspace --exclude smart_explorer_android --exclude smart_explorer --lib -- gui_design_task_ --include-ignored --nocapture --test-threads=1 \
    2>&1 | tee "$log_root/behavior.log"

python3 - "$log_root" "$candidate" <<'PY'
import hashlib, json, pathlib, re, sys
logs, candidate = pathlib.Path(sys.argv[1]), sys.argv[2]
sources = ['native/crates/gui/src/app/core/gui_design_task_tests.rs',
           'native/crates/gui/src/app/core/gui_design_task_ui.rs',
           'native/crates/engine/src/gdrive/core/gui_task_tests.rs']
expected = [name for source in sources for name in re.findall(
    r'fn (gui_design_task_\w+)\(', pathlib.Path(source).read_text())]
output = (logs / 'behavior.log').read_text()
for name in expected:
    assert re.search(r'test [^\n]*::' + re.escape(name) + r' \.\.\. ok', output), name
# One summary per test program of the workspace crates (D13: gui and engine).
summaries = re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', output, re.M)
assert summaries and all(state == 'ok' for state, *_ in summaries), summaries
assert sum(int(passed) for _, passed, _, _ in summaries) == len(expected), summaries
assert not any(int(failed) or int(ignored) for _, _, failed, ignored in summaries), summaries
report = {'candidate': candidate, 'test_command': 'cargo test --manifest-path native/Cargo.toml --locked --no-fail-fast --workspace --exclude smart_explorer_android --exclude smart_explorer --lib',
          'milestones': {'D1': 'compact neutral shell, same-row toolbar, search focus, filtering and split panes',
                         'D2': 'unboxed locations, capacity alignment, settings and bounded feature dialogs',
                         'D3': 'Drive empty-page semantics, safe names/mutations and full scan-report clipboard',
                         'D4': 'source-bound real egui geometry and software-rendered visual artifacts'},
          'expected_cases': expected}
(logs / 'approval.json').write_text(json.dumps(report, indent=2) + '\n')
PY
