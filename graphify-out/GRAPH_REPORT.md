# Graph Report - .  (2026-06-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 219 nodes · 430 edges · 11 communities
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 23 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `6bf24861`
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

## God Nodes (most connected - your core abstractions)
1. `MockBackend` - 24 edges
2. `scan_reclaim_backend()` - 14 edges
3. `duplicate_groups()` - 14 edges
4. `scan_backend_dir()` - 13 edges
5. `scan_reclaim()` - 13 edges
6. `scan_dir()` - 13 edges
7. `Acc` - 11 edges
8. `prepare_reclaim_trash_plan()` - 11 edges
9. `BackendAcc` - 10 edges
10. `record_backend_file()` - 10 edges

## Surprising Connections (you probably didn't know these)
- `scan_reclaim_backend()` --calls--> `now_ms()`  [INFERRED]
  native/src/analytics/os/shared/reclaim/backend.rs → native/src/analytics/os/shared/reclaim/util.rs
- `scan_reclaim_backend()` --calls--> `truncate()`  [INFERRED]
  native/src/analytics/os/shared/reclaim/backend.rs → native/src/analytics/os/shared/reclaim/util.rs
- `agent_walk_hashed_is_preferred()` --calls--> `scan_reclaim_backend()`  [INFERRED]
  native/src/analytics/os/shared/reclaim/backend_tests.rs → native/src/analytics/os/shared/reclaim/backend.rs
- `hashless_remote_does_not_download_to_hash()` --calls--> `scan_reclaim_backend()`  [INFERRED]
  native/src/analytics/os/shared/reclaim/backend_tests.rs → native/src/analytics/os/shared/reclaim/backend.rs
- `provider_md5_groups_without_open_read()` --calls--> `scan_reclaim_backend()`  [INFERRED]
  native/src/analytics/os/shared/reclaim/backend_tests.rs → native/src/analytics/os/shared/reclaim/backend.rs

## Import Cycles
- 1-file cycle: `native/src/analytics/os/shared/reclaim/backend_tests.rs -> native/src/analytics/os/shared/reclaim/backend_tests.rs`
- 1-file cycle: `native/src/analytics/os/shared/reclaim/cleanup.rs -> native/src/analytics/os/shared/reclaim/cleanup.rs`
- 1-file cycle: `native/src/analytics/os/shared/reclaim/duplicates.rs -> native/src/analytics/os/shared/reclaim/duplicates.rs`
- 1-file cycle: `native/src/analytics/os/shared/reclaim/local.rs -> native/src/analytics/os/shared/reclaim/local.rs`
- 1-file cycle: `native/src/analytics/os/shared/reclaim/types.rs -> native/src/analytics/os/shared/reclaim/types.rs`
- 1-file cycle: `native/src/analytics/os/shared/reclaim/util.rs -> native/src/analytics/os/shared/reclaim/util.rs`
- 1-file cycle: `native/src/analytics/os/shared/reclaim/verify.rs -> native/src/analytics/os/shared/reclaim/verify.rs`

## Communities (11 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.09
Nodes (24): AtomicUsize, Backend, Box, HashHit, Arc, AtomicBool, HashMap, Mutex (+16 more)

### Community 1 - "Community 1"
Cohesion: 0.13
Nodes (32): Duration, Arc, FileCandidate, FnOnce, Mutex, Path, PathBuf, ReclaimItem (+24 more)

### Community 2 - "Community 2"
Cohesion: 0.11
Nodes (20): AtomicU64, Default, Into, Arc, AtomicBool, Option, PathBuf, Self (+12 more)

### Community 3 - "Community 3"
Cohesion: 0.21
Nodes (24): DuplicateEvidence, BackendHandle, DuplicateGroup, Option, ReclaimItem, ReclaimOptions, ReclaimProgress, ReclaimReport (+16 more)

### Community 4 - "Community 4"
Cohesion: 0.21
Nodes (20): DuplicateGroup, FileCandidate, Option, Path, PathBuf, ReclaimOptions, ReclaimProgress, Result (+12 more)

### Community 5 - "Community 5"
Cohesion: 0.15
Nodes (11): App, append_reclaim_journal(), reclaim_items(), BackendHandle, Option, ReclaimItem, ReclaimOptions, ReclaimReport (+3 more)

### Community 6 - "Community 6"
Cohesion: 0.26
Nodes (15): Context, App, select_items(), selected_bytes(), ui_empty(), ui_item(), ui_items(), ui_section() (+7 more)

### Community 7 - "Community 7"
Cohesion: 0.24
Nodes (14): HashMap, PathBuf, ReclaimItem, ReclaimReport, String, Vec, changed_duplicate_is_skipped_before_trash(), dedupe_nested_paths() (+6 more)

### Community 8 - "Community 8"
Cohesion: 0.31
Nodes (11): Option, Path, CleanupDecision, dir_cleanup_by_name(), dir_cleanup_reason(), file_cleanup_reason(), git_is_never_auto(), has_build_context() (+3 more)

## Knowledge Gaps
- **40 isolated node(s):** `ReclaimReport`, `Result`, `DuplicateGroup`, `Mutex`, `AtomicUsize` (+35 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `scan_reclaim_backend()` connect `Community 3` to `Community 0`, `Community 1`?**
  _High betweenness centrality (0.223) - this node is a cross-community bridge._
- **Why does `scan_reclaim()` connect `Community 1` to `Community 4`?**
  _High betweenness centrality (0.206) - this node is a cross-community bridge._
- **Why does `duplicate_groups()` connect `Community 4` to `Community 1`?**
  _High betweenness centrality (0.144) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `scan_reclaim_backend()` (e.g. with `now_ms()` and `truncate()`) actually correct?**
  _`scan_reclaim_backend()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 3 inferred relationships involving `scan_backend_dir()` (e.g. with `remote_dir_cleanup_reason()` and `join_path()`) actually correct?**
  _`scan_backend_dir()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Are the 5 inferred relationships involving `scan_reclaim()` (e.g. with `duplicate_groups()` and `local_scan_threads()`) actually correct?**
  _`scan_reclaim()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **What connects `ReclaimReport`, `Result`, `DuplicateGroup` to the rest of the system?**
  _40 weakly-connected nodes found - possible documentation gaps or missing edges._