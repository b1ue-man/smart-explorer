# Graph Report - .  (2026-06-23)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 427 nodes · 932 edges · 17 communities (15 shown, 2 thin omitted)
- Extraction: 91% EXTRACTED · 9% INFERRED · 0% AMBIGUOUS · INFERRED: 82 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `93770ab9`
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
1. `App` - 73 edges
2. `eio()` - 37 edges
3. `PeerBackend` - 22 edges
4. `resolve()` - 19 edges
5. `App` - 18 edges
6. `handle_fs_request()` - 18 edges
7. `list_dir()` - 16 edges
8. `worker()` - 16 edges
9. `ShareProfiles` - 15 edges
10. `ShareService` - 15 edges

## Surprising Connections (you probably didn't know these)
- `recv_ctrl()` --calls--> `eio()`  [INFERRED]
  native/src/share/core/backend.rs → native/src/share/core/crypto.rs
- `recv_resp()` --calls--> `eio()`  [INFERRED]
  native/src/share/core/backend.rs → native/src/share/core/crypto.rs
- `ensure_under_root()` --calls--> `eio()`  [INFERRED]
  native/src/share/core/fs.rs → native/src/share/core/crypto.rs
- `handle_fs_request()` --calls--> `eio()`  [INFERRED]
  native/src/share/core/fs.rs → native/src/share/core/crypto.rs
- `list_dir()` --calls--> `eio()`  [INFERRED]
  native/src/share/core/fs.rs → native/src/share/core/crypto.rs

## Import Cycles
- 1-file cycle: `native/src/share/core/backend.rs -> native/src/share/core/backend.rs`
- 1-file cycle: `native/src/share/core/fs.rs -> native/src/share/core/fs.rs`
- 1-file cycle: `native/src/share/core/protocol.rs -> native/src/share/core/protocol.rs`
- 1-file cycle: `native/src/share/core/service.rs -> native/src/share/core/service.rs`
- 1-file cycle: `native/src/share/core/types.rs -> native/src/share/core/types.rs`
- 1-file cycle: `native/src/share/os/shared/transfer.rs -> native/src/share/os/shared/transfer.rs`

## Communities (17 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.03
Nodes (72): AccelAct, AgentBackend, AnalyticsScan, AppErrorEntry, BisyncCtx, ClipKey, Conflict, ConnectForm (+64 more)

### Community 1 - "Community 1"
Cohesion: 0.11
Nodes (53): clean_mount_label(), connection_mounts(), dir_meta(), ensure_under_root(), from_os_path(), FsMeta, handle_fs_request(), join_under() (+45 more)

### Community 2 - "Community 2"
Cohesion: 0.09
Nodes (30): Box, PeerBackend, PeerReader, PeerWriter, recv_ctrl(), recv_resp(), send_ctrl(), send_req() (+22 more)

### Community 3 - "Community 3"
Cohesion: 0.11
Nodes (38): ClientMsg, CmdTx, now_secs(), presence_payload(), verify_hmac(), build_presence(), handle_server_msg(), nonce_cache_detects_replay() (+30 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (23): b64_decode(), hex(), hex_decode(), hex_val(), hmac_proof(), public_fingerprint(), random_hex_token(), random_uuid_v4() (+15 more)

### Community 5 - "Community 5"
Cohesion: 0.15
Nodes (18): b64(), random_token(), direct_code_parses_lookup_ids_with_dashes(), direct_contact_secret_account(), DirectCode, room_code_parses_room_ids_with_dashes(), room_secret_account(), RoomCode (+10 more)

### Community 6 - "Community 6"
Cohesion: 0.15
Nodes (13): Context, App, selected_direct_label(), selected_room_label(), upsert_room_member(), PeerOpenTarget, PeerPresence, RoomProfile (+5 more)

### Community 7 - "Community 7"
Cohesion: 0.15
Nodes (28): sanitize_name(), FileMeta, Arc, AtomicBool, Channel, DirectContact, HashSet, Mutex (+20 more)

### Community 8 - "Community 8"
Cohesion: 0.23
Nodes (16): DirectContact, PeerEndpoint, PeerOpenTarget, PeerPresence, RoomMember, RoomProfile, ShareCmd, ShareEvent (+8 more)

### Community 9 - "Community 9"
Cohesion: 0.29
Nodes (12): eio(), Channel, read_raw_frame(), verify_remote_static(), write_raw_frame(), E, Error, Option (+4 more)

### Community 10 - "Community 10"
Cohesion: 0.19
Nodes (12): ClientMsg, Ctrl, FileMeta, FsMeta, FsRequest, FsResponse, PeerHello, PeerPrelude (+4 more)

### Community 11 - "Community 11"
Cohesion: 0.40
Nodes (5): Path, PathBuf, Result, quarantine_dir(), unique_in()

### Community 12 - "Community 12"
Cohesion: 0.40
Nodes (4): App, Option, PathBuf, Self

## Knowledge Gaps
- **137 isolated node(s):** `App`, `Option`, `PathBuf`, `Self`, `ShareCmd` (+132 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `eio()` connect `Community 9` to `Community 1`, `Community 2`, `Community 3`, `Community 4`, `Community 7`?**
  _High betweenness centrality (0.217) - this node is a cross-community bridge._
- **Why does `b64_decode()` connect `Community 4` to `Community 2`, `Community 3`, `Community 5`, `Community 7`?**
  _High betweenness centrality (0.089) - this node is a cross-community bridge._
- **Why does `publish_all()` connect `Community 3` to `Community 9`?**
  _High betweenness centrality (0.065) - this node is a cross-community bridge._
- **Are the 34 inferred relationships involving `eio()` (e.g. with `.channel()` and `.list_dir()`) actually correct?**
  _`eio()` has 34 INFERRED edges - model-reasoned connections that need verification._
- **What connects `App`, `Option`, `PathBuf` to the rest of the system?**
  _137 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.0273972602739726 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.11215538847117794 - nodes in this community are weakly interconnected._