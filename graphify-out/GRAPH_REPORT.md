# Graph Report - .  (2026-06-28)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 238 nodes · 341 edges · 14 communities (12 shown, 2 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 1 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `fb96b816`
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
1. `App` - 81 edges
2. `TabState` - 20 edges
3. `scan_reclaim()` - 12 edges
4. `scan_dir()` - 11 edges
5. `record_file()` - 9 edges
6. `duplicate_groups()` - 9 edges
7. `ui_items()` - 9 edges
8. `String` - 8 edges
9. `ReclaimReport` - 8 edges
10. `Acc` - 8 edges

## Surprising Connections (you probably didn't know these)
- `App` --references--> `ClipboardVirtualFile`  [EXTRACTED]
  native/src/app/core/state.rs → native/src/app/core/state.rs  _Bridges community 0 → community 8_
- `App` --references--> `HashMap`  [EXTRACTED]
  native/src/app/core/state.rs → native/src/app/core/state.rs  _Bridges community 0 → community 2_

## Import Cycles
- 1-file cycle: `native/src/analytics/os/shared/reclaim.rs -> native/src/analytics/os/shared/reclaim.rs`
- 1-file cycle: `native/src/app/core/state.rs -> native/src/app/core/state.rs`

## Communities (14 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.03
Nodes (79): AccelAct, AgentBackend, AnalyticsPanel, AnalyticsScan, AppErrorEntry, BisyncCtx, ClipKey, Conflict (+71 more)

### Community 1 - "Community 1"
Cohesion: 0.06
Nodes (42): Color32, AnalyticsPanel, AnalyticsScan, AppErrorEntry, empty_progress(), KbdAct, ReclaimScan, SummaryData (+34 more)

### Community 2 - "Community 2"
Cohesion: 0.12
Nodes (36): AtomicU64, Duration, HashMap, Mutex, Arc, AtomicBool, Default, Option (+28 more)

### Community 3 - "Community 3"
Cohesion: 0.18
Nodes (8): App, dedupe_nested_paths(), reclaim_items(), ReclaimItem, ReclaimReport, String, Vec, ReclaimOptions

### Community 4 - "Community 4"
Cohesion: 0.26
Nodes (15): App, select_items(), selected_bytes(), ui_empty(), ui_item(), ui_items(), ui_section(), FnOnce (+7 more)

### Community 5 - "Community 5"
Cohesion: 0.22
Nodes (7): App, Context, Option, PickerState, SavedConnection, String, PickerPurpose

### Community 6 - "Community 6"
Cohesion: 0.18
Nodes (9): ClipKey, PickerPurpose, PickerState, BackendHandle, ConnectResult, Option, Receiver, String (+1 more)

## Knowledge Gaps
- **135 isolated node(s):** `Default`, `Self`, `AtomicU64`, `AtomicBool`, `Option` (+130 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `Community 0` to `Community 8`, `Community 2`?**
  _High betweenness centrality (0.227) - this node is a cross-community bridge._
- **Why does `HashMap` connect `Community 2` to `Community 0`?**
  _High betweenness centrality (0.113) - this node is a cross-community bridge._
- **What connects `Default`, `Self`, `AtomicU64` to the rest of the system?**
  _135 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.02531645569620253 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.05851063829787234 - nodes in this community are weakly interconnected._
- **Should `Community 2` be split into smaller, more focused modules?**
  _Cohesion score 0.12307692307692308 - nodes in this community are weakly interconnected._