# Graph Report - .  (2026-06-27)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 2336 nodes · 4815 edges · 131 communities (121 shown, 10 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 230 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `f18e77a2`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 8|Community 8]]
- [[_COMMUNITY_Community 9|Community 9]]
- [[_COMMUNITY_Community 10|Community 10]]
- [[_COMMUNITY_Community 11|Community 11]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_Community 20|Community 20]]
- [[_COMMUNITY_Community 21|Community 21]]
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 23|Community 23]]
- [[_COMMUNITY_Community 24|Community 24]]
- [[_COMMUNITY_Community 25|Community 25]]
- [[_COMMUNITY_Community 26|Community 26]]
- [[_COMMUNITY_Community 27|Community 27]]
- [[_COMMUNITY_Community 28|Community 28]]
- [[_COMMUNITY_Community 29|Community 29]]
- [[_COMMUNITY_Community 30|Community 30]]
- [[_COMMUNITY_Community 31|Community 31]]
- [[_COMMUNITY_Community 32|Community 32]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 37|Community 37]]
- [[_COMMUNITY_Community 38|Community 38]]
- [[_COMMUNITY_Community 39|Community 39]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Community 43|Community 43]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 46|Community 46]]
- [[_COMMUNITY_Community 47|Community 47]]
- [[_COMMUNITY_Community 48|Community 48]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 51|Community 51]]
- [[_COMMUNITY_Community 52|Community 52]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 54|Community 54]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 56|Community 56]]
- [[_COMMUNITY_Community 57|Community 57]]
- [[_COMMUNITY_Community 58|Community 58]]
- [[_COMMUNITY_Community 59|Community 59]]
- [[_COMMUNITY_Community 60|Community 60]]
- [[_COMMUNITY_Community 61|Community 61]]
- [[_COMMUNITY_Community 62|Community 62]]
- [[_COMMUNITY_Community 63|Community 63]]
- [[_COMMUNITY_Community 64|Community 64]]
- [[_COMMUNITY_Community 65|Community 65]]
- [[_COMMUNITY_Community 66|Community 66]]
- [[_COMMUNITY_Community 67|Community 67]]
- [[_COMMUNITY_Community 68|Community 68]]
- [[_COMMUNITY_Community 69|Community 69]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 71|Community 71]]
- [[_COMMUNITY_Community 72|Community 72]]
- [[_COMMUNITY_Community 73|Community 73]]
- [[_COMMUNITY_Community 74|Community 74]]
- [[_COMMUNITY_Community 75|Community 75]]
- [[_COMMUNITY_Community 76|Community 76]]
- [[_COMMUNITY_Community 77|Community 77]]
- [[_COMMUNITY_Community 78|Community 78]]
- [[_COMMUNITY_Community 79|Community 79]]
- [[_COMMUNITY_Community 80|Community 80]]
- [[_COMMUNITY_Community 81|Community 81]]
- [[_COMMUNITY_Community 82|Community 82]]
- [[_COMMUNITY_Community 83|Community 83]]
- [[_COMMUNITY_Community 84|Community 84]]
- [[_COMMUNITY_Community 85|Community 85]]
- [[_COMMUNITY_Community 86|Community 86]]
- [[_COMMUNITY_Community 87|Community 87]]
- [[_COMMUNITY_Community 89|Community 89]]
- [[_COMMUNITY_Community 90|Community 90]]
- [[_COMMUNITY_Community 91|Community 91]]
- [[_COMMUNITY_Community 92|Community 92]]
- [[_COMMUNITY_Community 93|Community 93]]
- [[_COMMUNITY_Community 94|Community 94]]
- [[_COMMUNITY_Community 96|Community 96]]
- [[_COMMUNITY_Community 97|Community 97]]
- [[_COMMUNITY_Community 98|Community 98]]
- [[_COMMUNITY_Community 99|Community 99]]
- [[_COMMUNITY_Community 101|Community 101]]

## God Nodes (most connected - your core abstractions)
1. `App` - 78 edges
2. `ShareIrohNode` - 24 edges
3. `worker()` - 24 edges
4. `run()` - 23 edges
5. `PeerBackend` - 23 edges
6. `Result` - 23 edges
7. `WebdavBackend` - 23 edges
8. `ShareService` - 22 edges
9. `TabState` - 20 edges
10. `dispatch_backend()` - 20 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `arg_value()`  [INFERRED]
  native/src/bin/smart_explorer_updater.rs → native/src/bin/smart_explorer_updater/args.rs
