# Graph Report - .  (2026-06-28)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 227 nodes · 405 edges · 13 communities (12 shown, 1 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `0ab3e9a8`
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

## God Nodes (most connected - your core abstractions)
1. `TabState` - 20 edges
2. `LocalBackend` - 17 edges
3. `scan_reclaim()` - 13 edges
4. `scan_dir()` - 12 edges
5. `App` - 12 edges
6. `SizeNode` - 10 edges
7. `scan_backend_parallel()` - 10 edges
8. `build_from_listings()` - 10 edges
9. `Progress` - 9 edges
10. `record_file()` - 9 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Import Cycles
- 1-file cycle: `native/src/analytics/os/shared/analytics.rs -> native/src/analytics/os/shared/analytics.rs`
- 1-file cycle: `native/src/analytics/os/shared/reclaim.rs -> native/src/analytics/os/shared/reclaim.rs`

## Communities (13 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.07
Nodes (38): Color32, AnalyticsPanel, AnalyticsScan, AppErrorEntry, empty_progress(), KbdAct, ReclaimScan, SummaryData (+30 more)

### Community 1 - "Community 1"
Cohesion: 0.13
Nodes (36): Duration, Mutex, Arc, AtomicBool, AtomicU64, Default, Option, Path (+28 more)

### Community 2 - "Community 2"
Cohesion: 0.11
Nodes (16): Metadata, Backend, Box, Self, String, SystemTime, Vec, Read (+8 more)

### Community 3 - "Community 3"
Cohesion: 0.18
Nodes (26): HashMap, Arc, AtomicBool, AtomicU64, Backend, Box, Path, String (+18 more)

### Community 4 - "Community 4"
Cohesion: 0.15
Nodes (8): BackendHandle, App, Option, SizeNode, String, Ui, Vec, SummaryData

### Community 5 - "Community 5"
Cohesion: 0.18
Nodes (8): App, picker_child_path(), Context, Option, String, PickerPurpose, PickerState, SavedConnection

### Community 6 - "Community 6"
Cohesion: 0.18
Nodes (8): App, dedupe_nested_paths(), reclaim_items(), ReclaimItem, ReclaimReport, String, Vec, ReclaimOptions

### Community 7 - "Community 7"
Cohesion: 0.26
Nodes (15): App, select_items(), selected_bytes(), ui_empty(), ui_item(), ui_items(), ui_section(), FnOnce (+7 more)

### Community 8 - "Community 8"
Cohesion: 0.43
Nodes (4): App, Frame, Context, Option

## Knowledge Gaps
- **62 isolated node(s):** `AtomicU64`, `AtomicBool`, `WireNode`, `Default`, `Self` (+57 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Read` connect `Community 2` to `Community 1`?**
  _High betweenness centrality (0.079) - this node is a cross-community bridge._
- **Why does `HashMap` connect `Community 3` to `Community 1`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **What connects `AtomicU64`, `AtomicBool`, `WireNode` to the rest of the system?**
  _62 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06755260243632337 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.1282051282051282 - nodes in this community are weakly interconnected._
- **Should `Community 2` be split into smaller, more focused modules?**
  _Cohesion score 0.1053763440860215 - nodes in this community are weakly interconnected._
- **Should `Community 4` be split into smaller, more focused modules?**
  _Cohesion score 0.14736842105263157 - nodes in this community are weakly interconnected._