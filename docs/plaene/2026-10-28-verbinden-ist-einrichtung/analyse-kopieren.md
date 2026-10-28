# Analyse: Kopieren vieler kleiner Dateien

Stand: 2026-10-28, Quellstand `a67d7296`. Lesende Codeanalyse (nichts gebaut oder ausgeführt); Zahlen zu fsync/Dateien pro Sekunde sind Schätzungen aus dem Code, bis die Messung der Suite vorliegt. Basismessung: `se cp -r` lokal→lokal 5000 × 4 KiB in 0,85 s auf einem Linux-Server (4 Kerne).

# Small-file copy/paste performance in Smart Explorer (`se`): where the time goes and what to change

Your baseline measured the one path that skips the slowest per-file work. `se cp` builds its job with `recoverable: false, id: None` (native/src/cli/engine_copy.rs:306-309). That turns off the per-file crash-safety log ("receipts"). Every path a user starts in the GUI or the Android app uses `recoverable: true`, so every file pays for that log:

- Per file, 4 durable private JSON writes. On Unix each write does 2 fsyncs (the file and its folder) plus many directory re-checks.
- 3 of the 4 writes happen while holding one lock per job.
- On top of that, Share uploads and downloads send 2–3 extra requests per file to the other device, one after another.

Waiting on fsync and on network replies keeps CPU, disk throughput and network all idle. That matches the product owner's description. This is an inference from the code; nothing was built or run. The measurement in section 6 confirms or refutes it.

---

## 1. Every copy/paste path a user can trigger

| # | User action | Entry point → engine | Crash log on? | Packets possible? |
|---|---|---|---|---|
| a | Paste inside the GUI (Ctrl+V or menu, own clipboard) | clipboard.rs:110-138 → `submit_paste` (app/core/transfer_jobs.rs:72-86) → `paste_job` (transfer_route.rs:124-138) → `build_job` 249-284 (`recoverable: true`, `RecoveryMode::New`, folder conflict KeepBoth, file conflict Rename) → `submit_job` → lane.rs:147 `engine::run_job` (engine/mod.rs:107) → `run_view` → `prepare` (prepare.rs:10-57) → `Engine::run` (dispatch.rs:44-101) | **yes** (job_store.rs:77-205) | only local→Share and Share→local (mod.rs:242-247) |
| b1 | Copy local files in se, paste in the OS file manager | clipboard.rs:84-105 puts the paths on the OS clipboard; the OS copies, se is not involved | – | – |
| b2 | Same, but the view is filtered or has paths too long for the clipboard | clipboard_lifecycle.rs:72ff and 246-327 (full recursive collect first) → clipboard_materialize.rs:10-47 → snapshot_download.rs:8 → `download_file_progress` per file (download_file.rs:9-113) | no | no; 1 fsync per file (download_file.rs:84), `create_dir_all` per file (26) |
| b3 | Copy remote (Share) files in se, paste in the OS file manager | `set_remote_clipboard` (virtual files; code outside the allowed read area). Staging helpers: downloads.rs:134/198 (whole tree collected first, 42-93) → clipboard_parallel.rs:29-105 → `download_collected_file` per file (1 stream per file, fsync per file). Caller found by grep: app/os/shared/remote_helpers.rs (not read) | no | no |
| b4 | "Für andere Programme bereitstellen" (provide for other programs) | clipboard_provide.rs:15-95 → `paste_job` into a temp folder → engine | **yes**, even though the target is a disposable temp folder | as in (a) |
| c | Paste from the OS clipboard into se | clipboard.rs:139-177 `read_clipboard_files` → `os_paths_selection` → `submit_paste` → same as (a) | **yes** | as (a) |
| d | Drag & drop | Drop in: drag_drop.rs:22-51 and 69-124 → `submit_paste` (a). Drag out: drag_drop.rs:197-231 (local: the OS copies; remote: `drag_out_remote`, virtual files, outside the read area) | yes (drop in) | as (a) |
| e | Android app | Not in `domains/` (only sync code there). Found by one grep in native/src/mobile: mobile/os/shared/transfer.rs:30 `transfer` → 162 `run` → 176 `drive::run_job(plan_job…)`; `plan_job` 211-240 sets `recoverable: true` | **yes** | as (a) |
| f | CLI `se cp` | cli/engine_copy.rs:120-212 `try_copy` → `run` 299-352 with `JobView{recoverable:false, id:None, conflict: Skip, folders: Merge}` → `run_view` | **no** | same rules as (a) |
| – | Older entry points (uploads.rs:52, upload_pairs.rs:23, downloads.rs:285, remote_copy.rs:26) | `run_legacy` (engine/legacy.rs:12-39, `recoverable: true`, line 87) | yes | as (a) |