- `backup_failure_blocks_overwrite_and_delete()` --calls--> `tmp()`  [INFERRED]
  native/src/bisync/os/shared/tests/safety.rs → native/src/bisync/os/shared/tests.rs
- `serve()` --calls--> `read_frame()`  [INFERRED]
  native/src/agent_proto/core/server.rs → native/src/agent_proto/core/codec.rs
- `copy_remote_paths_progress()` --calls--> `rjoin()`  [INFERRED]
  native/src/app/os/shared/remote_helpers/remote_copy.rs → native/src/app/os/shared/remote_helpers.rs
- `upload_paths_progress()` --calls--> `rjoin()`  [INFERRED]
  native/src/app/os/shared/remote_helpers/uploads.rs → native/src/app/os/shared/remote_helpers.rs

## Import Cycles
- 1-file cycle: `native/src/agent/core/mux.rs -> native/src/agent/core/mux.rs`
- 1-file cycle: `native/src/bisync/core/types.rs -> native/src/bisync/core/types.rs`
- 1-file cycle: `native/src/app/core/state.rs -> native/src/app/core/state.rs`
- 1-file cycle: `native/src/app/os/linux_os.rs -> native/src/app/os/linux_os.rs`
- 1-file cycle: `native/src/app/os/windows/platform.rs -> native/src/app/os/windows/platform.rs`
- 1-file cycle: `native/src/app/os/shared/remote_helpers/downloads.rs -> native/src/app/os/shared/remote_helpers/downloads.rs`
- 1-file cycle: `native/src/app/os/shared/remote_helpers/entries.rs -> native/src/app/os/shared/remote_helpers/entries.rs`
- 1-file cycle: `native/src/app/os/shared/remote_helpers/remote_copy.rs -> native/src/app/os/shared/remote_helpers/remote_copy.rs`
- 1-file cycle: `native/src/app/os/shared/remote_helpers/temp.rs -> native/src/app/os/shared/remote_helpers/temp.rs`
- 1-file cycle: `native/src/app/os/shared/remote_helpers/tests.rs -> native/src/app/os/shared/remote_helpers/tests.rs`
- 1-file cycle: `native/src/app/os/shared/remote_helpers/uploads.rs -> native/src/app/os/shared/remote_helpers/uploads.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/args.rs -> native/src/bin/smart_explorer_updater/args.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/hash.rs -> native/src/bin/smart_explorer_updater/hash.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/launch.rs -> native/src/bin/smart_explorer_updater/launch.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/logging.rs -> native/src/bin/smart_explorer_updater/logging.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/process.rs -> native/src/bin/smart_explorer_updater/process.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/replace.rs -> native/src/bin/smart_explorer_updater/replace.rs`
- 1-file cycle: `native/src/bisync/os/shared/apply.rs -> native/src/bisync/os/shared/apply.rs`
- 1-file cycle: `native/src/bisync/os/shared/orchestration.rs -> native/src/bisync/os/shared/orchestration.rs`
- 1-file cycle: `native/src/bisync/os/shared/persistence.rs -> native/src/bisync/os/shared/persistence.rs`

## Communities (131 total, 10 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (73): Connection, endpoint_addr(), fs_request_label(), fs_response_summary(), handle_fs_stream(), iroh_direct_session_transfers_files(), normalize_tcp_addr(), PeerBackend (+65 more)

### Community 1 - "Community 1"
Cohesion: 0.05
Nodes (78): Arc, Backend, BackendHandle, Box, Duration, E, Error, Instant (+70 more)

### Community 2 - "Community 2"
Cohesion: 0.05
Nodes (86): copy_file_checked(), is_newer(), parse_sha256_file(), parse_ver(), replace_file_with_staged(), sha256_file(), staged_payload_path(), staged_sha256_from_path() (+78 more)

### Community 3 - "Community 3"
Cohesion: 0.03
Nodes (77): AgentBackend, AnalyticsScan, AppErrorEntry, BisyncCtx, ClipKey, CopyHandle, CopyMode, CopyMsg (+69 more)

### Community 4 - "Community 4"
Cohesion: 0.05
Nodes (35): CopyOptions, sel_key_path(), App, Context, EditProcess, OpenMode, Option, PathBuf (+27 more)

