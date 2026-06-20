# Graph Report - .  (2026-06-20)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 958 nodes · 1758 edges · 76 communities (74 shown, 2 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 56 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `d283902b`
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
- [[_COMMUNITY_Community 46|Community 46]]

## God Nodes (most connected - your core abstractions)
1. `String` - 17 edges
2. `IconCache` - 16 edges
3. `Result` - 14 edges
4. `App` - 14 edges
5. `authorize()` - 14 edges
6. `walk_parallel()` - 14 edges
7. `App` - 13 edges
8. `String` - 13 edges
9. `run_copy()` - 13 edges
10. `walk_folders()` - 13 edges

## Surprising Connections (you probably didn't know these)
- `basic()` --calls--> `fuzzy_score()`  [INFERRED]
  native/src/folder_index/core/tests.rs → native/src/folder_index/core/search.rs
- `s()` --calls--> `fuzzy_score()`  [INFERRED]
  native/src/folder_index/core/tests.rs → native/src/folder_index/core/search.rs
- `walk_parallel()` --calls--> `ext_of()`  [INFERRED]
  native/src/scanner/os/shared.rs → native/src/rscan/os/shared/rscan.rs
- `walk_into_vec()` --calls--> `ext_of()`  [INFERRED]
  native/src/scanner/os/shared.rs → native/src/rscan/os/shared/rscan.rs
- `list_archived_versions()` --calls--> `parse_ver()`  [INFERRED]
  native/src/updater/os/shared/archive.rs → native/src/updater/core/core.rs

## Import Cycles
- 1-file cycle: `native/src/agent/core/metadata.rs -> native/src/agent/core/metadata.rs`
- 1-file cycle: `native/src/sftp/os/shared/known_hosts.rs -> native/src/sftp/os/shared/known_hosts.rs`
- 1-file cycle: `native/src/updater/core/core.rs -> native/src/updater/core/core.rs`
- 1-file cycle: `native/src/autostart/os/linux_os.rs -> native/src/autostart/os/linux_os.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater.rs -> native/src/bin/smart_explorer_updater.rs`
- 1-file cycle: `native/src/bisync/core/types.rs -> native/src/bisync/core/types.rs`
- 1-file cycle: `native/src/bisync/os/shared/persistence.rs -> native/src/bisync/os/shared/persistence.rs`
- 1-file cycle: `native/src/cloud/os/shared.rs -> native/src/cloud/os/shared.rs`
- 1-file cycle: `native/src/connect/core/types.rs -> native/src/connect/core/types.rs`
- 1-file cycle: `native/src/copy/os/shared/copy.rs -> native/src/copy/os/shared/copy.rs`
- 1-file cycle: `native/src/creds/os/shared.rs -> native/src/creds/os/shared.rs`
- 1-file cycle: `native/src/filter/core/filter.rs -> native/src/filter/core/filter.rs`
- 1-file cycle: `native/src/folder_index/core/model.rs -> native/src/folder_index/core/model.rs`
- 1-file cycle: `native/src/folder_index/os/shared.rs -> native/src/folder_index/os/shared.rs`
- 1-file cycle: `native/src/rscan/os/shared/rscan.rs -> native/src/rscan/os/shared/rscan.rs`
- 1-file cycle: `native/src/scanner/os/shared.rs -> native/src/scanner/os/shared.rs`
- 1-file cycle: `native/src/sftp/core/metadata.rs -> native/src/sftp/core/metadata.rs`
- 1-file cycle: `native/src/support_dirs.rs -> native/src/support_dirs.rs`
- 1-file cycle: `native/src/syncjobs/os/shared/persistence.rs -> native/src/syncjobs/os/shared/persistence.rs`
- 1-file cycle: `native/src/types/core/types.rs -> native/src/types/core/types.rs`

## Communities (76 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (46): Backend, App, accel_key(), AccelAct, app_data_file(), BisyncCtx, cache_remote(), count_subtree() (+38 more)

### Community 1 - "Community 1"
Cohesion: 0.08
Nodes (30): Conflict, CopyOptions, sel_key_path(), PathBuf, String, Vec, Option, String (+22 more)

