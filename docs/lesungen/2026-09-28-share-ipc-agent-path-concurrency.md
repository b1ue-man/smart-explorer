# Share/Room IPC -> AgentBackend -> se-agent protocol: concurrency and round-trip facts

## Purpose

Hard facts (file:line + code-fact statements, no design judgement) about concurrency and
per-operation round trips on the desktop transfer path: GUI -> `daemon::open_share_backend`
(native/src/daemon/os/shared/ipc_client.rs:22) -> loopback-TCP `AgentBackend` (agent protocol,
`native/src/agent_proto`) -> daemon backend server (`native/src/daemon/os/shared/backend_server.rs`)
-> `PeerBackend` (Iroh/QUIC, `native/src/share/core`). The same `AgentBackend` client type also
drives the SSH `se-agent` binary, whose server loop lives in `native/src/agent_proto/core/server.rs`.
Collected to inform a from-scratch redesign of file transfers for maximum concurrent throughput
with few round trips per file.

## Files read

- native/src/daemon/os/shared/ipc_client.rs
- native/src/agent/mod.rs
- native/src/agent/core/{agent_error,backend,deploy,metadata,mux,stream,transfer,transport,walk}.rs
- native/src/agent_proto/mod.rs
- native/src/agent_proto/core/{codec,frame_encode,frame_io,node_codec,relative_path,server,session,types}.rs
- native/src/agent_proto/os/shared/{fs,hash,promotion,put_tree,search,transfer,write_new}.rs
- native/src/agent_proto/os/linux_os/{local_platform,sandbox}.rs
- native/src/agent_proto/os/windows/local_platform.rs
- native/src/daemon/mod.rs (module map only)
- native/src/daemon/os/shared/{backend_server,backend_transfer,backend_tree_send,backend_walk,backend_budget,backend_delete,request_workers,ipc_listener,ipc_host_service}.rs
- native/src/share/core/backend.rs
- native/src/share/core/peer_request.rs

(`*_tests.rs`/`tests.rs` files and `native/src/daemon/mod.rs`'s non-listed sibling modules were
skipped per scope; `vfs/*.rs`, `ipc.rs`, `ipc_host*.rs` other than `ipc_host_service.rs`, and
`share/core/*` other than `backend.rs`/`peer_request.rs` were out of scope and not read.)

## 1. Request dispatch: multiplexed vs. one-at-a-time; concurrency cap

Multiplexed by `req_id`, not mutex-serialized per call. `Frame` carries a `u64` id
(agent_proto/core/codec.rs:134, frame_encode.rs:26). `Mux` (agent/core/mux.rs:17-28) holds one
outgoing `Sender<RoutedFrame>` (`out`, shared) and a `pending: Arc<Mutex<HashMap<u64,Sender<Frame>>>>`.
`register()` (mux.rs:80-91) allocates a fresh id + bounded per-op reply channel
(`bounded(TRANSFER_FRAME_BACKLOG)` = 32, types.rs:21). `establish()` (agent/core/transport.rs:216-269)
spawns exactly **one** "agent-writer" thread that drains `out` and calls `agent_proto::write_frame`
serially onto the single stream, and **one** "agent-reader" thread that calls `read_frame` and routes
`(id, Frame)` via `route_frame` (mux.rs:307-327) to the waiting op's channel. So N threads calling
`list_dir`/`stat`/`open_read`/`open_write*` on a shared `Arc<AgentBackend>` each get their own
`req_id` + reply channel and can be in flight concurrently (agent/core/backend.rs methods each call
`connection.safe_call_timeout` / `mutation_call` / `agent_unit_op`, which register/send/recv per call).
Physical writes are still serialized frame-by-frame by the single writer thread (byte-level
interleaving only, not full parallel writes).

Bounding: `out` queue capacity `OUT_BACKLOG = 32` (mux.rs:14, shared by *all* ops on the connection);
`Mux.next_id: AtomicU64` is unbounded (mux.rs:23,81) and `pending` has no size cap client-side. The
**server** enforces the real concurrency ceiling: se-agent `MAX_ACTIVE_REQUESTS = 8`
(agent_proto/core/server.rs:19), replying `Frame::Err("too many concurrent agent requests")` once
exceeded (server.rs:194-200); daemon backend server `MAX_REQUEST_WORKERS = 16`
(daemon/os/shared/request_workers.rs:7), replying `Frame::Err("too many concurrent backend requests")`
(backend_server.rs:131-137). Comment at request_workers.rs:3-6: "Mount clients admit at most eight
live requests" — the 16 is deliberate slack over that 8. Rejection is immediate, not queued.

