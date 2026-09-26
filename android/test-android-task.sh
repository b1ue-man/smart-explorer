#!/usr/bin/env bash
# Single task-level suite of the Android APK batch
# (docs/superpowers/plans/2026-09-25-android-apk/umsetzung.md, "Gesamtablauf" G1–G7).
# One entry point; each job of .github/workflows/android-task.yml calls exactly one sub-command:
#
#   host                  G1 android_task_ unit tests (Linux host), G2 desktop unchanged
#                         (Linux + Windows-GNU type check via clippy, moved tests under their new
#                         paths, explorer-command metadata, rustfmt/clippy on the batch's lines),
#                         G7 static release path
#   desktop-bins --out D  G5 prerequisites: only the Linux `se` and `se-share-server` dev binaries
#   android-build --out D G3: Android dependency tree, NDK/cargo-ndk, arm64 check, x86_64 .so with
#                         16 KB alignment, debug APK + test APK, JVM unit tests
#   emulator-run          G4/G5/G6 on the booted emulator (inside android-emulator-runner);
#                         needs SE_TASK_ARTIFACTS=<dir with apk/ and bin/>
#
# AGENTS.md: never run on the workstation – remote CI only, outer timeout ≥ 30 minutes.
set -Eeuo pipefail
shopt -s inherit_errexit

usage() {
  sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
}

report_failure() {
  local status=$?
  echo "android task suite failed at line ${BASH_LINENO[0]}: $BASH_COMMAND" >&2
  exit "$status"
}
trap report_failure ERR

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
android_dir="$repo_root/android"
native_dir="$repo_root/native"
servers_dir="$android_dir/test-servers"
eval_py="$servers_dir/task_eval.py"
android_test_sources="$android_dir/app/src/androidTest/java"
unit_test_sources="$android_dir/app/src/test/java"
api_md="$repo_root/docs/superpowers/plans/2026-09-25-android-apk/api.md"
app_package=app.smartexplorer.android
test_runner="$app_package.test/androidx.test.runner.AndroidJUnitRunner"
task_package="$app_package.task"
# Last commit before the Android batch: the gates cover exactly the lines this batch changed.
batch_base=4487f6f1e7d4c92344da2dea0175df3b2a3e12a7

log_root="${SMART_EXPLORER_TASK_LOG_ROOT:-${TMPDIR:-/tmp}/android-task-logs}"
mkdir -p -- "$log_root"

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_TERM_COLOR=never

die() {
  echo "android task suite: $*" >&2
  exit 1
}

step() {
  echo
  echo "=== android task suite: $* ==="
}

require_tools() {
  local tool
  for tool in "$@"; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool missing: $tool"
  done
}

ensure_batch_base() {
  if ! git -C "$repo_root" cat-file -e "${batch_base}^{commit}" 2>/dev/null; then
    git -C "$repo_root" fetch --quiet --depth=1 origin "$batch_base"
  fi
}

# ---------------------------------------------------------------------------------------------
# G1 expectations: every android_task_ test the Linux host compiles. Rule: tests inside code that
# only Android compiles (cfg(target_os = "android") or an os/android module without a host test
# include) never run on the host and would be listed in android_only_tests with the reason. There
# are none today: daemon/os/android/platform.rs is compiled into Linux host tests as
# daemon::android_platform (cfg(all(test, target_os = "linux"))), and mobile/android_fs build
# under all(unix, test). The source check below forces this list to follow new tests.
g1_tests=(
  # B1a: support_dirs, apptrash, android_fs rename chain
  android_task_host_values_redirect_data_and_temp_roots
  android_task_without_host_values_desktop_roots_stay_unchanged
  android_task_apptrash_round_trip_restores_the_original
  android_task_apptrash_restore_never_overwrites_an_occupied_name
  android_task_apptrash_restore_recreates_a_removed_parent
  android_task_apptrash_folders_report_their_size_and_delete_permanently
  android_task_apptrash_purge_removes_only_old_entries
  android_task_apptrash_refuses_outside_volumes_and_the_trash_itself
  android_task_apptrash_ignores_planted_records
  android_task_apptrash_names_ids_and_exclusion_rules
  android_task_apptrash_hidden_app_folders_only_below_volume_roots
  android_task_linemerge_text_shape_restores_line_endings
  android_task_rename_fallback_decisions_follow_the_error_numbers
  android_task_rename_no_replace_moves_and_never_replaces
  android_task_hard_link_step_is_create_only_and_drops_the_source
  android_task_checked_rename_refuses_existing_names_and_moves_directories
  # B1b: embedded daemon, catch-up, boot marker, Android platform adapter
  android_task_cancel_jobs_stops_only_the_selected_work
  android_task_catch_up_selects_due_timers_missed_calendar_and_realtime_once
  android_task_catch_up_run_finishes_when_admitted_jobs_are_done
  android_task_catch_up_lists_supervisor_rejections_with_reason
  android_task_catch_up_cancel_touches_only_this_runs_jobs
  android_task_catch_up_closed_gate_ends_open_runs_with_reason
  android_task_catch_up_reports_load_failures_and_bounds_history
  android_task_startup_pass_runs_once_per_boot_marker
  android_task_host_state_encoding_keeps_both_conditions
  android_task_platform_reflects_host_state_and_private_lock_directory
  # B4: transfer reader upload, share removal notice
  android_task_upload_reader_reserves_a_free_name_and_never_replaces
  android_task_upload_reader_cancel_publishes_nothing
  android_task_cleanup_notice_joins_lines_and_reports_unclean_stores
  # B2: protocol, events, tasks, locations, filters, facade methods
  android_task_envelopes_carry_ok_values_and_error_kinds
  android_task_io_errors_map_to_protocol_kinds
  android_task_task_snapshots_are_bundled_per_task_and_terminal_is_immediate
  android_task_event_polls_are_capped_and_keep_order
  android_task_cancel_and_clear_touch_only_matching_tasks
  android_task_locations_parse_into_connection_and_path
  android_task_app_internal_locations_are_recognized
  android_task_favorite_keys_follow_the_desktop_format
  android_task_filter_and_sort_arguments_map_to_the_desktop_model
  android_task_entry_types_mime_and_problem_names
  android_task_scan_tree_folds_collapsed_folders_and_windows
  android_task_local_fs_methods_create_rename_check_and_list
  android_task_app_internal_locations_are_rejected
  android_task_local_copy_keeps_both_names_and_delete_is_permanent
  android_task_scan_view_returns_a_windowed_tree_and_revisions
  android_task_edit_register_round_trip_and_change_event
  android_task_bad_arguments_and_unknown_ids
  # B3: domains (sync, update, analysis, share, connections)
  android_task_job_json_round_trip_keeps_every_desktop_field
  android_task_job_validation_reports_desktop_errors_per_field
  android_task_editor_messages_map_to_their_fields
  android_task_sync_options_list_every_mode_without_device_triggers
  android_task_schedule_text_matches_the_desktop_list
  android_task_update_feed_reads_version_checksum_and_apk
  android_task_analysis_node_lists_children_by_size_with_locations
  android_task_share_status_maps_a_worker_snapshot
  android_task_share_server_rules_match_the_desktop
  android_task_exec_command_split_respects_quotes
  android_task_forget_host_key_removes_only_that_entry
  android_task_locations_keep_the_endpoint_prefix
  android_task_worker_log_tail_starts_at_a_line
  # Review and limits batch (2026-09-26): locations, init home, edit register, exec errors, promotion
  android_task_locations_keep_trailing_blanks_and_bang_folders
  android_task_home_fallback_is_not_the_private_data_dir
  android_task_full_edit_register_forgets_the_oldest_unchanged_copy
  android_task_changed_or_unreadable_edit_registers_are_kept
  android_task_exec_errors_name_a_refused_grant_only
  android_task_promote_staged_with_creates_new_names_and_hands_existing_files_to_replace
  android_task_webdav_promote_replaces_with_one_move_overwrite_true
  android_task_webdav_promote_creates_a_new_name_without_overwrite
)
android_only_tests=()

