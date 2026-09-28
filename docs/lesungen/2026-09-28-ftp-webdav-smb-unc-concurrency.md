# FTP/FTPS, WebDAV, SMB, UNC — Concurrency Model and Per-Operation Cost

## Purpose

Input for a transfer redesign targeting maximum total throughput: no upfront full scan, several
files in flight concurrently per transfer (worker threads sharing one `Arc<dyn Backend>`), few
round trips per file. This reading complements the API/integration surveys in
`docs/lesungen/2026-09-25-android-core-ops-api-and-portability.md` and
`docs/lesungen/2026-09-26-smb-backend-integration.md` (signatures/integration, not repeated here)
and focuses only on: connection/lock model, whether an open reader/writer blocks sibling
operations, streaming vs. buffering, round-trip counts per `Backend` method, retry/timeout
handling, and documented concurrency caps. Facts only, with `file:line` citations; no design
judgment.

**Scope note (why some items say "unresolved"):** the assignment authorized only the four
backends' `core/` non-test files plus their `mod.rs`, and `net/core/backend.rs` +`net/mod.rs`
(the latter only to locate `backend.rs`). The `Backend` trait itself, its default method bodies,
helper functions (`promote_staged_with`, `promote_staged_no_replace_with`,
`promote_staged_replace`, `unique_staging_path`, `StagedWriteCapabilities`), `LocalBackend`, and
`net/core/net.rs` (`NetConnection`) all live outside this surface, so any method a backend does
not override is marked unresolved with that reason rather than guessed at.

## Files actually read

- `native/src/ftp/core/connection.rs`, `ftp.rs`, `io_adapters.rs`, `resolver.rs`, `staging.rs`, `writer.rs`, `native/src/ftp/mod.rs`
- `native/src/webdav/core/multistatus.rs`, `webdav.rs`, `writer.rs`, `native/src/webdav/mod.rs`
- `native/src/smb/core/backend.rs`, `errors.rs`, `io.rs`, `listing.rs`, `replace.rs`, `session.rs`, `url.rs`, `wire.rs`, `native/src/smb/mod.rs`
- `native/src/net/core/backend.rs`, `native/src/net/mod.rs` (module wiring only)

---

## FTP / FTPS (`suppaftp`, blocking, rustls/ring)

**1. Connection model.** One control connection (`RustlsFtpStream`) owned by `FtpConnection`,
guarded by `Mutex<ControlState>` + `Condvar` — no pool, no HTTP client, no async runtime for the
control path (`native/src/ftp/core/io_adapters.rs:14-19`). Module doc states the design directly:
"single control connection is serialized behind a `Mutex` (`parallelism() == 1`)"
(`native/src/ftp/core/ftp.rs:6-7`), and `parallelism()` returns `1`
(`native/src/ftp/core/ftp.rs:246-248`). Every call path (`with_stream_mutation`,
`with_stream_read`, `open_reader`) first calls `wait_for_stream()`, which blocks on the `Condvar`
until the stream is available (`io_adapters.rs:76-88, 105-124, 126-167, 169-218`) — **N threads
sharing one `FtpBackend` cannot run stat/list/open_read/open_write in parallel; they fully
serialize.** DNS resolution (used only at connect/reconnect, not per-op) runs on one dedicated
background thread with its own single-threaded Tokio runtime and a bounded 16-request queue
(`native/src/ftp/core/resolver.rs:8, 30, 36-64`); a full queue fails fast with `WouldBlock`
(`resolver.rs:162-166`) instead of blocking indefinitely.

**2. Reader/writer holding the connection.** Yes for reads: `open_reader` removes the stream from
shared state (`state.stream.take()`, `io_adapters.rs:210`) and hands it to `FtpReader`
(`control: Some(control)`); it is not returned via `return_stream` until `close()` runs on EOF,
error, or `Drop` (`io_adapters.rs:343-389`). **While a `FtpReader` is alive and not fully
drained/closed, every other operation on the same `FtpConnection` (from any thread) blocks on the
`Condvar`** — concrete deadlock risk if one thread holds an open reader while another thread tries
to write/rename/list/stat on the same backend. Writes do not check the connection out early:
`FtpWriter` buffers locally and only calls `with_stream_mutation` once, synchronously, inside
`flush()` (`native/src/ftp/core/writer.rs:59-61`, `io_adapters.rs:105-124`). **Yes — a data
transfer occupies the single control connection** for both directions: for RETR, for the reader's
entire open lifetime; for STOR, for the duration of the synchronous `put_file` call at `flush()`.

