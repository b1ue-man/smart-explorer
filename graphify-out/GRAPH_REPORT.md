# Graph Report - .  (2026-06-25)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 225 nodes · 449 edges · 9 communities
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 2 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `392f97ce`
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

## God Nodes (most connected - your core abstractions)
1. `App` - 76 edges
2. `App` - 25 edges
3. `Result` - 15 edges
4. `String` - 14 edges
5. `open_share_backend()` - 14 edges
6. `ShareHost` - 13 edges
7. `UnavailableBackend` - 13 edges
8. `handle_client()` - 12 edges
9. `read_response()` - 12 edges
10. `ShareHostState` - 11 edges

## Surprising Connections (you probably didn't know these)
- `start_listener()` --references--> `ShareHost`  [EXTRACTED]
  native/src/daemon/os/shared/ipc.rs → native/src/daemon/os/shared/ipc.rs  _Bridges community 3 → community 2_
- `drain_share_worker_events()` --references--> `ShareWorkerSnapshot`  [EXTRACTED]
  native/src/daemon/os/shared/ipc.rs → native/src/daemon/os/shared/ipc.rs  _Bridges community 5 → community 2_
- `ShareWorkerSnapshot` --references--> `String`  [EXTRACTED]
  native/src/daemon/os/shared/ipc.rs → native/src/daemon/os/shared/ipc.rs  _Bridges community 5 → community 3_
- `UnavailableBackend` --references--> `String`  [EXTRACTED]
  native/src/daemon/os/shared/ipc.rs → native/src/daemon/os/shared/ipc.rs  _Bridges community 3 → community 4_

## Import Cycles
- None detected.

## Communities (9 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.03
Nodes (76): AccelAct, AgentBackend, AnalyticsScan, AppErrorEntry, AtomicBool, BisyncCtx, ClipKey, Conflict (+68 more)

### Community 1 - "Community 1"
Cohesion: 0.11
Nodes (22): AsRef, Context, App, export_summary(), selected_direct_label(), selected_room_label(), share_diag_log_is_bounded_on_line_boundary(), share_input_width() (+14 more)

### Community 2 - "Community 2"
Cohesion: 0.16
Nodes (33): Duration, Option, PathBuf, Result, accepted_ipc_stream_is_forced_back_to_blocking(), clear_ipc_addr(), drain_share_worker_events(), ensure_worker_ready() (+25 more)

### Community 3 - "Community 3"
Cohesion: 0.14
Nodes (19): App, Mutex, Option, PathBuf, Self, Arc, BackendHandle, PeerOpenTarget (+11 more)

### Community 4 - "Community 4"
Cohesion: 0.16
Nodes (9): Backend, Box, Read, Scheme, Send, UnavailableBackend, VfsMeta, VfsResult (+1 more)

### Community 5 - "Community 5"
Cohesion: 0.21
Nodes (13): Instant, PeerPresence, RoomProfile, ShareIdentity, ShareProfiles, ShareService, Vec, configure_or_restart_locked() (+5 more)

### Community 6 - "Community 6"
Cohesion: 0.67
Nodes (3): E, Error, eio()

## Knowledge Gaps
- **99 isolated node(s):** `App`, `Option`, `PathBuf`, `Self`, `ShareCmd` (+94 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `UnavailableBackend` connect `Community 4` to `Community 2`, `Community 3`?**
  _High betweenness centrality (0.031) - this node is a cross-community bridge._
- **Why does `ShareHostState` connect `Community 5` to `Community 2`, `Community 3`?**
  _High betweenness centrality (0.017) - this node is a cross-community bridge._
- **What connects `App`, `Option`, `PathBuf` to the rest of the system?**
  _99 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.025974025974025976 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.10823311748381129 - nodes in this community are weakly interconnected._
- **Should `Community 3` be split into smaller, more focused modules?**
  _Cohesion score 0.13675213675213677 - nodes in this community are weakly interconnected._