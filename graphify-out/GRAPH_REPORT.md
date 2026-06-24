# Graph Report - .  (2026-06-24)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 362 nodes · 887 edges · 11 communities
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `86f34b47`
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

## God Nodes (most connected - your core abstractions)
1. `PeerBackend` - 25 edges
2. `worker()` - 23 edges
3. `ShareService` - 19 edges
4. `App` - 18 edges
5. `ShareProfiles` - 18 edges
6. `Result` - 15 edges
7. `ShareAuthState` - 15 edges
8. `Result` - 15 edges
9. `handle_server_msg()` - 15 edges
10. `exercise_peer_backend()` - 14 edges

## Surprising Connections (you probably didn't know these)
- `direct_peer_backend_opens_folder_and_transfers_files()` --calls--> `accept_loop()`  [INFERRED]
  native/src/share/core/backend.rs → native/src/share/os/shared/transfer.rs
- `start_relay_responder()` --calls--> `recv_from_peer()`  [INFERRED]
  native/src/share/core/backend.rs → native/src/share/os/shared/transfer.rs
- `recv_from_peer()` --calls--> `read_raw_frame()`  [INFERRED]
  native/src/share/os/shared/transfer.rs → native/src/share/core/protocol.rs
- `spawn_relay_responder()` --calls--> `recv_from_peer()`  [INFERRED]
  native/src/share/core/service.rs → native/src/share/os/shared/transfer.rs
- `verify_direct_access_accepted_using()` --calls--> `fingerprint_matches()`  [INFERRED]
  native/src/share/core/service.rs → native/src/share/core/profiles.rs

## Import Cycles
- 1-file cycle: `native/src/share/core/backend.rs -> native/src/share/core/backend.rs`
- 1-file cycle: `native/src/share/core/relay.rs -> native/src/share/core/relay.rs`
- 1-file cycle: `native/src/share/core/service.rs -> native/src/share/core/service.rs`
- 1-file cycle: `native/src/share/core/types.rs -> native/src/share/core/types.rs`
- 1-file cycle: `native/src/share/os/shared/transfer.rs -> native/src/share/os/shared/transfer.rs`

## Communities (11 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (56): Backend, bridge_test_streams(), direct_peer_backend_opens_folder_and_transfers_files(), direct_presence(), exercise_peer_backend(), Fixture, handle_test_relay_conn(), PeerBackend (+48 more)

### Community 1 - "Community 1"
Cohesion: 0.09
Nodes (59): BackendHandle, Clone, CmdTx, fingerprint_matches(), build_presence(), configure_updates_auth_state_synchronously(), direct_accept_or_reject_requires_signed_owner_presence(), dropping_probe_clone_does_not_stop_owner_service() (+51 more)

### Community 2 - "Community 2"
Cohesion: 0.13
Nodes (20): direct_code_parses_lookup_ids_with_dashes(), direct_contact_secret_account(), direct_grant_upsert_persists_state_by_device(), DirectCode, room_code_parses_room_ids_with_dashes(), room_secret_account(), RoomCode, ShareProfiles (+12 more)

### Community 3 - "Community 3"
Cohesion: 0.14
Nodes (27): connect_one(), normalize_signal_endpoint(), normalize_tcp_addr(), relation_presence(), RelayInner, RelayStream, send_relay_request(), send_signal_pair() (+19 more)

### Community 4 - "Community 4"
Cohesion: 0.15
Nodes (14): Context, App, export_summary(), selected_direct_label(), selected_room_label(), upsert_room_member(), PeerOpenTarget, PeerPresence (+6 more)

### Community 5 - "Community 5"
Cohesion: 0.13
Nodes (30): FileMeta, Arc, AtomicBool, Channel, DirectContact, DirectGrant, HashSet, Mutex (+22 more)

### Community 6 - "Community 6"
Cohesion: 0.15
Nodes (18): Channel, IoStream, read_raw_frame(), T, verify_remote_static(), write_raw_frame(), Box, Option (+10 more)

### Community 7 - "Community 7"
Cohesion: 0.17
Nodes (20): default_direct_access_state(), DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerEndpoint, PeerOpenTarget, PeerPresence (+12 more)

### Community 8 - "Community 8"
Cohesion: 0.19
Nodes (12): ClientMsg, Ctrl, FileMeta, FsMeta, FsRequest, FsResponse, PeerHello, PeerPrelude (+4 more)

### Community 9 - "Community 9"
Cohesion: 0.33
Nodes (4): String, Vec, local_lan_ips(), ShareProfiles

### Community 10 - "Community 10"
Cohesion: 0.50
Nodes (4): set_ws_timeout(), Duration, MaybeTlsStream, TcpStream

## Knowledge Gaps
- **77 isolated node(s):** `ShareCmd`, `ShareStatus`, `PeerOpenTarget`, `Context`, `RoomProfile` (+72 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `recv_from_peer()` connect `Community 5` to `Community 0`, `Community 1`, `Community 6`?**
  _High betweenness centrality (0.277) - this node is a cross-community bridge._
- **Why does `spawn_relay_responder()` connect `Community 1` to `Community 5`?**
  _High betweenness centrality (0.234) - this node is a cross-community bridge._
- **Why does `fingerprint_matches()` connect `Community 1` to `Community 2`?**
  _High betweenness centrality (0.139) - this node is a cross-community bridge._
- **What connects `ShareCmd`, `ShareStatus`, `PeerOpenTarget` to the rest of the system?**
  _77 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06210670314637483 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.0918918918918919 - nodes in this community are weakly interconnected._
- **Should `Community 2` be split into smaller, more focused modules?**
  _Cohesion score 0.12773109243697478 - nodes in this community are weakly interconnected._