**3. Reader/writer implementation.** `FtpReader::read` streams directly from `retr_as_stream`,
forwarding each caller buffer to the underlying data socket with no internal buffering
(`io_adapters.rs:364-380`); `Ok(0)` triggers `finalize_retr_stream` (`close(true)`,
`io_adapters.rs:350-361`). `FtpWriter` spools the **entire file to an anonymous temp file** via
plain `Write` calls (`tempfile::tempfile()`, `writer.rs:44,84`); upload happens only on `flush()`
via one streaming `STOR` (`put_file`) after seeking to start (`writer.rs:49-73`). Drop without a
successful flush never uploads, only deletes the temp file (doc `writer.rs:23-25`, test
`writer.rs:132-140`). A failed upload sets `FailedAmbiguous` and is never auto-replayed
(`writer.rs:67-71`).

**4. Round trips per operation.**
- `try_exists`: not overridden — trait default, **unresolved** (vfs, out of scope).
- `stat` (`ftp.rs:154-171`): no native STAT; lists the **parent** directory and searches for the
  basename = **1 LIST round trip** (except root, answered locally with 0 round trips).
- `list_dir` (`ftp.rs:144-152`): **1 LIST round trip**; `parse_list_line` via `suppaftp::list::File`
  returns full metadata (`is_dir`, `is_symlink`, `size`, `mtime`) per entry, no per-entry follow-up
  (`ftp.rs:69-91`).
- `mkdir_all` (`ftp.rs:218-244`): **1 MKD per path ancestor** (N segments = N round trips), plus up
  to 2 extra (`CWD` verify + `CWD` restore) per ancestor that already exists (lines 233-239).
- `open_write_new`: **not implemented for FTP** — no override exists; unresolved trait default.
- `open_write_copy_stage` (`ftp.rs:190-193`): `staging::require_absent` (1 existence-check round
  trip, itself unresolved cost — routes through `try_exists`) + `open_write` (0 round trips until
  flush).
- `promote_copy_stage`: no override; generic vfs-level flow, **unresolved** (out of scope).
- `promote_staged_no_replace` (`ftp.rs:197-202`): 1 absence-check + 1 `rename` (RNFR/RNTO as one
  `with_stream_mutation` call, `ftp.rs:184-187`).
- `rename_no_replace`: **unsupported** by design — doc: "stays unsupported (trait contract)"
  (`ftp.rs:195-196`); FTP has no atomic no-replace rename (`staging.rs:1-7`).
- `copy_file`: not overridden — **unresolved** trait default (FTP has no server-side copy command).
- `read_size`, `case_sensitive_paths`: not overridden — **unresolved** (trait defaults, out of scope).

**5. Retry/backoff/timeouts.** Setup timeouts: 10s total setup, 3s/address connect attempt, 10s
data-connect, 60s steady-state I/O (`connection.rs:130-133`), capped at 8 resolved addresses
(`connection.rs:134, 226-241`). Reads get exactly **one** reconnect-and-retry on first failure
(`with_stream_read`, `io_adapters.rs:126-167`; `open_reader`, `io_adapters.rs:172-207`) — no
backoff loop. Mutations **never** auto-retry (marks `Suspect`, returns the error immediately,
`io_adapters.rs:105-124`) — verified by test `ambiguous_mutation_marks_channel_suspect...`
(`ftp.rs:454-497`). Background keepalive pings every 15s with a 10s I/O timeout when idle, and
proactively reconnects if `Suspect` (`io_adapters.rs:40-41, 256-290`). No FTP-specific
rate-limit (e.g., 421/530) handling found.

**6. Concurrency caps / server limits in comments.** `parallelism() == 1` is the hard, explicit
cap; the module doc frames it as inherent to having one control channel
(`ftp.rs:1-11, 246-248`). No other server-limit comments found.

---

## WebDAV (`ureq`, blocking, PROPFIND/GET/PUT/DELETE/MKCOL/MOVE/COPY)

