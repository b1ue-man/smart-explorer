# Graph Report - .  (2026-06-25)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 212 nodes · 433 edges · 13 communities
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 12 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `2347fc6b`
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

## God Nodes (most connected - your core abstractions)
1. `App` - 76 edges
2. `String` - 31 edges
3. `copy_remote_paths_progress()` - 17 edges
4. `Backend` - 15 edges
5. `collect_remote_entries()` - 15 edges
6. `download_paths_progress()` - 14 edges
7. `upload_file_progress()` - 13 edges
8. `download_file_progress()` - 13 edges
9. `download_remote_clipboard_items()` - 13 edges
10. `RemoteFilterCtx` - 11 edges

## Surprising Connections (you probably didn't know these)
- `MergeUi` --references--> `String`  [EXTRACTED]
  native/src/app/os/shared/remote_helpers.rs → native/src/app/os/shared/remote_helpers.rs  _Bridges community 2 → community 3_
- `collect_upload_entries()` --references--> `String`  [EXTRACTED]
  native/src/app/os/shared/remote_helpers.rs → native/src/app/os/shared/remote_helpers.rs  _Bridges community 3 → community 1_
- `ensure_dir_root()` --references--> `String`  [EXTRACTED]
  native/src/app/os/shared/remote_helpers.rs → native/src/app/os/shared/remote_helpers.rs  _Bridges community 3 → community 5_
- `RemoteEdit` --references--> `String`  [EXTRACTED]
  native/src/app/os/shared/remote_helpers.rs → native/src/app/os/shared/remote_helpers.rs  _Bridges community 3 → community 6_
- `collect_upload_entries()` --references--> `Vec`  [EXTRACTED]
  native/src/app/os/shared/remote_helpers.rs → native/src/app/os/shared/remote_helpers.rs  _Bridges community 2 → community 1_

## Import Cycles
- None detected.

## Communities (13 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.03
Nodes (76): AccelAct, AgentBackend, AnalyticsScan, AppErrorEntry, Arc, AtomicBool, BisyncCtx, ClipKey (+68 more)

### Community 1 - "Community 1"
Cohesion: 0.10
Nodes (30): App, Option, PathBuf, Self, PathBuf, Sig, Path, cleanup_session_temp() (+22 more)

### Community 2 - "Community 2"
Cohesion: 0.18
Nodes (22): CompiledFilter, FileEntry, FilterDef, Option, Row, Vec, collect_remote_entries(), compile_remote_filter() (+14 more)

### Community 3 - "Community 3"
Cohesion: 0.35
Nodes (20): Backend, Instant, Result, String, TransferMsg, TransferProgress, Sender, copy_remote_paths_progress() (+12 more)

### Community 4 - "Community 4"
Cohesion: 0.19
Nodes (11): App, BackendHandle, Context, FilterDef, Option, Pos2, String, Vec (+3 more)

### Community 5 - "Community 5"
Cohesion: 0.21
Nodes (8): App, Context, Option, PickerState, SavedConnection, String, PickerPurpose, ensure_dir_root()

### Community 6 - "Community 6"
Cohesion: 0.25
Nodes (6): Drop, HANDLE, BackendHandle, Self, EditProcess, RemoteEdit

### Community 7 - "Community 7"
Cohesion: 0.29
Nodes (4): PathBuf, String, Vec, App

## Knowledge Gaps
- **97 isolated node(s):** `App`, `Option`, `PathBuf`, `Self`, `PickerPurpose` (+92 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `ensure_dir_root()` connect `Community 5` to `Community 1`, `Community 3`?**
  _High betweenness centrality (0.052) - this node is a cross-community bridge._
- **Why does `is_local_style()` connect `Community 4` to `Community 1`, `Community 5`?**
  _High betweenness centrality (0.052) - this node is a cross-community bridge._
- **What connects `App`, `Option`, `PathBuf` to the rest of the system?**
  _97 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.025974025974025976 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.10416666666666667 - nodes in this community are weakly interconnected._