### Community 5 - "Community 5"
Cohesion: 0.07
Nodes (54): CompiledFilter, Backend, FilterDef, Instant, Option, Path, Result, Sender (+46 more)

### Community 6 - "Community 6"
Cohesion: 0.08
Nodes (29): Agent, basename(), decode_path(), encode_path(), extract_md5(), href_path(), io_err(), parse_http_date_ms() (+21 more)

### Community 7 - "Community 7"
Cohesion: 0.10
Nodes (48): clean_mount_label(), connection_mounts(), dir_meta(), ensure_under_root(), from_os_path(), FsMeta, join_under(), list_dir() (+40 more)

### Community 8 - "Community 8"
Cohesion: 0.07
Nodes (36): App, Context, Frame, Option, BackendHandle, EditProcess, Option, Path (+28 more)

### Community 9 - "Community 9"
Cohesion: 0.09
Nodes (34): DateTime, Backend, Box, E, Error, HashMap, Option, Path (+26 more)

### Community 10 - "Community 10"
Cohesion: 0.07
Nodes (31): HANDLE, ClipboardEffect, ClipboardVirtualFile, Drop, OpenMode, Option, Path, Result (+23 more)

### Community 11 - "Community 11"
Cohesion: 0.12
Nodes (44): Arc, AtomicBool, BackendHandle, HashSet, Option, PathBuf, Receiver, ScanHandle (+36 more)

### Community 12 - "Community 12"
Cohesion: 0.13
Nodes (31): bad(), Frame, get_meta(), get_node(), put_bool(), put_bytes(), put_i64(), put_meta() (+23 more)

### Community 13 - "Community 13"
Cohesion: 0.10
Nodes (38): apply_update(), main(), ApplyArgs, Option, Path, PathBuf, Result, String (+30 more)

### Community 14 - "Community 14"
Cohesion: 0.09
Nodes (24): presence_hmac_covers_iroh_discovery_fields(), RecordingBackend, recursive_delete_does_not_follow_local_symlink_child_when_supported(), recursive_delete_rejects_symlink_like_directory_child(), recursive_delete_removes_normal_local_tree(), temp_path(), test_meta(), IntoIterator (+16 more)

### Community 15 - "Community 15"
Cohesion: 0.08
Nodes (19): BTreeMap, Action, BisyncOptions, BisyncStats, CompareMode, ConflictMode, DeletePolicy, Direction (+11 more)

### Community 16 - "Community 16"
Cohesion: 0.07
Nodes (34): AnalyticsScan, AppErrorEntry, empty_progress(), KbdAct, SummaryData, TabState, TmCell, TransferKind (+26 more)

### Community 17 - "Community 17"
Cohesion: 0.15
Nodes (23): App, landing_basename(), landing_sync_meta(), landing_tile(), landing_tile_grid(), landing_time_secs(), LandingAction, LandingTile (+15 more)

### Community 18 - "Community 18"
Cohesion: 0.17
Nodes (37): AtomicBool, AtomicU64, BackendHandle, Frame, Option, Read, Receiver, Result (+29 more)

### Community 19 - "Community 19"
Cohesion: 0.10
Nodes (22): ClipboardEffect, ClipboardVirtualFile, OpenMode, Option, Path, Result, String, Vec (+14 more)

### Community 20 - "Community 20"
Cohesion: 0.15
Nodes (28): Path, PathBuf, Duration, Into, Option, Path, PathBuf, Result (+20 more)

### Community 21 - "Community 21"
Cohesion: 0.11
Nodes (19): BOOL, FILETIME, FORMATETC, HRESULT, IAdviseSink, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA (+11 more)

### Community 22 - "Community 22"
Cohesion: 0.11
Nodes (16): Backend, Box, Metadata, Read, Scheme, Self, Send, String (+8 more)

### Community 23 - "Community 23"
Cohesion: 0.16
Nodes (25): default_direct_access_state(), direct_endpoint_round_trips_with_path(), DirectAccessState, DirectContact, DirectGrant, DirectGrantState, normalize_endpoint_path(), PeerEndpoint (+17 more)

### Community 24 - "Community 24"
Cohesion: 0.18
Nodes (28): Option, Path, PathBuf, Result, app_data_dir(), String, SyncJob, Vec (+20 more)

