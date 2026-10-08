# Google Drive backend: per-file transfer cost

> Stand-Hinweis 2026-10-08: Abschnitt 1 ist überholt. Metadaten-, Listing- und Upload-Aufrufe
> laufen inzwischen über gepoolte Clients (`gdrive/core/http.rs`), Backoff mit `Retry-After`
> und Zufallsanteil (`overload.rs`). Aktueller Vergleich: `docs/refs/gdrive-opensource-sync-2026-10-08.md`.

## Purpose

Hard facts about `native/src/gdrive`'s Drive v3 backend, gathered to inform a
transfer-throughput redesign (no upfront full scan, several files in flight
concurrently, few API calls per file). Factual inventory only, no design
judgment. Citations are `path:line`, relative to `native/src/gdrive/`.

## Files actually read

`mod.rs`; `core/api.rs`, `core/auth.rs`, `core/backend.rs`, `core/cache.rs`,
`core/changes.rs`, `core/core.rs`, `core/duplicates.rs`, `core/file_list.rs`,
`core/folder_create_journal.rs`, `core/gui_task_http.rs`, `core/metadata.rs`,
`core/names.rs`, `core/promotion.rs`, `core/promotion_api.rs`,
`core/resolution.rs`, `core/resumable.rs`, `core/state.rs`, `core/transfer.rs`,
`core/trash.rs`; `os/shared/copy_writer.rs`.

## 1. HTTP client and concurrency

Client is **ureq** (blocking). No `reqwest`/`tokio`/`async fn`/`.await`
anywhere in scope (grep-verified). Concurrency = whatever OS threads the
caller spawns.

Three pooling patterns: (a) `stream_agent: ureq::Agent` built **once**
(`state.rs:109`, built by `state.rs:316-322`) and stored as a backend field
(`state.rs:76`), reused for every streaming download
(`backend.rs:119-124,143-147`); (b) metadata/mutation calls (`get_json`
`auth.rs:24-34`, folder create `metadata.rs:171`, rename
`promotion_api.rs:107`, trash `trash.rs:74`) use bare `ureq::get/post/
request(...)` each time, no stored `Agent` threaded through; (c) uploads
build a **brand-new** Agent per call: `transfer.rs:241` (per resumable
initiation) and `resumable.rs:258` (per `Session`, i.e. per file
upload/replace) — chunks of one file reuse that Agent, but every new file
starts a fresh pool. `redirects(0)` is deliberate; the `Location` header is
followed manually (`resumable.rs:262-269`).

Token refresh: shared `Arc<Mutex<cloud::Tokens>>` (`state.rs:34`). `bearer()`
(`auth.rs:8-14`) locks it and, if expired, **refreshes over the network while
still holding the lock** — every thread's `bearer()` call blocks until that
completes. Every Drive call goes through `bearer()` first.

Two **global, un-keyed** locks serialize otherwise-unrelated work
process-wide: `create_lock` (`state.rs:50`, all folder creation, §4) and
`mutation_lock` (`state.rs:54`, all rename/promote transitions, §3). By
contrast `upload_paths` (`state.rs:66-68,243-286`) is a per-path
Condvar-guarded lock — only same-path writers block each other. Per-cache
mutexes (`ids`, `untrusted_ids`, `mimes`, `listed`, `pending_upload_ids`,
`pending_folder_creates`, `state.rs:36-63`) hold only briefly.

Net: safe for N threads on one shared instance; per-file spool/PUT is
genuinely parallel via the per-path lock. **Not** parallel for (a) any two
folder creations anywhere, or (b) a copy-staged promote's full network
sequence to a new destination, which runs **while holding `mutation_lock`**
(§3). `parallelism()` (`backend.rs:238-244`) just returns `16` — advisory
metadata, not an enforced semaphore; nothing in scope spawns/caps threads.

## 2. Path -> file-id resolution

`resolve()` (`resolution.rs:8-38`) checks a full-path cache hit, else walks
segment-by-segment from `"root"` via `find_child` (one `files.list` per
uncached segment, `resolution.rs:93-146`, `fields=id,name,modifiedTime`,
`pageSize=1000`), caching each hop. 0 calls warm, 1/uncached segment cold.

IDs loaded from the on-disk cache at startup (`state.rs:88`, via
`cache.rs:158-160`) start **untrusted**; first use each run costs one extra
validation GET (`resolution.rs:56-90`) before being trusted for the rest of
the run.