## 2. Reader/writer in flight: does it block other ops on the same backend?

No exclusive hold. `AgentReadStream` (agent/core/stream.rs:10-71) and `AgentWriteStream`
(stream.rs:74-169) each own their own `id`/`rx`/`Arc<Mux>`; `.read()`/`.write()` only touch that op's
channel or call `mux.send()` (enqueue onto the shared bounded `out` queue), never a lock spanning the
whole connection. `Mux.pending` (a `Mutex`) and `AgentConnection.state` (a `Mutex`, transport.rs:40)
are each held only for brief critical sections (insert/remove/clone), never across a blocking
`recv()`/`send()`. So one thread holding an open reader from backend A and opening a writer to the
same backend A cannot deadlock structurally — verified: `open_read`/`open_write*` both just call
`mux.register()` (a new id) and proceed. Caveat: `Mux::send()` (mux.rs:104-137) has a shared
`stall_timeout` (= `HeartbeatPolicy::deadline`, default 30s, transport.rs:26); if the wire itself
stalls (remote stops reading) and the 32-deep `out` queue stays full, **every** op sharing that mux —
not just the stalled one — blocks until `stall_timeout`, after which `self.close()` (mux.rs:112)
disconnects every pending operation with an error (not a permanent deadlock, but a shared 30s stall
window across all concurrent ops on one connection).

## 3. Round trips, chunk size, pipelining, ack/flush semantics

`CHUNK = 256 * 1024` bytes (agent_proto/core/types.rs:17). `TRANSFER_FRAME_BACKLOG = 32`
(types.rs:19-21, "~8 MiB of pipelining"); `OUT_BACKLOG = 32` (agent/core/mux.rs:12-14, same ~8 MiB
figure, but this bound is shared across *all* ops on the mux, not per-op).

**Read**: `agent_open_read` sends one `Frame::Read{path,offset:0,len:0}` and waits for the first reply
frame (stream.rs:174-215, `open_read_once` at 272-298) — one round trip to open. After that the
**server pushes `Data` chunks continuously** without waiting for any per-chunk client ack
(agent_proto/os/shared/transfer.rs `handle_read` 17-44: loop of `f.read`+`emit(Data)` until EOF, then
`Frame::End`; daemon `handle_read_backend` backend_server.rs:329-358 identical pattern over
`backend.open_read`). Client-side buffering is bounded by the per-op reply channel (32 frames / ~8 MiB,
mux.rs:82) via `route_with_backpressure` (mux.rs:331-347): so reads are prefetched/pipelined up to
~8 MiB ahead of the consumer, not fetched chunk-by-chunk-request. One client request serves an
entire file.

**Write**: `agent_open_write_request` sends `Frame::Write`/`WriteNew(path)` and waits for a
`Frame::Progress` ack before returning a writer (stream.rs:227-262) — one round trip to open. Each
`.write()` call is fire-and-forget: `mux.send(id, Frame::Data(...))` with no per-chunk ack
(stream.rs:140-151), backpressured only by the shared `OUT_BACKLOG=32`. `.flush()` calls `.finish()`
(stream.rs:92-129,153-155), which sends `Frame::End` and **blocks for the reply** (`Frame::Ok`/`Err`)
— this is the real commit round trip (server-side staged-write + atomic promote must complete first).
`Drop` while still `Open` (flush never called) sends `Cancel`+`End` and unregisters **without**
waiting for a reply — "Dropping a writer is an abort, not an implicit commit" (stream.rs:161-163).

**Metadata/unit ops** (list_dir, stat, try_exists, copy_file, rename, rename_no_replace,
promote_staged(_no_replace), remove, mkdir): exactly one round trip each via
`call_absolute_timeout`/`mutation_call` (mux.rs:188-207; transport.rs:122-132). `copy_file` on
`AgentBackend` is 2 round trips: `Frame::Copy` then a separate `self.stat(dst)` for the resulting size
(agent/core/backend.rs:359-365).