### Community 25 - "Community 25"
Cohesion: 0.20
Nodes (22): Error, Option, Path, PathBuf, Result, String, Vec, app_release_asset_name() (+14 more)

### Community 26 - "Community 26"
Cohesion: 0.13
Nodes (21): CompiledFilter, entry(), normalize_loose_spaces(), parse_size_input(), regex_filter_keeps_commas_literal(), substring_filter_is_lenient_about_spaces(), substring_filter_matches_plain_numbers(), substring_filter_uses_commas_as_and_terms() (+13 more)

### Community 27 - "Community 27"
Cohesion: 0.12
Nodes (13): GDriveBackend, Backend, Box, Fn, Option, Read, Scheme, Send (+5 more)

### Community 28 - "Community 28"
Cohesion: 0.10
Nodes (19): ClientMsg, Clone, CmdTx, ShareService, signal_endpoints(), ws_to_io(), AtomicBool, BackendHandle (+11 more)

### Community 29 - "Community 29"
Cohesion: 0.18
Nodes (26): Baseline, BisyncOptions, BisyncStats, Conflict, LocalBackend, Path, PathBuf, String (+18 more)

### Community 30 - "Community 30"
Cohesion: 0.17
Nodes (20): is_generic_id(), path_has_skipped_segment(), should_skip(), Arc, AtomicBool, AtomicU64, IndexMsg, Instant (+12 more)

### Community 31 - "Community 31"
Cohesion: 0.17
Nodes (22): Option, PathBuf, Result, Self, String, Vec, Path, PathBuf (+14 more)

### Community 32 - "Community 32"
Cohesion: 0.25
Nodes (24): Action, AtomicBool, Backend, BisyncOptions, BisyncStats, Option, Path, Result (+16 more)

### Community 33 - "Community 33"
Cohesion: 0.17
Nodes (21): CompareMode, GlobSet, AtomicBool, Backend, Baseline, Option, Result, Self (+13 more)

### Community 34 - "Community 34"
Cohesion: 0.12
Nodes (17): connect_rejects_non_unc(), connect_unsupported_off_windows(), is_unc(), NetConnection, share_root(), Drop, Option, Result (+9 more)

### Community 35 - "Community 35"
Cohesion: 0.12
Nodes (20): CopyMode, CopyOptions, CopyProgress, FileEntry, FilterDef, Range, Range<T>, ScanProgress (+12 more)

### Community 36 - "Community 36"
Cohesion: 0.17
Nodes (8): App, AccelAct, Context, OmniAction, Rect, Vec, OmniItem, fuzzy_contains()

### Community 37 - "Community 37"
Cohesion: 0.14
Nodes (15): icon_key(), IconCache, IconKind, IconResult, key_to_kind(), IconWorker, Context, Default (+7 more)

### Community 38 - "Community 38"
Cohesion: 0.20
Nodes (16): build_presence(), configure_updates_auth_state_synchronously(), direct_accept_or_reject_requires_signed_owner_presence(), dropping_probe_clone_does_not_stop_owner_service(), local_direct_request_requires_own_direct_secret(), nonce_cache_detects_replay(), presence_binds_node_id_and_relay_url(), set_ws_timeout() (+8 more)

### Community 39 - "Community 39"
Cohesion: 0.22
Nodes (19): HMENU, HWND, IContextMenu, Drop, Option, to_wide(), Result, String (+11 more)

### Community 40 - "Community 40"
Cohesion: 0.21
Nodes (19): b64(), b64_decode(), eio(), fill_random(), hex(), hex_decode(), hex_val(), hmac_proof() (+11 more)

### Community 41 - "Community 41"
Cohesion: 0.15
Nodes (9): Box, LocalBackend, Read, Scheme, Send, VfsMeta, VfsResult, Write (+1 more)

### Community 42 - "Community 42"
Cohesion: 0.16
Nodes (8): App, BackendHandle, Option, SizeNode, String, SummaryData, Ui, Vec

### Community 43 - "Community 43"
Cohesion: 0.17
Nodes (11): DriveWriter, GDriveBackend, open_writer(), Box, Drop, Result, Send, String (+3 more)

### Community 44 - "Community 44"
Cohesion: 0.20
Nodes (15): FILE_FLAGS_AND_ATTRIBUTES, HICON, IconKind, IconResult, Option, Receiver, Self, Sender (+7 more)