### Community 2 - "Community 2"
Cohesion: 0.06
Nodes (34): App, NaiveDate, ConnectForm, Option, SavedConnection, String, Color32, FileEntry (+26 more)

### Community 3 - "Community 3"
Cohesion: 0.10
Nodes (39): ClientConfig, accept_with_deadline(), auth_url_has_required_params(), authorize(), build_auth_url(), ClientConfig, now_secs(), parse_redirect() (+31 more)

### Community 4 - "Community 4"
Cohesion: 0.11
Nodes (39): archived_versions_parse_and_sort_numerically(), pin_roundtrip(), Duration, Feed, Path, PathBuf, Result, String (+31 more)

### Community 5 - "Community 5"
Cohesion: 0.12
Nodes (36): is_newer(), parse_sha256_file(), parse_ver(), payload_suffix(), sha256_file(), staged_payload_path(), verify_sha256(), Path (+28 more)

### Community 6 - "Community 6"
Cohesion: 0.07
Nodes (26): BisyncOptions, CompiledFilter, parse_size_input(), gen_id(), glob_set_matches_ignores(), SyncJob, Trigger, GlobMatcher (+18 more)

### Community 7 - "Community 7"
Cohesion: 0.17
Nodes (26): bad(), Frame, get_meta(), get_node(), put_bool(), put_bytes(), put_i64(), put_meta() (+18 more)

### Community 8 - "Community 8"
Cohesion: 0.08
Nodes (18): Action, BisyncOptions, BisyncStats, CompareMode, ConflictMode, DeletePolicy, Direction, Sig (+10 more)

### Community 9 - "Community 9"
Cohesion: 0.16
Nodes (32): Option, PathBuf, Result, String, Duration, Error, Feed, Path (+24 more)

### Community 10 - "Community 10"
Cohesion: 0.15
Nodes (30): Option, PathBuf, String, autopause_flags(), autopause_path(), cadence_path(), cadence_secs(), clear_heartbeat() (+22 more)

### Community 11 - "Community 11"
Cohesion: 0.18
Nodes (28): Option, Path, PathBuf, Result, app_data_dir(), String, SyncJob, Vec (+20 more)

### Community 12 - "Community 12"
Cohesion: 0.19
Nodes (24): Metadata, Arc, AtomicBool, AtomicU64, FileEntry, Instant, Mutex, Option (+16 more)

### Community 13 - "Community 13"
Cohesion: 0.20
Nodes (23): appdata_dir(), append_log(), apply_update(), ApplyArgs, arg_value(), default_error_file(), join_windows_args(), main() (+15 more)

### Community 14 - "Community 14"
Cohesion: 0.12
Nodes (20): CopyMode, CopyOptions, CopyProgress, FileEntry, FilterDef, Range, Range<T>, ScanProgress (+12 more)

### Community 15 - "Community 15"
Cohesion: 0.17
Nodes (19): IndexMsg, Metadata, Arc, AtomicBool, AtomicU64, Instant, Mutex, walk_parallel() (+11 more)

### Community 16 - "Community 16"
Cohesion: 0.13
Nodes (16): icon_key(), IconCache, IconKind, IconResult, key_to_kind(), Context, Default, HashMap (+8 more)

### Community 17 - "Community 17"
Cohesion: 0.24
Nodes (22): Arc, AtomicBool, BackendHandle, HashSet, Option, PathBuf, Receiver, ScanMessage (+14 more)

### Community 18 - "Community 18"
Cohesion: 0.18
Nodes (21): Fn, Baseline, Option, Path, PathBuf, Result, app_data_dir(), String (+13 more)

### Community 19 - "Community 19"
Cohesion: 0.23
Nodes (21): Option, Path, PathBuf, Result, SavedConnection, String, Vec, app_data_dir() (+13 more)

### Community 20 - "Community 20"
Cohesion: 0.23
Nodes (18): assemble(), assemble_rows(), Choice, choices_assemble_correctly(), diff(), Hunk, identical_is_one_equal_hunk(), middle_change_splits_into_three() (+10 more)

### Community 21 - "Community 21"
Cohesion: 0.18
Nodes (13): enc(), ep_prefix(), gdrive_endpoint(), norm_root(), parse_remote_url(), remote_endpoint(), remote_url_detection_and_parse(), saved_and_path() (+5 more)

