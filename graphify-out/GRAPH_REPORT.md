# Graph Report - .  (2026-06-24)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 488 nodes · 1213 edges · 17 communities (16 shown, 1 thin omitted)
- Extraction: 93% EXTRACTED · 7% INFERRED · 0% AMBIGUOUS · INFERRED: 79 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `0ebdefd5`
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
- [[_COMMUNITY_Community 11|Community 11]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]

## God Nodes (most connected - your core abstractions)
1. `App` - 75 edges
2. `eio()` - 37 edges
3. `worker()` - 24 edges
4. `PeerBackend` - 23 edges
5. `Result` - 23 edges
6. `ShareIrohNode` - 22 edges
7. `ShareService` - 22 edges
8. `handle_fs_stream()` - 21 edges
9. `App` - 18 edges
10. `ShareProfiles` - 18 edges

## Surprising Connections (you probably didn't know these)
- `resolve_incoming_session()` --calls--> `verify_hmac()`  [INFERRED]
  native/src/share/core/backend.rs → native/src/share/core/crypto.rs
- `resolve_incoming_session()` --calls--> `fingerprint_matches()`  [INFERRED]
  native/src/share/core/backend.rs → native/src/share/core/profiles.rs
- `test_identity()` --calls--> `public_fingerprint()`  [INFERRED]
  native/src/share/core/backend.rs → native/src/share/core/crypto.rs
- `ensure_under_root()` --calls--> `eio()`  [INFERRED]
  native/src/share/core/fs.rs → native/src/share/core/crypto.rs
- `list_dir()` --calls--> `eio()`  [INFERRED]
  native/src/share/core/fs.rs → native/src/share/core/crypto.rs

## Import Cycles
- 1-file cycle: `native/src/share/core/backend.rs -> native/src/share/core/backend.rs`
- 1-file cycle: `native/src/share/core/fs.rs -> native/src/share/core/fs.rs`
- 1-file cycle: `native/src/share/core/service.rs -> native/src/share/core/service.rs`
- 1-file cycle: `native/src/share/core/types.rs -> native/src/share/core/types.rs`

## Communities (17 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.07
Nodes (72): Box, Connection, endpoint_addr(), fs_request_label(), fs_response_summary(), handle_fs_stream(), iroh_direct_session_transfers_files(), normalize_tcp_addr() (+64 more)

### Community 1 - "Community 1"
Cohesion: 0.08
Nodes (68): ClientMsg, Clone, CmdTx, now_secs(), presence_payload(), verify_hmac(), fingerprint_matches(), build_presence() (+60 more)

### Community 2 - "Community 2"
Cohesion: 0.03
Nodes (74): AccelAct, AgentBackend, AnalyticsScan, AppErrorEntry, BisyncCtx, ClipKey, Conflict, ConnectForm (+66 more)

### Community 3 - "Community 3"
Cohesion: 0.15
Nodes (38): clean_mount_label(), connection_mounts(), dir_meta(), ensure_under_root(), from_os_path(), join_under(), list_dir(), local_mounts() (+30 more)

### Community 4 - "Community 4"
Cohesion: 0.11
Nodes (26): b64(), b64_decode(), hex(), hex_decode(), hex_val(), hmac_proof(), public_fingerprint(), random_hex_token() (+18 more)

### Community 5 - "Community 5"
Cohesion: 0.12
Nodes (20): direct_code_parses_lookup_ids_with_dashes(), direct_contact_secret_account(), direct_grant_upsert_persists_state_by_device(), DirectCode, room_code_parses_room_ids_with_dashes(), room_secret_account(), RoomCode, ShareProfiles (+12 more)

### Community 6 - "Community 6"
Cohesion: 0.15
Nodes (14): App, export_summary(), selected_direct_label(), selected_room_label(), upsert_room_member(), Context, PeerOpenTarget, PeerPresence (+6 more)

### Community 7 - "Community 7"
Cohesion: 0.17
Nodes (23): default_direct_access_state(), DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerEndpoint, PeerOpenTarget, PeerPresence (+15 more)

### Community 8 - "Community 8"
Cohesion: 0.20
Nodes (10): ClientMsg, Ctrl, FsMeta, FsRequest, FsResponse, PeerHello, SrvMsg, Option (+2 more)

### Community 9 - "Community 9"
Cohesion: 0.39
Nodes (8): Path, Result, String, Vec, ensure_firewall_rule(), ensure_firewall_rule_for(), lan_ips(), request_firewall_rule_elevated()

### Community 10 - "Community 10"
Cohesion: 0.43
Nodes (4): App, Frame, Context, Option

### Community 11 - "Community 11"
Cohesion: 0.50
Nodes (4): FsMeta, From, Self, VfsMeta

### Community 12 - "Community 12"
Cohesion: 0.40
Nodes (4): App, Option, PathBuf, Self

### Community 13 - "Community 13"
Cohesion: 0.67
Nodes (3): VfsMeta, From, FsMeta

## Knowledge Gaps
- **136 isolated node(s):** `Option`, `Frame`, `App`, `Option`, `PathBuf` (+131 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `eio()` connect `Community 0` to `Community 1`, `Community 3`, `Community 4`?**
  _High betweenness centrality (0.220) - this node is a cross-community bridge._
- **Why does `random_token()` connect `Community 4` to `Community 0`, `Community 1`, `Community 5`?**
  _High betweenness centrality (0.035) - this node is a cross-community bridge._
- **Why does `resolve_incoming_session()` connect `Community 0` to `Community 1`?**
  _High betweenness centrality (0.034) - this node is a cross-community bridge._
- **Are the 34 inferred relationships involving `eio()` (e.g. with `handle_fs_stream()` and `.copy_file()`) actually correct?**
  _`eio()` has 34 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Option`, `Frame`, `App` to the rest of the system?**
  _136 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06501831501831502 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.07578659370725034 - nodes in this community are weakly interconnected._