### Community 45 - "Community 45"
Cohesion: 0.17
Nodes (18): Action, AtomicBool, Backend, Baseline, BisyncOptions, BisyncStats, Conflict, Option (+10 more)

### Community 46 - "Community 46"
Cohesion: 0.17
Nodes (15): Drop, Duration, ExitStatus, Option, Result, String, Vec, acquire_daemon_instance_guard() (+7 more)

### Community 47 - "Community 47"
Cohesion: 0.23
Nodes (17): Duration, Option, Path, PathBuf, Result, String, Vec, clear_daemon_runtime_markers() (+9 more)

### Community 48 - "Community 48"
Cohesion: 0.24
Nodes (12): make_out_channel(), Mux, route_frame(), AtomicU64, Frame, Option, Receiver, Result (+4 more)

### Community 49 - "Community 49"
Cohesion: 0.30
Nodes (17): Backend, Instant, Path, PathBuf, Result, Sender, String, TransferMsg (+9 more)

### Community 50 - "Community 50"
Cohesion: 0.22
Nodes (17): Fn, Option, Path, PathBuf, app_data_dir(), Sig, String, baseline_path() (+9 more)

### Community 51 - "Community 51"
Cohesion: 0.21
Nodes (16): drive_err(), err(), export_ext(), export_format(), is_rate_limited(), not_found(), open_stream(), parse_json() (+8 more)

### Community 52 - "Community 52"
Cohesion: 0.21
Nodes (16): plan(), sig_eq(), sig_mtime(), sig_size(), update_baseline(), Action, Baseline, BisyncOptions (+8 more)

### Community 53 - "Community 53"
Cohesion: 0.17
Nodes (16): dispatch(), handle_walk_tree(), lock_or_recover(), serve(), AtomicBool, Frame, Mutex, MutexGuard (+8 more)

### Community 54 - "Community 54"
Cohesion: 0.24
Nodes (10): GDriveBackend, Arc, HashMap, HashSet, Mutex, MutexGuard, Result, Self (+2 more)

### Community 55 - "Community 55"
Cohesion: 0.20
Nodes (16): HashSet, Option, Path, PathBuf, Receiver, Sender, String, SyncJob (+8 more)

### Community 56 - "Community 56"
Cohesion: 0.39
Nodes (16): now_secs(), presence_payload(), verify_hmac(), handle_server_msg(), remember_nonce(), verify_direct_access_accepted(), verify_direct_access_accepted_using(), verify_direct_presence() (+8 more)

### Community 57 - "Community 57"
Cohesion: 0.21
Nodes (8): App, Context, Option, PickerState, SavedConnection, String, PickerPurpose, ensure_dir_root()

### Community 58 - "Community 58"
Cohesion: 0.27
Nodes (8): BackendHandle, Context, FilterDef, Option, Pos2, String, Vec, App

### Community 59 - "Community 59"
Cohesion: 0.22
Nodes (14): Backend, Result, Sig, String, Vec, Row, conflict_rel_name(), ep_join() (+6 more)

### Community 60 - "Community 60"
Cohesion: 0.15
Nodes (5): App, ConnectForm, Option, SavedConnection, String

### Community 61 - "Community 61"
Cohesion: 0.27
Nodes (7): GDriveBackend, meta_from_json_requires_a_usable_name(), Option, String, Value, VfsMeta, VfsResult

### Community 62 - "Community 62"
Cohesion: 0.16
Nodes (11): acquire_daemon_instance_guard(), DaemonInstanceGuard, DriveInfo, removable_drives(), run_shell_command(), Duration, ExitStatus, Option (+3 more)

### Community 63 - "Community 63"
Cohesion: 0.15
Nodes (12): Instant, Sender, TransferMsg, TransferProgress, Backend, FilterDef, Option, Sender (+4 more)

### Community 64 - "Community 64"
Cohesion: 0.19
Nodes (11): App, Option, PathBuf, Self, FolderIndex, PathBuf, dirs_home(), favorites_path() (+3 more)

### Community 65 - "Community 65"
Cohesion: 0.18
Nodes (11): Backend, Baseline, BisyncOptions, BisyncStats, Conflict, Path, String, Vec (+3 more)