`list_dir` (`backend.rs:38-92,275-311`) fetches
`fields=...files(id,name,mimeType,size,modifiedTime,createdTime,
md5Checksum)`, `pageSize=1000` — **one listing yields everything a transfer
decision needs** (id, is_dir, size, mtime, md5), no follow-up per-file GET.
Every entry is cached and marked trusted immediately
(`backend.rs:81-86`->`cache.rs:113-121`), so a download right after a listing
costs zero extra metadata calls (§5).

`stat()` (`backend.rs:94-96,315-352`) always does `resolve()` **plus one
unconditional** `files/{id}?fields=id,name,mimeType,size,modifiedTime,
createdTime,md5Checksum` GET (`backend.rs:332-334`) even if just resolved —
`resolve()`'s own walk only fetches `id,name,modifiedTime`.

`try_exists` is not overridden in `backend.rs` (grep-confirmed) — uses the
`vfs::Backend` trait default, outside scope (unresolved).

Disk persistence: `cache.rs` writes the **entire** `ids`+`mimes` maps to
`<app_data_dir>/gdrive/path_cache.json` (`cache.rs:7-8,27-31,45-70`) via
tmp-file + rename, **no explicit fsync before rename**. Called after nearly
every mutating op (list_dir end `backend.rs:90`, every upload
`transfer.rs:229`, every folder create `metadata.rs:244`, every
rename/promotion/trash) — a full-map rewrite each time, not batched.

## 3. Uploads

`open_write` -> `DriveWriter` (`transfer.rs:301-310`) spools to an
**anonymous temp file**, not memory. `write()` (`transfer.rs:342-357`) only
appends + hashes; **network activity starts at `flush()`**
(`commit()`, `transfer.rs:322-338`: fsync spool, final MD5, then
`upload_spooled`). `Drop` is abort-only — discards the spool without
uploading (`transfer.rs:364-368`).

Protocol is **always resumable** (`uploadType=resumable`), never simple or
multipart (grep-confirmed zero `media`/`multipart` uploadType), even for a
0-byte file (`resumable.rs:120-176`). Chunk size `CHUNK_SIZE = 8 MiB`
(`resumable.rs:6`), sent strictly sequentially per file, no intra-file
chunk parallelism (`resumable.rs:58-73`).

`files.generateIds` (1 call) is fetched before every new object, so retries
reconcile by exact ID: per new folder (`metadata.rs:117-118`), per new file
upload with no cached/found id (`transfer.rs:174-187`, unconditional for
internal staging-name paths `transfer.rs:290-298`), and unconditionally per
copy-stage commit (`copy_writer.rs:90-92`).

**Ordinary path, new file ≤8 MiB, warm parent** (`transfer.rs:16-123`):
`find_child` probe (1) + `generateIds` (1) + `initiate` (1) + PUT chunk (1).
Happy-path PUT (200/201 with matching `id`) needs no verify call
(`resumable.rs:191-192,211-225`); an ambiguous completion adds one
`verify_uploaded_id` GET (`transfer.rs:206-220`). = **4 calls happy path**,
5 if ambiguous, +1 per extra 8 MiB.

**Copy-stage path** (`copy_writer.rs`, `promotion.rs`) costs more:
- Stage commit (`copy_writer.rs:64-134`): `named_objects` probe (1) +
  `generateIds` (1) + `initiate` (1) + PUT chunk(s) + `verify_stage`
  (`copy_writer.rs:170-207`, GET by id + `named_objects`) = **5 + chunks**.
- Promote to an *absent* destination (`promotion.rs:119-153`, normal
  new-file case): `named_objects`(staged) + `named_objects`(dest), both
  **while holding the global `mutation_lock`** (`promotion.rs:120`), then
  (still under that lock — `return` evaluates its expression before locals
  drop, `promotion.rs:148`) `rename_absent_locked` (`promotion.rs:160-221`):
  `rename_id` PATCH + `named_objects` verify = **4 calls**, all serialized
  process-wide.
- Promote onto an *existing* destination (`promotion.rs:223-313`) is
  costliest: `spool_staged` (`promotion.rs:315-337`) **re-downloads the
  whole staged file** (1 streaming GET) to re-verify size/MD5, then
  `replace_spooled_id` **re-uploads those same bytes again** via a second
  full resumable PUT sequence (`transfer.rs:129-172`) — the file's bytes
  cross the wire **three times** (stage upload, stage download, dest.
  upload), plus 2 `named_objects` re-verifies and a `trash_id` PATCH
  (+1 if ambiguous) for the staging copy.