Absolute per-request timeout for metadata: `METADATA_REQUEST_TIMEOUT = 20s`
(agent/core/backend.rs:15), unaffected by unrelated traffic (mux.rs:185-187 doc comment).

## 4. Backend trait methods: overridden vs. default

Complete `impl Backend for AgentBackend` (agent/core/backend.rs:95-455), one method per line below.
Sent over the wire (Frame variant in parens) unless marked "local":

scheme:96(local, ->inner) · root_display:100(local) · state_identity:104(local) ·
namespace_identity:107(local) · list_dir:111(`ListDir`) · stat:125(`Stat`) ·
try_exists:139(`TryExists`) · supports_walk_tree:153(local,true) · walk_tree:157(`WalkTree`) ·
scan_storage:162(**local, delegates to `self.inner.scan_storage`, i.e. does NOT go through the
agent Frame/mux protocol at all**) · supports_bulk_tree:167(local) · get_tree:175(`GetTree`) ·
put_tree:179(`PutTree`) · supports_search:183(local,true) · search:187(`Search`) ·
supports_walk_hashed:258(local,true) · walk_hashed:262(`WalkHashed`) · open_read:338(`Read`) ·
open_read_id:342(**ignores `id`, forwards to open_read — no id-qualified reopen on the wire; `Frame::Read`
has no id field, types.rs ~79-83**) · open_write:347(`Write`) · open_write_new:351(`WriteNew`) ·
download_name:355(local, ->inner) · copy_file:359(`Copy`, + follow-up `Stat`) · rename:367(`Rename`) ·
rename_no_replace:374(`RenameNoReplace`, via `connection.mutation_call` directly) ·
promote_staged:392(`Promote`) · promote_staged_no_replace:399(`PromoteNoReplace`) ·
remove_file:406(`Remove{recursive:false}`) · remove_file_id:413(ignores id, forwards to remove_file) ·
remove_dir:417(**also `Remove{recursive:false}`** — identical wire call to remove_file; se-agent's
server-side recursive-delete capability, `remove_path(path,recursive)` at
agent_proto/os/shared/transfer.rs:167-178, and the daemon's `remove_tree_backend`
(daemon/os/shared/backend_delete.rs:38-84) dispatched from `Frame::Remove{recursive:true}`
(backend_server.rs:284-294), are never reached with `recursive:true` from anywhere in
`AgentBackend`'s own trait impl) · mkdir_all:424(`Mkdir`) · parallelism:428(local, ->inner) ·
rename_overwrites:432(local, hardcoded `true`) ·
staged_write_capabilities:436(local, hardcoded `StagedWriteCapabilities::complete()`) ·
root_confinement:440(local) · is_local:448(local, ->inner) · provides_content_hash:452(local, ->inner).

For the Share/Room path specifically, `self.inner` is `UnavailableBackend`
(daemon/os/shared/ipc_client.rs:371-421), whose `scan_storage` calls a **separate** IPC path,
`super::ipc_analysis::scan(...)` (ipc_client.rs:377-379) — outside the agent protocol/mux entirely,
and outside this task's read scope.

**Unresolved**: `open_write_copy_stage` / `promote_copy_stage` (named in the task prompt as possibly
defaulted methods) do **not** appear anywhere in the `impl Backend for AgentBackend` block, so they
fall back to whatever default the `Backend` trait itself provides — the trait definition lives in
`native/src/vfs/*.rs`, which is outside this task's allowed read scope, so the exact default body
could not be confirmed.

## 5. `supports_bulk_tree`, `get_tree`, `put_tree` protocol

`supports_bulk_tree` (agent/core/backend.rs:167-173) returns `self.root_confined.is_none()`. Comment:
the remote `PutTree` receiver spools under the system temp dir, which a Landlock-root-confined agent
(SFTP-deployed `se-agent`, agent/core/deploy.rs:174-180) cannot write to, so confined agents advertise
`false` (generic per-entry fallback). The Share/Room `AgentBackend` (ipc_client.rs, plain
`from_streams`, no `root_confined`) always has `supports_bulk_tree() == true`.

**Framing**: not tar-like; it reuses the same `Frame` enum. Wire shape for both directions is a flat
sequence: `TreeEntry{rel,is_dir,size,mtime_ms}` header, then (if a file) zero-or-more `Data(chunk)`
frames of the file's bytes, then the *next* `TreeEntry` (which implicitly closes the previous file —
see `BufferedTreeReceiver::accept`'s `finish_pending()` call at the top of its `TreeEntry` arm,
agent_proto/os/shared/put_tree.rs:110-133), finally one `Frame::End` for the **whole tree** (not per
file). `rel` is validated via `ValidatedRelativePath::parse` (rejects `..`, absolute, backslash,
drive/stream-colon components — agent_proto/core/relative_path.rs:16-50).