---

## 2. What happens per file

Shorthand:
- **DW** (durable write) = one `write_private_atomic` (support_dirs.rs:67-97). It does: `ensure_private_dir(parent)` (75), an existence check through `open_private_file` (76-80), `getrandom`, `create_private_file` of a stage file (86), write, **`sync_all` (88)**, `replace_local_file` (90), **`sync_directory(parent)` (91)**.
  - Unix: every `ensure_private_dir`/open re-walks the whole path from `/` with `openat`, plus `canonicalize` and `fstat` (private_storage_unix.rs:15-103 and 116-144). That happens about 5 times per DW. Rough estimate (inference): ~150 syscalls and 2 fsyncs per DW.
  - Windows: each step re-opens the folder and re-checks its access rules (private_storage_windows.rs:6-72). `sync_directory` does nothing there; the comment says it relies on flush plus write-through promotion (78-82).
- **RT** = one request/response on its own QUIC stream (`PeerBackend::request`, peer_request.rs:58-98; `stat`, backend.rs:177-184).
- `start_file` (file_recovery_receipts.rs:315-390) holds the per-job lock `JobRun.data` (324) through: read the index file (326), repair check (327), read the receipt (328), **DW index with pending entry (366-372), DW receipt (374), DW index with count+1 (381-387)**. That is 3 DW under one lock, which caps a whole job at roughly 1/(3 × DW latency) files per second (inference).

### 2.1 Local → local (GUI paste, journal on)

| Step | Operation | Where |
|---|---|---|
| 1 | Take the next item from the queue (lock) | worker.rs:27, queue.rs:196-218 |
| 2 | No packets for local→local; single-file path | batch.rs:22 (`engine.batch` is None, mod.rs:242-247) → worker.rs:39 |
| 3 | Make sure the parent folder exists; the first file of a folder creates it (`ensure_plain_dir`) under a metadata permit | worker.rs:81 → folders.rs:193-265, 384-399 |
| 4 | Reserve 256 KiB buffer budget; take permits on the source and target volume flows | worker.rs:90-96, ops.rs:294-299, flow.rs:444-473 |
| 5 | Mark the file as active in the progress list (lock + string copy) | worker.rs:102, stats.rs:253-260 |
| 6 | Check whether an earlier run already copied this file: **private read of the receipt** (not found for a new job) | ops.rs:270 → missing_retry.rs:99-108 → `read_file` (receipts:278-305) → read_record (record_io.rs:83-138) |
| 7 | Snapshot of the source: stat + file identity | local.rs:63 → observer.rs:17-23 → job_store_validation.rs:30-59 |
| 8 | Copy into a private stage: lstat, stage create, kernel copy. **No fsync for a new copy** (`durable` only for move/overwrite) | safe_file.rs:80-155, 128 |
| 9 | Check that the stage path still names the copied file | safe_file.rs:161-175 |
| 10 | Before publishing: stamp of the stage (stat + identity), stat of the destination, **`start_file` = 3 DW under the job lock** | safe_file.rs:187 → observer.rs:54-77 |
| 11 | Publish without replacing (`commit_staged`); if the name is taken, try "Name (2)" and so on | safe_file.rs:202-251 |
| 12 | After publishing: read the receipt, check stamp (stat + identity), new stamp (stat + identity), **1 DW** (no lock) | safe_file.rs:265 → `after_checked`, file_recovery.rs:105-178 |
| 13 | Release permits, count the file done | worker.rs:116-128, 172-178 |

Total per file: **4 DW (8 fsyncs on Unix), 1 extra private read, about 10 extra stat/identity calls.** The CLI skips steps 6, 10 and 12, apart from the snapshot in step 7.

### 2.2 Local → Share host (upload, packet path)

**Client side**

