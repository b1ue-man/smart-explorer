# SFTP / CachingBackend / Local backend — concurrency and per-operation cost

## Purpose
Hard facts for a file-transfer redesign targeting maximum total throughput (no upfront
full scan, several files in flight concurrently per transfer via worker threads sharing
one `Arc<dyn Backend>`, few round trips per file): the SFTP backend's session model and
I/O cost, the directory-listing cache every remote is browsed through, and the local
backend's write/rename/copy primitives. Does not repeat API-signature/portability
material already in `docs/lesungen/2026-09-25-android-core-ops-api-and-portability.md`.

## Files actually read
- `native/src/sftp/mod.rs`
- `native/src/sftp/core/{backend,config,connection,errors,io_adapters,metadata,posix_rename,reconnect_gate,session,url}.rs`
- `native/src/sftp/os/shared/known_hosts.rs`
- `native/src/vfs/core/{cache,cache_index,cache_load,cache_retirement,cache_support,cache_writer}.rs`
- `native/src/vfs/os/shared/{local,copy_transfer}.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/vfs/os/linux_os/local_platform.rs`
- `native/src/connect/os/shared/connector.rs`

Not read (out of assigned scope), referenced below only where a call site names them:
`native/src/vfs/core/backend.rs` (the `Backend` trait itself, incl. default method
bodies), `vfs/os/shared/promotion.rs` (`super::promotion::*` in `local.rs`),
`crate::agent` (`deploy_over_sftp`), `crate::vfs::promote_staged_with`, and the
`russh`/`russh-sftp` crate internals.

---

## 1. Session/channel model, parallelism

- One private multi-threaded Tokio runtime per `SftpBackend`, `worker_threads(2)`
  (`connection.rs:49-54`). `SftpBackend` is `Clone` (`backend.rs:20-28`), sharing
  `Arc<Runtime>` + `Arc<SftpConnection>` — cloning it for worker threads creates no new
  session.
- Exactly one live transport at a time: `SftpTransport{session: client::Handle<Client>,
  sftp: Arc<SftpSession>}` (`connection.rs:22-25`), held as one
  `Generation<SftpTransport>` behind a `ReconnectGate` (`connection.rs:39-45,56-61`) —
  one SSH session with one SFTP subsystem channel, **not** a pool.