**Collect-first, not walk-while-streaming**, on every side observed:
- se-agent `GetTree`: `handle_get_tree` (agent_proto/os/shared/transfer.rs:412-461) calls
  `collect_local_tree` (191-326) first — a full recursive, link-rejecting, identity-capturing walk
  into `Vec<LocalTreeEntry>` — *then* streams `TreeEntry`+`Data` per entry, re-validating identity
  before (`open_local_tree_file`, 328-357) and after (`finish_local_tree_file`, 359-385) each file's
  bytes to detect concurrent modification.
- daemon `GetTree` for a Share peer: `handle_get_tree_backend` (backend_tree_send.rs:16-67) calls
  `collect_source` (69-131) first — a full recursive `backend.list_dir`/`backend.stat` walk into
  `Vec<SourceEntry>` — then streams per entry, re-`stat`-ing before and after each file
  (lines 31-32, 62-64).
- se-agent `PutTree` (client `agent_put_tree`, agent/core/transfer.rs:69-103): calls
  `agent_proto::collect_local_tree` first to build the manifest (with sizes), then
  `send_tree_manifest` (106-149) streams `TreeEntry`+chunked `Data` while reading each file lazily
  from disk (file bytes are *not* all buffered in client memory, only the manifest is precomputed).
- **Receive side always buffers the entire tree before touching the destination.**
  `BufferedTreeReceiver`/`StagingArea` (agent_proto/os/shared/put_tree.rs:88-176,
  promotion.rs `StagingArea` 8-57) spools every incoming file into a private flat temp directory;
  only after `Frame::End` does `BufferedTree::publish_local` (put_tree.rs:190-220) run a read-only
  preflight over every destination ancestor, `mkdir_all` every directory, then per-file
  staged-write+atomic-promote. Daemon side: `handle_put_tree_backend`
  (backend_transfer.rs:22-63) buffers via the same `BufferedTreeReceiver`, then `publish_backend_tree`
  (73-134) does the equivalent through the generic `Backend` trait (`backend.mkdir_all` /
  `backend.open_write` / `backend.promote_staged`).

**Limits**: `MAX_TREE_ENTRIES=1_000_000`, `MAX_TREE_TEXT_BYTES=128 MiB`, `MAX_TREE_DEPTH=512`
(agent_proto/os/shared/put_tree.rs:15-17; mirrored for the size-tree wire decode in
agent_proto/core/node_codec.rs:5-7, and for daemon walk/delete planning in
daemon/os/shared/backend_budget.rs:3-5 and backend_delete.rs:6-8). `MAX_FRAME = 64 MiB`
(agent_proto/core/codec.rs:7) caps any single encoded frame; `Data` frames are separately capped at
`CHUNK` by `bounded_bytes(CHUNK)` at decode time (codec.rs:165).

**Errors mid-tree**: `GetTree` — a source mismatch (size/identity/mtime) after preflight returns
`InvalidData` and the whole op aborts (`Frame::Err`); nothing is published on the receiving side
because `publish_local` is only reached after a clean `Frame::End` (explicit test:
`disconnected_get_tree_preserves_existing_local_destination`, agent/core/transfer.rs:159-227).
`PutTree` — receive-phase failures (disconnect, cancel, bad manifest) leave the real destination
untouched (explicit tests in put_tree.rs and backend_transfer.rs: `invalid_manifest_never_creates_...`,
`disconnect_preserves_existing_destination_...`). **The apply/publish phase itself is not
transactional across files**: after the full-tree preflight passes, `publish_local` /
`publish_backend_tree` loop over entries and `?`-propagate the first per-file promote failure
(put_tree.rs `BufferedTree::publish_local` 190-220; backend_transfer.rs:130 `result?;`) — files
already promoted earlier in that same loop stay promoted; only files at/after the failing one are
not applied.

