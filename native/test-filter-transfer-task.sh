#!/usr/bin/env bash
# Single task-level suite for the filter-pruned recursive scan / Win32-hostile
# names / concurrent Share transfers / Room lifecycle batch. Run this one
# checked-in entrypoint with an outer timeout of at least 30 minutes; the
# Linux job additionally runs the Share Room end-to-end script, the Windows
# job the real-filesystem hostile-name cases.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-filter-transfer-task.sh [--bounded|--direct]" >&2
    echo "  --bounded  use native/run-task-memory-bounded.sh (default for local users)" >&2
    echo "  --direct   rely on the remote runner's resource controls" >&2
}

execution_mode=bounded
case "$#" in
    0) ;;
    1)
        case "$1" in
            --bounded) execution_mode=bounded ;;
            --direct) execution_mode=direct ;;
            *) usage; exit 2 ;;
        esac
        ;;
    *) usage; exit 2 ;;
esac

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) platform=windows ;;
    *) platform=linux ;;
esac

report_failure() {
    local status=$?
    echo "filter/transfer task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-filter-transfer-task.XXXXXX")"
fi
native_log="$suite_tmp/native.log"
integration_log="$suite_tmp/integration.log"
e2e_log="$suite_tmp/share-e2e.log"
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -f "$native_log" "$integration_log" "$e2e_log" "$suite_tmp/batch-ranges.txt" "$suite_tmp"/clippy-*.log
        rmdir "$suite_tmp" 2>/dev/null || true
    else
        echo "filter/transfer task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in awk cargo git grep mktemp sed sort tee uname; do
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
# Cargo runs inside native/ and therefore uses native/target by default; the
# explicit variable only names the E2E binaries on Linux (a POSIX path would
# not be a safe Cargo environment value under Git Bash on Windows).
if [[ "$platform" == linux && -z "${CARGO_TARGET_DIR:-}" ]]; then
    export CARGO_TARGET_DIR="$repo_root/native/target"
fi

run_task() {
    if [[ "$execution_mode" == bounded && "$platform" == linux ]]; then
        "$repo_root/native/run-task-memory-bounded.sh" "$@"
    else
        "$@"
    fi
}

# Milestone expectations
# (docs/superpowers/plans/2026-09-16-recursive-filter-pruning-and-win32-names.md).
native_tests=(
    # M1 Win32 name rules
    recursive_filter_task_reserved_device_names_are_detected
    recursive_filter_task_trailing_and_invalid_characters_are_detected
    recursive_filter_task_ordinary_names_pass
    recursive_filter_task_safe_names_are_addressable
    # M2 filter flag and scope comparison
    recursive_filter_task_problem_names_filter_keeps_only_win32_hostile_names
    recursive_filter_task_pass_all_filter_never_prunes
    recursive_filter_task_substring_text_narrows_by_containment
    recursive_filter_task_structured_constraints_narrow_by_containment
    recursive_filter_task_restart_only_for_broader_or_truncated_listings
    recursive_filter_task_filter_retention_keeps_matches_and_descends_within_view
    # M3 local scanner retention
    recursive_filter_task_lineage_emits_each_pending_ancestor_once
    recursive_filter_task_lineage_stops_when_the_sink_refuses
    recursive_filter_task_unfiltered_scan_still_emits_every_entry
    recursive_filter_task_filtered_scan_emits_matches_with_their_ancestors_only
    recursive_filter_task_filtered_scan_prunes_subtrees_it_may_not_descend
    recursive_filter_task_depth_limit_still_bounds_a_filtered_scan
    # M4 remote scanner retention
    recursive_filter_task_remote_retention_emits_matches_with_their_ancestors_only
    recursive_filter_task_remote_retention_can_prune_whole_subtrees
    # M7 hostile-name recycling detour
    recursive_filter_task_numbered_names_keep_their_extension
    recursive_filter_task_safe_sibling_rename_never_replaces_an_existing_entry
    recursive_filter_task_plain_names_never_take_the_rename_detour
    # M9 concurrent transfer lane
    recursive_filter_task_transfer_lane_runs_up_to_capacity_and_queues_the_rest
    recursive_filter_task_transfer_lane_reports_lost_workers_and_shuts_down
    # M10 typed agent-protocol errors (Room/Direct uploads through the daemon)
    recursive_filter_task_agent_errors_recover_not_found_and_exists_kinds
    recursive_filter_task_unrecognized_agent_errors_stay_other_with_their_text
)
if [[ "$platform" == windows ]]; then
    # M5 verbatim addressing of real NUL / trailing-dot entries (Windows only)
    native_tests+=(
        recursive_filter_task_verbatim_only_for_hostile_components
        recursive_filter_task_local_backend_addresses_real_hostile_names
        recursive_filter_task_scanner_lists_real_hostile_names_with_metadata
    )
fi

# Directly affected integrations: unchanged scanner/rscan/filter/delete
# behavior around the new retention and verbatim code paths, and the room
# relation persistence the E2E relies on.
integration_paths=(
    rscan::imp::tests::walks_full_tree_via_backend
    rscan::imp::tests::flat_depth_one_does_not_recurse
    rscan::imp::tests::recursive_scan_uses_parallel_backend_width
    rscan::imp::search::tests::duplicate_search_hits_are_terminal_errors
    filter::imp::tests::substring_filter_uses_semicolons_as_or_groups
    scanner::collect::tests::recursive_collection_honors_preexisting_cancellation
    vfs::delete_tests::recursive_delete_stops_at_link_like_root
    app::delete_actions::tests::nested_targets_are_collapsed_without_prefix_confusion
    app::delete_lifecycle::tests::only_confirmed_successes_enter_the_success_path_list
    share::share_remote_discovery_task_tests::share_remote_task_discovery_persists_direct_and_room_relations_idempotently
)
if [[ "$platform" == linux ]]; then
    integration_paths+=(
        scanner::collect::tests::recursive_collection_does_not_follow_directory_symlinks
    )
