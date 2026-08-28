#!/usr/bin/env bash
# Run this one checked-in entrypoint with an outer timeout of at least 30 minutes.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-share-remote-task.sh [--bounded|--direct]" >&2
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

report_failure() {
    local status=$?
    echo "share/remote task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-share-remote-task.XXXXXX")"
fi
native_log="$suite_tmp/native.log"
server_log="$suite_tmp/share-server.log"
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -f "$native_log" "$server_log"
        rmdir "$suite_tmp"
    else
        echo "share/remote task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in cargo grep mktemp tee; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "$command_name is required" >&2
        exit 1
    }
done
if [[ "$execution_mode" == bounded ]]; then
    test -x "$repo_root/native/run-task-memory-bounded.sh" || {
        echo "task memory wrapper is missing or not executable" >&2
        exit 1
    }
fi
test -s "$repo_root/testdata/share-discovery-wire-v1.jsonl" || {
    echo "shared discovery wire fixture is missing" >&2
    exit 1
}

export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_TERM_COLOR=never
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
    export CARGO_TARGET_DIR="$repo_root/native/target"
fi

run_task() {
    if [[ "$execution_mode" == bounded ]]; then
        "$repo_root/native/run-task-memory-bounded.sh" "$@"
    else
        "$@"
    fi
}

native_tests=(
    share_remote_task_discovery_wire_fixture_roundtrips
    share_remote_task_discovery_accepts_empty_and_zero_pin
    share_remote_task_discovery_rejects_wrong_pin_tamper_replay_and_binding
    share_remote_task_discovery_lease_renewal_exchange_order_and_rejections
    share_remote_task_discovery_persists_direct_and_room_relations_idempotently
    share_remote_task_reciprocal_direct_fresh_autoaccepts_both_sides
    share_remote_task_reciprocal_direct_repairs_legacy_pins_and_retries_idempotently
    share_remote_task_reciprocal_direct_denial_unsupported_and_identity_conflict_fail_closed
    share_remote_task_legacy_direct_autoaccept_retry_tombstone_and_denial
    share_remote_task_reciprocal_incoming_auth_is_reread_after_transition_permit
    share_remote_task_reciprocal_stale_generation_sends_no_repair_hello
    share_remote_task_reciprocal_direct_offline_snapshot_plans_nothing
    share_remote_task_reciprocal_exec_grant_epoch_resynchronizes_coordinator
    share_remote_task_reciprocal_timeout_holds_transition_and_incoming_slots_until_store_finishes
    share_remote_task_reciprocal_full_event_channel_does_not_block_tokio_worker
    share_remote_task_storage_snapshot_finished_tree_and_legacy_fallback
    share_remote_task_storage_snapshot_corruption_is_rejected
    share_remote_task_remote_context_menu_plans_actions_and_open_with_boundary
    share_remote_task_discovery_ui_tracks_duration_list_renewal_and_cancel
    share_remote_task_discovery_ui_retains_pending_and_prunes_rotated_terminal_records
)

server_tests=(
    share_remote_task_server_and_native_discovery_wire_fixture
    share_remote_task_server_discovery_lease_renewal_and_complete_lifecycle
    share_remote_task_server_discovery_rejections_cancel_and_expiry
)

verify_test_log() {
    local log=$1
    local expected_count=$2
    shift 2
    local actual_count test_name occurrences
    actual_count="$(grep -Ec '^test .*share_remote_task_.* \.\.\. ok$' "$log" || true)"
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

echo "share/remote task suite: native library behavior"
if [[ "$execution_mode" == bounded ]]; then
    echo "share/remote task suite: local bounded runner (hard cap 2G)"
else
    echo "share/remote task suite: direct remote runner execution"
fi
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib share_remote_task_ -- --test-threads=1
) 2>&1 | tee "$native_log"
verify_test_log "$native_log" "${#native_tests[@]}" "${native_tests[@]}"

echo "share/remote task suite: share-server wire and lifecycle"
(
    cd "$repo_root/share-server"
    run_task cargo test --locked --bin se-share-server share_remote_task_ -- --test-threads=1
) 2>&1 | tee "$server_log"
verify_test_log "$server_log" "${#server_tests[@]}" "${server_tests[@]}"

suite_succeeded=true
echo "share/remote task suite passed with the exact expected native and server results"