# G2: existing tests of the logic moved out of app/ in this batch, under their new module paths
# (transfer, vfs::remote_util, syncjobs::editor, share::{profile_edits, lifecycle_view,
# poll_status}, connect cleanup scope) plus the desktop tests that reach the moved tree/clipboard
# logic through the app shells. share::discovery_state and filter::tree carry no own tests; the
# nine transfer::copy_paste_task_tests are #[ignore] fixture tests (live Share fixture) and stay out.
g2_paths=(
  transfer::temp::tests::allocation_creates_a_unique_parent_and_sanitizes_the_name
  transfer::temp::tests::allocation_propagates_directory_creation_failure
  transfer::temp_delete::tests::cleanup_target_must_be_a_plain_descendant
  transfer::tests::remote_clipboard_downloads_folder_tree
  transfer::tests::remote_clipboard_filters_folder_tree
  transfer::tests::remote_upload_copies_folder_tree_without_bulk
  transfer::tests::remote_upload_preserves_copy_semantics_for_bulk_backend
  transfer::tests::remote_download_preserves_copy_semantics_for_bulk_backend
  transfer::tests::remote_download_filters_selected_folder
  transfer::cancel_tests::pre_canceled_transfers_are_terminal_without_mutation
  transfer::cancel_tests::canceled_download_removes_partial_staging_file
  transfer::cancel_tests::canceled_upload_reports_retained_remote_staging_file
  transfer::lane::tests::recursive_filter_task_transfer_lane_runs_up_to_capacity_and_queues_the_rest
  transfer::lane::tests::recursive_filter_task_transfer_lane_reports_lost_workers_and_shuts_down
  vfs::remote_util::tests::remote_unique_name_checks_the_bound_and_never_reuses_it
  syncjobs::editor::tests::rejects_invalid_glob_instead_of_silently_skipping_it
  syncjobs::editor::tests::rejects_equal_and_nested_endpoints_without_prefix_confusion
  syncjobs::editor::tests::malformed_delete_guard_and_ambiguous_mirror_are_not_saved
  share::profile_edits::tests::gui_edit_rebases_without_reverting_worker_runtime_state
  share::lifecycle_view::tests::local_transport_never_calls_relay_forwarding_peer_received
  share::lifecycle_view::tests::decision_and_timestamp_labels_are_stable
  share::lifecycle_view::tests::outgoing_projection_separates_relay_forwarding_from_peer_receipt
  share::lifecycle_view::tests::incoming_delete_stays_available_pending_and_waits_for_terminal_history
  share::lifecycle_view::tests::authorized_card_never_links_same_device_id_with_a_different_key
  share::lifecycle_view::tests::incoming_identity_conflict_keeps_reject_and_delete_but_disables_accept
  share::lifecycle_view::tests::active_old_grant_is_named_as_the_resolution_for_a_new_identity
  share::poll_status::tests::connected_snapshot_replaces_worker_unreachable_status
  share::poll_status::tests::running_snapshot_replaces_poll_start_failure
  share::poll_status::tests::inactive_snapshot_still_reports_reachable_daemon
  share::poll_status::tests::unrelated_and_domain_errors_are_preserved
  connect::removal_scope::tests::lan_cleanup_task_direct_scope_matches_only_its_own_paths
  connect::removal_scope::tests::lan_cleanup_task_saved_url_connection_scope_uses_the_endpoint_prefix
  connect::removal_scope::tests::lan_cleanup_task_unc_connection_scope_uses_the_forward_slashed_root
  connect::removal_scope::tests::lan_cleanup_task_favourites_and_prefs_filter_by_scope
  connect::removal_scope::tests::lan_cleanup_task_report_suffix_lists_only_what_happened
  app::search_recursive_access_task::search_recursive_access_task_literal_suffixes_invalid_patterns_and_scope
  app::search_recursive_access_task::search_recursive_access_task_roots_orphans_and_hidden_folder_rows
  app::search_recursive_access_task::search_recursive_access_task_fold_selection_keyboard_and_tab_isolation
  app::search_recursive_access_task::search_recursive_access_task_wide_scan_folded_copy_preserves_exact_structure
  app::search_recursive_access_task::search_recursive_access_task_unfiltered_folded_folder_copy_keeps_empty_directories
  app::search_recursive_access_task::search_recursive_access_task_mid_scan_filter_restart_and_partial_channel
  app::search_recursive_access_task::search_recursive_access_task_long_clipboard_snapshot_and_remote_materialization
)

