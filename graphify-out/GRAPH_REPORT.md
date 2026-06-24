# Graph Report - .  (2026-06-24)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 71 nodes · 175 edges · 7 communities (6 shown, 1 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `7ae53c9a`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]

## God Nodes (most connected - your core abstractions)
1. `worker()` - 21 edges
2. `ShareService` - 18 edges
3. `Result` - 12 edges
4. `publish_all()` - 10 edges
5. `handle_server_msg()` - 10 edges
6. `String` - 9 edges
7. `SignalConnection` - 9 edges
8. `send_line()` - 8 edges
9. `ShareAuthState` - 7 edges
10. `send_hello()` - 7 edges

## Surprising Connections (you probably didn't know these)
- `ShareService` --references--> `Arc`  [EXTRACTED]
  native/src/share/core/service.rs → native/src/share/core/service.rs  _Bridges community 0 → community 2_
- `test_service()` --references--> `ShareService`  [EXTRACTED]
  native/src/share/core/service.rs → native/src/share/core/service.rs  _Bridges community 0 → community 1_
- `publish_all()` --references--> `Result`  [EXTRACTED]
  native/src/share/core/service.rs → native/src/share/core/service.rs  _Bridges community 3 → community 2_
- `verify_direct_presence()` --calls--> `remember_nonce()`  [EXTRACTED]
  native/src/share/core/service.rs → native/src/share/core/service.rs  _Bridges community 2 → community 1_

## Import Cycles
- 1-file cycle: `native/src/share/core/service.rs -> native/src/share/core/service.rs`

## Communities (7 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.18
Nodes (12): AtomicBool, BackendHandle, Clone, CmdTx, ShareService, Drop, PeerEndpoint, PeerOpenTarget (+4 more)

### Community 1 - "Community 1"
Cohesion: 0.15
Nodes (13): configure_updates_auth_state_synchronously(), dropping_probe_clone_does_not_stop_owner_service(), nonce_cache_detects_replay(), normalize_signal_endpoint(), normalize_tcp_addr(), remember_nonce(), signal_endpoints(), test_service() (+5 more)

### Community 2 - "Community 2"
Cohesion: 0.28
Nodes (15): Arc, build_presence(), handle_server_msg(), publish_all(), send_hello(), verify_direct_presence(), verify_room_presence(), worker() (+7 more)

### Community 3 - "Community 3"
Cohesion: 0.41
Nodes (6): ClientMsg, send_line(), SignalConnection, Option, Result, Self

### Community 5 - "Community 5"
Cohesion: 0.50
Nodes (4): set_ws_timeout(), Duration, MaybeTlsStream, TcpStream

## Knowledge Gaps
- **13 isolated node(s):** `CmdTx`, `BackendHandle`, `PeerEndpoint`, `ShareProfiles`, `Clone` (+8 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `ShareService` connect `Community 0` to `Community 1`, `Community 2`?**
  _High betweenness centrality (0.171) - this node is a cross-community bridge._
- **Why does `worker()` connect `Community 2` to `Community 0`, `Community 1`, `Community 3`?**
  _High betweenness centrality (0.146) - this node is a cross-community bridge._
- **Why does `set_ws_timeout()` connect `Community 5` to `Community 1`, `Community 3`?**
  _High betweenness centrality (0.082) - this node is a cross-community bridge._
- **What connects `CmdTx`, `BackendHandle`, `PeerEndpoint` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.14705882352941177 - nodes in this community are weakly interconnected._