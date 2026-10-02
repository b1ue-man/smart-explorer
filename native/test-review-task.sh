#!/usr/bin/env bash
# Single task-level suite of the review batch RV1 (remote analysis on the
# exporting host, sync reliability and real-time watching, Share security;
# docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/). Run this one
# checked-in entrypoint on the remote runner with an outer timeout of at least
# 30 minutes (.github/workflows/review-task.yml). Never run it on the
# workstation (AGENTS.md).
#
#   --check   narrow compile check while the batch is being built: rustfmt of
#             every changed Rust file, clippy over all targets of the crate for
#             the host and the Windows target (diagnostics on changed lines are
#             reported, compile errors fail) and the Share server's clippy.
#             No tests run.
#   (none)    the complete suite (milestone tests `review_task_`, affected
#             modules, end-to-end stages); its stages are added once the batch
#             is implemented.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-review-task.sh [--check] [--bounded|--direct]" >&2
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

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) platform=windows ;;
    *) platform=linux ;;
esac

report_failure() {
    local status=$?
    echo "review task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-review-task.XXXXXX")"
fi
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -f "$suite_tmp/batch-ranges.txt" "$suite_tmp/server-ranges.txt" "$suite_tmp"/clippy-*.log
        rmdir "$suite_tmp" 2>/dev/null || true
    else
        echo "review task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in awk cargo git grep mktemp rustfmt sed sort tee timeout tr uname; do
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

echo "review task suite: batch gates ($platform, $suite_mode)"
# The crate as a whole carries older formatting and dead-code drift outside
# this batch (docs/TODO.md, H1), so the gates cover exactly what this batch
# touched: every changed file is format-checked on its own through rustfmt's
# stdin mode, and clippy runs over all targets, but only diagnostics on lines
# this batch changed are reported. A compile error anywhere fails. The batch
# base is the last commit before the batch.
batch_base=7fc17ecf355b6473a506d756ad4c1fb2d2c0d173
if ! git -C "$repo_root" cat-file -e "${batch_base}^{commit}" 2>/dev/null; then
    git -C "$repo_root" fetch --quiet --depth=1 origin "$batch_base"
fi
mapfile -t batch_files < <(
    git -C "$repo_root" diff --name-only --diff-filter=AM "$batch_base" HEAD -- \
        'native/src/*.rs' 'native/tests/*.rs' 'native/android-bridge/src/*.rs' \
        'share-server/src/*.rs'
)
if [[ "${#batch_files[@]}" -eq 0 ]]; then
    if [[ "$suite_mode" != check ]]; then
        echo "no batch source files found relative to $batch_base" >&2
        exit 1
    fi
    # A check of the unchanged base only proves that every target compiles.
    echo "review task suite: no batch source files yet; compile check only"
fi
# In the complete suite a failing stage is recorded and the next one still
# runs, so one remote run reports every problem; the suite fails at its end.
# The check mode also records and goes on, so one run shows every compile
# problem of every target.
failed_stages=()
stage_failed() {
    failed_stages+=("$1")
    echo "review task suite: FAILED: $1" >&2
}
finish_suite() {
    if [[ "${#failed_stages[@]}" -ne 0 ]]; then
        printf 'review task suite: failed stage: %s\n' "${failed_stages[@]}" >&2
        exit 1
    fi
    suite_succeeded=true
    echo "$1"
}

format_failures=0
for batch_file in "${batch_files[@]}"; do
    format_diff="$(rustfmt --check --color never --edition 2021 < "$repo_root/$batch_file" || true)"
    if [[ -n "$format_diff" ]]; then
        printf '%s\n' "$format_diff" | sed "s#<stdin>#$batch_file#"
        format_failures=$((format_failures + 1))
    fi
done
if [[ "$format_failures" -ne 0 ]]; then
    stage_failed "rustfmt: $format_failures batch source files are not rustfmt-clean"
else
    echo "review task suite: ${#batch_files[@]} batch source files are rustfmt-clean"
fi

