# Share host transport & throughput — findings

## Purpose
Hard facts (file:line) about the peer-share HOST side (`native/src/share/core/`) and its
Iroh/QUIC transport configuration, gathered to support redesigning file transfers for
maximum total throughput (no upfront full scan, many files in flight, few round trips,
possibly a batched multi-file request). No design judgment below, only observed behavior.

## Files actually read
`mod.rs` (module map only); `server.rs`, `server_transfer.rs`, `server_capabilities.rs`,
`fs.rs`, `fs_copy.rs`, `fs_access.rs`, `fs_response.rs`, `fs_paths.rs`, `fs_capabilities.rs`,
`fs_error.rs`, `wire.rs`, `framing.rs`, `node.rs`, `node_sessions.rs`, `node_accept.rs`,
`session.rs`, `io_deadline.rs`, `blocking.rs`, `peer_walk.rs`, `walk.rs`, `walk_assembly.rs`,
`keepalive.rs`, `authorization_policy.rs`, `peer_fs_logging.rs`, `peer_telemetry.rs`,
`types.rs` (grepped for FsRequest/FsResponse/limit — none present besides an unrelated
`MAX_PRESENCE_FUTURE_SECS` at types.rs:130), `backend.rs`, `peer_request.rs`, `peer_read.rs`,
`peer_writer.rs` — all under `native/src/share/core/`.

## 1. Accept path
- One Tokio task per QUIC connection accept loop; `node_accept.rs:13-61` `spawn_accept_loop`
  calls `endpoint.accept()` in a loop, takes a global handshake permit
  (`handshake_slots`, `MAX_PENDING_APPLICATION_HANDSHAKES=64`, `node.rs:25,118`) and a
  per-remote-endpoint permit (`peer_handshake_slots`, `MAX_PENDING_HANDSHAKES_PER_ENDPOINT=4`,
  `node.rs:26`, acquired `node_accept.rs:33`), then `tokio::spawn`s connection setup.
  These two limits bound **new-connection handshake admission only**, not streams on an
  already-established connection.
- Per stream: `server.rs:99-123` — `handle_connection` runs `loop { conn.accept_bi().await; tokio::spawn(handle_peer_stream(...)) }`.
  Every accepted bidi stream gets its own Tokio task; there is no additional
  application-level semaphore gating *how many streams* a connection may have open — that
  cap is enforced purely at the QUIC transport layer: `IROH_MAX_CONCURRENT_BIDI_STREAMS = 64`
  (`keepalive.rs:8`, wired in `node.rs:96` via `iroh_transport_config()`); uni-streams are
  disabled (`IROH_MAX_CONCURRENT_UNI_STREAMS = 0`, `keepalive.rs:9`).
- Actual filesystem work is bounded separately and **globally per process**, not per
  connection: `blocking.rs:12` `MAX_BLOCKING_OPERATIONS = 32`, a single `OnceLock<Semaphore>`
  (`blocking.rs:14-19`) shared by every connection/stream calling `blocking::spawn`/`run`
  (used by every `blocking_fs(...)` call in `server.rs`, and by `walk.rs`, `server_transfer.rs`,
  `fs_copy.rs`, `server_capabilities.rs`). So 64 concurrent streams per connection can be
  admitted, but only 32 concurrent blocking FS operations run process-wide at any moment;
  excess requests queue on the semaphore (`Semaphore::acquire_owned`, `blocking.rs:42-45`).
- `handle_connection` itself (`server.rs:26-124`) is one long-lived task per connection
  (not per stream) that only does the initial `PeerHello`/auth handshake, then loops
  accepting streams; it does not otherwise multiplex or buffer stream bodies itself.

## 2. Per-request overhead
- Authorization is re-checked on **every** accepted stream, not cached per-connection:
  `server.rs:149` calls `session.authorize(&auth)`, which is `session.rs:85-94`
  `IncomingSession::authorize` → `authorize_state` (`session.rs:167-238`). It reads only
  **in-memory** state: `auth: &Arc<Mutex<ShareAuthState>>` (locked at `session.rs:92`), doing
  contact/grant/room lookups and an HMAC session-proof check (`verify_hmac`,
  `session.rs:197,231`) — no disk I/O in this step. Comment at `session.rs:85-87` states this
  is deliberate: "The QUIC connection alone is never an authorization cache."