| Step | Operation | Where |
|---|---|---|
| 1 | Packet check, packet sizer lock, `take_small` (scans up to 4096 queued items under the queue lock; `VecDeque::remove`) | batch.rs:21-63, queue.rs:222-251 |
| 2 | **Media probe for every local candidate file** (`source_media::probe`) | batch.rs:52, 65-69 |
| 3 | Parent folder for each member, one after another; each new folder is **1 RT** (`CreateDir`) | batch.rs:73-76 → folders.rs:260-261 → peer_transfer.rs:169-176 |
| 4 | Open every member: media guard, lstat, open, verify (fstat + lstat) | batch_put.rs:28-41, source.rs:48-71 |
| 5 | Reserve memory for the whole packet; **one** flow permit for the whole packet | batch_put.rs:51-57 |
| 6 | **For each member, one after another, while holding the permit:** `prepare_batch` → `prepare_direct`: source snapshot (local stat + identity) + `previous_presence(target)` = **1 RT Stat** + `start_file` = **3 DW under the lock** | batch_put.rs:76-84 → file_recovery.rs:180-206, file_recovery_verify.rs:15-29 |
| 7 | `put_batch`: entries JSON-encoded once just to measure them, new stream, header, data in 256 KiB frames (each its own `block_on`), commit message (`WriteDone`), then wait for `Ready` + outcomes. About 1 RT plus the host's commit time | batch_put.rs:93 → peer_batch_put.rs:66-192, 194-234, 332-349 |
| 8 | Permit released, packet speed recorded. **Everything below runs outside the permit and outside the "active" count** | batch_put.rs:101-110 |
| 9 | **For each member, one after another:** `after()`: private read of the receipt; no proof recorded for a remote target, so → `validate_copy_target` = **1 RT Stat** of the destination; `source_unchanged` (local stat + identity); `stamp_of` → `file_identity(Peer)` = **1 RT FileIdentity** (if the host supports it); **1 DW** | batch_put.rs:132-147 → file_recovery.rs:101-178, file_evidence.rs:162-176, 223-253, file_identity_rpc.rs:142-178 |

Total per member: **about 3 serial RTs + 4 DW**, plus 1/N of the packet RT. Because the receipts add requests per file, batching saves almost nothing.

**Host side, one blocking thread per batch** (server_fs.rs:364-384 → server_batch_put.rs:60-101, spawned at 74)

| Step | Operation | Where |
|---|---|---|
| 1 | Check entries, take a transfer slot | server_fs.rs:365-372 |
| 2 | Frames → channel (capacity 2) → worker | server_batch_put.rs:104-146 |
| 3 | Per entry: permission check (memory only: sharing active, auth lock, read check) | server_fs.rs:69-80 |
| 4 | Per entry: open the stage. **Two path resolutions** (stage and destination), each = 2× `canonicalize` + `exists`/`canonicalize` | server_batch_put.rs:367-389 → fs.rs:161-205 → fs_local_paths.rs:12-48 |
| 5 | Same-backend check, guard write check | fs_guard_backend.rs:184-191 |
| 6 | Stage opened with the durable variant → **fsync per entry** when it is closed. The single-file Share upload on the host uses `open_write_new`, **no fsync** (server_transfer.rs:236-239) | local.rs:221-227, local_stage.rs:15-25 |
| 7 | Commit, per entry: permission check + 1 path resolution + publish without replacing (stat of the stage + no-replace rename), with numbering on conflict | server_batch_put.rs:344-364, 392-415; promotion.rs:95-103 |

### 2.3 Share host → local (download)

**Packet path, client side**

| Step | Operation | Where |
|---|---|---|
| 1 | Packet check (no media probe, the source is remote) | batch.rs |
| 2 | Local parent folder for each member | batch.rs:73-76 |
| 3 | Per member: `existing()` (no I/O unless resume/skip), `read_size` (no RT for Peer) | batch_get.rs:34-45; vfs core.rs:188-190 |
| 4 | One free-space check, memory, one permit | batch_get.rs:54-70 |
| 5 | `get_batch`: new stream, header, `Ready`, then per item a length header, data frames, end marker | peer_batch_get.rs:31-183 |
| 6 | Sink, per item: `.part` file created; data hashed (if MD5 known) and written | batch_get.rs:264-327, download.rs:33-52, local_stage.rs:8-38 |
| 7 | **`end` → `publish_local`, still inside the stream loop:** `before()`: snapshot of the remote source = **1 RT Stat + 1 RT FileIdentity**; local stat/identity checks; `start_file` = **3 DW under the lock**; publish without replacing (`promote_local_copy`; if taken, `unpublished()` + one probe per candidate name); `after()`: read + verify (stat + identity) + stamp + **1 DW** | batch_get.rs:329-358 → download.rs:270-336 → file_recovery.rs:63-99, 101-178 |

The stream stops being read while the client works through these steps for each file.

Host GetBatch, one thread per batch (server_batch_get.rs:33-111): per item a permission check, path resolution (canonicalize), stat, read size, open, read with a new buffer allocated per chunk (156), and a stat afterwards. No fsync.