Totals: ~9+chunks calls for new-destination copy-stage; the replace case
adds one full extra download and one full extra upload of the bytes.

## 4. mkdir_all

`mkdir_all` -> `ensure_dir` (`backend.rs:234-236`), recursive per segment
(`metadata.rs:78-125`, parent resolved first). Per uncached level: optional
`find_child` probe (**skipped** if parent already in `listed`,
`metadata.rs:100-107`) + `generateIds` (1) + create POST (1,
`metadata.rs:169-175`) + optional verify GET only on ambiguous completion
(`metadata.rs:214-263`). Best case **2 calls**, worst realistic **4**. A
freshly created folder is marked `listed` immediately (`metadata.rs:241`,
no children yet), so siblings under it skip their own folder-existence
probe.

Duplicate-folder race prevention, two layers:
- **In-process**: `create_lock` (`state.rs:50`) is one un-keyed `Mutex<()>`
  held for the whole check-then-create section (`metadata.rs:92-124`) —
  serializes **all** folder creation process-wide, not just same-path.
- **Cross-process/restart**: `folder_create_journal.rs` persists a
  reservation under `<app_data_dir>/gdrive/pending-folder-creates/`
  (`folder_create_journal.rs:28-32`), keyed by SHA-256 of
  `(account_key, path)` (`folder_create_journal.rs:106-118`). `claim()`
  (`folder_create_journal.rs:213-246`) uses `std::fs::create_dir` — atomic
  OS primitive — as the real mutual exclusion, so two separate processes
  can't both create the same folder. The reservation records the exact
  pre-generated ID; resume (`metadata.rs:127-151`) re-verifies that ID by
  GET before ever retrying create; the journal entry clears
  (`folder_create_journal.rs:267-293`) only after the local cache durably
  records the mapping (`metadata.rs:239-245`). At most one Drive folder per
  reservation, at the cost of extra fsynced local FS I/O per new folder.

## 5. Downloads

`open_read`/`open_read_id` (`backend.rs:102-150`) pick `files/{id}?
alt=media` or `files/{id}/export?mimeType=...` via `export_format(mime)`
(`api.rs:39-54`: Docs->docx, Sheets->xlsx, Slides->pptx, Drawing->png, other
Google-native->pdf). Wrapped in `open_stream`'s request-level retry
(`api.rs:106-137`); body returned as a **streaming `Read`**
(`resp.into_reader()`, `backend.rs:125,149`) — no in-backend buffering,
unlike uploads.

Cost: 1 call if mime already cached+trusted (true right after listing the
parent, §2); +1 `files/{id}?fields=mimeType` GET (`metadata.rs:30-34`) if
not, plus `resolve()` cost if the id is also uncached.

`read_size()` (`backend.rs:169-177`) is `None` for exportable Google-native
files, `Some(metadata_size)` (caller-supplied, not re-fetched) otherwise.

**No Range/partial-download support.** Grep finds exactly one `"Range"` in
scope, a *response*-header read in the upload path
(`resumable.rs:233`) — not a download request header. Downloads always
start at byte 0. No post-download MD5 re-verification exists either
(contrast uploads, §3).

## 6. Rate limiting

`is_rate_limited` (`api.rs:97-100`): true for `429/500/502/503/504`, or
`403` whose body substring-matches `"ateLimitExceeded"` or `"uotaExceeded"`.

**Reads/metadata** (`api.rs:106-137,178-213`): up to `RETRY_ATTEMPTS = 6`,
exponential backoff from `400ms`, doubling, capped `16s`
(`api.rs:9-11,139-142`). **No jitter** anywhere (grep-confirmed) — concurrent
threads hitting the same limit back off on identical schedules.

**Mutations** (`api.rs:147-173`) are sent **exactly once** at the HTTP
layer — never blindly resent. A `5xx` or transport error is `Ambiguous`;
callers do **one** exact-ID verify GET, never a resend. A `429`/`403
rateLimitExceeded` mutation therefore is **not** backed off/retried at the
HTTP layer either — same ambiguous-then-verify handling as `5xx`.

**Resumable PUT chunks** (`resumable.rs:20-117,344-393`) have a third,
independent retry loop: `MAX_RETRIES = 6`, delay `min(250ms<<n, 8s)`, also
no jitter, plus `MAX_NO_PROGRESS = 6` stalled-offset guard. On transient
failure it status-checks first to learn the committed offset before
resuming, rather than blind resend.