# Each expected name passes exactly once, the count matches and nothing failed or was ignored.
verify_test_log() {
  local log=$1 expected_count=$2
  shift 2
  local actual_count test_name occurrences
  actual_count="$(grep -Ec '^test .* \.\.\. ok$' "$log" || true)"
  [[ "$actual_count" -eq "$expected_count" ]] ||
    die "expected $expected_count passing tests in $log, found $actual_count"
  for test_name in "$@"; do
    occurrences="$(grep -Ec "^test (.*::)?${test_name} \.\.\. ok$" "$log" || true)"
    [[ "$occurrences" -eq 1 ]] || die "expected exactly one passing result for $test_name, found $occurrences"
  done
  grep -Eq "^test result: ok\\. ${expected_count} passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in .+$" "$log" ||
    die "test summary in $log does not show $expected_count passed, 0 failed, 0 ignored"
}

g1_source_check() {
  local found expected
  found="$(grep -rhoE 'fn android_task_[a-z0-9_]+' "$native_dir/src" | sed 's/^fn //' | sort)"
  expected="$(printf '%s\n' "${g1_tests[@]}" "${android_only_tests[@]}" | sed '/^$/d' | sort)"
  if [[ "$found" != "$expected" ]]; then
    diff <(printf '%s\n' "$expected") <(printf '%s\n' "$found") >&2 || true
    die "android_task_ tests in native/src differ from the suite's list (< suite, > sources)"
  fi
  [[ -z "$(uniq -d <<<"$found")" ]] || die "duplicate android_task_ test names: $(uniq -d <<<"$found")"
}

run_g1() {
  step "G1 android_task_ tests on the Linux host (${#g1_tests[@]} expected)"
  g1_source_check
  local log="$log_root/g1-android-task-tests.log"
  (cd "$native_dir" && cargo test --locked --lib android_task_ -- --test-threads=1) 2>&1 | tee "$log"
  verify_test_log "$log" "${#g1_tests[@]}" "${g1_tests[@]}"
}

run_g2_moved_tests() {
  step "G2 moved desktop tests under their new paths (${#g2_paths[@]} expected)"
  local list="$log_root/g2-test-list.txt" log="$log_root/g2-moved-tests.log" path missing=()
  (cd "$native_dir" && cargo test --locked --lib -- --list) >"$list" 2>&1
  for path in "${g2_paths[@]}"; do
    grep -Fxq "$path: test" "$list" || missing+=("$path")
  done
  [[ "${#missing[@]}" -eq 0 ]] || die "moved tests not found in the compiled test list: ${missing[*]}"
  local names=()
  for path in "${g2_paths[@]}"; do names+=("${path##*::}"); done
  # The moved app/ GUI-task tests build App through their task constructors, which require the
  # same opt-in variables as their original suites (test-search-recursive-access-task.py).
  (cd "$native_dir" && SMART_EXPLORER_ANALYTICS_TASK=1 SMART_EXPLORER_COPY_PASTE_TASK=1 \
    SMART_EXPLORER_GUI_TASK=1 cargo test --locked --lib -- --test-threads=1 --exact "${g2_paths[@]}") 2>&1 | tee "$log"
  verify_test_log "$log" "${#g2_paths[@]}" "${names[@]}"
}

batch_rust_files() {
  git -C "$repo_root" diff --name-only --diff-filter=AM "$batch_base" HEAD -- 'native/src/*.rs' 'native/android-bridge/*.rs'
}

run_g2_rustfmt() {
  step "G2 rustfmt (stdin mode) for every Rust file this batch added or changed"
  local files=() file diff status failures=0
  mapfile -t files < <(batch_rust_files)
  [[ "${#files[@]}" -gt 0 ]] || die "no batch Rust files relative to $batch_base"
  for file in "${files[@]}"; do
    status=0
    diff="$(rustfmt --check --color never --edition 2021 <"$repo_root/$file" 2>&1)" || status=$?
    if [[ "$status" -ne 0 || -n "$diff" ]]; then
      printf '%s\n' "$diff" | sed "s#<stdin>#$file#"
      failures=$((failures + 1))
    fi
  done
  [[ "$failures" -eq 0 ]] || die "$failures batch Rust files are not rustfmt-clean"
  echo "${#files[@]} batch Rust files are rustfmt-clean"
}