**Single-file download** (download.rs:123-194): free-space check (statfs) per file, `.part` file, open stream (1 RT), data, then a check stat of the source (**1 RT**, source.rs:343-388), then `publish_local` (**2 RT + 4 DW**). About 4 RTs per file.

**Single-file upload** (upload.rs:12-72 → publish.rs:46-262), used for files above the small-file limit or retried files: stage opened through `WriteNew` (1 stream), data, commit; `before()` = Stat RT of the stage + FileIdentity RT of the stage + Stat RT of the destination + 3 DW; `PromoteNoReplace` (1 RT, peer_transfer.rs:188-222); `after()` = Stat RT + FileIdentity RT + 1 DW. About **7–8 RTs + 4 DW per file**.

**Progress overhead (small):** per file one active-list lock and string copy (stats.rs:253-260); per chunk one or two flow locks (flow.rs:339-345, 412-417); progress message every 150 ms (mod.rs:96). Retries write log notes (worker.rs:136-139), successes do not.

---

## 3. Concurrency model

### Constants

| Setting | Value | Where |
|---|---|---|
| Starting concurrency per connection | 2, or half of the limit learned last time | flow_control.rs:33; flow_window.rs:19-21; flow_registry.rs:56 |
| Hard ceiling | 256 | flow_control.rs:32 |
| Share ceiling | 56 = min(60 admission, 60) − 4 kept for browsing | peer_transfer.rs:148-158; keepalive.rs:9-17 |
| Measuring window | ≥100 ms (2× average op time), ends after 8 completions or 1 s, max 5 s | flow_control.rs:35-40, 243-254 |
| Growth | doubles while +10 %; after that probes ±1/8 every 4 windows | flow_control.rs:50-57 |
| Work credit per finished op | 64 KiB; a whole packet counts as one op | flow_control.rs:28, 233-237 |
| Extra metadata permit | 1 beyond the limit | flow.rs:200-217 |
| Packet target | 250 ms of transfer at the measured rate, minimum 256 KiB | engine_policy.rs:40-42, 206-212 |
| Small-file limit | target ÷ 8 → **32 KiB at start** | engine_policy.rs:45, 215-217 |
| Host packet limits | 256 files / 16 MiB | batch_wire.rs:20-25 |
| Queue scan for packet members | 4096 items; up to `max_files − 1` extra members | queue.rs:25; batch.rs:41-45 |
| Queue memory bound | 16 MiB | queue.rs:19 |
| Worker idle time before exit / progress interval | 250 ms / 150 ms | mod.rs:96-102 |
| Copy buffer / network chunk | 256 KiB / 256 KiB | ops.rs:17; fs.rs:19 |
| Host slots | 256 for all clients, 60 per device | server_admission.rs:25-32 |
| Host blocking pool for control requests | 32 | blocking.rs:17 |
| Host buffer per stream | 2 chunks | server_transfer.rs:25 |
| QUIC limits | 64 streams, 16 MiB per stream, 64 MiB per connection | keepalive.rs |

### How many run at once

- A worker thread starts when files are queued, no worker is idle, and either there is no worker yet or all are running and the flow has a free permit (dispatch.rs:65-69).
- "Running" is counted with `stats.begin`. A packet worker drops that count before its per-member `after()` loop (batch_put.rs:108). While a worker is in that phase, `busy` is false, so **no new worker starts** (inference).
- The time spent on receipts is mostly not seen by the flow or by the "limited by" verdict:
  - The packet's prepare loop runs inside the permit but outside any timed account; the `after()` loop runs outside the permit (batch_put.rs:76-110).
  - In local→local the receipts are inside `timed_target`, so they count as "target drive" (local.rs:64).
  - Likely result (inference): concurrency stays low because adding workers doesn't help when they all wait on fsync and one lock.

### Everything that turns packets off

