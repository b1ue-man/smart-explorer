#!/usr/bin/env bash
# Single task-level suite for the streaming transfer engine batch
# (docs/superpowers/plans/2026-09-28-transfer-engine/). Run this one checked-in
# entrypoint on the remote runner with an outer timeout of at least 30 minutes.
#
#   --check   narrow compile check while the batch is being built: the batch
#             gates only (rustfmt of every changed file, clippy for the host and
#             the Windows target over all targets, diagnostics on changed lines
#             fail). No tests run.
#   (none)    the complete suite: batch gates plus every milestone test.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-transfer-engine-task.sh [--check] [--bounded|--direct]" >&2
    echo "  --check    batch gates only (compile, rustfmt, clippy on changed lines)" >&2
    echo "  --bounded  use native/run-task-memory-bounded.sh (default for local users)" >&2
    echo "  --direct   rely on the remote runner's resource controls" >&2
}

execution_mode=bounded
suite_mode=suite
for argument in "$@"; do
    case "$argument" in
        --check) suite_mode=check ;;
        --bounded) execution_mode=bounded ;;
        --direct) execution_mode=direct ;;
        *) usage; exit 2 ;;
    esac
done
if [[ "$suite_mode" != check ]]; then
    echo "the milestone suite is not defined yet; use --check" >&2
    exit 2
fi

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) platform=windows ;;
    *) platform=linux ;;
esac

report_failure() {
    local status=$?
    echo "transfer engine task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-transfer-engine-task.XXXXXX")"
fi
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -f "$suite_tmp/batch-ranges.txt" "$suite_tmp"/clippy-*.log
        rmdir "$suite_tmp" 2>/dev/null || true
    else
        echo "transfer engine task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in awk cargo git grep mktemp rustfmt sed sort tee uname; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "$command_name is required" >&2
        exit 1
    }
done
if [[ "$execution_mode" == bounded && "$platform" == linux ]]; then
    test -x "$repo_root/native/run-task-memory-bounded.sh" || {
        echo "task memory wrapper is missing or not executable" >&2
        exit 1
    }
fi

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_TERM_COLOR=never

run_task() {
    if [[ "$execution_mode" == bounded && "$platform" == linux ]]; then
        "$repo_root/native/run-task-memory-bounded.sh" "$@"
    else
        "$@"
    fi
}

echo "transfer engine task suite: batch gates ($platform, $suite_mode)"
# The crate as a whole carries older formatting and dead-code drift outside
# this batch (docs/TODO.md, H1), so the gates cover exactly what this batch
# touched: every changed file is format-checked on its own through rustfmt's
# stdin mode, and clippy runs over all targets of the crate, but only
# diagnostics on lines this batch changed fail. A compile error anywhere still
# fails. The batch base is the last commit before the batch.
batch_base=6535f767c08314e051e99b1c53ec634c6b8a9525
if ! git -C "$repo_root" cat-file -e "${batch_base}^{commit}" 2>/dev/null; then
    git -C "$repo_root" fetch --quiet --depth=1 origin "$batch_base"
fi
mapfile -t batch_files < <(
    git -C "$repo_root" diff --name-only --diff-filter=AM "$batch_base" HEAD -- 'native/src/*.rs'
)
if [[ "${#batch_files[@]}" -eq 0 ]]; then
    echo "no batch source files found relative to $batch_base" >&2
    exit 1
fi
format_failures=0
for batch_file in "${batch_files[@]}"; do
    format_diff="$(rustfmt --check --color never --edition 2021 < "$repo_root/$batch_file")"
    if [[ -n "$format_diff" ]]; then
        printf '%s\n' "$format_diff" | sed "s#<stdin>#$batch_file#"
        format_failures=$((format_failures + 1))
    fi
done
if [[ "$format_failures" -ne 0 ]]; then
    echo "$format_failures batch source files are not rustfmt-clean" >&2
    exit 1
fi
echo "transfer engine task suite: ${#batch_files[@]} batch source files are rustfmt-clean"

batch_ranges="$suite_tmp/batch-ranges.txt"
: > "$batch_ranges"
for batch_file in "${batch_files[@]}"; do
    git -C "$repo_root" diff -U0 "$batch_base" HEAD -- "$batch_file" | awk -v file="${batch_file#native/}" '
        /^@@ / {
            split($3, plus, ",")
            start = substr(plus[1], 2) + 0
            count = (length(plus) > 1) ? plus[2] + 0 : 1
            if (count > 0) {
                print file, start, start + count - 1
            }
        }' >> "$batch_ranges"
done
batch_diagnostics() {
    local log=$1
    { grep -E '^src/[^:]+:[0-9]+:[0-9]+: (warning|error)' "$log" || true; } | sort -u | awk -F: -v ranges="$batch_ranges" '
        BEGIN {
            while ((getline line < ranges) > 0) {
                split(line, range, " ")
                n++
                range_file[n] = range[1]
                range_start[n] = range[2]
                range_end[n] = range[3]
            }
        }
        {
            for (i = 1; i <= n; i++) {
                if ($1 == range_file[i] && $2 >= range_start[i] && $2 <= range_end[i]) {
                    print
                    break
                }
            }
        }'
}
clippy_targets=(host)
if [[ "$platform" == linux ]]; then
    if ! rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$'; then
        echo "x86_64-pc-windows-gnu target is required on Linux" >&2
        exit 1
    fi
    command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || {
        echo "x86_64-w64-mingw32-gcc is required on Linux" >&2
        exit 1
    }
    clippy_targets+=(x86_64-pc-windows-gnu)
fi
for clippy_target in "${clippy_targets[@]}"; do
    clippy_log="$suite_tmp/clippy-$clippy_target.log"
    target_arguments=()
    if [[ "$clippy_target" != host ]]; then
        target_arguments=(--target "$clippy_target")
    fi
    (
        cd "$repo_root/native"
        run_task cargo clippy --locked "${target_arguments[@]}" --all-targets \
            --message-format short
    ) 2>&1 | tee "$clippy_log"
    batch_diagnostic_lines="$(batch_diagnostics "$clippy_log")"
    if [[ -n "$batch_diagnostic_lines" ]]; then
        printf '%s\n' "$batch_diagnostic_lines" >&2
        echo "clippy ($clippy_target) reported diagnostics on lines this batch changed" >&2
        exit 1
    fi
    echo "transfer engine task suite: clippy ($clippy_target) is clean on the lines this batch changed"
done

suite_succeeded=true
echo "transfer engine task suite: batch gates passed ($suite_mode)"