**1. Connection model.** No control connection/mutex — **two `ureq::Agent` instances**:
`agent` for reads (PROPFIND/GET), pooled, default ureq keep-alive/pool
(`webdav.rs:74-78`); `mutation_agent` for writes, explicitly **unpooled**
(`.redirects(0).max_idle_connections(0)`, `webdav.rs:79-85`) — comment: "so ureq cannot replay a
DELETE from a stale recycled connection after an ambiguous response loss" (`webdav.rs:53-55`), so
every mutation opens a fresh connection. No async runtime; `ureq` is synchronous. `parallelism()`
returns `2`, comment "HTTP keep-alive; a couple of concurrent requests are fine"
(`webdav.rs:362-364`). Because there is no shared connection object, **N threads issuing
reads/mutations concurrently genuinely run in parallel** (each request gets its own or a pooled
socket); the `2` is a chosen default, not an enforced limit in this file.

**2. Reader/writer holding a lock.** No — there is no shared mutex to hold. `open_read` performs
an independent `GET` and returns a streaming body reader bound to its own response/socket
(`webdav.rs:148-158, 233-240`); comment notes retry is only safe "until a response body is handed
to the caller" (`webdav.rs:234-237`). Reading and writing concurrently through the same
`WebdavBackend` cannot deadlock at this layer — no FTP-style single-channel checkout exists.