**No global/shared throttle found** anywhere in scope (grep-confirmed no
semaphore/token-bucket/rate-limiter). Concurrency bound = however many
threads the caller spawns, each with independent unsynchronized backoff.

## 7. Server-side copy

**Not implemented.** Grep for `files.copy`/`/copy` finds nothing;
`backend.rs`'s `impl Backend` has no `copy_file`. The "copy" vocabulary that
exists — `open_write_copy_stage`, `promote_copy_stage`,
`os/shared/copy_writer.rs` — is **client-side staging**: it re-uploads
fresh bytes from a local spool (`copy_writer.rs:19-35,64-134`) to a new
Drive object, then renames it into place (§3). It never calls Drive's
server-side `files.copy`. Every "copy" here costs a full client re-upload
(and, for replace-existing, a full extra download too, §3).

## 8. Other high-cost factors

- **Triple bytes-over-the-wire** on copy-stage replace (§3): stage upload +
  stage download + destination re-upload for one logical replacement.
- **Global `mutation_lock` held across a full network round trip** for
  new-destination promotes (`promotion.rs:120-148`) — serializes the commit
  step of every concurrent copy-staged transfer regardless of thread count.
- **Global `create_lock` held across a full network round trip** for every
  folder create anywhere (§4) — same shape of bottleneck.
- **Full path-cache JSON rewrite to disk** on nearly every op (§2),
  unbatched, scales with cache size, adds mutex contention under
  concurrency.
- **`load_drive_account_key`** (`state.rs:292-313`): one extra `/about?
  fields=user(permissionId)` call per backend construction (not per file),
  keys the durable folder journal.
- **Change-feed** (`changes.rs`, `start_page_token`/`drive_changes_since`,
  paged 1000/request) is separate from any single-file transfer; only used
  for whole-tree incremental-sync detection (`backend.rs:259-269`).
- **Cache invalidation** (`forget_path_prefix`, on ambiguous
  trash/rename/promotion or stale validation) is local, not a Drive call,
  but forces the next access back through full resolve cost (§2).
- Timeouts: connect 10s, ordinary request 60s (`api.rs:7-8`); resumable PUT
  chunk 10 min (`resumable.rs:9`) — a stalled 8 MiB chunk can occupy its
  thread up to 10 minutes before failing.

## API calls per operation (happy path)

| Operation | Calls | Notes |
|---|---|---|
| Upload new file ≤8 MiB, `open_write`, warm parent | **4** | probe+generateIds+initiate+PUT; +1/extra 8 MiB; +1 if ambiguous |
| Upload new file, copy-stage + promote (new dest.) | **~9**+chunks | stage 5+chunks, promote 4 under global lock |
| Copy-stage + promote, replacing existing dest. | **~10**+chunks, **+1 full extra download +1 full extra upload** | `spool_staged`+`replace_spooled_id` |
| Download, mime cached (just listed) | **1** | the streaming GET |
| Download, mime/id cold | **2-3** | +mimeType GET, +resolve() cost |
| Create folder level, parent known-absent | **2** | generateIds+create POST |
| Create folder level, existence unknown | **3-4** | +probe, +1 if ambiguous-verify |
| `stat()` on a resolved path | resolve +**1** | unconditional full-fields GET |

## Unresolved

- **Does bare `ureq::get/post/request(...)` pool connections via an
  implicit default agent, or open fresh sockets each time?** Depends on the
  `ureq` crate version/internals (`Cargo.toml`/`Cargo.lock`, crate source),
  outside the allowed file set. Certain from code alone: these sites don't
  thread through a stored `Agent`, unlike `stream_agent` and `Session.agent`.
- **`vfs::Backend`'s default `try_exists`.** Not overridden here; trait
  default lives outside the allowed file set.
- **Whether the sync engine actually calls `open_write` vs.
  `open_write_copy_stage`** for ordinary new-file transfers — that decision
  is caller code outside `gdrive/`; this document covers both paths' cost.
- **`cloud::refresh_access`'s own HTTP behavior** (timeout/retry/endpoint) —
  defined in `native/src/cloud.rs`, outside scope; only the lock around it
  (`auth.rs:8-22`) is in scope.
- **What spawns/caps worker threads** calling this backend — `parallelism()`
  (`backend.rs:238-244`) is read-only advisory metadata; no thread pool
  exists in the allowed file set.