| Condition | Where | Common in normal GUI pastes? |
|---|---|---|
| Not local→Share or Share→local (local→local, Share→Share, other backends) | mod.rs:242-247 | always for those directions |
| Host without transfer v1, or limits zero/≤1 | peer_transfer.rs:115-124 | old hosts only |
| Packet check runs on `"/"` or the connections root | peer_transfer.rs:127-132; fs_capabilities.rs:46-48 | **Yes for downloads:** the check uses the folder *above the first selected item* (mod.rs:244). Copying a whole export (e.g. `/A`) from the Share root gives `"/"`, so the whole job runs without packets |
| `file.retried` | batch.rs:23 | after transient errors |
| `file.alone` | batch.rs:24, 55, 154-155; batch_get.rs:175 | after a failed packet, optical media, files changed since listing |
| `engine.resume` ("Fehlende übertragen") | batch.rs:25; mod.rs:273 | whenever the user resumes |
| `RecoveryMode::Resume` | batch.rs:26; transfer_route.rs:183; legacy.rs:63 | resume and crash recovery |
| Local source that is optical or whose media probe fails | batch.rs:27, 65-69; batch_put.rs:33-36 | rare |
| File larger than the small-file limit (32 KiB at start) | batch.rs:38-40 | **Likely common for "small files":** 32 KiB–2 MiB documents and photos take the 7–8-RT single path. The limit only grows if the measured rate grows, and host fsyncs keep it low (inference) |
| Fewer than 2 members opened / not 2 members left | batch_put.rs:42-48; batch_get.rs:46-51 | at the start of a walk and at the tail |
| Host does not report the source size up front | server_batch_get.rs:134-139 | exports and Drive files |

### Folder creation

- Each folder costs **1 RT** on the Share (`CreateDir`, peer_transfer.rs:169-176), under a metadata permit (folders.rs:384-399). Parents are created first, recursively, so a new path d levels deep costs d serial RTs (folders.rs:251-254).
- New top-level folders are created exclusively and renumbered on conflict (folders.rs:267-303).
- Folders are created lazily by their first file; files go ahead of folders in the queue (queue.rs:6-9).
- This **does not hold up the walk**: discovery runs in parallel in its own thread (walk.rs:1-6, 154-172; dispatch.rs:46-48).
- It **does serialize inside a packet**: each member's parent is ensured one after another before upload (batch.rs:73-76). Other workers needing the same folder wait without a permit (folders.rs:206-214).

---

## 4. Share host side

- **One thread per batch, entries in strict sequence.** `spawn_holding` once per batch (server_batch_put.rs:74; server_batch_get.rs:42), then `for item in &items` / `Upload::accept`/`commit`.
- **Batches at the same time:** one per stream. The cap is the client's flow limit (starts at 2, ceiling 56), the host's 60 slots per device and 256 overall (server_admission.rs:25-32), plus about 1 MiB of buffer budget per transfer (TRANSFER_BUFFER_BYTES, server_admission.rs). There is no separate limit for batches.
- **Blocking work per entry:**
  - PutBatch: **fsync per entry** (local.rs:221-227 → local_stage.rs:22-25). The single-file upload does not fsync, so this is inconsistent with the "copies don't force a disk sync" rule in publish.rs:70-73 and core.rs:108-114.
  - PutBatch: about 3 path resolutions per entry, each with 2–3 `canonicalize` calls (fs_local_paths.rs:12-48). On Windows each `canonicalize` opens a handle; on Android shared storage it likely goes through FUSE (inference).
  - Per entry: permission check (memory only, session.rs:150-156, node.rs:355-360) and guard checks (fs_guard_backend.rs:184-191).
  - GetBatch: 1 path resolution plus stat before and after per item.
  - No `spawn_blocking` per file.
- Single requests (Stat, FileIdentity, CreateDir) run on the 32-slot control pool (server_fs.rs:131-140; blocking.rs:17).

---

## 5. Bottlenecks ranked, and the proposed architecture

### Ranking (by expected effect on "thousands of small files")

1. **Per-file crash log on the client.** 4 DW per file (8 fsyncs on Unix; on Windows `FlushFileBuffers` plus write-through rename plus access-rule checks), with 3 of them under one lock per job.
   - Applies to every GUI and Android path, including local→local; not to the CLI (evidence in sections 1 and 2.1).
   - Rough estimate (inference): with fsync at 0.5–2 ms the job tops out at about 80–300 files/s. On Windows with 2–20 ms flushes, about 15–100 files/s. CLI local→local measured about 6000 files/s.
2. **Extra requests per file inside the receipts.** Upload packet: 3 serial RTs per file; download packet: 2 serial RTs per file, done inside the stream loop; single upload about 7–8 RTs. These undo the packet savings; over relay (about 50 ms RTT) that is roughly 150 ms per file per worker.
3. **Packets limited or switched off.** Small-file limit 32 KiB, sized by bytes only; no packets when downloading a whole export; none during resume.
4. **Host fsync per PutBatch entry**, sequential within a batch, plus 3× path resolution per entry.
5. **Slow concurrency ramp:** starts at 2, slow-start windows need 8 completions; worker spawning stalls while workers are in their `after()` phase; receipt time is invisible to the "limited by" display.
6. **Folder creation:** 1 RT per folder, serial within a packet. Matters a lot for deep trees with few files per folder.
7. **OS clipboard staging paths:** pre-collect the whole tree, one stream and one fsync per file, no packets.