### Community 22 - "Community 22"
Cohesion: 0.16
Nodes (13): io_err(), backend_from_url(), parse_sftp_url(), SftpUrl, url_defaults(), url_full(), url_without_password_needs_dialog(), E (+5 more)

### Community 23 - "Community 23"
Cohesion: 0.17
Nodes (14): Connected, ConnectForm, ConnectResult, RemoteState, Arc, BackendHandle, Default, Option (+6 more)

### Community 24 - "Community 24"
Cohesion: 0.17
Nodes (13): hm_to_min(), JobEditor, min_to_hm(), CompareMode, ConflictMode, DeletePolicy, Direction, Option (+5 more)

### Community 25 - "Community 25"
Cohesion: 0.18
Nodes (3): App, PathBuf, String

### Community 26 - "Community 26"
Cohesion: 0.18
Nodes (7): FolderIndex, IndexMsg, Item, Iterator, HashSet, Self, String

### Community 27 - "Community 27"
Cohesion: 0.18
Nodes (8): FolderIndex, fuzzy_score(), basic(), s(), FolderIndex, Option, String, Vec

### Community 28 - "Community 28"
Cohesion: 0.33
Nodes (9): PathBuf, Result, String, autostart_dir(), desktop_file_path(), disable(), enable(), is_enabled() (+1 more)

### Community 29 - "Community 29"
Cohesion: 0.22
Nodes (7): CmdTx, ShareService, EventRx, Result, String, ShareCmd, TcpListener

### Community 30 - "Community 30"
Cohesion: 0.33
Nodes (4): App, Context, String, Ui

### Community 31 - "Community 31"
Cohesion: 0.27
Nodes (7): cloud_urlenc(), norm(), norm_and_split(), parse_rfc3339_ms(), split_parent(), Option, String

### Community 32 - "Community 32"
Cohesion: 0.29
Nodes (5): Result, String, disable(), enable(), exe_path()

### Community 33 - "Community 33"
Cohesion: 0.38
Nodes (6): Frame, SearchSpec, WireMeta, WireNode, String, Vec

### Community 34 - "Community 34"
Cohesion: 0.33
Nodes (4): DriveInfo, removable_drives(), String, Vec

### Community 35 - "Community 35"
Cohesion: 0.47
Nodes (5): basename(), to_vfs(), FileAttributes, String, VfsMeta

### Community 36 - "Community 36"
Cohesion: 0.47
Nodes (5): IconData, Result, install_panic_logger(), main(), window_icon()

### Community 37 - "Community 37"
Cohesion: 0.60
Nodes (5): PathBuf, PublicKey, app_data_dir(), known_hosts_accept(), known_hosts_path()

### Community 38 - "Community 38"
Cohesion: 0.80
Nodes (5): PathBuf, app_data_dir(), app_data_file(), data_home(), sync_data_dir()

### Community 39 - "Community 39"
Cohesion: 0.40
Nodes (3): Option, Result, connect_impl()

### Community 40 - "Community 40"
Cohesion: 0.67
Nodes (3): SftpAuth, SftpConfig, String

### Community 41 - "Community 41"
Cohesion: 0.83
Nodes (3): wire_to_vfs(), VfsMeta, WireMeta

## Knowledge Gaps
- **146 isolated node(s):** `Self`, `Error`, `Vec`, `Frame`, `Context` (+141 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `sel_key_path()` connect `Community 1` to `Community 14`?**
  _High betweenness centrality (0.014) - this node is a cross-community bridge._
- **Why does `parse_ver()` connect `Community 5` to `Community 4`?**
  _High betweenness centrality (0.014) - this node is a cross-community bridge._
- **Why does `confirm_yes_no()` connect `Community 1` to `Community 2`?**
  _High betweenness centrality (0.012) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `authorize()` (e.g. with `load_config()` and `store_refresh_token()`) actually correct?**
  _`authorize()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Self`, `Error`, `Vec` to the rest of the system?**
  _146 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05429864253393665 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.07890070921985816 - nodes in this community are weakly interconnected._