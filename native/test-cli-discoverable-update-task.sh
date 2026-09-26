#!/usr/bin/env bash
# Single task-level suite for the terminal batch: `se share discoverable`
# (offers kept by the Share daemon) and `se update` (terminal-only and desktop
# installations). Run this one checked-in entrypoint with an outer timeout of
# at least 30 minutes.
set -Eeuo pipefail

usage() {
    echo "Usage: native/test-cli-discoverable-update-task.sh [--bounded|--direct]" >&2
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
    echo "cli discoverable/update task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
    exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${SMART_EXPLORER_TASK_LOG_ROOT:-}" ]]; then
    mkdir -p -- "$SMART_EXPLORER_TASK_LOG_ROOT"
    suite_tmp="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/run.XXXXXX")"
else
    suite_tmp="$(mktemp -d "${TMPDIR:-/tmp}/se-cli-discoverable-update-task.XXXXXX")"
fi
suite_succeeded=false

cleanup() {
    local status=$?
    if [[ "$suite_succeeded" == true ]]; then
        rm -rf "$suite_tmp"
    else
        echo "cli discoverable/update task suite diagnostics: $suite_tmp" >&2
    fi
    return "$status"
}
trap cleanup EXIT

for command_name in awk cargo git grep jq mktemp realpath rustup sed sha256sum sort tee timeout; do
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

# Milestone expectations (docs/superpowers/plans/2026-09-26-cli-discoverable-update.md).
milestone_tests=(
    # M1 the Share daemon owns the terminal-visible offer state
    cli_task_offer_book_tracks_prepared_published_and_stopped
    cli_task_offer_book_lists_only_unexpired_offers_sorted
    cli_task_offer_book_ends_all_and_bounds_recent_ends
    cli_task_share_command_reply_resolves_offer_from_the_book
    cli_task_snapshot_offers_default_and_share_command_reply_roundtrips
    cli_task_worker_stop_ends_tracked_offers
    cli_task_handoff_never_starts_a_missing_worker
    cli_task_daemon_refuses_a_second_offer_for_a_target
    cli_task_client_snapshot_leaves_the_gui_events
    cli_task_gui_state_drops_stopped_and_vanished_offers
    # M2 se share discoverable
    cli_task_discoverable_pin_source_rules
    cli_task_discoverable_pin_stdin_reads_one_line
    cli_task_discoverable_name_and_duration_rules
    cli_task_discoverable_room_selector_matches_id_relation_or_unique_name
    cli_task_discoverable_stop_selection_by_id_prefix_single_and_all
    cli_task_discoverable_offer_text_and_json
    cli_task_discoverable_parses_one_line_publish_and_subcommands
    # M3 updater terminal API
    cli_task_installation_detects_terminal_only_and_desktop
    cli_task_install_replaces_in_place_commits_and_rolls_back
    cli_task_install_rejects_a_hash_mismatch_before_replacing
    cli_task_install_checks_the_backup_and_the_installed_file
    cli_task_update_lock_is_exclusive_and_names_leftovers
    cli_task_orphans_of_ended_updates_are_removed_and_stale_locks_taken_over
    cli_task_desktop_update_needs_a_graphical_session
    cli_task_staged_payload_is_executable_and_install_keeps_the_mode
    # M4 se update
    cli_task_update_parses_check_reinstall_source_and_hidden_completion
    cli_task_update_completion_answer_and_source_rules
)