**3. Reader/writer implementation.** Reader: streaming (`resp.into_reader()`,
`webdav.rs:239`), no full buffering. Writer (`WebdavWriter`, `writer.rs:72-163`): spools the
**entire file to an anonymous temp file** first (`tempfile::tempfile()`, `writer.rs:112`); upload
happens only on `flush()`/`commit()` (`writer.rs:128-157`) as one PUT streamed from the spooled
file with `Content-Length` set from the spooled size (`writer.rs:139`). Exclusive create uses
`If-None-Match: *` and requires HTTP 201 specifically — "202 is not an acknowledgement of
completion" (`writer.rs:39-43, 53-56`). Drop without flush never PUTs (`writer.rs:70-71,
230-238`).

**4. Round trips per operation.**
- `try_exists`: not overridden — **unresolved** trait default.
- `stat` (`webdav.rs:194-231`): **1 `PROPFIND Depth:0` round trip**.
- `list_dir` (`webdav.rs:189-192`): **1 `PROPFIND Depth:1` round trip**; `parse_multistatus`
  returns full metadata per child — `is_dir` (`<collection/>`), `size` (`getcontentlength`),
  `mtime_ms` (`getlastmodified`), and optionally `content_md5` from Nextcloud/ownCloud's
  `oc:checksums` extension, "a free content hash, no download"
  (`multistatus.rs:191-217`, `webdav.rs:124-126`).
- `mkdir_all` (`webdav.rs:322-360`): **1 `MKCOL` per ancestor**; on `405` (exists) **+1 `stat`**
  round trip to confirm it's a directory (`webdav.rs:335-354`).
- `open_write_new` (`webdav.rs:250-256`): 0 round trips to open; exclusivity enforced by the
  single PUT's `If-None-Match` at flush time (no separate existence probe).
- `open_write_copy_stage`: not overridden — **unresolved** trait default.
- `promote_copy_stage`: not overridden — **unresolved** (vfs-level generic); WebDAV reports
  `staged_write_capabilities = { create: true, replace: false, namespace_replace: false }`
  (`webdav.rs:372-378`).
- `promote_staged` (`webdav.rs:304-308`): **1 `MOVE Overwrite:T` round trip**; RFC 4918 comment:
  the server deletes-then-moves in one request, "not an old-or-new atomic guarantee"
  (`webdav.rs:300-303`), hence `rename_overwrites() == false` (`webdav.rs:366-370`).
- `promote_staged_no_replace`: not overridden — wraps `rename_no_replace` via a vfs-level default,
  **unresolved** exact wrapper cost beyond the 1 `MOVE` it calls.
- `rename_no_replace` (`webdav.rs:289-298`): **1 `MOVE Overwrite:F` round trip**.
- `copy_file` (`webdav.rs:258-276`): **server-side `COPY`** is used (not client read+write) —
  1 `COPY Overwrite:F` + 1 `stat` (for size) + 1 `promote_staged_replace` (≥1 `MOVE`) = **≥3 round
  trips**, plus 1 more `remove_file` only on the failure path (`webdav.rs:272-274`).
- `read_size`, `case_sensitive_paths`: not overridden — **unresolved**.
- Note: `remove_dir` delegates straight to `remove_file`'s single `DELETE`
  (`webdav.rs:318-320`) — WebDAV `DELETE` on a collection is a single request regardless of
  directory size (server-side recursive per RFC 4918; not re-verified here since it's outside the
  read scope of the RFC text, only the code's 1:1 delegation is a fact).

**5. Retry/backoff/timeouts/rate-limits.** `CONNECT_TIMEOUT = 10s`, `IO_INACTIVITY_TIMEOUT = 60s`
on both agents (`webdav.rs:35-36`). `propfind()` and `get()` each retry **once**, and only on
`ureq::Error::Transport` (network-level) or a first-attempt body-read failure
(`webdav.rs:128-146, 148-158`) — no delay/backoff. Mutations (`mutation()`, `webdav.rs:160-169`;
PUT in `writer.rs`) have **no retry at all** — single attempt, errors propagate immediately.
**No HTTP 429/503-specific handling anywhere in these files** — `request_err()` only special-cases
404→`NotFound` and 412→`AlreadyExists` (`webdav.rs:26-33`); everything else, including 429/503,
becomes a generic `io::ErrorKind::Other` with no retry-after / backoff logic. This is a fact worth
flagging for a many-concurrent-requests redesign, not a judgment.

**6. Concurrency caps / server limits in comments.** `parallelism() == 2`
(`webdav.rs:362-364`) is a code-chosen default ("a couple of concurrent requests are fine"), not
a server-reported limit. No other server-limit comments found.

---

## SMB 2/3 (`smb2` crate, async, private multi-threaded Tokio runtime)

**1. Connection model.** `SmbSession` holds `current: Mutex<Option<Arc<Generation>>>`
(one `Generation` = one authenticated TCP+NEGOTIATE+SESSION_SETUP connection,
`session.rs:33-38, 61-66`) plus a shared **2-worker-thread** Tokio runtime
(`worker_threads(2)`, `session.rs:77-83`). Each `Generation` lazily tree-connects per share into
`trees: Mutex<HashMap<String, Arc<Tree>>>` (`session.rs:34, 116-136`).
`Generation::connection()` returns a cheap `Connection` clone — comment: "smb2 multiplexes
clones" (`session.rs:42-44`), i.e. the underlying connection supports SMB2 protocol-level request
multiplexing, unlike FTP's single-channel serialization. Every `Backend` call does
`self.rt.block_on(operation(...))` on the caller's thread, executing on the shared runtime
(`session.rs:156-204`). `parallelism()` returns `2`, comment: "One session multiplexes requests;
two keep a walk moving without starving the credit window of a small NAS"
(`backend.rs:238-242`) — a deliberate cap tied to SMB2 credit-based flow control on constrained
servers, not a hard protocol maximum. **N threads can issue concurrent requests over the one
connection** (smb2 multiplexing), but all async work funnels through only 2 Tokio workers.

**2. Reader/writer holding a lock.** No. `SmbReader`/`SmbWriter` hold their own `FileReader`/
`FileWriter` handle plus cloned `Arc<Generation>`/`Arc<Tree>`/`Arc<Runtime>`
(`io.rs:23-31, 100-110`), not the session's `current`/`trees` mutexes — those are held only
briefly to fetch/clone handles (`session.rs:104-113, 116-136`), not for the I/O duration.
**Reading and writing concurrently on the same `SmbBackend` does not deadlock** at this layer; the
only soft bottleneck is the fixed 2-worker runtime serializing/queuing many simultaneous
`block_on` calls from more than 2 OS threads.

**3. Reader/writer implementation.** Both are genuinely streaming with **1 MiB chunking**, not
full-file spooling. `SmbReader`: fetches up to `READ_CHUNK = 1<<20` bytes per `read_at` call,
serves caller reads from that buffer, refetches on exhaustion (`io.rs:18, 52-80`); comment: "smb2
splits it at MaxReadSize" (server-negotiated, possibly smaller). `SmbWriter`: accumulates writes
into `pending: Vec<u8>` and flushes to the wire via `write_chunk` once `pending.len() >=
WRITE_CHUNK (1<<20)` (`io.rs:19-21, 176-184`) — so upload happens continuously as data streams in,
not deferred entirely to `flush()`. `flush()` sends any remaining `pending` bytes, then calls
`writer.finish()` (FLUSH+CLOSE) as the actual commit boundary (`io.rs:186-210`); a dropped writer
without successful flush aborts and removes the partial file (`io.rs:163-173, 213-219`).

**4. Round trips per operation.**
- `try_exists`: not overridden — **unresolved** trait default.
- `stat` (`backend.rs:132-147` → `wire.rs:159-188`): **1 round trip** — CREATE +
  QUERY_INFO(FileAttributeTagInformation) + CLOSE sent as **one compound request**
  (`execute_compound`, `wire.rs:173-178`); returns size, mtime, creation time, is_dir, reparse tag.
- `list_dir` (`backend.rs:120-130` → `wire.rs:200-259`): CREATE (1) + QUERY_DIRECTORY loop
  (1+ requests until `NO_MORE_FILES`/`NO_SUCH_FILE`, buffer = min(server `max_transact_size`,
  65536), `wire.rs:35, 222-258`) + CLOSE (1). Each `FileBothDirectoryInformation` entry carries
  full metadata already (`listing.rs:144-191`) — no per-entry follow-up.
- `mkdir_all` (`backend.rs:201-236`): **1 `create_directory` per ancestor** (N round trips,
  tolerating `AlreadyExists`) **+ 1 final `wire::stat`** to confirm the leaf is a directory
  (`backend.rs:213-232`) = N+1, all inside one non-retried `session.write`.
- `open_write_new` (`backend.rs:179-181, 58-82`): **1 round trip** — single CREATE with
  `FileCreate` disposition (`create_file_writer_exclusive`); an existing name fails `AlreadyExists`
  directly, no separate probe.
- `open_write_copy_stage`, `promote_copy_stage`: not overridden — **unresolved** trait defaults;
  `staged_write_capabilities() == StagedWriteCapabilities::complete()` (`backend.rs:251-253`).
- `promote_staged`/`promote_staged_no_replace`: **no named overrides at all** — SMB relies on
  `rename`/`rename_no_replace` (which are overridden) via a vfs-level default; exact wrapper
  mechanics **unresolved** (out of scope), but the underlying rename cost is known (next line).
- `rename` / `rename_no_replace` (`backend.rs:184-191` → `replace.rs:37-60` →
  `wire.rs:264-292`): **1 round trip each** — CREATE(open source, DELETE access) +
  SET_INFO(FileRenameInformation, `ReplaceIfExists` = 1 or 0) + CLOSE as **one compound request**;
  server replaces atomically when `ReplaceIfExists=1` — "the new file becomes visible without the
  name ever missing" (`replace.rs:1-6`).
- `copy_file`: not overridden — no server-side SMB2 copy-offload used; **unresolved** trait
  default (client read+write, presumably, per vfs generic — not confirmed in scope).
- `read_size`, `case_sensitive_paths`: not overridden — **unresolved**.

**5. Retry/backoff/timeouts/rate-limits.** `CONNECT_TIMEOUT = 15s` for the whole TCP+NEGOTIATE+
SESSION_SETUP budget (`session.rs:20, 211`). Reads (`SmbSession::read`, `session.rs:156-182`):
**exactly one** retry, and only on a proven-dead connection (`generation.note()` → `is_dead`,
`errors.rs:163-171`); a merely "suspect" (timeout) classification does not retry — comment: "a
timeout does not prove the request never arrived... nothing is replayed"
(`session.rs:154-155, 175-177`). Writes (`SmbSession::write`, `session.rs:186-204`): **never**
retried — "it is never replayed (it may have taken effect before the connection was lost)"
(`session.rs:184-185`). `open_read` has its own one-shot retry-on-reopen before any byte is read
(`backend.rs:149-171`, flag-bounded at line 167). `target()` retries tree-connect once after a
proven connection loss (`session.rs:141-152`). No backoff/delay anywhere — all retries are
single-shot immediate. No explicit SMB2 credit-window tracking/backpressure code in these files
(smb2 crate internals are a dependency, out of scope); `QUERY_BUFFER_LEN = 65_536` is documented
as consuming "One credit per QUERY_DIRECTORY (MS-SMB2 3.2.4.1.5)" (`wire.rs:34-35`).

**6. Concurrency caps / server limits in comments.** `parallelism() == 2` with explicit rationale
tying it to SMB2 credit flow control on "a small NAS" (`backend.rs:238-242`); the private runtime
is also fixed at `worker_threads(2)` (`session.rs:77-83`), reinforcing the same "2" budget at the
executor level.

---

## UNC (`net::UncBackend`, wraps `LocalBackend` + a retained WNet lease)

**1. Connection model.** `UncBackend` is a **thin wrapper**: `local: LocalBackend` (all actual
I/O) plus `connection: NetConnection`, a cloneable opaque lease handle
(`net/core/backend.rs:10-13`). Doc: "Windows' SMB redirector owns wire keepalive/reconnect;
retaining the lease prevents Smart Explorer itself from cancelling the session while in use"
(`backend.rs:7-9`). **`LocalBackend`'s connection model, threading, and `NetConnection`'s
reconnect mechanics are unresolved** — both live outside the authorized reading surface
(`LocalBackend` in `vfs/`, `NetConnection` in `net/core/net.rs`; only `net/core/backend.rs` and
`net/mod.rs` — the latter for module-location only — were in scope). No `parallelism()` override
exists in this file at all, so the effective value is whatever `LocalBackend::parallelism()`
returns — **unresolved**.

**2. Reader/writer holding a lock.** `UncReader`/`UncWriter` each hold only their `inner` handle
plus their own clone of `NetConnection` as a keepalive guard (`backend.rs:15-39`) — no shared
mutex is visible in this file. Whether the underlying `LocalBackend` handle can block a sibling
operation is **unresolved** (out of scope).

**3. Reader/writer implementation.** Pure pass-through: `read`/`write`/`flush` forward directly to
`self.inner` with no buffering/chunking added at this layer (`backend.rs:20-24, 31-39`). Any
chunking/streaming behavior lives entirely in `LocalBackend` — **unresolved** (out of scope).

**4. Round trips per operation.** Every method present (`try_exists`, `stat`, `list_dir`,
`mkdir_all`, `open_write_new`, `promote_staged_no_replace`, `rename_no_replace`, `copy_file`) is a
**one-line delegation** to the identically named `LocalBackend` method
(`backend.rs:66-129`, e.g. `list_dir` at 66-68, `stat` at 70-72, `try_exists` at 74-76,
`mkdir_all` at 127-129) — the wrapper itself adds **zero** round trips; actual cost is 100%
determined by `LocalBackend`, **unresolved** (out of scope). `open_write_copy_stage` and
`promote_copy_stage` have **no override at all** in this file — fully unresolved trait defaults.
`read_size` and `case_sensitive_paths` are likewise not overridden here — unresolved.

**5/6. Retry/backoff/timeouts/concurrency caps.** Nothing is visible in
`net/core/backend.rs` itself — no timeout constants, no retry loops, no `parallelism()` override.
**Entirely unresolved**; deferred to `LocalBackend` and the OS SMB redirector via
`NetConnection`/`net/core/net.rs`, both outside this task's authorized surface.

---

## Unresolved items (summary)

All items below are unresolved because the deciding code lives outside the files this task
authorized:
- `Backend` trait defaults (`try_exists`, `open_write_copy_stage`, `promote_copy_stage`,
  `read_size`, `case_sensitive_paths`, and the generic `promote_staged`/`promote_staged_no_replace`
  wrappers used by SMB and partially by WebDAV) — defined in `vfs/mod.rs`, not in scope.
- `copy_file` for FTP and SMB (no override; whatever the vfs generic default does) — same reason.
- `LocalBackend`'s entire implementation (connection model, buffering, round trips, retries,
  `parallelism()`) underlying `UncBackend` — lives in `vfs/`, not in scope.
- `NetConnection` (`net/core/net.rs`) — lease/reconnect mechanics — explicitly out of scope
  (`net/mod.rs` was authorized only to locate `backend.rs`).
- Exact internal round-trip/pipelining behavior inside the `suppaftp`, `ureq`, and `smb2` crates
  themselves (e.g., ureq's default pool size for the WebDAV read `agent`, or smb2's own credit
  accounting) — third-party crate internals, not repository source in scope.