### Community 66 - "Community 66"
Cohesion: 0.18
Nodes (9): ClipKey, PickerPurpose, PickerState, BackendHandle, ConnectResult, Option, Receiver, String (+1 more)

### Community 67 - "Community 67"
Cohesion: 0.44
Nodes (12): publish_all(), send_direct_answer(), send_direct_request(), send_direct_request_locked(), send_hello(), send_line(), worker(), DirectContact (+4 more)

### Community 68 - "Community 68"
Cohesion: 0.33
Nodes (4): App, Context, String, Ui

### Community 70 - "Community 70"
Cohesion: 0.22
Nodes (6): IconKind, IconResult, Self, String, Vec, IconWorker

### Community 71 - "Community 71"
Cohesion: 0.42
Nodes (4): normalize_signal_endpoint(), normalize_tcp_addr(), SignalConnection, Self

### Community 72 - "Community 72"
Cohesion: 0.31
Nodes (7): HashMap, Self, String, appdata_file(), load_dir_sort(), save_dir_sort(), UiState

### Community 74 - "Community 74"
Cohesion: 0.25
Nodes (7): FileEntry, Option, Receiver, ScanMessage, ScanProgress, Vec, drain_scan_channel()

### Community 75 - "Community 75"
Cohesion: 0.32
Nodes (4): App, Context, Ui, is_local_style()

### Community 76 - "Community 76"
Cohesion: 0.32
Nodes (5): NaiveDate, ClipboardEffect, ClipboardVirtualFile, date_to_ms_end(), date_to_ms_start()

### Community 77 - "Community 77"
Cohesion: 0.29
Nodes (4): BufRead, read_line_limited(), Result, String

### Community 78 - "Community 78"
Cohesion: 0.38
Nodes (4): GDriveBackend, String, Value, VfsResult

### Community 79 - "Community 79"
Cohesion: 0.29
Nodes (5): FolderIndex, fuzzy_score(), Option, String, Vec

### Community 80 - "Community 80"
Cohesion: 0.52
Nodes (6): Path, Result, String, ensure_firewall_rule(), ensure_firewall_rule_for(), request_firewall_rule_elevated()

### Community 81 - "Community 81"
Cohesion: 0.52
Nodes (6): Metadata, PathBuf, file_attributes(), is_reparse_point(), local_attrs(), to_os()

### Community 82 - "Community 82"
Cohesion: 0.47
Nodes (5): is_reparse_point(), local_attrs(), to_os(), Metadata, PathBuf

### Community 83 - "Community 83"
Cohesion: 0.80
Nodes (5): PathBuf, app_data_dir(), app_data_file(), data_home(), sync_data_dir()

### Community 84 - "Community 84"
Cohesion: 0.40
Nodes (4): drive_request(), DriveRequestResult, Response, Result

### Community 85 - "Community 85"
Cohesion: 0.40
Nodes (4): Mutex, MutexGuard, T, lock_or_recover()

### Community 86 - "Community 86"
Cohesion: 0.50
Nodes (3): io_err(), E, Error

### Community 89 - "Community 89"
Cohesion: 0.50
Nodes (3): ensure_firewall_rule(), Result, String

### Community 90 - "Community 90"
Cohesion: 0.50
Nodes (4): Color32, Rect, Ui, paint_cell_text()

### Community 91 - "Community 91"
Cohesion: 0.83
Nodes (3): SyncJob, run_cmd(), run_one()

### Community 92 - "Community 92"
Cohesion: 0.50
Nodes (3): String, Vec, lan_ips()

## Knowledge Gaps
- **464 isolated node(s):** `AtomicU64`, `Self`, `Option`, `Self`, `Error` (+459 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `now_secs()` connect `Community 56` to `Community 38`, `Community 40`, `Community 78`, `Community 55`, `Community 91`?**
  _High betweenness centrality (0.160) - this node is a cross-community bridge._
- **Why does `run_daemon()` connect `Community 55` to `Community 56`, `Community 1`?**
  _High betweenness centrality (0.146) - this node is a cross-community bridge._
- **Why does `start_listener()` connect `Community 1` to `Community 55`?**
  _High betweenness centrality (0.146) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `run()` (e.g. with `plan()` and `update_baseline()`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `AtomicU64`, `Self`, `Option` to the rest of the system?**
  _464 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05901696088611977 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.05171717171717172 - nodes in this community are weakly interconnected._