# Directly affected integrations: the Share worker stop barrier, the IPC wire
# and its bounds, the GUI and Android discovery/status consumers, the updater
# feed/staging helpers, the worker version handshake, and the CLI surface.
integration_tests=(
    explicit_stop_barrier_blocks_periodic_auto_connect_reload
    stop_refuses_to_strand_a_pending_profile_commit
    response_read_preserves_following_stream_bytes
    share_snapshot_carries_the_profile_cas_revision
    maximum_profile_and_event_backlog_fit_one_ipc_response
    every_append_path_keeps_only_the_newest_bounded_events
    share_remote_task_discovery_ui_tracks_duration_list_renewal_and_cancel
    android_task_share_status_maps_a_worker_snapshot
    rejects_manifest_payload_outside_appdata
    is_newer_compares_semver
    classify_distinguishes_transports
    staged_payload_path_binds_sha256_in_name
    worker_ping_requires_current_version
    replacement_rejects_the_retiring_generation_but_accepts_another_successor
    stdin_secret_is_trimmed_and_bounded
    parses_doctor_and_headless_share_commands
    top_level_help_shows_targets_and_setup_examples
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
        occurrences="$(grep -Ec "^test (.*::)?${test_name} \.\.\. ok$" "$log" || true)"
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

echo "cli discoverable/update task suite: milestone behavior"
if [[ "$execution_mode" == bounded ]]; then
    echo "cli discoverable/update task suite: local bounded runner (hard cap 2G)"
else
    echo "cli discoverable/update task suite: direct remote runner execution"
fi
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib cli_task_ -- --test-threads=1
) 2>&1 | tee "$suite_tmp/milestones.log"
verify_test_log "$suite_tmp/milestones.log" "${#milestone_tests[@]}" "${milestone_tests[@]}"

echo "cli discoverable/update task suite: affected integrations"
(
    cd "$repo_root/native"
    run_task cargo test --locked --lib -- --test-threads=1 "${integration_tests[@]}"
) 2>&1 | tee "$suite_tmp/integrations.log"
verify_test_log "$suite_tmp/integrations.log" "${#integration_tests[@]}" "${integration_tests[@]}"

echo "cli discoverable/update task suite: development binaries for the CLI checks"
(
    cd "$repo_root/native"
    run_task cargo build --locked --bin se
) 2>&1 | tee "$suite_tmp/build-se.log"
(
    cd "$repo_root/share-server"
    CARGO_TARGET_DIR="$repo_root/share-server/target" run_task cargo build --locked --bin se-share-server
) 2>&1 | tee "$suite_tmp/build-share-server.log"
se_binary="$CARGO_TARGET_DIR/debug/se"
test -x "$se_binary"

echo "cli discoverable/update task suite: CLI help surface"
"$se_binary" share discoverable --help >"$suite_tmp/help-discoverable.txt"
grep -Fq -- "--pin-stdin" "$suite_tmp/help-discoverable.txt"
grep -Fq "se share discoverable --minutes 5 --pin 1454" "$suite_tmp/help-discoverable.txt"
"$se_binary" update --help >"$suite_tmp/help-update.txt"
grep -Fq -- "--check" "$suite_tmp/help-update.txt"
grep -Fq "~/.local/bin/se" "$suite_tmp/help-update.txt"
if grep -Fq "complete-install" "$suite_tmp/help-update.txt"; then
    echo "the internal completion flag is visible in se update --help" >&2
    exit 1
fi
"$se_binary" --help >"$suite_tmp/help-top.txt"
grep -Fq "se update --check" "$suite_tmp/help-top.txt"

echo "cli discoverable/update task suite: end-to-end against a local Share server and feed"
# TMPDIR keeps the E2E diagnostics inside the uploaded suite logs on failure.
TMPDIR="$suite_tmp" \
SMART_EXPLORER_SE_BINARY="$se_binary" \
SMART_EXPLORER_SHARE_SERVER_BINARY="$repo_root/share-server/target/debug/se-share-server" \
    bash "$repo_root/native/test-cli-discoverable-update-e2e.sh" 2>&1 | tee "$suite_tmp/e2e.log"
grep -Fq "se share discoverable passed" "$suite_tmp/e2e.log"
grep -Fq "se update passed" "$suite_tmp/e2e.log"

echo "cli discoverable/update task suite: installer update source"
sh -n "$repo_root/install-linux.sh"
bash -n "$repo_root/native/test-share-lifecycle-e2e.sh"
installer_dry_run() {
    local install_dir=$1
    shift
    SMART_EXPLORER_RELEASE_TAG=v9.8.7 \
    SMART_EXPLORER_REQUIRE_RELEASE_ASSETS=1 \
    SMART_EXPLORER_INSTALL_DIR="$install_dir" \
    SMART_EXPLORER_BIN_DIR="$suite_tmp/installer-bin" \
        sh "$repo_root/install-linux.sh" --dry-run "$@" 2>&1
}
fresh_cli="$(installer_dry_run "$suite_tmp/installer-fresh" --cli-only)"
grep -Fq "create the missing $suite_tmp/installer-fresh/update_source.txt" <<<"$fresh_cli"
mkdir -p "$suite_tmp/installer-existing"
printf '%s\n' 'https://example.invalid/desktop-feed' >"$suite_tmp/installer-existing/update_source.txt"
existing_cli="$(installer_dry_run "$suite_tmp/installer-existing" --cli-only)"
grep -Fq "Keeping the existing update source" <<<"$existing_cli"
if grep -Fq "create the missing" <<<"$existing_cli"; then
    echo "a CLI-only install would rewrite an existing update source" >&2
    exit 1
