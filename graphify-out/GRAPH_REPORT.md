# Graph Report - .  (2026-06-24)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 74 nodes · 183 edges · 7 communities (5 shown, 2 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `63719139`
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
1. `App` - 18 edges
2. `ShareService` - 18 edges
3. `worker()` - 17 edges
4. `publish_all()` - 10 edges
5. `handle_server_msg()` - 9 edges
6. `ShareAuthState` - 7 edges
7. `send_hello()` - 7 edges
8. `send_line()` - 7 edges
9. `verify_direct_presence()` - 7 edges
10. `verify_room_presence()` - 7 edges

## Surprising Connections (you probably didn't know these)
- `selected_direct_label()` --references--> `App`  [EXTRACTED]
  native/src/app/core/share.rs → native/src/app/core/share.rs  _Bridges community 6 → community 5_
- `ShareService` --references--> `Arc`  [EXTRACTED]
  native/src/share/core/service.rs → native/src/share/core/service.rs  _Bridges community 2 → community 1_
- `ShareService` --references--> `ShareIdentity`  [EXTRACTED]
  native/src/share/core/service.rs → native/src/share/core/service.rs  _Bridges community 2 → community 0_
- `worker()` --references--> `ShareIdentity`  [EXTRACTED]
  native/src/share/core/service.rs → native/src/share/core/service.rs  _Bridges community 0 → community 1_

## Import Cycles
- 1-file cycle: `native/src/share/core/service.rs -> native/src/share/core/service.rs`

## Communities (7 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.21
Nodes (14): BackendHandle, ClientMsg, build_presence(), publish_all(), send_hello(), send_line(), PeerOpenTarget, String (+6 more)

### Community 1 - "Community 1"
Cohesion: 0.36
Nodes (13): Arc, handle_server_msg(), nonce_cache_detects_replay(), remember_nonce(), verify_direct_presence(), verify_room_presence(), worker(), HashSet (+5 more)

### Community 2 - "Community 2"
Cohesion: 0.20
Nodes (10): AtomicBool, Clone, CmdTx, configure_updates_auth_state_synchronously(), dropping_probe_clone_does_not_stop_owner_service(), ShareService, test_service(), Drop (+2 more)

### Community 4 - "Community 4"
Cohesion: 0.36
Nodes (3): Context, ShareCmd, Ui

### Community 5 - "Community 5"
Cohesion: 0.33
Nodes (6): selected_direct_label(), selected_room_label(), upsert_room_member(), PeerPresence, String, RoomProfile

## Knowledge Gaps
- **14 isolated node(s):** `ShareCmd`, `ShareStatus`, `PeerOpenTarget`, `Context`, `RoomProfile` (+9 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `ShareExportConfig` connect `Community 6` to `Community 1`?**
  _High betweenness centrality (0.486) - this node is a cross-community bridge._
- **Why does `App` connect `Community 6` to `Community 3`, `Community 4`, `Community 5`?**
  _High betweenness centrality (0.375) - this node is a cross-community bridge._
- **Why does `ShareService` connect `Community 2` to `Community 0`, `Community 1`?**
  _High betweenness centrality (0.239) - this node is a cross-community bridge._
- **What connects `ShareCmd`, `ShareStatus`, `PeerOpenTarget` to the rest of the system?**
  _14 weakly-connected nodes found - possible documentation gaps or missing edges._