### Proposed architecture

**A. Crash log v2: one append-only log per job with grouped fsync** (fixes 1)
- Change:
  - Open the job's private folder once and keep the handle in `JobRun` (on Unix an open dir fd and `openat`/`renameat`).
  - Append records (intent / staged / published / discarded) to one log, each with a CRC, or sealed per record for vault targets with the existing `TransferBlobContext`.
  - A commit thread fsyncs once for everything pending.
  - A file publishes only after its intent record is durable.
  - "Published" records do not wait for fsync; they are flushed with the next group commit or when the job finishes.
  - One intent record per packet.
  - The log replaces `index.json`, its `pending` and `count` fields, and the folder of per-file JSON files.
- Expected gain: from 8 fsyncs per file to well under 1 per file. Local→local in the GUI should come close to the CLI (estimate ×10–50 on Windows and Android).
- Invariant touched: intent durable before publication; a lost answer never authorizes a resend.
  - Kept by waiting for the intent's log position. Intent is still written before `put_batch` and before the local rename.
  - A torn last record (bad CRC) is treated like today's `pending` repair (receipts:144-212).
  - A missing "published" record after a crash leaves the file "in doubt", which `doubt`/`missing_retry` already resolve by metadata or digest.
  - Vault names stay sealed.
- Interim fix, smaller diff, estimated 2–4×:
  - Cache the private folder handle; drop the repeated `ensure_private_dir` and existence check (support_dirs.rs:75-80).
  - Stop writing `index.json` twice per new file (receipts:364-388).
  - Make the `after()` write non-durable.
  - Skip the `resume()` read for `RecoveryMode::New` jobs that have no saved files (ops.rs:270).
  - Use `recoverable: false` for temp targets ("Provide", clipboard staging).

**B. Let the host send the evidence back instead of separate requests** (fixes 2)
- Change: extend the protocol behind a capability flag, older hosts keep the current requests:
  - `FsBatchOutcome::Published` returns path, size, mtime and file identity.
  - The PutBatch `Ready` reply returns whether each destination existed before (or mark it known-absent when this job created the folder, `parent_created`).
  - The GetBatch item header carries the `stat_item` and identity the host already reads (server_batch_get.rs:118); `ReadAt` and `PromoteNoReplace` replies likewise.
  - `unchanged_since_listing` (source.rs:367) and `snapshot`/`stamp_of` then use these values.
- Expected gain: 0 extra RTs per packet member (from 3 up / 2 down); single upload from about 7 RTs to about 2. Over relay roughly ×20–100 for small files.
- Invariant: the same evidence (identity, size, mtime) from the same authenticated host under the same permission check. It just travels in the reply.

**C. Don't block the download stream with publishing** (with A and B in place this matters less)
- Change: hand finished `.part` files to a publish queue or thread, so the GetBatch stream is read at network speed.
- Invariants: no-replace and keep-both numbering unchanged. If the stream fails, files already finished are published, unfinished ones go alone, as today.

**D. Host PutBatch durability and path resolution** (fixes 4)
- Change:
  - Use unsynced stages (`open_write_copy_stage_unsynced`), consistent with the single-file path. Or keep durability but do one sync point before the commit reply (`syncfs` once on Linux; parallel `FlushFileBuffers` on Windows).
  - Resolve each distinct parent folder once per batch instead of 3 resolutions per entry.
- Expected gain: from N fsyncs to ≤1 per batch.
- Invariant: whether "Published" must mean "on disk" is a product decision. Today the single-file path does not guarantee it, so the batch path would be aligned with it. No-replace and numbering stay unchanged.

**E. Packet and concurrency policy** (fixes 3 and 5)
- Change:
  - Small-file limit = max(target ÷ 8, 1–2 MiB).
  - Allow up to 256 files per packet whenever the files are tiny (count-based target, not only bytes).
  - Compute packet limits for the folder actually being transferred, not the folder above the first selected item (mod.rs:243-244).
  - Start Share flows at about 8.
  - Count packet members as completions when judging a measuring window.
  - Spawn workers based on free flow permits and idle workers, not on `running ≥ workers`; or keep a file "active" until its `after()` is done.
  - Optionally keep 2 packets in flight per worker.
  - Allow packets during resume; the "missing" check could be a per-entry host flag.