fi
[[ "$(cat "$suite_tmp/installer-existing/update_source.txt")" == 'https://example.invalid/desktop-feed' ]]
desktop="$(installer_dry_run "$suite_tmp/installer-desktop")"
grep -Fq "dry-run: write $suite_tmp/installer-desktop/update_source.txt" <<<"$desktop"

echo "cli discoverable/update task suite: release gates on the lines this batch changed"
# The crate carries older formatting and dead-code drift (docs/TODO.md, H1), so
# rustfmt and clippy fail the suite only for lines this batch added or changed.
batch_base=f7a4802b0cc1e9efe3ef5602eea15ab37779417d
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
batch_ranges="$suite_tmp/batch-ranges.txt"
: >"$batch_ranges"
for batch_file in "${batch_files[@]}"; do
    git -C "$repo_root" diff -U0 "$batch_base" HEAD -- "$batch_file" | awk -v file="${batch_file#native/}" '
        /^@@ / {
            split($3, plus, ",")
            start = substr(plus[1], 2) + 0
            count = (length(plus) > 1) ? plus[2] + 0 : 1
            if (count > 0) {
                print file, start, start + count - 1
            }
        }' >>"$batch_ranges"
done

command -v rustfmt >/dev/null 2>&1 || {
    echo "rustfmt is required" >&2
    exit 1
}
format_failures=0
for batch_file in "${batch_files[@]}"; do
    format_diff="$(rustfmt --check --color never --edition 2021 <"$repo_root/$batch_file" || true)"
    [[ -n "$format_diff" ]] || continue
    # A hunk counts only where it removes or inserts at a line this batch changed.
    changed_lines="$(awk -v file="${batch_file#native/}" -v ranges="$batch_ranges" '
        BEGIN {
            while ((getline line < ranges) > 0) {
                split(line, range, " ")
                if (range[1] == file) {
                    n++
                    first[n] = range[2]
                    last[n] = range[3]
                }
            }
        }
        function check(at) {
            for (i = 1; i <= n; i++) {
                if (at >= first[i] && at <= last[i]) {
                    print at
                    return
                }
            }
        }
        /^Diff in .* at line [0-9]+:?$/ {
            match($0, /at line [0-9]+/)
            current = substr($0, RSTART + 8, RLENGTH - 8) + 0
            inside = 1
            next
        }
        inside && /^ / { current++; next }
        inside && /^-/ { check(current); current++; next }
        inside && /^\+/ { check(current); next }' <<<"$format_diff")"
    if [[ -n "$changed_lines" ]]; then
        printf '%s\n' "$format_diff" | sed "s#<stdin>#$batch_file#"
        echo "rustfmt differs on changed lines of $batch_file: $(tr '\n' ' ' <<<"$changed_lines")" >&2
        format_failures=$((format_failures + 1))
    fi
done
if [[ "$format_failures" -ne 0 ]]; then
    echo "$format_failures batch source files are not rustfmt-clean on changed lines" >&2
    exit 1
fi
echo "cli discoverable/update task suite: ${#batch_files[@]} batch source files are rustfmt-clean on changed lines"

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
# The Windows target covers the console-echo and helper paths; it needs the
# rustup target and the mingw-w64 C compiler, which the remote workflow installs.
if ! rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$'; then
    echo "x86_64-pc-windows-gnu target is not installed" >&2
    exit 1
elif ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "x86_64-w64-mingw32-gcc is not installed" >&2
    exit 1
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
    echo "cli discoverable/update task suite: clippy ($clippy_target) is clean on the lines this batch changed"
done

suite_succeeded=true
echo "task-level suite passed with the exact expected milestone, integration, CLI, end-to-end, installer and gate results"
