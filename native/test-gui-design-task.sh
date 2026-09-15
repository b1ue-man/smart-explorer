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

# An explicitly reused binary must be bound to this source and to its bytes.
# Otherwise Cargo discovers/reuses the host library development output. There
# is no workspace, cross-platform, all-target or release build in this suite.
if [[ -n "${SMART_EXPLORER_TASK_BINARY:-}" ]]; then
    : "${SMART_EXPLORER_TASK_BINARY_SHA256:?required for an existing binary}"
    : "${SMART_EXPLORER_TASK_SOURCE_SHA:?required for an existing binary}"
    [[ "$SMART_EXPLORER_TASK_SOURCE_SHA" == "$candidate" ]]
    binary="$(realpath "$SMART_EXPLORER_TASK_BINARY")"
    [[ "$(sha256sum "$binary" | cut -d ' ' -f 1)" == "$SMART_EXPLORER_TASK_BINARY_SHA256" ]]
else
    cargo test --manifest-path native/Cargo.toml --locked --lib --no-run \
        --message-format=json-render-diagnostics > "$log_root/build.jsonl" 2> "$log_root/build.log"
    binary="$(python3 - "$log_root/build.jsonl" <<'PY'
import json, sys
artifacts = [json.loads(line) for line in open(sys.argv[1]) if line.startswith('{')]
binaries = [row['executable'] for row in artifacts
            if row.get('reason') == 'compiler-artifact' and row.get('executable')
            and row.get('target', {}).get('name') == 'smart_explorer'
            and row.get('profile', {}).get('test')]
assert len(binaries) == 1, binaries
print(binaries[0])
PY
    )"
fi

"$binary" gui_design_task_ --include-ignored --nocapture --test-threads=1 \
    2>&1 | tee "$log_root/behavior.log"

python3 - "$log_root" "$candidate" "$binary" <<'PY'
import hashlib, json, pathlib, re, sys
logs, candidate, binary = pathlib.Path(sys.argv[1]), sys.argv[2], pathlib.Path(sys.argv[3])
sources = ['native/src/app/core/gui_design_task_tests.rs',
           'native/src/app/core/gui_design_task_ui.rs',
           'native/src/gdrive/core/gui_task_tests.rs']
expected = [name for source in sources for name in re.findall(
    r'fn (gui_design_task_\w+)\(', pathlib.Path(source).read_text())]
output = (logs / 'behavior.log').read_text()
for name in expected:
    assert re.search(r'test [^\n]*::' + re.escape(name) + r' \.\.\. ok', output), name
assert f'{len(expected)} passed; 0 failed; 0 ignored;' in output
report = {'candidate': candidate, 'binary': str(binary),
          'binary_sha256': hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest(),
          'milestones': {'D1': 'compact neutral shell, same-row toolbar, search focus, filtering and split panes',
                         'D2': 'unboxed locations, capacity alignment, settings and bounded feature dialogs',
                         'D3': 'Drive empty-page semantics, safe names/mutations and full scan-report clipboard',
                         'D4': 'source-bound real egui geometry and software-rendered visual artifacts'},
          'expected_cases': expected}
(logs / 'approval.json').write_text(json.dumps(report, indent=2) + '\n')
PY