- Path resolution **does** touch disk on essentially every real-path operation: for
  stateless/dynamic access, `fs::resolve` (`fs.rs:146-183`) calls `secure_local_target`
  (`fs.rs:311-322`) which does `std::fs::canonicalize` on the export root (`fs.rs:314`) and
  then again on the resolved target or its nearest existing ancestor
  (`ensure_under_root`, `fs.rs:324-344`, canonicalize at `fs.rs:326,339`) as a symlink/reparse
  escape guard. This runs per FsRequest (List/Stat/Read/Write/Copy/Rename/.../Capabilities),
  there is no cross-request cache of a canonicalized root in this file. For a mounted/lease
  stream, resolution instead goes through `PeerMountLease::resolve` (`mount_lease.rs`, out of
  scope) — whether that path re-canonicalizes per call is **unresolved** (file not in scope).
- Logging/telemetry per successful op is intentionally cheap: `peer_telemetry.rs:13-32`
  `report_fs_success` samples 1-in-128 (`SAMPLE_EVERY=128`, line 9) unless the op took
  ≥500 ms (`ALWAYS_REPORT_AFTER`, line 10), and always uses non-blocking
  `crossbeam_channel::Sender::try_send` (`peer_telemetry.rs:39`, drops silently if the events
  channel is full, verified by test at lines 46-54). String formatting
  (`peer_fs_logging.rs:3-24,26-103`) only runs when a sample is actually emitted.
- `authorization_policy.rs:6-17` `configuration_changed` is a pure comparator (contacts/
  grants/rooms/export-config) used to decide whether stored config changed enough to trigger
  `ShareIrohNode::invalidate_sessions()` (`node.rs:214-243`); it is not itself a per-request
  check, it gates session-wide invalidation on policy edits.

## 3. FsRequest variants — host behavior (`server.rs:245-361` unless noted)
- **Capabilities** (`server.rs:176-194`→`server_capabilities.rs:7-85`): resolves mount
  capabilities (`fs_capabilities.rs:22-41`, itself doing `fs::resolve` + disk canonicalize)
  and optionally acquires a mount lease (`mount_leases.acquire`/`existing_acquisition`,
  internals in out-of-scope `mount_lease.rs`). One blocking op, one reply, no data phase.
- **ReleaseLease** (`server.rs:195-211`): `mount_leases.release`, reply `Ok`/`Err`.
- **ListDir** (`server.rs:248-253`): `access.list_dir` → backend `list_dir`, reply `Entries`.
- **Stat** (`server.rs:254-259`): backend `stat`, reply `Meta`.
- **WalkTree** (`server.rs:260`→`walk.rs`): see §8; streams `WalkBatch`/`WalkDone`.
- **StorageSnapshot** / **StorageAnalysis** (`server.rs:261-266`): dispatched to
  `storage_snapshot.rs` / `storage_analysis_server.rs` — **out of scope**, not read; only the
  dispatch line and the `FsResponse::SnapshotProgress/SnapshotReady/SnapshotDone` and
  `Analysis{message}` shapes (`fs_response.rs:42-62`) are visible from allowed files.
- **Read** (`server.rs:267`→`server_transfer.rs:22-97`): stats size, sends
  `FsResponse::Data{size}` (line 39) **before** any bytes, then streams `fs::CHUNK`
  (256 KiB, `fs.rs:15`) reads as `TAG_DATA` frames from a blocking worker through a bounded
  `mpsc::channel(STREAM_BUFFER_CHUNKS=2)` (`server_transfer.rs:14,28`).
- **Write** (Replace) / **WriteNew** (Create) (`server.rs:268-289`→`server_transfer.rs:99-297`):
  replies `Ready` (line 127) immediately after the writer opens, **before** any data. Replace
  mode opens a **private staging file** (`crate::vfs::unique_staging_path` +
  `backend.open_write_new(staging)`, `server_transfer.rs:222-230`), never writes in place;
  Create mode opens the target path directly via exclusive create
  (`backend.open_write_new(target.path)`, lines 232-235). On the `WriteDone` control frame
  (lease re-validated against the one used to open, lines 165-178), the worker calls
  `output.flush()` (line 273) then, for Replace mode, `crate::vfs::promote_staged_replace`
  (lines 275-278) to atomically commit; final `FsResponse::Ok` is sent only after that
  completes (lines 184-187). **fsync**: only `Write::flush()` is visible here; whether the
  concrete VFS backend's flush implies `fsync`/`FlushFileBuffers` is inside backend
  implementations, not in the allowed file set — **unresolved, out of scope**.
