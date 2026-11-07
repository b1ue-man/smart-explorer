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
        rm -f "$native_log" "$integration_log" "$e2e_log"
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
    recursive_filter_task_safe_names_are_addressable
    # M2 filter flag and scope comparison
    recursive_filter_task_problem_names_filter_keeps_only_win32_hostile_names
    recursive_filter_task_pass_all_filter_never_prunes
    recursive_filter_task_substring_text_narrows_by_containment
    recursive_filter_task_restart_only_for_broader_or_truncated_listings
    recursive_filter_task_filter_retention_keeps_matches_and_descends_within_view
    # M3 local scanner retention
    recursive_filter_task_lineage_emits_each_pending_ancestor_once
    recursive_filter_task_lineage_stops_when_the_sink_refuses
    recursive_filter_task_filtered_scan_emits_matches_with_their_ancestors_only
    # M4 remote scanner retention
    recursive_filter_task_remote_retention_emits_matches_with_their_ancestors_only
    # M7 hostile-name recycling detour
    recursive_filter_task_numbered_names_keep_their_extension
    recursive_filter_task_safe_sibling_rename_never_replaces_an_existing_entry
    recursive_filter_task_plain_names_never_take_the_rename_detour
    # M9 (concurrent transfer lane with a fixed capacity) was superseded by
    # the uncapped lane of the transfer-engine batch; its tests moved to
    # native/test-transfer-engine-task.sh.
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
    rscan::imp::tests::recursive_scan_uses_parallel_backend_width
    rscan::imp::search::tests::duplicate_search_hits_are_terminal_errors
    filter::imp::tests::substring_filter_uses_commas_as_and_terms
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
    # One summary per test program of the workspace crates (D13); together they
    # show $expected_count passed and nothing failed, ignored or measured.
    awk -v expected="$expected_count" '
        /^test result: / {
            summaries++
            if ($3 != "ok.") bad = 1
            passed += $4; failed += $6; ignored += $8; measured += $10
        }
        END { exit !(summaries > 0 && !bad && passed == expected && failed == 0 && ignored == 0 && measured == 0) }
    ' "$log" || {
        echo "filtered test summaries in $log do not add up to $expected_count passed, 0 failed, 0 ignored" >&2
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
    run_task cargo test --locked --no-fail-fast --workspace --exclude smart_explorer_android --exclude smart_explorer --lib recursive_filter_task_ -- --test-threads=1
) 2>&1 | tee "$native_log"
verify_test_log "$native_log" "${#native_tests[@]}" "${native_tests[@]}"

echo "filter/transfer task suite: affected integrations"
(
    cd "$repo_root/native"
    run_task cargo test --locked --no-fail-fast --workspace --exclude smart_explorer_android --exclude smart_explorer --lib -- --test-threads=1 --exact "${integration_paths[@]}"
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

suite_succeeded=true
echo "task-level suite passed with the exact expected milestone, integration and end-to-end results"