- Invariants: none touched; packet semantics stay the same.

**F. Folder creation in bulk** (fixes 6)
- Change: a `CreateDirs` request carrying many folders, or "create missing parents" inside PutBatch. The host reports which folders it created, so `parent_created` stays correct. Folders can be created in bulk from discovery's folder events.
- Invariants: top-level folders keep their separate exclusive creation with keep-both numbering. Nested folders keep merge semantics.

**G. Stream a whole new folder as one unit** (largest structural gain, for new top-level folders)
- Change:
  - PutTree/GetTree: one stream carries the subtree; the host writes it into a private hidden stage folder `.se-tree-<nonce>`.
  - Publish with **one** no-replace folder rename, numbered on conflict, like `move_whole_roots` does for local (local.rs:274-340).
  - One receipt for the folder (stage identity plus intent) instead of per-file receipts.
  - Same for local→local: copy into a stage folder on the target volume, then one `renameat2(NOREPLACE)` / `MoveFileEx` without replace.
- Expected gain: per-file publish, numbering and receipt cost close to zero; the whole folder moves at link or disk speed.
- Invariants:
  - No-replace is the folder rename.
  - Keep-both is the numbered folder name.
  - Cancel deletes the stage folder.
  - Resume continues into the recorded stage folder (its contents are private, so they can be checked by listing).
  - Files never appear half-published, which is stronger than today.
  - Merges into existing folders (`FolderConflict::Merge`) and vault targets without folder staging or no-replace folder rename fall back to the per-file path.

**H. OS clipboard paths** (fixes 7)
- Change: stage remote clipboard content through the engine (`recoverable: false`, GetBatch, streaming instead of collect-first). Drop the fsync and per-file `create_dir_all` for disposable temp files (download_file.rs:26, 84).
- Invariant: the destination is still published without replacing.

---

## 6. Measurement plan for GitHub Actions (ubuntu-24.04, windows-2025)

**Existing test helpers**
- Fake backend: engine/test_backend.rs — `Fake` with `batch`, `ceiling`, `counters` (`put_batches`, `stages`, `promotes`, `opens`…).
- Job runners: test_run.rs:63-97 — `run(job)` and `job()`. `job()` matches `paste_job` exactly (`recoverable: true`, KeepBoth, Rename).
- Wrapper macro: `forward!` (batch_put.rs:304-310) plus the `SlowWire` example (393-422). These allow a wrapper that counts and delays `stat`, `file_identity`, `create_dir` and `put_batch`/`get_batch`.
- Existing packet tests to copy from: tests_batch.rs.
- Real Share over loopback: `CopyPastePeerFixture::for_transfer_engine_task()` (share/core/copy_paste_task_fixture.rs:61-64). Two real Iroh nodes on 127.0.0.1, no relay, **no opt-in environment variable needed**, exports `/A` → `root_a` and `/B` → `root_b`, `fixture.backend` is a `BackendHandle`. It is `pub(super)`, so a benchmark must live in `native/src/share/core/` and call `crate::transfer::run_view(JobView{…})` (transfer/mod.rs:147). Examples of use: transfer_engine_task_path_tests.rs:112-123, lz4_stream_loopback_tests.rs.

**Benchmarks** (proposed as `#[ignore]` `speed_task_*`; run with `cargo test --release -- --ignored speed_task_small --nocapture --test-threads=1`)
1. Local→local, 5000 × 4 KiB in 50 folders, `recoverable: true` vs `false` (mirror the CLI `JobView` from engine_copy.rs:306-324). The difference between the two is the cost of the crash log.
2. Local→`Fake` (batch 256 / 16 MiB) and `Fake`→local, with injected delay per request (0 / 2 / 20 ms). Count calls per file to `stat`, `file_identity`, `create_dir` and batches. These counts are deterministic and can be asserted as regression guards (for example "remote stats per file ≤ 0 after B").
3. Real loopback Share, upload to `/B/dst` and download of `/A/src` (also downloading the export `/A` itself, to hit the `"/"` packet case). Same file set, plus 1000 × 64 KiB to exercise the single-file path above the 32 KiB limit.
4. Crash-log microbenchmark: N × (`start_file` + `after`) on the runner's disk.