- **MkdirAll** (`server.rs:290-300`): `simple()` helper → `backend.mkdir_all`.
- **Rename** / **RenameNoReplace** (`server.rs:301-322`): `access.rename(..., no_replace)` →
  backend `rename`/`rename_no_replace`, gated by `run_authorized(lease)`.
- **PromoteStaged** (`server.rs:323-335`): `access.promote_staged` → backend `promote_staged`.
- **CopyFile** (`server.rs:336-338`→`fs_copy.rs:10-33`): same-backend uses the backend's
  native `copy_file`; cross-backend falls back to generic `crate::vfs::copy_between`
  (`fs_access.rs:66-80`, `fs_copy.rs:26-33`, out-of-scope internals). Single blocking call,
  reply `Data{size}` only after the whole copy finishes — **no progress frames**.
- **RemoveFile** (`server.rs:339-349`): `simple()` → `backend.remove_file`.
- **RemoveDir** (`server.rs:350-360`): `simple()` → `fs::remove_dir_recursive`
  (`fs.rs:213-233`), which refuses to recurse into a symlinked child directory
  (`fs.rs:220-222`, "Symlink/Reparse-Point wird nicht rekursiv geloescht").
- **WriteDone** as a top-level request (not the in-stream finishing frame) is always rejected:
  checked twice, early at `server.rs:214-216` and again defensively at `server.rs:361`.

## 4. Data framing
- Every frame: 4-byte big-endian `u32` length (covers tag+payload) + 1-byte tag + payload,
  then an explicit `send.flush()` (`framing.rs:57-71` `send_tagged`; read side
  `framing.rs:77-88` `recv_tagged_limited`). `TAG_CTRL=0`, `TAG_DATA=1` (`framing.rs:9-10`).
  Caps: `MAX_FRAME=16 MiB` absolute (`framing.rs:11`), `MAX_HANDSHAKE_CTRL_FRAME=64 KiB`
  (line 12) for the hello, `MAX_REQUEST_CTRL_FRAME=256 KiB` (line 13) for the first frame of
  each stream (`server.rs:137`).
- `fs::CHUNK = 256 * 1024` bytes (`fs.rs:15`) is the single chunk unit for **both** directions:
  host reads use `vec![0u8; fs::CHUNK]` per iteration (`server_transfer.rs:81`); the client
  writer splits outgoing buffers via `buf.chunks(fs::CHUNK)` (`peer_writer.rs:139`).
- No per-chunk application-level acknowledgement in either direction — data frames are
  fire-and-flush; backpressure comes from QUIC stream flow control plus small in-process
  bounded channels. Read-side prefetch is real but small: the host's blocking read worker can
  run up to `STREAM_BUFFER_CHUNKS=2` chunks (512 KiB) ahead of what has been sent, blocking on
  `chunks.blocking_send` once the `mpsc::channel(2)` is full (`server_transfer.rs:14,28,93`).
  The client write path has no such lookahead: `PeerWriter::write` awaits each chunk's
  `send_tagged` (i.e., its QUIC-level flush) before returning (`peer_writer.rs:139-153`), so
  from the application's perspective one chunk is "in flight" at a time per write stream;
  actual wire-level pipelining depth is whatever QUIC's own send buffer/flow control allows
  (see §6 — no explicit window sizes found).

## 5. Wire format
- Plain `serde_json` (not bincode): `send_ctrl` does `serde_json::to_vec(ctrl)`
  (`framing.rs:24`), `recv_ctrl_limited` does `serde_json::from_slice::<Ctrl>`
  (`framing.rs:36`). `FsRequest` is an internally-tagged enum `#[serde(tag="op",
  rename_all="snake_case")]` (`wire.rs:247-248`); `FsResponse` likewise `tag="r"`
  (`fs_response.rs:5`); `Ctrl` likewise `tag="c"` (`wire.rs:335-336`).
- Adding a new `FsRequest` variant: add the variant (`wire.rs:248-313`), handle it in the
  exhaustive match in `server.rs:245-360`, extend `mutates_filesystem()` if it writes
  (`wire.rs:316-329`), add a label in `peer_fs_logging.rs:4-24`, add an expected-response case
  in `response_matches()` (`peer_request.rs:289-315`) if it goes through the generic
  `request()` path, and add a client call site (new `PeerBackend`/`Backend` method,
  `backend.rs`). New optional **fields** on existing variants already use
  `#[serde(default, skip_serializing_if=...)]` throughout (e.g. `wire.rs:254-260` on
  `Capabilities`) for graceful old-peer compatibility.