**Replace/no-replace**: both directions use replace semantics. `PutTree` publish calls
`promote_staged_replace` (se-agent: promotion.rs:271-291, replaces an existing plain-file destination
via `replace_file_atomic`/`rename_no_replace` fallback for a missing destination, refuses
dir/link-like destinations) or, on the daemon, `backend.promote_staged` (generic `Backend` trait,
which for `PeerBackend` sends `Frame::Promote` over its own peer wire — share/core/backend.rs:283-291).
`GetTree` publish likewise ends in `promote_staged_replace` via `StagedLocalFile::publish_local`
(promotion.rs:164-186). No `PutTree`/`GetTree`-specific no-replace variant exists in the `Frame` enum
(types.rs) — only the single-file `PromoteNoReplace`/`RenameNoReplace` frames exist, and bulk-tree
code paths read here do not use them.

## 6. Daemon backend server concurrency

`serve_backend` (daemon/os/shared/backend_server.rs:75-203) reads frames off **one** client TCP
connection in a loop. Continuation frames (`Data`/`TreeEntry`/`End`) for an already-open transfer are
routed to that transfer's inbound channel (104-114); `Cancel` sets that request's cancel flag
(115-117, `cancel_request` 59-69). Any other (new top-level) request frame spawns a dedicated OS
thread, `"daemon-backend-request-{id}"` (157-186), running `dispatch_backend` — i.e., **requests from
one client connection run concurrently**, not serially, via one worker thread per outstanding
request id. Concurrency cap: `RequestWorkers` (request_workers.rs) with
`MAX_REQUEST_WORKERS = 16` (request_workers.rs:7); `has_capacity()` (backend_server.rs:119-130)
rejects new requests past that with `Frame::Err("too many concurrent backend requests")`
(132-137) after reaping finished workers.

`dispatch_backend` (backend_server.rs:205-316) matches the `Frame` and calls the matching method
directly on the already-open `BackendHandle` (e.g. `Frame::ListDir(p) => backend.list_dir(&p)`,
`Frame::Read{..} => handle_read_backend(...)` which itself loops `backend.open_read` + `emit(Data)`).
**No re-authorization, profile re-read, or per-request logging is present in any of
backend_server.rs / backend_transfer.rs / backend_tree_send.rs / backend_walk.rs /
backend_delete.rs** — these files are pure Frame<->Backend-trait plumbing. Connection-level
auth/pre-auth gating (`MAX_PRE_AUTH_CONNECTIONS = 16`, token check) happens once, earlier, at
`ipc_listener.rs:17,32,51` and in `super::ipc::handle_client` (ipc_listener.rs:64) — **outside this
task's read scope** (ipc.rs not in the allowed file list) — so the exact per-connection auth
handshake before `serve_backend` starts, and the exact call site that invokes `serve_backend` for an
accepted `OpenShare` request, could not be directly confirmed; `daemon/mod.rs:204` only exposes
`serve_backend` under `#[cfg(test)]` as `serve_sync_link_fixture`, consistent with (but not proof of)
production wiring living in `ipc.rs`/`ipc_host_commands.rs`.

Budgets: `backend_budget::WalkBudget` (daemon/os/shared/backend_budget.rs:3-29,
`MAX_BACKEND_WALK_NODES=1_000_000` / `MAX_BACKEND_WALK_TEXT_BYTES=128 MiB` /
`MAX_BACKEND_WALK_DEPTH=512`) bounds `WalkTree`/`Search`/`WalkHashed` traversal
(daemon/os/shared/backend_walk.rs). `backend_delete::DeleteBudget` (backend_delete.rs:6-36, same
numeric limits) bounds recursive `Remove{recursive:true}` planning. Both are memory/DoS budgets, not
concurrency throttles.

## 7. Server-side `CopyFile` primitive

