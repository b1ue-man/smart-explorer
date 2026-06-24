# Graph Report - .  (2026-06-24)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 142 nodes · 339 edges · 10 communities
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `dffb972a`
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
1. `App` - 18 edges
2. `ShareService` - 18 edges
3. `worker()` - 17 edges
4. `ShareAuthState` - 11 edges
5. `recv_from_peer()` - 11 edges
6. `resolve_incoming()` - 11 edges
7. `publish_all()` - 10 edges
8. `accept_loop()` - 10 edges
9. `Result` - 10 edges
10. `String` - 10 edges

## Surprising Connections (you probably didn't know these)
- `send_hello()` --calls--> `lan_ips()`  [INFERRED]
  native/src/share/core/service.rs → native/src/share/os/shared/system.rs
- `build_presence()` --calls--> `lan_ips()`  [INFERRED]
  native/src/share/core/service.rs → native/src/share/os/shared/system.rs

## Import Cycles
- 1-file cycle: `native/src/share/core/service.rs -> native/src/share/core/service.rs`
- 1-file cycle: `native/src/share/os/shared/transfer.rs -> native/src/share/os/shared/transfer.rs`
- 1-file cycle: `native/src/updater/os/windows.rs -> native/src/updater/os/windows.rs`

## Communities (10 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.15
Nodes (13): Context, App, selected_direct_label(), selected_room_label(), upsert_room_member(), PeerOpenTarget, PeerPresence, RoomProfile (+5 more)

### Community 1 - "Community 1"
Cohesion: 0.15
Nodes (28): Channel, DirectContact, FileMeta, Arc, AtomicBool, HashSet, Mutex, PeerEndpoint (+20 more)

### Community 2 - "Community 2"
Cohesion: 0.28
Nodes (22): Duration, Error, Feed, Path, PathBuf, Result, String, apply_via_installed_updater() (+14 more)

### Community 3 - "Community 3"
Cohesion: 0.31
Nodes (10): Path, PathBuf, Result, String, Vec, ensure_firewall_rule(), ensure_firewall_rule_for(), lan_ips() (+2 more)

### Community 4 - "Community 4"
Cohesion: 0.47
Nodes (10): handle_server_msg(), verify_direct_presence(), verify_room_presence(), worker(), Arc, Mutex, PeerPresence, Sender (+2 more)

### Community 5 - "Community 5"
Cohesion: 0.33
Nodes (5): BackendHandle, PeerEndpoint, PeerOpenTarget, String, Self

### Community 6 - "Community 6"
Cohesion: 0.43
Nodes (8): ClientMsg, build_presence(), publish_all(), send_hello(), send_line(), Result, ShareIdentity, TcpStream

### Community 7 - "Community 7"
Cohesion: 0.25
Nodes (7): Clone, CmdTx, ShareService, Drop, AtomicBool, ShareCmd, Receiver

### Community 8 - "Community 8"
Cohesion: 0.39
Nodes (6): configure_updates_auth_state_synchronously(), dropping_probe_clone_does_not_stop_owner_service(), nonce_cache_detects_replay(), remember_nonce(), test_service(), HashSet

### Community 9 - "Community 9"
Cohesion: 0.33
Nodes (4): String, Vec, local_lan_ips(), ShareProfiles

## Knowledge Gaps
- **26 isolated node(s):** `ShareCmd`, `ShareStatus`, `PeerOpenTarget`, `Context`, `RoomProfile` (+21 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `accept_loop()` connect `Community 1` to `Community 5`?**
  _High betweenness centrality (0.323) - this node is a cross-community bridge._
- **Why does `Duration` connect `Community 2` to `Community 1`?**
  _High betweenness centrality (0.198) - this node is a cross-community bridge._
- **Why does `lan_ips()` connect `Community 3` to `Community 6`?**
  _High betweenness centrality (0.103) - this node is a cross-community bridge._
- **What connects `ShareCmd`, `ShareStatus`, `PeerOpenTarget` to the rest of the system?**
  _26 weakly-connected nodes found - possible documentation gaps or missing edges._