- `FsRequest`/`Ctrl` have **no** `#[serde(other)]` catch-all (only `FsErrorKind` does,
  `wire.rs:242-243`). So an **old host receiving an unknown new request variant** fails to
  deserialize the `Ctrl` frame at `recv_ctrl_limited` (`framing.rs:36`, propagated through
  `server.rs:135-139`). Effect: only that one stream's task errors and is reported via
  `node.emit_connection_error(ConnectionErrorKind::FsStream, ...)` (`server.rs:108-121`); the
  surrounding connection and its other streams are unaffected (streams are independent
  spawned tasks, `server.rs:99-123`).
- Separately, `PeerHello.protocol_version` (`wire.rs:166`) is a **hard, non-negotiable gate**:
  `server.rs:49` requires exact equality `!= 3` → reject with "Inkompatibles Share-Protokoll"
  before any session/auth exists. `PeerHello.requested_capabilities: Vec<String>`
  (`wire.rs:177`) is a separate, additive mechanism — the client always sends
  `["fs","fs_walk_batches_v1"]` (+ `DIRECT_RECIPROCAL_CAPABILITY` for Direct)
  (`node_sessions.rs:326-329`). Within the allowed files, only `session.rs:134-137`
  (`authorize_direct_repair`) actually branches on a capability string
  (`DIRECT_RECIPROCAL_CAPABILITY`); no host-side check of `"fs"`/`"fs_walk_batches_v1"` was
  found — **unresolved**, possibly handled in an out-of-scope file or currently advisory only.
  `FsResponse::Capabilities` carries its own `contract_version` (`MOUNT_PATH_CAPABILITY_CONTRACT_VERSION=1`,
  `wire.rs:10`) plus additive `storage_snapshot_v1`/`storage_analysis_v2` booleans
  (`#[serde(default)]`, `fs_response.rs:19-22`) — this is the actual graceful old/new
  negotiation surface for mount capabilities specifically.

## 6. QUIC/Iroh transport config
- ALPN `b"smart-explorer/share-fs/3"` (`node.rs:24`); a separate `EXEC_ALPN` is registered
  alongside it (`node.rs:94`).
- `iroh_transport_config()` (`keepalive.rs:57-70`) sets exactly: `max_idle_timeout=20s`
  (`IROH_CONNECTION_IDLE_TIMEOUT`, line 7), `keep_alive_interval=5s` (line 5),
  `default_path_keep_alive_interval=5s` (line 6), `max_concurrent_bidi_streams=64` (line 8),
  `max_concurrent_uni_streams=0` (line 9). **No** explicit `stream_receive_window`,
  `receive_window`, `send_window`, or congestion-control override appears in this builder
  chain — those are left at iroh/Quinn defaults (not customized anywhere in scope).
- Relay vs. direct: `session.rs:269-276` `transport_label` inspects
  `connection.paths()`, returns `"relay"` if the selected path `is_relay()` else `"direct"`;
  surfaced as `ShareStatus::ConnectedRelay`/`ConnectedDirect` in `backend.rs:71-80`. Relay
  mode itself: `RelayMode::custom(...)` if `transport_options.relay_urls` non-empty, else
  `RelayMode::Disabled` (`node.rs:86-91`); `clear_ip_transports()` is called when
  `relay_only` is set, forcing relay-only paths (`node.rs:97-99`). The source of
  `transport_options` (`transport_options.rs`) is out of scope.
- Session cache/reuse (`node_sessions.rs`): connections are cached in
  `sessions: Mutex<HashMap<String, Connection>>` keyed by
  `"{kind}:{relation_id}:{node_id}"` (`session.rs:285-288`). `open_stream_until`
  (`node_sessions.rs:147-174`) first calls `session_connection_until`, which checks
  `healthy_cached_session` (lines 236-238, 272-287: cached **and**
  `connection.close_reason().is_none()`) — if healthy, opening a stream on it is just
  `connection.open_bi()` (lines 216-219), i.e. **no new handshake, no PeerHello/PeerHelloOk
  round trip**, just a native QUIC stream open. Only a cold/dead session pays the full
  `connect_session_until` cost: `endpoint.connect` + `open_bi` + send `PeerHello` + await
  `PeerHelloOk` (lines 356-387). Concurrent connects to the same key are singleflighted via a
  per-key `tokio::sync::Mutex<()>` (`connect_gate`, lines 239-258).