`Frame::Copy{src,dst}` exists (agent_proto/core/types.rs Frame::Copy). se-agent dispatch
(agent_proto/core/server.rs:308-311) calls `copy_file_safe` (agent_proto/os/shared/transfer.rs:111-128):
opens `src`, copies into a staged temp file (`create_staged_file`), `sync_all`, then
`promote_staged_replace(staged, dst)` — **replace** semantics, atomic; on failure the staged temp is
removed and `dst` is untouched. Daemon dispatch (backend_server.rs:258-261) calls generic
`backend.copy_file(src,dst)`; for `PeerBackend` this sends `FsRequest::CopyFile{src,dst}` over the
peer's own Iroh/QUIC wire (share/core/backend.rs:252-261), replying `FsResponse::Data{size}` or `Ok`
(client falls back to a `stat(dst)` if just `Ok`). The `AgentBackend` client's own `copy_file`
(agent/core/backend.rs:359-365) always issues a **second** round trip (`self.stat(dst)`) after `Copy`
to learn the resulting size. Copy is always executed entirely server-side (bytes never cross back to
the client).

## 8. Timeouts and cancellation propagation

**Reader drop**: `AgentReadStream::drop` (agent/core/stream.rs:64-71) — if not yet exhausted, sends
`Frame::Cancel` (best-effort, error ignored) then `mux.unregister(id)`; does not block waiting for
server acknowledgement. **Writer drop**: `AgentWriteStream::drop` (stream.rs:158-169) — if still
`Open` (never flushed), sends `Cancel` then `End` and unregisters without waiting for a reply
("Dropping a writer is an abort, not an implicit commit"). Both are fire-and-forget from the dropping
thread's point of view.

**Server-side cancellation is cooperative**, via a per-request `Arc<AtomicBool>` polled in each
handler's loop (e.g. `handle_read` checks every chunk, os/shared/transfer.rs:32-34; daemon
`handle_read_backend` backend_server.rs:346-348; tree/search/hash-walk loops similarly). A `Cancel`
frame sets that flag and, for upload-style ops, drops the sender side of the inbound channel to wake
a blocked `recv()` (`cancel_request`, agent_proto/core/server.rs:38-48 and
daemon/os/shared/backend_server.rs:59-69). A handler blocked inside one long syscall only notices
cancellation at its next poll.

**Transport-level timeouts**: `Mux::send()` (mux.rs:104-137) uses a shared `stall_timeout`
(= `HeartbeatPolicy::deadline`, default 30s, transport.rs:26); on expiry it closes the **whole** mux
(`close_transport`, mux.rs:299-305), failing every pending op on that connection, not just the
stalled one. `call_absolute_timeout` (mux.rs:188-207, used for metadata ops,
`METADATA_REQUEST_TIMEOUT=20s`) is a fixed wall-clock deadline unaffected by unrelated traffic.
`call_inactivity_timeout` (mux.rs:252-289, used for the Hello handshake/heartbeat) resets on any
observed activity. A dedicated heartbeat thread (`heartbeat_loop`, transport.rs:297-358) pings with
`Frame::Hello` after `policy.idle` (30s) of silence and, on failure, closes the mux and — **only if a
`reconnect` closure was supplied** — transparently reconnects (`replace_unusable`, transport.rs:166-196).

**Share/Room path has no reconnect**: `open_share_backend` builds its `AgentBackend` via
`AgentBackend::from_streams` (daemon/os/shared/ipc_client.rs:58-62), which passes `reconnect: None`
(agent/core/backend.rs:30-36, `from_streams_inner(..., None, None)`). Only the SFTP-deployed
`se-agent` path (`deploy_over_sftp`, agent/core/deploy.rs:164-180) supplies a real `AgentReconnect`
closure. Consequence: once the daemon IPC connection backing a Share/Room `AgentBackend` drops or
times out, that instance cannot self-heal — `replace_unusable` returns
`"agent transport closed or retired and reconnect is unavailable"` (transport.rs:180-185) — every
subsequent (and any still-pending) operation on it fails permanently; the caller must call
`open_share_backend` again (which itself retries the *initial* TCP connect/handshake up to 8 times,
ipc_client.rs:30-89) to obtain a fresh instance.