fi
integration_tests=()
for path in "${integration_paths[@]}"; do
    integration_tests+=("${path##*::}")
done

verify_test_log() {
    local log=$1
    local expected_count=$2
    shift 2
    local actual_count test_name occurrences
    actual_count="$(grep -Ec '^test .* \.\.\. ok$' "$log" || true)"
    if [[ "$actual_count" -ne "$expected_count" ]]; then
        echo "expected $expected_count passing filtered tests in $log, found $actual_count" >&2
        exit 1
    fi
    for test_name in "$@"; do
        occurrences="$(grep -Ec "^test .*::${test_name} \.\.\. ok$" "$log" || true)"
        if [[ "$occurrences" -ne 1 ]]; then
            echo "expected exactly one passing result for $test_name, found $occurrences" >&2
            exit 1
        fi
    done
    grep -Eq \
        "^test result: ok\\. ${expected_count} passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in .+$" \
        "$log" || {
        echo "filtered test summary did not match the expected result in $log" >&2
        exit 1
    }
}

echo "filter/transfer task suite: milestone behavior ($platform)"
if [[ "$execution_mode" == bounded && "$platform" == linux ]]; then
    echo "filter/transfer task suite: local bounded runner (hard cap 2G)"
else
    echo "filter/transfer task suite: direct runner execution"
fi
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib recursive_filter_task_ -- --test-threads=1
) 2>&1 | tee "$native_log"
verify_test_log "$native_log" "${#native_tests[@]}" "${native_tests[@]}"

echo "filter/transfer task suite: affected integrations"
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib -- --test-threads=1 --exact "${integration_paths[@]}"
) 2>&1 | tee "$integration_log"
verify_test_log "$integration_log" "${#integration_tests[@]}" "${integration_tests[@]}"

if [[ "$platform" != linux ]]; then
    suite_succeeded=true
    echo "task-level suite passed on $platform with the exact expected milestone and integration results"
    exit 0
fi

echo "filter/transfer task suite: Share Room lifecycle and concurrent transfers end to end"
for command_name in jq timeout cmp head; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "$command_name is required for the Share Room end-to-end script" >&2
        exit 1
    }
done
# The E2E drives the real CLI and Share server binaries; build only those two
# development binaries (no workspace, cross or release build).
(
    cd "$repo_root/native"
    run_task cargo build --locked --bin se
)
(
    cd "$repo_root/share-server"
    CARGO_TARGET_DIR="$repo_root/share-server/target" run_task cargo build --locked --bin se-share-server
)
SMART_EXPLORER_SE_BINARY="${CARGO_TARGET_DIR:-$repo_root/native/target}/debug/se" \
SMART_EXPLORER_SHARE_SERVER_BINARY="$repo_root/share-server/target/debug/se-share-server" \
    bash "$repo_root/native/test-share-room-e2e.sh" 2>&1 | tee "$e2e_log"
grep -Fq 'Room lifecycle passed:' "$e2e_log"

echo "filter/transfer task suite: release gates for the batch's files (rustfmt, clippy on both targets)"
# The crate as a whole carries older formatting and dead-code drift outside
# this batch (docs/TODO.md, H1), so both gates cover exactly what this batch
# touched: every changed file is format-checked on its own through rustfmt's
# stdin mode, and clippy runs over the whole crate for the host and the Windows
# target but only diagnostics on lines this batch changed fail the suite. A
# compile error anywhere still fails. The batch base is the last commit before
# the batch (the 0.5.158 release).
batch_base=99463f6d75178aa36928cadd15d1e6af1d4beee7
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
command -v rustfmt >/dev/null 2>&1 || {
    echo "rustfmt is required" >&2
    exit 1
}
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
echo "filter/transfer task suite: ${#batch_files[@]} batch source files are rustfmt-clean"

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
if ! rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$'; then
    echo "x86_64-pc-windows-gnu target is not installed; Windows clippy skipped" >&2
elif ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "x86_64-w64-mingw32-gcc is not installed; Windows clippy skipped" >&2
else
    clippy_targets+=(x86_64-pc-windows-gnu)
fi
for clippy_target in "${clippy_targets[@]}"; do
    clippy_log="$suite_tmp/clippy-$clippy_target.log"
    if [[ "$clippy_target" == host ]]; then
        (
            cd "$repo_root/native"
            run_task cargo clippy --locked --lib --bins --message-format short
        ) 2>&1 | tee "$clippy_log"
    else
        (
            cd "$repo_root/native"
            run_task cargo clippy --locked --target "$clippy_target" --lib --bins \
                --message-format short
        ) 2>&1 | tee "$clippy_log"
    fi
    batch_diagnostic_lines="$(batch_diagnostics "$clippy_log")"
    if [[ -n "$batch_diagnostic_lines" ]]; then
        printf '%s\n' "$batch_diagnostic_lines" >&2
        echo "clippy ($clippy_target) reported diagnostics on lines this batch changed" >&2
        exit 1
    fi
    echo "filter/transfer task suite: clippy ($clippy_target) is clean on the lines this batch changed"
done

suite_succeeded=true
echo "task-level suite passed with the exact expected milestone, integration, end-to-end, and gate results"