# Clippy type-checks the library and every binary (it fails on any compile error, so it is the
# `cargo check --lib --bins` of G2) and only its diagnostics on lines this batch changed fail the
# suite; the crate carries older lint debt outside the batch (docs/TODO.md H1).
run_g2_check_and_clippy() {
  local ranges="$log_root/batch-ranges.txt" file target log hits
  : >"$ranges"
  while IFS= read -r file; do
    [[ "$file" == native/src/* ]] || continue
    git -C "$repo_root" diff -U0 "$batch_base" HEAD -- "$file" | awk -v file="${file#native/}" '
      /^@@ / {
        split($3, plus, ",")
        start = substr(plus[1], 2) + 0
        count = (length(plus) > 1) ? plus[2] + 0 : 1
        if (count > 0) print file, start, start + count - 1
      }' >>"$ranges"
  done < <(batch_rust_files)
  for target in host x86_64-pc-windows-gnu; do
    step "G2 cargo check + clippy --lib --bins ($target)"
    log="$log_root/g2-clippy-$target.log"
    if [[ "$target" == host ]]; then
      (cd "$native_dir" && cargo clippy --locked --lib --bins --message-format short) 2>&1 | tee "$log"
    else
      rustup target list --installed | grep -qx "$target" || die "Rust target $target is not installed"
      require_tools x86_64-w64-mingw32-gcc
      (cd "$native_dir" && cargo clippy --locked --target "$target" --lib --bins --message-format short) 2>&1 | tee "$log"
    fi
    hits="$({ grep -E '^src/[^:]+:[0-9]+:[0-9]+: (warning|error)' "$log" || true; } | sort -u | awk -F: -v ranges="$ranges" '
      BEGIN { while ((getline line < ranges) > 0) { split(line, r, " "); n++; f[n] = r[1]; s[n] = r[2]; e[n] = r[3] } }
      { for (i = 1; i <= n; i++) if ($1 == f[i] && $2 >= s[i] && $2 <= e[i]) { print; break } }')"
    if [[ -n "$hits" ]]; then
      printf '%s\n' "$hits" >&2
      die "clippy ($target) reported diagnostics on lines this batch changed"
    fi
    echo "clippy ($target): no diagnostics on the batch's lines"
  done
}

run_g2_explorer_command() {
  step "G2 explorer-command manifest resolves with its own lock"
  cargo metadata --locked --format-version 1 --manifest-path "$native_dir/explorer-command/Cargo.toml" >/dev/null
}

yaml_parse() {
  local file=$1
  if python3 -c 'import yaml' 2>/dev/null; then
    python3 -c 'import sys, yaml; yaml.safe_load(open(sys.argv[1], encoding="utf-8"))' "$file"
  elif command -v ruby >/dev/null 2>&1; then
    ruby -ryaml -e 'YAML.safe_load(File.read(ARGV[0]), aliases: true)' "$file"
  else
    die "neither PyYAML nor Ruby is available to parse $file"
  fi
}

run_g7() {
  step "G7 release path, static"
  require_tools pwsh python3
  local plan="$log_root/g7-release-plan.json" script file shell_files=()
  script="$(mktemp "$log_root/g7-XXXXXX.ps1")"
  cat >"$script" <<'PS1'
$ErrorActionPreference = 'Stop'
$repo = $args[0]
foreach ($file in 'native/publish-release-local.ps1', 'native/release-publication.ps1', 'native/release-version.ps1') {
    $tokens = $null
    $errors = $null
    [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $repo $file), [ref]$tokens, [ref]$errors) | Out-Null
    if ($errors.Count -gt 0) {
        foreach ($problem in $errors) { [Console]::Error.WriteLine(('{0}:{1}: {2}' -f $file, $problem.Extent.StartLineNumber, $problem.Message)) }
        exit 1
    }
    [Console]::Error.WriteLine(('{0}: PowerShell parser ok' -f $file))
}
. (Join-Path $repo 'native/release-publication.ps1')
$version = '0.0.0'
$assets = @(Get-PublicationReleaseAssetMap -RepoRoot $repo -Version $version | ForEach-Object {
    [pscustomobject]@{
        local = [System.IO.Path]::GetRelativePath($repo, $_.LocalPath).Replace('\', '/')
        published = $_.PublishedName
    }
})
$paths = @(Get-PublicationReleaseCommitPaths -Version $version)
[pscustomobject]@{ version = $version; assets = $assets; commitPaths = $paths } | ConvertTo-Json -Depth 5
PS1
  pwsh -NoProfile -NonInteractive -File "$script" "$repo_root" >"$plan"
  python3 "$eval_py" release-lists --repo "$repo_root" --plan "$plan"
  for file in "$repo_root/.github/workflows/build.yml" "$repo_root/.github/workflows/android-task.yml"; do
    yaml_parse "$file"
    echo "${file#"$repo_root"/}: YAML ok"
  done
  mapfile -t shell_files < <(
    {
      printf '%s\n' android/build-release-apk.sh android/test-android-task.sh android/test-servers/servers.sh android/test-servers/share-desktop.sh
      git -C "$repo_root" diff --name-only --diff-filter=AM "$batch_base" HEAD -- '*.sh'
    } | sort -u
  )
  for file in "${shell_files[@]}"; do
    bash -n "$repo_root/$file"
    echo "$file: bash -n ok"
  done
  python3 -m py_compile "$eval_py"
}

cmd_host() {
  require_tools cargo rustup rustfmt git grep awk sed sort tee python3
  ensure_batch_base
  run_g7
  run_g1
  run_g2_moved_tests
  run_g2_explorer_command
  run_g2_rustfmt
  run_g2_check_and_clippy
  step "host job passed: G1 (${#g1_tests[@]} tests), G2, G7"
}

# ---------------------------------------------------------------------------------------------
cmd_desktop_bins() {
  local out=$1
  require_tools cargo
  step "G5 prerequisites: Linux se and se-share-server (dev profile, nothing else)"
  (cd "$native_dir" && cargo build --locked --bin se)
  (cd "$repo_root/share-server" && CARGO_TARGET_DIR="$repo_root/share-server/target" cargo build --locked --bin se-share-server)
  mkdir -p "$out"
  cp -- "${CARGO_TARGET_DIR:-$native_dir/target}/debug/se" "$out/se"
  cp -- "$repo_root/share-server/target/debug/se-share-server" "$out/se-share-server"
  "$out/se" --help >/dev/null
  [[ -x "$out/se-share-server" ]] || die "se-share-server was not built"
  echo "desktop binaries staged in $out"
}

# ---------------------------------------------------------------------------------------------
resolve_ndk() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}" version sdkmanager
  [[ -n "$sdk" && -d "$sdk" ]] || die "set ANDROID_HOME to the Android SDK"
  export ANDROID_HOME="$sdk"
  version="$(tr -d '[:space:]' <"$android_dir/ndk-version")"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "android/ndk-version must hold one NDK version, got '$version'"
  ndk_home="$sdk/ndk/$version"
  if [[ ! -f "$ndk_home/source.properties" ]]; then
    sdkmanager="$sdk/cmdline-tools/latest/bin/sdkmanager"
    [[ -x "$sdkmanager" ]] || sdkmanager="$(command -v sdkmanager || true)"
    [[ -n "$sdkmanager" ]] || die "NDK $version missing and sdkmanager not found"
    echo "installing the pinned NDK $version"
    set +o pipefail
    yes | "$sdkmanager" --licenses >/dev/null
    yes | "$sdkmanager" --install "ndk;$version"
    set -o pipefail
  fi
  grep -Fq "$version" "$ndk_home/source.properties" || die "NDK $version is not installed at $ndk_home"
  export ANDROID_NDK_HOME="$ndk_home"
  llvm_bin="$ndk_home/toolchains/llvm/prebuilt/linux-x86_64/bin"
  [[ -x "$llvm_bin/llvm-objdump" && -x "$llvm_bin/llvm-readelf" ]] || die "NDK $version lacks llvm-objdump/llvm-readelf"
}

assert_16k_aligned() {
  local library=$1 aligns align
  "$llvm_bin/llvm-readelf" -lW "$library" >"$log_root/g3-readelf-$(basename "$(dirname "$library")").txt"
  aligns="$("$llvm_bin/llvm-objdump" -p "$library" | awk '$1 == "LOAD" { for (i = 1; i < NF; i++) if ($i == "align") print $(i + 1) }')"
  [[ -n "$aligns" ]] || die "no LOAD segments reported for $library"
  while IFS= read -r align; do
    [[ "$align" =~ ^2\*\*([0-9]+)$ ]] || die "unexpected LOAD alignment '$align' in $library"
    ((BASH_REMATCH[1] >= 14)) || die "$library is not 16 KB aligned (LOAD align $align)"
  done <<<"$aligns"
  echo "$library: every LOAD segment aligned to at least 16 KB"
}

cmd_android_build() {
  local out=$1 target tree forbidden maven apk test_apk jni_libs="$android_dir/app/src/main/jniLibs"
  require_tools cargo rustup jq java python3 sha256sum
  resolve_ndk
  for target in aarch64-linux-android x86_64-linux-android; do
    rustup target list --installed | grep -qx "$target" || rustup target add "$target"
  done
  if ! cargo ndk --version 2>/dev/null | grep -q '4\.1\.2'; then
    cargo install cargo-ndk --locked --version 4.1.2
  fi
  cargo ndk --version || true

  step "G3 Android dependency tree without desktop GUI/trash/D-Bus crates"
  for target in aarch64-linux-android x86_64-linux-android; do
    tree="$log_root/g3-tree-$target.txt"
    (cd "$native_dir" && cargo tree --locked --target "$target" -e normal -p smart_explorer_android --prefix none --format '{p}') >"$tree"
    forbidden="$(awk '{ print $1 }' "$tree" | grep -xE 'eframe|egui_extras|winit|rfd|trash|zbus' | sort -u || true)"
    [[ -z "$forbidden" ]] || die "$target dependency tree contains: $forbidden"
    grep -q '^smart_explorer ' "$tree" || die "$target tree does not contain smart_explorer"
    echo "$target: no eframe/egui_extras/winit/rfd/trash/zbus"
  done

  step "G3 arm64-v8a type check of the JNI bridge (cargo ndk check)"
  (cd "$native_dir" && cargo ndk -t arm64-v8a --platform 30 check --locked -p smart_explorer_android)

  step "G3 x86_64 JNI library for the emulator (cargo ndk build, dev profile)"
  rm -f -- "$jni_libs"/*/libsmart_explorer_android.so
  (cd "$native_dir" && cargo ndk -t x86_64 --platform 30 -o "$jni_libs" build --locked -p smart_explorer_android)
  [[ -s "$jni_libs/x86_64/libsmart_explorer_android.so" ]] || die "cargo-ndk produced no x86_64 library"
  assert_16k_aligned "$jni_libs/x86_64/libsmart_explorer_android.so"

  step "G3 rustls-platform-verifier Maven repository (cargo metadata + jq)"
  maven="$(cd "$native_dir" && cargo metadata --locked --format-version 1 --filter-platform aarch64-linux-android |
    jq -r '[.packages[] | select(.name == "rustls-platform-verifier-android") | .manifest_path] | unique |
           if length == 1 then .[0] else error("expected exactly one rustls-platform-verifier-android") end')"
  maven="$(dirname "$maven")/maven"
  [[ -d "$maven/rustls/rustls-platform-verifier" ]] || die "no Maven module rustls/rustls-platform-verifier in $maven"

  step "G3 Gradle: debug APK, test APK and JVM unit tests"
  local results="$android_dir/app/build/test-results/testDebugUnitTest" gradle_status=0
  rm -rf -- "$results"
  (cd "$android_dir" && sh ./gradlew --no-daemon --console=plain --stacktrace --continue "-PrustlsVerifierMaven=$maven" \
    :app:assembleDebug :app:assembleDebugAndroidTest :app:testDebugUnitTest) || gradle_status=$?
  cp -r -- "$results" "$log_root/unit-test-results" 2>/dev/null || true
  cp -r -- "$android_dir/app/build/reports" "$log_root/gradle-reports" 2>/dev/null || true
  if [[ -d "$results" ]]; then
    python3 "$eval_py" unit-tests --results "$results" --sources "$unit_test_sources" || gradle_status=1
  fi
  [[ "$gradle_status" -eq 0 ]] || die "Gradle build or JVM unit tests failed (reports in the job artifact)"
  apk="$(find "$android_dir/app/build/outputs/apk/debug" -maxdepth 1 -name '*.apk' | sort | head -n 1)"
  test_apk="$(find "$android_dir/app/build/outputs/apk/androidTest/debug" -maxdepth 1 -name '*.apk' | sort | head -n 1)"
  [[ -s "$apk" && -s "$test_apk" ]] || die "debug APK or test APK missing"
  python3 - "$apk" <<'PY'
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as apk:
    names = set(apk.namelist())
if "lib/x86_64/libsmart_explorer_android.so" not in names:
    sys.exit("debug APK lacks lib/x86_64/libsmart_explorer_android.so")
print("debug APK contains lib/x86_64/libsmart_explorer_android.so")
PY
  mkdir -p "$out"
  cp -- "$apk" "$out/app-debug.apk"
  cp -- "$test_apk" "$out/app-debug-androidTest.apk"
  step "android-build passed: G3 (tree, arm64 check, x86_64 .so 16 KB, APKs, JVM unit tests)"
}

# ---------------------------------------------------------------------------------------------
# Emulator (G4/G5/G6). Every stage runs; failures are collected and the artifacts are always kept.
emulator_failures=()
emulator_out=""
logcat_pid=""

fail_stage() {
  emulator_failures+=("$1")
  echo "::error::android task suite: $1" >&2
}

adb_shell() {
  adb shell "$@" | tr -d '\r'
}

wait_boot() {
  local deadline=$((SECONDS + 600))
  adb wait-for-device
  until [[ "$(adb_shell getprop sys.boot_completed 2>/dev/null)" == 1 ]]; do
    ((SECONDS < deadline)) || die "emulator did not finish booting"
    sleep 2
  done
}

wait_sdcard() {
  local deadline=$((SECONDS + 240)) volumes disk tried=0
  while ((SECONDS < deadline)); do
    volumes="$(adb_shell sm list-volumes public 2>/dev/null || true)"
    if grep -q ' mounted ' <<<"$volumes"; then
      echo "SD card: $volumes"
      return 0
    fi
    if ((tried == 0 && SECONDS > deadline - 180)); then
      disk="$(adb_shell sm list-disks 2>/dev/null | head -n 1)"
      if [[ -n "$disk" ]]; then
        echo "partitioning $disk as portable storage"
        adb_shell sm partition "$disk" public || true
      fi
      tried=1
    fi
    sleep 3
  done
  echo "public volumes: $volumes" >&2
  return 1
}

# Writes a file into the app's private data through run-as (debuggable build).
app_write() {
  local target=$1 source=$2
  adb shell "run-as $app_package sh -c 'mkdir -p \$(dirname $target) && cat > $target'" <"$source"
  [[ "$(adb exec-out run-as "$app_package" cat "$target")" == "$(cat "$source")" ]] || die "could not write $target into the app data"
}

# am instrument with strict evaluation; a failed run is recorded and the suite continues.
instrumentation_spec() {
  local spec="" class
  for class in ${1//,/ }; do
    spec+="${spec:+,}$task_package.$class"
  done
  printf '%s\n' "$spec"
}

# `am instrument` only (may run in the background); instrumentation_check evaluates the output.
instrumentation_start() {
  local name=$1 classes=$2
  shift 2
  timeout 5400 adb shell am instrument -w -r -e class "$(instrumentation_spec "$classes")" "$@" "$test_runner" \
    >"$emulator_out/instrumentation-$name.txt" 2>&1 || true
}

instrumentation_check() {
  local name=$1 classes=$2
  if ! python3 "$eval_py" instrumentation --name "$name" --output "$emulator_out/instrumentation-$name.txt" \
    --sources "$android_test_sources" --classes "$(instrumentation_spec "$classes")" \
    --summary "$emulator_out/instrumentation-$name.tsv"; then
    fail_stage "instrumentation $name failed (see instrumentation-$name.txt)"
    adb exec-out screencap -p >"$emulator_out/screen-after-$name.png" 2>/dev/null || true
    return 1
  fi
}

run_instrumentation() {
  step "instrumentation $1: $2"
  instrumentation_start "$@"
  instrumentation_check "$1" "$2"
}

# Exec host (phase A2): the phone's test and the desktop's `se exec` run at the same time and
# coordinate through marker files on the phone (share-desktop.sh `exec`).
share_exec_host() {
  local tool=$1 root=$2
  shift 2
  step "instrumentation share-exec: ShareExecTaskTest while the desktop CLI runs commands on the phone"
  adb_shell rm -rf /sdcard/SmartExplorerTask/exec-host || true
  instrumentation_start share-exec ShareExecTaskTest "$@" &
  local instrument_pid=$!
  if bash "$tool" exec "$root" "$emulator_out/instrumentation-share-exec.txt" >"$emulator_out/share-desktop-exec.log" 2>&1; then
    cat "$emulator_out/share-desktop-exec.log"
  else
    cat "$emulator_out/share-desktop-exec.log" >&2
    fail_stage "the desktop CLI could not run, cancel or be refused commands on the phone (share-desktop-exec.log)"
  fi
  wait "$instrument_pid" || true
  instrumentation_check share-exec ShareExecTaskTest || true
}

# A real reboot: only the system's BOOT_COMPLETED carries the exemption that lets a receiver start
# a foreground service (a shell broadcast has none, so the start is denied).
boot_check() {
  step "G4 device reboot: BOOT_COMPLETED starts only the specialUse background service"
  adb reboot
  sleep 5
  wait_boot
  if [[ -n "$logcat_pid" ]]; then
    kill "$logcat_pid" 2>/dev/null || true
    wait "$logcat_pid" 2>/dev/null || true
  fi
  adb logcat -v threadtime >"$emulator_out/logcat-after-reboot.txt" 2>&1 &
  logcat_pid=$!
  adb_shell getprop sys.boot_completed | sed 's/^/sys.boot_completed after reboot: /' | tee "$emulator_out/boot-broadcast.txt"
  local deadline=$((SECONDS + 180)) services=""
  while ((SECONDS < deadline)); do
    services="$(adb_shell dumpsys activity services "$app_package" || true)"
    if grep -q 'service.BackgroundService' <<<"$services" && grep -q 'isForeground=true' <<<"$services"; then
      break
    fi
    sleep 2
  done
  printf '%s\n' "$services" >"$emulator_out/boot-services.txt"
  adb_shell dumpsys notification --noredact >"$emulator_out/boot-notifications.txt" 2>&1 || true
  if ! { grep -q 'service.BackgroundService' <<<"$services" && grep -q 'isForeground=true' <<<"$services"; }; then
    fail_stage "BackgroundService did not start in the foreground after the reboot"
    return 1
  fi
  if grep -q 'service.TaskForegroundService' <<<"$services"; then
    fail_stage "BOOT_COMPLETED started the dataSync task service"
    return 1
  fi
  grep -Eq "pkg=$app_package .*id=1002" "$emulator_out/boot-notifications.txt" ||
    { fail_stage "no background notification (id 1002) after the reboot"; return 1; }
  echo "boot: BackgroundService in the foreground with its notification; no dataSync service"
}

collect_emulator() {
  local report="$emulator_out/app-report"
  mkdir -p "$report"
  adb exec-out run-as "$app_package" tar -cf - -C files task-report >"$emulator_out/app-report.tar" 2>/dev/null || true
  tar -xf "$emulator_out/app-report.tar" -C "$report" 2>/dev/null || true
  adb exec-out run-as "$app_package" cat files/smart_explorer/android-fs.log >"$emulator_out/android-fs.log" 2>/dev/null || true
  adb exec-out run-as "$app_package" cat files/smart_explorer/crash.log >"$emulator_out/crash.log" 2>/dev/null || true
  adb exec-out screencap -p >"$emulator_out/screen-final.png" 2>/dev/null || true
  adb_shell dumpsys activity services "$app_package" >"$emulator_out/services-final.txt" 2>&1 || true
}

emulator_cleanup() {
  local status=$?
  # Cleanup steps may fail (a killed logcat reports 143 to wait); the ERR trap fires even
  # under `set +e` and must not turn a passed run into a failure.
  trap - ERR
  set +e
  collect_emulator
  bash "$servers_dir/share-desktop.sh" down "$emulator_out/share" "$emulator_out/share-desktop" >/dev/null 2>&1
  servers_down "$emulator_out/servers" 2>/dev/null
  if [[ -n "$logcat_pid" ]]; then
    kill "$logcat_pid" 2>/dev/null
    wait "$logcat_pid" 2>/dev/null
  fi
  return "$status"
}

cmd_emulator_run() {
  local artifacts="${SE_TASK_ARTIFACTS:-}" app_apk test_apk se_bin server_bin version next_version feed_sha
  [[ -n "$artifacts" ]] || die "SE_TASK_ARTIFACTS must point to the downloaded apk/ and bin/ artifacts"
  app_apk="$artifacts/apk/app-debug.apk"
  test_apk="$artifacts/apk/app-debug-androidTest.apk"
  se_bin="$artifacts/bin/se"
  server_bin="$artifacts/bin/se-share-server"
  for file in "$app_apk" "$test_apk" "$se_bin" "$server_bin"; do
    [[ -s "$file" ]] || die "artifact missing: $file"
  done
  chmod +x "$se_bin" "$server_bin"
  require_tools adb docker python3 jq curl sha256sum timeout tar
  # shellcheck source=android/test-servers/servers.sh
  source "$servers_dir/servers.sh"
  emulator_out="$log_root/emulator"
  mkdir -p "$emulator_out"
  trap emulator_cleanup EXIT

  step "device preparation"
  wait_boot
  adb root >/dev/null 2>&1 || true
  sleep 3
  wait_boot
  adb logcat -c || true
  adb logcat -v threadtime >"$emulator_out/logcat.txt" 2>&1 &
  logcat_pid=$!
  wait_sdcard || die "the emulator reports no mounted SD card (sdcard-path-or-size)"
  adb_shell getprop ro.build.version.sdk | sed 's/^/API level: /'

  servers_up
  version="$(sed -nE '/^version[[:space:]]*=[[:space:]]*"/{s/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p;q}' "$native_dir/Cargo.toml")"
  [[ "$version" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || die "native/Cargo.toml has no X.Y.Z version"
  next_version="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((BASH_REMATCH[3] + 1))"
  feed_prepare "$emulator_out/feed" "$app_apk" "$next_version"
  feed_sha="$(awk '{ print $1 }' "$emulator_out/feed/smart-explorer-android.apk.sha256")"
  feed_up "$emulator_out/feed" "$emulator_out/feed-server.log"

  adb install -r -t -g "$app_apk"
  adb install -r -t -g "$test_apk"
  adb_shell pm list instrumentation | grep -F "$test_runner" || die "instrumentation $test_runner not installed"
  adb_shell appops set --uid "$app_package" MANAGE_EXTERNAL_STORAGE allow
  adb_shell pm grant "$app_package" android.permission.POST_NOTIFICATIONS
  adb_shell am force-stop "$app_package"
  cat >"$emulator_out/app_prefs.xml" <<'XML'
<?xml version='1.0' encoding='utf-8' standalone='yes' ?>
<map>
    <boolean name="onboarding_done" value="true" />
    <boolean name="auto_update_check" value="false" />
</map>
XML
  printf '{"updateFeedUrl":"%s"}\n' "$(feed_url)" >"$emulator_out/test-overrides.json"
  app_write shared_prefs/app_prefs.xml "$emulator_out/app_prefs.xml"
  app_write files/test-overrides.json "$emulator_out/test-overrides.json"

  local server_args=() all_classes=() covered=() class
  mapfile -t server_args < <(servers_instrumentation_args)
  server_args+=(-e seFeedVersion "$next_version" -e seFeedSha256 "$feed_sha")
  local main_classes=SystemTaskTest,LocalFilesTaskTest,ScanAnalyzeTaskTest,RemoteTaskTest,SyncTaskTest,BackgroundTaskTest,ServicesTaskTest,IntentsTaskTest,UpdateTaskTest,UiTaskTest
  IFS=, read -r -a covered <<<"$main_classes"
  covered+=(RenameProbeTaskTest ShareRoomTaskTest ShareExecTaskTest ShareCleanupTaskTest BootPrepTaskTest)
  mapfile -t all_classes < <(python3 "$eval_py" classes --sources "$android_test_sources")
  for class in "${all_classes[@]}"; do
    [[ " ${covered[*]} " == *" ${class##*.} "* ]] || die "instrumented test class $class is not part of any run"
  done

  run_instrumentation main "$main_classes" "${server_args[@]}" || true
  # The core logs each rename fallback (attempt, errno) once per process: one process per volume.
  run_instrumentation rename-internal "RenameProbeTaskTest#internalVolume" "${server_args[@]}" || true
  run_instrumentation rename-sdcard "RenameProbeTaskTest#sdCardVolume" "${server_args[@]}" || true

  step "G5 Share with the desktop CLI"
  local share_args=() share_tool="$servers_dir/share-desktop.sh" share_root="$emulator_out/share"
  # Own processes with errexit; output to files (the CLI daemon may inherit the descriptors).
  if bash "$share_tool" up "$share_root" "$se_bin" "$server_bin" >"$emulator_out/share-desktop-up.log" 2>&1; then
    cat "$emulator_out/share-desktop-up.log"
    mapfile -t share_args < <(bash "$share_tool" args "$share_root")
    if run_instrumentation share-join ShareRoomTaskTest "${server_args[@]}" "${share_args[@]}"; then
      if bash "$share_tool" members "$share_root" 1 >"$emulator_out/share-desktop-members.log" 2>&1; then
        cat "$emulator_out/share-desktop-members.log"
        share_exec_host "$share_tool" "$share_root" "${server_args[@]}" "${share_args[@]}"
      else
        cat "$emulator_out/share-desktop-members.log" >&2
        fail_stage "the desktop CLI does not see the phone as a Room member"
      fi
    fi
    run_instrumentation share-cleanup ShareCleanupTaskTest "${server_args[@]}" "${share_args[@]}" || true
  else
    cat "$emulator_out/share-desktop-up.log" >&2
    fail_stage "desktop Share side could not be prepared (share-desktop-up.log)"
  fi

  run_instrumentation boot-prep BootPrepTaskTest || true
  boot_check || true

  step "api.md coverage and evidence"
  collect_emulator
  python3 "$eval_py" coverage --api "$api_md" --calls "$emulator_out/app-report/task-report/calls.tsv" --out "$emulator_out" ||
    fail_stage "api.md coverage incomplete (api-coverage.md)"
  local probe
  for probe in internal sdcard; do
    if [[ -s "$emulator_out/app-report/task-report/rename-errno-$probe.txt" ]]; then
      sed "s/^/rename $probe: /" "$emulator_out/app-report/task-report/rename-errno-$probe.txt"
    else
      fail_stage "rename errno record for the $probe volume missing"
    fi
  done
  find "$emulator_out/app-report/task-report/screens" -name '*.png' -printf 'screenshot: %f\n' 2>/dev/null | sort || true

  if [[ "${#emulator_failures[@]}" -gt 0 ]]; then
    printf 'FAILED: %s\n' "${emulator_failures[@]}" | tee "$emulator_out/summary.txt" >&2
    exit 1
  fi
  echo "emulator-run passed: G4, G5, G6" | tee "$emulator_out/summary.txt"
}

# ---------------------------------------------------------------------------------------------
[[ "$#" -ge 1 ]] || {
  usage
  exit 2
}
command_name=$1
shift
case "$command_name" in
  host)
    [[ "$#" -eq 0 ]] || { usage; exit 2; }
    cmd_host
    ;;
  desktop-bins | android-build)
    [[ "$#" -eq 2 && "$1" == --out ]] || { usage; exit 2; }
    if [[ "$command_name" == desktop-bins ]]; then cmd_desktop_bins "$2"; else cmd_android_build "$2"; fi
    ;;
  emulator-run)
    [[ "$#" -eq 0 ]] || { usage; exit 2; }
    cmd_emulator_run
    ;;
  -h | --help)
    usage
    ;;
  *)
    usage
    exit 2
    ;;
esac