- What invalidates a cached session: (a) lazily, on next use, if
  `connection.close_reason().is_some()` (lines 272-287); (b) a failed `open_bi()` on a
  supposedly-healthy cached connection triggers one invalidate-and-reconnect retry
  (`open_stream_until`, lines 158-174); (c) **any** failed operation proactively evicts its
  generation — `peer_request.rs:104,116-126,187-190,197-199`, `peer_read.rs:88-96`
  (`close_with`), `peer_writer.rs:100-112,114-125` all call
  `node.invalidate_outgoing_session(&session_key, generation)` on error, and eviction only
  removes the entry if `Connection::stable_id()` still matches (no clobbering a concurrent
  reconnect, `node_sessions.rs:181-195`); (d) global `invalidate_sessions()`
  (`node.rs:214-243`) bumps `session_epoch` and force-closes every cached connection
  (incoming + outgoing) with close code `0x5345`, used e.g. on `stop_sharing()`/policy
  changes; new sessions re-check the epoch before caching (`node_sessions.rs:259-266,297-303`)
  so a race loses the connection instead of caching a stale-authorization one.

## 7. Deadlines
- `io_deadline::PEER_OP_TIMEOUT = 60s` (`io_deadline.rs:7`) is the dominant constant, but it
  is applied **per operation/step**, not cumulatively per file or per stream:
  `io_deadline::run`/`run_for` (lines 30-47) wrap one future in `tokio::time::timeout` and
  return a fresh error if that single future doesn't finish in time — each call site gets its
  own window.
- Host side: reading the first Ctrl frame of a stream is bounded (`server.rs:135-139`, 60s
  budget via `io_deadline::run`), but the actual filesystem work in `blocking_fs`
  (`server.rs:416-422`) is **not** wrapped in any `io_deadline` call — it can run as long as
  the blocking operation takes, bounded only by whatever the backend/OS does. The QUIC
  connection itself stays alive independently via the 5s transport keepalive
  (`keepalive.rs:5-6`), so a long blocking FS call does not risk an idle-timeout disconnect.
- Client side, per-chunk not per-file: `PeerWriter::write` wraps **each** `fs::CHUNK` send in
  its own fresh `io_deadline::run` (60s) (`peer_writer.rs:144-147`); `PeerReader::read` wraps
  **each** chunk receive the same way (`peer_read.rs:57-60`). Consequently a long single-file
  transfer does not hit these deadlines merely from being long — only a single 256 KiB chunk
  stalling ≥60s would trip it (worst case ~4.3 KiB/s sustained per chunk avoids timeout).
  The write-finish step (`WriteDone` + await `Ok`, i.e. host flush/promote) also gets its own
  fresh 60s from when it's issued (`peer_writer.rs:62-74`), so a very slow flush/rename on a
  huge file could time out independently of the data phase.
- Non-retryable requests get a 60s overall budget (`io_deadline::PEER_OP_TIMEOUT`,
  `peer_request.rs:22-23`); retryable reads (`Capabilities`/`ListDir`/`Stat`,
  `peer_request.rs:282-287`) get `IDEMPOTENT_CONTROL_BUDGET=40s` split across up to 2 attempts
  (line 14, 42), each attempt itself sub-bounded by `CONTROL_ATTEMPT_TIMEOUT=20s` (line 13) for
  the connect phase.
- `WalkTree` has its own, separately-defined idle timeout, coincidentally the same value:
  `WALK_IDLE_TIMEOUT=60s` (`peer_walk.rs:13`) with a 250 ms cancellation poll
  (`CANCEL_POLL`, line 12), reset by **any** received batch (`peer_walk.rs:112-142`) — so a
  large walk with steady batch flow never trips it, only a full stall does.
- `HANDSHAKE_TIMEOUT=20s` (`server.rs:24`) is the one **cumulative absolute** deadline in
  scope: it bounds the whole accept_bi+PeerHello+auth+PeerHelloOk sequence as a single
  `handshake_deadline` reused across several `io_deadline::run_until` calls
  (`server.rs:34-85`), unlike the per-step deadlines elsewhere.