**Runner setup**
- Point the crash log at runner storage: Linux `XDG_DATA_HOME=$RUNNER_TEMP/sexdg` (support_dirs.rs:171-184); Windows `APPDATA=$RUNNER_TEMP\seappdata`.
- Linux only: add delay on loopback with `sudo tc qdisc add dev lo root netem delay 1ms`, remove afterwards. Windows has no netem; use the delaying `Fake` wrapper there.
- Windows: one variant with `Add-MpPreference -ExclusionPath $env:RUNNER_TEMP` to separate Defender cost.
- Run base and branch in the same job (git worktree for the base) so both use the same hardware. Three runs each, report the median. Print `BENCH <case> files_per_s=… fsyncs_est=… rts_per_file=…` into `$GITHUB_STEP_SUMMARY` and upload JSON as an artifact.
- Quick check available today without code changes: GUI-style job vs CLI-style job on the same data and target. If the gap is ×10 or more, bottleneck 1 is confirmed.

---

## Files read
transfer: core/{engine_policy, flow_control, flow_window, limiter(head)}.rs; os/shared/{flow (parts), flow_registry (head), clipboard_parallel, downloads, download_file, external (head), snapshot_download (head), walk (head and constants), lane (head), local_stage (parts)}.rs; os/shared/engine/{mod, worker, queue, dispatch, discovery, batch, batch_put, batch_get, prepare, ops, upload, publish, download, local, file_recovery, file_recovery_receipts, file_recovery_observer, file_recovery_verify (parts), file_recovery_doubt (parts), file_recovery_paths (parts), file_evidence (150-421), job_store (60-351), job_store_validation (1-80), record_io, view, legacy, folders, folder_reservation (head), roots (head), source (1-140, 158-198, 300-389), stats (240-270), missing_retry (99-129), test_backend (parts), test_run, tests_batch (head), test_faults (parts)}.rs. copy: os/shared/safe_file.rs (1-300), staging.rs (grep), publication.rs (grep). share/core: backend, peer_batch_put, peer_batch_get, peer_transfer, peer_extensions (head), file_identity_rpc (142-182), peer_stream (features), peer_request (58-98), batch_wire (head), keepalive (head), server_admission (head), server_batch_put, server_batch_get, server_fs (1-140, 340-419), server_transfer (20-60, 200-260), fs_guard_backend (150-240), fs.rs resolve, fs_access (resolve), fs_capabilities (synthetic container), session (authorize), copy_paste_task_fixture (head), transfer_engine_task_support (head), transfer_engine_task_path_tests (100-179); share/os/shared/fs_local_paths.rs. app: core/transfer_route.rs; os/shared/{clipboard, clipboard_provide, clipboard_materialize, clipboard_lifecycle (240-327), recursive_clipboard, drag_drop (parts)}.rs; core/transfer_jobs.rs (grep, 72-112). cli/engine_copy.rs (1-355). mobile/os/shared/transfer.rs (195-240, grep). Where calls led: vfs core/core.rs (85-140, 188, 225-240), core/file_identity.rs, core/promotion.rs (60-140), os/shared/local.rs (158-260, 285-305), os/shared/local_stage.rs (15-26); support_dirs.rs (1-97, 160-210); creds/os/private_storage_{unix,windows}.rs.

## Uncertain / unverified
- All fsync, syscall and files/s figures are estimates from reading the code, not measurements.
- How expensive `source_media::probe` is per packet candidate (outside the allowed read area).
- Whether Windows `replace_local_file` uses write-through: I only saw `MOVEFILE_WRITE_THROUGH` in vfs/os/windows (local_platform.rs:221-238, local_writes.rs:111), not how `replace_local_file` maps to it.
- That `PeerBackend::open_write_fresh` returns `None` (trait default; backend_defaults::open_write_fresh not read).
- How host `FileIdentity` is served, and whether the `FsAccess::Authorized` path for Direct peers goes through `fs::resolve` with canonicalize (I only saw the delegation).
- How the virtual-file paths (Explorer pulling remote files, `drag_out_remote`, `set_remote_clipboard`) and `remote_helpers.rs` are implemented; not read.
- Android host costs through FUSE and `canonicalize` on shared storage.
- The claims that worker spawning stalls in the `after()` phase and that the flow controller "settles low" are inferences from reading dispatch.rs:65-69 and flow_control.rs.
- Whether `se cp` accepts Share peers as targets, which would allow a quick CLI-vs-GUI comparison on a Share.
- Whether the host's Capabilities reply currently sets `file_identity_v1`. If not, the FileIdentity RTs in 2.2 and 2.3 don't happen and those per-file request counts drop by one.