batch_ranges="$suite_tmp/batch-ranges.txt"
: > "$batch_ranges"
for batch_file in "${batch_files[@]}"; do
    # Share-server ranges are kept apart (server-ranges.txt): both crates
    # have a src/main.rs, so one shared list could match the wrong file.
    case "$batch_file" in
        native/src/*|native/tests/*) ;;
        *) continue ;;
    esac
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
    batch_diagnostics_with "$batch_ranges" "$1"
}
batch_diagnostics_with() {
    local ranges_file=$1 log=$2
    # Cargo prints Windows paths with backslashes; the ranges use slashes.
    { tr '\\' '/' < "$log" | grep -E '^(src|tests)/[^:]+:[0-9]+:[0-9]+: (warning|error)' || true; } |
        sort -u | awk -F: -v ranges="$ranges_file" '
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
    if ! (
        cd "$repo_root/native"
        run_task cargo clippy --locked "${target_arguments[@]}" --all-targets \
            --message-format short -- --cap-lints warn
    ) 2>&1 | tee "$clippy_log"; then
        stage_failed "clippy ($clippy_target) did not complete"
        continue
    fi
    batch_diagnostic_lines="$(batch_diagnostics "$clippy_log")"
    if [[ -n "$batch_diagnostic_lines" ]]; then
        printf '%s\n' "$batch_diagnostic_lines" >&2
        if [[ "$suite_mode" == check ]]; then
            # While the batch is built, not yet wired entry points are expected
            # to be unused; the check reports them and goes on.
            echo "clippy ($clippy_target): diagnostics on changed lines (reported, not gating in --check)" >&2
            continue
        fi
        stage_failed "clippy ($clippy_target) reported diagnostics on lines this batch changed"
        continue
    fi
    echo "review task suite: clippy ($clippy_target) is clean on the lines this batch changed"
done

if [[ "$platform" == linux ]]; then
    # Pre-existing deny-level lints in the server's tests (docs/TODO.md, H1)
    # are capped to warnings; compile errors still fail and diagnostics on
    # lines this batch changed are reported like the crate's.
    server_ranges="$suite_tmp/server-ranges.txt"
    server_clippy_log="$suite_tmp/clippy-share-server.log"
    if ! (
        cd "$repo_root/share-server"
        CARGO_TARGET_DIR="$repo_root/share-server/target" run_task cargo clippy --locked \
            --all-targets --message-format short -- --cap-lints warn
    ) 2>&1 | tee "$server_clippy_log"; then
        stage_failed "clippy (share-server) did not complete"
    else
        : > "$server_ranges"
        for batch_file in "${batch_files[@]}"; do
            [[ "$batch_file" == share-server/* ]] || continue
            git -C "$repo_root" diff -U0 "$batch_base" HEAD -- "$batch_file" | awk -v file="${batch_file#share-server/}" '
                /^@@ / {
                    split($3, plus, ",")
                    start = substr(plus[1], 2) + 0
                    count = (length(plus) > 1) ? plus[2] + 0 : 1
                    if (count > 0) {
                        print file, start, start + count - 1
                    }
                }' >> "$server_ranges"
        done
        server_lines="$(batch_diagnostics_with "$server_ranges" "$server_clippy_log")"
        if [[ -n "$server_lines" ]]; then
            printf '%s\n' "$server_lines" >&2
            if [[ "$suite_mode" == check ]]; then
                echo "clippy (share-server): diagnostics on changed lines (reported, not gating in --check)" >&2
            else
                stage_failed "clippy (share-server) reported diagnostics on lines this batch changed"
            fi
        else
            echo "review task suite: clippy (share-server) is clean on the lines this batch changed"
        fi
    fi
fi

if [[ "$suite_mode" == check ]]; then
    finish_suite "review task suite: batch gates passed (check)"
    exit 0
fi

# ---------------------------------------------------------------------------
# The complete suite stages (milestones `review_task_`, affected modules,
# end-to-end runs) are added by the suite block once the batch is implemented.
stage_failed "complete suite stages are not implemented yet"
finish_suite "review task suite: passed"
