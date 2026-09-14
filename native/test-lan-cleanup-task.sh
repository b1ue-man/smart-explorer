#!/usr/bin/env bash
# Single task-level suite for the connection-cleanup / Drive-duplicates /
# LAN-presence / uplink-sharing batch. Run this one checked-in entrypoint with
# an outer timeout of at least 30 minutes.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-lan-cleanup-task.sh [--bounded|--direct]" >&2
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
    echo "lan/cleanup task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-lan-cleanup-task.XXXXXX")"
fi
native_log="$suite_tmp/native.log"
integration_log="$suite_tmp/integration.log"
cli_log="$suite_tmp/cli.log"
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -f "$native_log" "$integration_log" "$cli_log" "$suite_tmp/gates.log" "$suite_tmp/analytics.log"
        rmdir "$suite_tmp"
    else
        echo "lan/cleanup task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in cargo grep mktemp rustup tee; do
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

# Milestone expectations (docs/superpowers/plans/2026-09-14-connection-cleanup-lan-batch.md).
native_tests=(
    # M1 Google Drive duplicate siblings stay path-addressable
    lan_cleanup_task_unique_names_are_untouched
    lan_cleanup_task_duplicate_folders_keep_newest_plain_and_mark_the_rest
    lan_cleanup_task_equal_timestamps_break_ties_by_id
    lan_cleanup_task_marker_collision_with_literal_name_uses_full_id
    lan_cleanup_task_entries_without_id_keep_their_raw_name
    lan_cleanup_task_parse_marker_rejects_non_markers
    lan_cleanup_task_canonical_and_prefix_selection
    # M2 complete peer removal and durable denial of automatic re-pairing
    lan_cleanup_task_record_lookup_and_readmit
    lan_cleanup_task_ledger_is_bounded_and_keeps_newest_records
    lan_cleanup_task_re_recording_a_device_replaces_its_record
    lan_cleanup_task_forgetting_removes_contact_grant_and_records_denial
    lan_cleanup_task_forgetting_a_pending_contact_records_no_denial
    lan_cleanup_task_deleting_a_grant_records_denial_for_its_device
    lan_cleanup_task_removed_peer_blocks_automatic_repair_until_user_pairs_again
    # M3 derived-store cleanup and favourite labels
    lan_cleanup_task_direct_scope_matches_only_its_own_paths
    lan_cleanup_task_saved_url_connection_scope_uses_the_endpoint_prefix
    lan_cleanup_task_unc_connection_scope_uses_the_forward_slashed_root
    lan_cleanup_task_favourites_and_prefs_filter_by_scope
    lan_cleanup_task_report_suffix_lists_only_what_happened
    lan_cleanup_task_basename_of_keys
    # M4 link facts, classification, LAN presence matching
    lan_cleanup_task_apipa_without_gateway_is_router_less
    lan_cleanup_task_static_addresses_without_gateway_are_router_less_too
    lan_cleanup_task_dhcp_lease_without_gateway_is_routed
    lan_cleanup_task_gateway_is_uplink_unless_platform_denies_internet
    lan_cleanup_task_a_link_with_a_paired_peer_never_counts_as_uplink
    lan_cleanup_task_shared_and_inactive_links
    lan_cleanup_task_link_local_detection
    lan_cleanup_task_scoped_candidates_round_trip
    lan_cleanup_task_ipv4_default_routes_are_detected_by_flag_and_destination
    lan_cleanup_task_ipv6_default_routes_need_zero_prefix_and_gateway_flag
    lan_cleanup_task_hashed_ids_are_stable_short_and_not_the_key
    lan_cleanup_task_only_accepted_contacts_with_a_device_match
    lan_cleanup_task_candidates_cover_v4_global_v6_and_scoped_link_local
    lan_cleanup_task_peer_interfaces_follow_shared_prefixes
    lan_cleanup_task_effective_presence_merges_or_synthesizes
    lan_cleanup_task_defaults_and_round_trip
    lan_cleanup_task_instance_names_carry_the_hashed_id
    lan_cleanup_task_lan_only_operation_needs_no_server
    # M5 uplink sharing contract, platform helpers, policy
    lan_cleanup_task_adapter_ids_are_validated_strictly
    lan_cleanup_task_state_round_trip
    lan_cleanup_task_uuids_are_version_4
    lan_cleanup_task_profile_names_are_per_interface
    lan_cleanup_task_rule_names_both_actions_and_the_user
    lan_cleanup_task_starts_after_debounce_and_stops_after_grace
    lan_cleanup_task_idle_reasons_cover_missing_uplink_peer_and_disabled
    lan_cleanup_task_both_sides_with_internet_do_not_share
    lan_cleanup_task_uplink_loss_stop_request_and_disable_stop_sharing
)