- `sftp/mod.rs:6-12` (module doc): "a worker thread continuously drives russh's
  background connection task, while each blocking Backend method runs
  `rt.block_on(...)`" — of the 2 worker threads, one is effectively committed to
  connection upkeep. No explicit `tokio::spawn` for this appears in `connection.rs`/
  `session.rs` (it's inside `russh::client::connect_stream`, external, unresolved).
- `SftpBackend::parallelism()` returns `1` unconditionally: "Conservative: one SFTP
  session, sequential remote walk. Safe default until a real-server concurrency spike"
  (`backend.rs:372-376`).
- `SftpConnection::current()` (`connection.rs:100-102`) → `ReconnectGate::acquire()`
  holds its `Mutex<ReconnectState>` only long enough to clone the current
  `Arc<Generation<_>>` or flip `reconnecting` (`reconnect_gate.rs:106-134`); the guard
  is dropped before any I/O — so our own locking does not itself serialize concurrent
  `stat`/`list_dir`/`open_read`/`open_write` from different threads.
- **Unresolved**: whether concurrent requests over the one shared `Arc<SftpSession>`
  (`generation.sftp()`) truly execute in parallel or are serialized/pipelined
  internally is inside `russh_sftp::client::SftpSession` (external crate, not read).
  The code's own declared advice (`parallelism()==1`) is not to try.

## 2. Does an open reader/writer hold a lock that blocks/deadlocks other ops?

- `SftpReader` (`io_adapters.rs:76-84`) / `SftpWriter` (`io_adapters.rs:119-124`) each
  hold an owned `Arc<SftpGeneration>` (a refcount, not a `MutexGuard`) plus their own
  `russh_sftp::client::fs::File`. Neither re-enters `ReconnectGate` for ordinary
  reads/writes, so a long-lived open reader/writer holds no gate mutex and does not
  block unrelated `current()` callers.
- Direct in-repo evidence that reader+writer overlap on one backend is treated as
  unsafe: `copy_transfer.rs:1` — "Host-backed spool for providers that cannot overlap
  a reader and writer." `copy_between()` fully drains the source into a local
  `tempfile::tempfile()` spool and `drop(reader)` (`copy_transfer.rs:24-25`) **before**
  opening the destination writer (`copy_transfer.rs:36`). `SftpBackend` has no own
  `copy_file` override (§4), so it falls through to this spool path.
- No nested-`block_on` hazard in the reviewed code: each Backend method does one
  top-level `self.rt.block_on(...)` and returns before the next call.
- **Unresolved**: whether the single SFTP session would itself deadlock (vs. merely
  serialize) if source and destination handles were kept open at once — `russh_sftp`'s
  internal request routing is out of scope. The in-repo signal is that this is avoided
  by design (the spool), not proven safe.

## 3. Reader/writer implementation: request size, pipelining, flush, fsync

- `SftpReader::read()`: one `self.file.read(buf).await` per call, sized by the
  caller's buffer (`io_adapters.rs:86-96`); no read-ahead/pipelining in our code. On
  error, if the transport is proven dead **and** `delivered==0` **and** not already
  retried, it reopens the same path from a fresh generation once
  (`io_adapters.rs:97-113`); once any bytes were delivered, a dead transport is a hard
  error (no resume).
- `SftpWriter::write()`: one `file.write(buf).await` per call (`io_adapters.rs:126-138`);
  chunk size is caller-controlled (`std::io::copy`'s buffer), not adapter-controlled.
- `SftpWriter::flush()` is the commit boundary, not a cheap buffer flush: it does
  `file.flush().await?; file.shutdown().await` and **takes** `self.file`
  (`io_adapters.rs:140-157`) — no further writes possible afterward
  ("Datei geschlossen", `io_adapters.rs:132,145`). Comment (`io_adapters.rs:146-148`):
  "Await both outstanding WRITE acknowledgements/fsync and SSH_FXP_CLOSE so a close
  failure cannot be hidden by Drop before staged promotion." So `flush()` does wait for
  server acknowledgement and also performs the SFTP close before returning.
- `Drop for SftpWriter` (`io_adapters.rs:160-172`): best-effort `file.shutdown().await`
  if `flush()` was never called (`std::io::copy` never calls `flush`).
- No explicit `fsync@openssh.com` extension request appears anywhere in
  `native/src/sftp/**` (checked `backend.rs`, `io_adapters.rs`, `posix_rename.rs`); the
  "fsync" in the flush comment is presumably internal to
  `russh_sftp::client::fs::File::flush()` — **unresolved**, external crate. The only
  `sync_all()` in the SFTP module is unrelated to transfer (host-key trust store,
  `known_hosts.rs:51`). `BlockingRead`/`BlockingWrite` (`io_adapters.rs:11-74`) are a
  separate, simpler pair used only for the exec-stdio bridge (remote-agent deploy
  protocol), not the file read/write path.

## 4. try_exists / mkdir_all / open_write_new / open_write_copy_stage /
   promote_copy_stage / promote_staged_no_replace / rename_no_replace / copy_file / read_size

`SftpBackend` (`backend.rs`):
- `try_exists` (243-246): one round trip via `SftpConnection::safe_metadata`
  (`connection.rs:71-95`; up to 2 attempts only if the first hits a proven-dead
  transport).
- `mkdir_all` (337-370): walks every path component and calls `create_dir` on **each**
  unconditionally — does **not** stat ancestors first; `Ok` or `SftpError::Status(_)`
  ("already exists") both count as success (355-361); one final `metadata(cur)`
  confirms the leaf (363-369). Cost = N create_dir round trips + 1 metadata round trip.
- `open_write_new` (280-296): one round trip, `OpenFlags::WRITE|CREATE|EXCLUDE` —
  atomicity is the SFTP protocol's own O_EXCL.
- `rename_no_replace` (305-313): plain SFTP v3 `SSH_FXP_RENAME` (same call as
  `rename`), relying on the protocol guarantee that it fails if the destination exists
  (306-308) — one round trip, no client-side existence probe, no hardlink trick.
- `promote_staged` (317-321) **is** implemented: `crate::vfs::promote_staged_with(self,
  staged, destination, |from,to| self.posix_rename(from,to))` (helper out of scope).
  The replace closure calls `self.posix_rename()` (181-190), which opens a **second,
  ephemeral** SSH channel + fresh SFTP subsystem (`posix_rename.rs:26-36`):
  `channel_open_session` (via `open_session_channel`, `backend.rs:99-129`) →
  `request_subsystem("sftp")` → `RawSftpSession::new` → `init()` handshake
  (`posix_rename.rs:39`) → one `extended("posix-rename@openssh.com",…)`
  (`posix_rename.rs:54-58`) → `close_session()`. Several round trips plus a full
  secondary handshake — not a reuse of the persistent main session; the most expensive
  mutating op found. Extension support is read from the handshake's advertised
  extensions (40-46); unsupported servers get a hard `Unsupported` error, no fallback.
- **Not implemented on `SftpBackend`** (inherits the `Backend` trait default, body out
  of scope): `open_write_copy_stage`, `promote_copy_stage`, `promote_staged_no_replace`,
  `copy_file`, `read_size`. No SFTP server-side copy extension appears anywhere in the
  read files; given §2's spool comment, `copy_file` for SFTP most likely resolves to
  `copy_transfer::copy_between` — exact trait wiring **unresolved**.
- `open_write` (263-278, adjacent): one round trip via `sftp().create(path)`,
  non-exclusive create/truncate, no EXCL flag.
- Retry asymmetry: `mutate_sftp` (192-201; used by `rename`, `rename_no_replace`,
  `remove_file`, `remove_dir`) calls `self.connection.current()` **once**, no retry —
  a proven-dead transport fails the call immediately. `mkdir_all`/`posix_rename`
  likewise call `current()` once inline. Only `safe_metadata`-routed calls
  (`list_dir`/`stat`/`try_exists`) and `safe_sftp_on`-routed `open_read` get the
  one-retry-after-reconnect behavior (`connection.rs:71-95`, `backend.rs:159-178`).

## 5. reconnect_gate.rs — serialization during reconnect

- `ReconnectGate<T>{state: Mutex<ReconnectState<T>>, ready: Condvar}` (90-93);
  `ReconnectState{current: Arc<Generation<T>>, reconnecting: bool}` (69-72).
- `acquire()` (106-134): locks `state`; if current is `usable`, returns it immediately
  (lock dropped on return, no I/O held). If not usable and nobody else is
  reconnecting, this caller becomes the reconnector: sets `reconnecting=true`, returns
  a `ReconnectPermit`, and the **lock is released before** the actual network reconnect
  runs (`connection.rs:112-126` calls `connect_transport[_until]` directly, then
  `reconnect.finish_until(...)`). If someone else is already reconnecting, this caller
  parks on the `Condvar` (`wait()`, 168-191), releasing the mutex while waiting,
  bounded by its own deadline if it has one.
- `finish_reconnect()` (136-160) publishes the new generation and `notify_all()`s every
  waiter, which re-checks `usable()` against the fresh generation.
  `abandon_reconnect()` (162-166; also from `ReconnectPermit::drop`, 206-212, if a
  reconnect is dropped without finishing) resets `reconnecting=false` and wakes
  waiters so a failed attempt cannot strand the gate.
- Concurrency effect: callers whose own generation is still usable are **not**
  serialized behind someone else's reconnect. Callers that also see a stale generation
  collapse behind the single in-flight reconnect (one network attempt, not N).
  `lock_with_deadline` (220-239) uses `try_lock` + `park_timeout` (capped at 1 ms) so
  deadline-bound callers don't block indefinitely on the state mutex itself.
- **Deadline asymmetry**: `safe_metadata` sites pass a 20 s `AbsoluteDeadline`
  (`connection.rs:17,75`), so `list_dir`/`stat`/`try_exists` stay time-bounded even
  while waiting on someone else's reconnect. `current()` (used by `open_read`,
  `open_write[_new]`, `mkdir_all`, `mutate_sftp`, `posix_rename`) calls
  `current_with_deadline(None)` (`connection.rs:100-102`) — **no deadline** — so a
  waiter behind a stuck reconnect on these paths blocks on the `Condvar` with no
  timeout of its own (bounded only by the reconnecting thread's internal
  `SFTP_CONNECT_DEADLINE`=30 s, `connection.rs:16,166`, which applies only to the
  thread actually doing the connect).

## 6. Agent-deployed SFTP connection representation (connector.rs)

- `connect_sftp()` (113-190) always builds the plain `Arc<SftpBackend>` first: `be_arc`
  (153), kept as `sftp_handle` for `RemoteState.sftp` (154,179).
- If `form.use_agent` (156-172): calls `crate::agent::deploy_over_sftp(&be_arc, inner)`
  where `inner: BackendHandle = be_arc.clone()` (157-158). On success the agent object
  is wrapped as the generic handle: `(Arc::new(agent), Some(ver))` (159-161) — a
  **different concrete type** than `SftpBackend` becomes the `BackendHandle` used for
  browsing/transfer, from `crate::agent` (out of scope — its struct name, and whether
  it keeps using `inner`/`SftpBackend` as a fallback transport, are **unresolved**). On
  deploy failure: `AgentFallback::Allow` (default, 26-27,43-46) silently falls back to
  plain `be_arc` (163); `AgentFallback::RequireConfined` (mounts with root security
  `Enforced`, 31-41) makes deploy failure a hard connect error instead (164-168) —
  plain SFTP is refused as a fallback there.
- If `form.use_agent` is false: `(be_arc, None)` — plain `SftpBackend` directly
  (170-172).
- **Unresolved**: `connector.rs` never calls `CachingBackend::new`/`for_mount` itself —
  where the returned handle gets wrapped by the cache (§7) is outside this file/scope.

## 7. CachingBackend (vfs/core/cache.rs + helpers)

- `CachingBackend{inner: BackendHandle, cache: Arc<Mutex<CacheState>>, child_key,
  limits}` (68-73). Two `CacheLimits`: `BROWSING` (4096 dirs/50k entries/32 MiB) and
  `MOUNT` (unbounded dirs/entries, 64 MiB) — mount limits "govern retention, never
  directory validity or traversal" (41-45).
- **Forwarded unchanged** (straight `self.inner.X(...)`, no cache interaction):
  `scheme`/`root_display`/`state_identity`/`namespace_identity` (169-180);
  `exists`/`item_id`/`open_read`/`open_read_id` (200-211); `download_name`/`read_size`
  (243-248); `parallelism` (312-314 — passes the inner value through unmodified, e.g.
  SFTP's `1`); `rename_overwrites`/`staged_write_capabilities`/`case_sensitive_paths`/
  `root_confinement`/`mount_path_capabilities` (315-329); `plan_dedupe_recursive`
  (330-336); `is_local`/`provides_content_hash`/`supports_changes`/`change_root_id`/
  `current_change_cursor`/`changes_since` (347-364); `delete_disposition` (379-381);
  `scan_storage` (384-387); `supports_walk_tree`/`walk_tree` (388-397);
  `supports_bulk_tree`/`get_tree` (398-403); `supports_search`/`search` (409-420);
  `supports_walk_hashed`/`walk_hashed` (421-432).
- **Cached (read path)**: `list_dir` → `directory_snapshot(path)` TTL cache (185-187);
  `stat` → `cached_child_meta` (lookup inside the *parent's* cached snapshot) first,
  else `self.inner.stat(path)` (189-195) — a miss does **not** trigger a fresh
  `list_dir`.
- `try_exists` **bypasses the cache entirely** — always `self.inner.try_exists`:
  "Existence gates mutations, so bypass potentially stale listing data" (196-199).
- **Forwarded + invalidating** (all present): `open_write`/`open_write_new` (212-242,
  invalidate before calling inner and again on error; writer wrapped in
  `InvalidatingWriter`); `open_write_copy_stage` (249-256, **yes forwarded**,
  invalidates before+after, also wrapped); `promote_copy_stage` (257-262, **yes
  forwarded**, invalidates `staged`+`destination` exact); `copy_file` (263-267,
  forwarded, invalidates `dst` only); `rename`/`rename_no_replace` (268-279, forwarded,
  invalidate the **prefix/subtree** of both `src`/`dst`); `promote_staged`/
  `promote_staged_no_replace` (280-291, forwarded, invalidate exact `staged`+
  `destination`); `remove_file`/`remove_file_id` (292-301, exact invalidate);
  `remove_dir` (302-306, prefix invalidate); `mkdir_all` (307-311, invalidates
  **ancestors** up to root, not a prefix-down walk); `apply_dedupe_plan`/
  `dedupe_recursive` (337-346, whole-cache `invalidate_cache()`); `put_tree` (404-408,
  prefix invalidate of `root`).
- `invalidate_cache()` (365-378): bumps a `generation` counter and replaces the
  directory/recency/expiry maps, but **keeps** the `loads` weak-table so in-flight
  waiters still resolve to a coherent (now-fenced) result.
- **Locks**: one `Arc<Mutex<CacheState>>` per `CachingBackend` (70), held only for
  short bookkeeping, never across I/O. Concurrent identical `list_dir` calls
  de-duplicate via a **per-directory** single-flight lock instead: `acquire_directory()`
  briefly locks the shared mutex to look up/register a `Weak<DirectoryLoad>` slot
  (`cache_load.rs:83-100`), then locks that slot's own
  `result: Mutex<Option<CompletedLoad>>` (`cache_load.rs:35-36,104`) — **this**
  per-directory mutex is held across the real `self.inner.list_dir(path)` call
  (`cache_load.rs:104-134`, call site 123). N threads listing the *same* directory
  collapse into one network call; N threads listing *different* directories run
  independently — the shared mutex is not a global I/O serialization point.
  `InvalidatingWriter` (`cache_writer.rs`) likewise only takes it briefly, inside
  `invalidate_shared` (`cache_support.rs:89-98`), on `flush()`/`Drop` (37-56).

## 8. LocalBackend + copy_transfer

`LocalBackend` (`vfs/os/shared/local.rs`):
- `open_write_new` (126-133): `OpenOptions::new().write(true).create_new(true)` — one
  syscall, OS-level O_EXCL/CREATE_NEW; no fsync.
- `open_write_copy_stage`, `promote_copy_stage`, `promote_staged_no_replace`,
  `read_size`: **not implemented** — inherit the `Backend` trait default (out of scope;
  `local.rs` references `super::promotion::unique_staging_path`/`promote_staged_replace`,
  a `promotion.rs`-or-similar module, also out of scope).
- `rename_no_replace` (155-157) → `local_platform::rename_no_replace`:
  - **Windows** (`local_platform.rs:98-124`): raw `MoveFileExW` with
    `MOVEFILE_WRITE_THROUGH`, **without** `MOVEFILE_REPLACE_EXISTING` — "Omitting
    MOVEFILE_REPLACE_EXISTING is the Win32 no-replace primitive" (line 111). Paths are
    pre-normalized via `crate::local_access::normalize_scan_root` to a verbatim form
    since raw Win32 calls need it explicit for long paths (103-104).
  - **Linux** (`local_platform.rs:25-59`): raw `renameat2` syscall
    (`libc::syscall(SYS_renameat2, AT_FDCWD, src, AT_FDCWD, dst, RENAME_NOREPLACE)`),
    bypassing the `libc` wrapper ("omits the renameat2 wrapper on musl", 41-43);
    `ENOSYS` propagates rather than falling back to check-then-rename (42-43). Android
    delegates to `crate::android_fs::rename_no_replace` (63-69, out of scope).
  - `rename_overwrites()` returns `true` for plain `rename` (158-160, `std::fs::rename`
    atomically replaces).
- `mkdir_all` → `mkdir_all_plain` (173-175,181-239): unlike SFTP, this **does** stat
  every component: `ensure_plain_component` (208-220) calls `symlink_metadata` first;
  if present, validates it's a plain directory and rejects symlink/junction/reparse
  ancestors (`validate_plain_component`, 222-238 — hardened vs.
  `std::fs::create_dir_all`, which would follow such links, per 178-180); if absent,
  `create_dir`, re-validating on a racing `AlreadyExists`.
- `copy_file` (134-151) is a **custom, non-spooled** implementation: opens source
  reader + destination writer (`open_write_new` on a staged path) and streams directly
  with `std::io::copy` — no temp-file spool, confirming Local can overlap an open
  reader and writer (unlike the providers `copy_transfer.rs` targets, §2). No
  `fsync`/`sync_all` anywhere in this function; `writer.flush()` on a `std::fs::File`
  is a no-op beyond the prior `write()` calls. Sets destination permissions from the
  source, then `super::promotion::promote_staged_replace` (out of scope); on error the
  staged temp file is removed (147-150) — unlike the generic spool path below, which
  does not auto-clean.
- `remove_file` → `local_platform::remove_file_like`: Windows special-cases a
  reparse-point **directory** (uses `remove_dir`, `local_platform.rs:88-96` [win]);
  Linux is plain `std::fs::remove_file` (`local_platform.rs:21-23` [linux]).
- `staged_write_capabilities` returns `StagedWriteCapabilities::complete()`
  (161-163) vs. SFTP's `{create:true, replace:false, namespace_replace:false}`
  (`backend.rs:378-384`).

`copy_transfer.rs` (used by backends without their own `copy_file`, per §4 — **not**
used by `LocalBackend`, which has its own):
- `copy_file(backend,src,dst)` = `copy_between(backend,src,backend,dst)` (5-9).
- `copy_between()` (11-46): `stat` source ("before"); reject dir/symlink; create an OS
  temp file (`tempfile::tempfile()`, line 20); read `read_size()` hint; `open_read_id`
  bounded to `read_size+1` bytes (over-read-by-one to detect growth, 21-23);
  `io::copy` reader→spool, then `drop(reader)` (24-25) **before** any destination
  writer opens; re-`stat` source ("after") and compare size/mtime/id/content_md5/
  dir/symlink — any mismatch is a hard "copy source changed during transfer" error
  (26-31); seek spool to 0; compute a staged destination path; `open_write_copy_stage`
  → `io::copy` spool→writer → `flush()` → `drop(writer)` → `promote_staged_replace`
  (34-40). On a write-phase error the message notes the staged file may remain, but no
  automatic cleanup runs (43-45), unlike `LocalBackend::copy_file`. **Cost
  implication**: any backend routed through this default (SFTP included, §4) pays a
  full local-disk round trip (source→temp file→destination) instead of streaming
  directly, because source and destination cannot overlap open handles there (§2).