## 8. Existing walk facility (WalkTree) — and its relation to StorageSnapshot
- `WalkTree` is served host-side by `walk.rs`/`walk_assembly.rs` and consumed client-side by
  `peer_walk.rs`. The host **streams progressively**: `walk.rs:44-102` `walk_worker` runs an
  iterative (explicit `Vec<WalkWork>` stack, not recursion) depth-first traversal on a
  blocking thread, emitting `FsResponse::WalkBatch` every `WALK_BATCH_NODES=256` nodes
  (`walk_assembly.rs:6`, batch push at `walk.rs:74-79`) **or** every ≥250 ms if a partial batch
  is pending (`CANCEL_POLL`, `walk.rs:19,81-86`), interleaved with directory descent — the
  first batch can go out long before the whole tree is walked.
- The client, however, currently **buffers the entire tree before returning anything**:
  `peer_walk.rs:69-110` `receive_responses` feeds every `WalkBatch` into a `TreeAssembler`
  (`walk_assembly.rs:39-138`), which only yields the completed root `WireNode` from `finish()`
  (lines 140-154) after `WalkDone`. The caller's `on_progress(files, bytes)` callback only
  gets cumulative counters (`peer_walk.rs:86,96`), not individual entries — so today's
  `WalkTree` client API is not usable to start transferring file N as soon as it is
  discovered without a new consumer; the wire protocol itself is already batch-streamed.
- Limits (enforced independently by both sides, and cross-checked — `walk_assembly.rs:57-67`
  errors if the client's recomputed totals disagree with the server-reported per-batch
  totals): `MAX_WALK_NODES=1,000,000` total files+dirs (`walk_assembly.rs:7`, checked
  `walk.rs:284-286` and `walk_assembly.rs:71-73`); `MAX_WALK_DEPTH=512`
  (`walk_assembly.rs:9`, checked both sides); `MAX_WALK_NAME_BYTES=128 MiB` **cumulative**
  across all names in one walk, not per-name (`walk_assembly.rs:8`, checked
  `walk.rs:287-291` and `walk_assembly.rs:75-79`). Symlinked entries are skipped mid-walk
  (`walk.rs:228-230`); a symlinked root is a hard error (`walk.rs:197-201`).
- **Important scope-limited finding**: `backend.rs:222-228`, the `Backend::walk_tree` trait
  method that the generic VFS layer actually calls, invokes
  `super::peer_storage_snapshot::walk_peer(self, root, on_progress)` — **not**
  `peer_walk::walk_peer`. `peer_storage_snapshot.rs` and its host counterpart
  `storage_snapshot.rs` are **out of scope** (not in the allowed file list), so their framing,
  batching, and limits cannot be documented here; only the dispatch line in `server.rs:261-263`
  and the `FsResponse::SnapshotProgress/SnapshotReady/SnapshotDone` shapes
  (`fs_response.rs:42-62`, plus a `sha256`/`encoded_len` on `SnapshotReady`) are visible.
  No call site for `peer_walk::walk_peer` (the `WalkTree`-based client function) was found
  anywhere in the 30 allowed files — **unresolved**: it may be legacy/superseded by
  `StorageSnapshot`, or called from a file outside this task's scope (e.g. a task/orchestration
  module). The server still implements and would respond to `FsRequest::WalkTree`
  (`server.rs:260`), so the host-side facility is live even if its originally-paired client
  function looks unused from what was read.

## Unresolved / out of scope (summary)
- `mount_lease.rs` internals (lease resolve/authorize/acquire cost, whether mounted access
  re-canonicalizes per call) — not in the allowed file list.
- Whether a VFS backend's `Write::flush()` performs `fsync`/`FlushFileBuffers` — backend
  implementations are not in the allowed file list.
- `storage_snapshot.rs` / `peer_storage_analysis.rs` / `peer_storage_snapshot.rs` internals
  (the mechanism `Backend::walk_tree` actually uses) — not in the allowed file list.
- `crate::vfs::copy_between` / `promote_staged_replace` / `unique_staging_path` internals —
  outside `share/core/`.
- Host-side handling (if any) of the `"fs"`/`"fs_walk_batches_v1"` capability strings sent in
  `PeerHello.requested_capabilities` — no branch on them found in the allowed files.
- Call site(s) of `peer_walk::walk_peer` — none found in the allowed files.
- iroh/Quinn default values for stream/connection flow-control windows and congestion control
  (not overridden in `keepalive.rs`, and the defaults themselves live outside this crate).
- `transport_options.rs` (source of relay URLs / relay-only flag) — not in the allowed list.