# Storage-analysis hardening: no early stop, exact totals with folded files,
# unrepresentable/erroring entries recorded instead of aborting.
analytics_tests=(
    analytics_access_task_sizes_and_counts
    analytics_access_task_missing_local_root_is_failed_not_empty_success
    analytics_access_task_first_entry_error_preserves_readable_sibling
    analytics_access_task_huge_directory_keeps_exact_totals_with_folded_files
    analytics_access_task_unrepresentable_and_erroring_entries_never_end_the_directory
    analytics_access_task_backend_child_error_is_partial_and_root_error_is_failed
    analytics_access_task_existing_budget_stops_honestly
    analytics_access_task_diagnostics_keep_denial_identity_when_report_is_full
    analytics_access_task_cancellation_never_becomes_partial_success
    analytics_access_task_exact_startup_admission
    analytics_access_task_report_lists_each_retained_path_and_omissions
    analytics_access_task_new_scan_resets_prompt_without_canceling_consented_launch
    analytics_access_task_remote_access_never_requests_local_privileges
    analytics_access_task_treemap_origin_change_invalidates_geometry
    analytics_access_task_invalid_startup_cannot_spawn_or_restart_a_scan
)

# Directly affected integrations: profile schema 8 migration and the worker
# start condition.
integration_tests=(
    v5_profile_migration_preserves_exec_and_defaults_tombstones_empty
    corrupt_evidence_and_future_schema_fail_closed_while_v6_defaults_empty
    explicit_stop_barrier_blocks_periodic_auto_connect_reload
    worker_runtime_update_preserves_concurrent_user_configuration
)

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

echo "lan/cleanup task suite: milestone behavior"
if [[ "$execution_mode" == bounded ]]; then
    echo "lan/cleanup task suite: local bounded runner (hard cap 2G)"
else
    echo "lan/cleanup task suite: direct remote runner execution"
fi
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib lan_cleanup_task_ -- --test-threads=1
) 2>&1 | tee "$native_log"
verify_test_log "$native_log" "${#native_tests[@]}" "${native_tests[@]}"

echo "lan/cleanup task suite: storage-analysis hardening"
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib analytics_access_task_ -- --test-threads=1
) 2>&1 | tee "$suite_tmp/analytics.log"
verify_test_log "$suite_tmp/analytics.log" "${#analytics_tests[@]}" "${analytics_tests[@]}"

echo "lan/cleanup task suite: affected integrations"
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib -- --test-threads=1 --exact \
        share::direct_request_tombstone::tests::v5_profile_migration_preserves_exec_and_defaults_tombstones_empty \
        share::legacy_direct_request_tests::corrupt_evidence_and_future_schema_fail_closed_while_v6_defaults_empty \
        daemon::ipc_host::service_lifecycle::tests::explicit_stop_barrier_blocks_periodic_auto_connect_reload \
        daemon::ipc_host::profile_merge::tests::worker_runtime_update_preserves_concurrent_user_configuration
) 2>&1 | tee "$integration_log"
verify_test_log "$integration_log" "${#integration_tests[@]}" "${integration_tests[@]}"

echo "lan/cleanup task suite: CLI surface parses the new commands"
(
    cd "$repo_root/native"
    run_task cargo run --locked --bin se -- share lan --help
    run_task cargo run --locked --bin se -- share grants delete --help
    run_task cargo run --locked --bin se -- share grants removed --help
) 2>&1 | tee "$cli_log"
grep -q "presence" "$cli_log"
grep -q "automatic re-pairing" "$cli_log"

echo "lan/cleanup task suite: release gates the batch must pass (format, clippy, Windows check)"
(
    cd "$repo_root/native"
    run_task cargo fmt --all -- --check
    run_task cargo clippy --locked --lib --bins -- -D warnings
    if rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$'; then
        run_task cargo check --locked --target x86_64-pc-windows-gnu --lib --bins
    else
        echo "x86_64-pc-windows-gnu target is not installed; Windows check skipped" >&2
    fi
) 2>&1 | tee "$suite_tmp/gates.log"

suite_succeeded=true
echo "task-level suite passed with the exact expected milestone, integration, CLI, and gate results"
