# Graph Report - .  (2026-06-24)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 224 nodes · 406 edges · 10 communities
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 2 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `61eee367`
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
1. `App` - 74 edges
2. `worker()` - 21 edges
3. `App` - 18 edges
4. `ShareService` - 18 edges
5. `Result` - 12 edges
6. `handle_server_msg()` - 11 edges
7. `publish_all()` - 10 edges
8. `String` - 9 edges
9. `SignalConnection` - 9 edges
10. `ShareAuthState` - 8 edges

## Surprising Connections (you probably didn't know these)
- `send_hello()` --calls--> `lan_ips()`  [INFERRED]
  native/src/share/core/service.rs → native/src/share/os/shared/system.rs
- `build_presence()` --calls--> `lan_ips()`  [INFERRED]
  native/src/share/core/service.rs → native/src/share/os/shared/system.rs

## Import Cycles
- 1-file cycle: `native/src/share/core/service.rs -> native/src/share/core/service.rs`
- 1-file cycle: `native/src/share/core/types.rs -> native/src/share/core/types.rs`

## Communities (10 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.03
Nodes (74): AccelAct, AgentBackend, AnalyticsScan, AppErrorEntry, BisyncCtx, ClipKey, Conflict, ConnectForm (+66 more)

### Community 1 - "Community 1"
Cohesion: 0.16
Nodes (28): Clone, CmdTx, build_presence(), configure_updates_auth_state_synchronously(), dropping_probe_clone_does_not_stop_owner_service(), handle_server_msg(), local_direct_request_requires_own_direct_secret(), nonce_cache_detects_replay() (+20 more)

### Community 2 - "Community 2"
Cohesion: 0.15
Nodes (14): Context, App, export_summary(), selected_direct_label(), selected_room_label(), upsert_room_member(), PeerOpenTarget, PeerPresence (+6 more)

### Community 3 - "Community 3"
Cohesion: 0.13
Nodes (20): ClientMsg, normalize_signal_endpoint(), normalize_tcp_addr(), send_hello(), send_line(), signal_endpoints(), SignalConnection, ws_to_io() (+12 more)

### Community 4 - "Community 4"
Cohesion: 0.23
Nodes (16): DirectContact, PeerEndpoint, PeerOpenTarget, PeerPresence, RoomMember, RoomProfile, ShareCmd, ShareEvent (+8 more)

### Community 5 - "Community 5"
Cohesion: 0.19
Nodes (12): ClientMsg, Ctrl, FileMeta, FsMeta, FsRequest, FsResponse, PeerHello, PeerPrelude (+4 more)

### Community 6 - "Community 6"
Cohesion: 0.30
Nodes (11): PathBuf, Result, String, Vec, Path, ensure_firewall_rule(), ensure_firewall_rule_for(), lan_ips() (+3 more)

### Community 7 - "Community 7"
Cohesion: 0.40
Nodes (4): App, Option, PathBuf, Self

### Community 8 - "Community 8"
Cohesion: 0.50
Nodes (4): set_ws_timeout(), Duration, MaybeTlsStream, TcpStream

## Knowledge Gaps
- **109 isolated node(s):** `App`, `Option`, `PathBuf`, `Self`, `ShareCmd` (+104 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `worker()` connect `Community 1` to `Community 3`?**
  _High betweenness centrality (0.065) - this node is a cross-community bridge._
- **Why does `Sender` connect `Community 1` to `Community 4`?**
  _High betweenness centrality (0.061) - this node is a cross-community bridge._
- **What connects `App`, `Option`, `PathBuf` to the rest of the system?**
  _109 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.02666666666666667 - nodes in this community are weakly interconnected._
- **Should `Community 3` be split into smaller, more focused modules?**
  _Cohesion score 0.12643678160919541 - nodes in this community are weakly interconnected._