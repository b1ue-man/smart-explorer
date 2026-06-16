# Wave-2 — Adversarial RED-TEAM audit of the CfAPI remote-file feature

Scope: `native/src/cfprovider.rs`, `cfsync.rs`, `app.rs` (open / poll_remote_edits /
RemoteEdit), `Cargo.toml`. Ground truth re-derived from `cloud-filter-0.0.6` source and
Microsoft primary docs. Wave-1 (`w1_*.md`) is treated as a prior to *refute or exceed* —
findings here are either NEW or explicitly overturn/sharpen a Wave-1 claim. No code edited.

Legend: **BUG** = wrong behavior on a real, reachable input. **UB** = undefined behavior /
abort. **RISK** = latent / conditional. **OK/REFUTE** = Wave-1 claim corrected. Severity is
*this auditor's* call, not Wave-1's.

Primary citations used repeatedly:
- **[MS-OP]** CF_OPERATION_PARAMETERS, TransferData section, learn.microsoft.com/.../ns-cfapi-cf_operation_parameters — "The only requirement is that both offset and length are **4KB aligned unless the range described ends on the logical file size (EoF)**, in which case the length is not required to be 4KB aligned **as long as the resulting range ends on or beyond the logical file size**."
- **[MS-SIZE]** MS Q&A "Altering file size in CF_CALLBACK_TYPE_FETCH_DATA" (Fei Xue, MSFT) — when actual cloud size ≠ placeholder size, "**CfExecute fails after reaching the expected size**"; fix is `CF_OPERATION_TYPE_RESTART_HYDRATION`, which "changes the metadata for the placeholder on disk, resets the hydration state machine and invokes the fetch data callback again."
- **[CRATE]** cloud-filter 0.0.6 source under `/root/.cargo/.../cloud-filter-0.0.6/src`.

---

## 1. NTFS-illegal char / Google-Drive title containing "/", and the NUL panic

**Input.** A Drive file whose `name` (from `list_dir`) is literally `Q3/Q4.xlsx`, or any remote
name containing `: * ? " < > |`, a trailing dot/space, or a reserved device name
(`CON`, `NUL`, `COM1`…). Separately: a name containing an interior NUL (`U+0000`).

**Code path.** `cfprovider.rs:120` `PlaceholderFile::new(&display)` where `display` is the raw
remote leaf (`m.name`, or `download_name(...)` which only *appends* an extension, never
sanitizes). For the leaf the OS path is `local_path_named(... san(leaf))` (`cfsync.rs:66`) —
but `san()` is applied to the **on-disk dest the app launches**, NOT to the
`RelativeFileName` the provider hands CfAPI. Those two names now disagree (see Finding 11).

**Predicted outcome — NEW, split into three cases Wave-1 merged:**
- **(a) `/` in a Drive title (the most reachable case):** `PlaceholderFile::new("Q3/Q4.xlsx")`
  builds a `U16CString` (`placeholder_file.rs:24`) and stores it as `RelativeFileName`. CfAPI
  treats `RelativeFileName` as a **single path component**; an embedded `/` (or `\`) is a path
  separator, so `CfCreatePlaceholders`/`TRANSFER_PLACEHOLDERS` rejects the entry with
  `STATUS_OBJECT_NAME_INVALID` → surfaced as the `CfExecute` HRESULT.
- **(b) other illegal chars (`:*?"<>|`, trailing dot/space, `CON`):** same `TRANSFER_PLACEHOLDERS`
  rejection, `STATUS_OBJECT_NAME_INVALID` / `0x80070057`.
- **(c) interior NUL:** `U16CString::from_os_str(...).unwrap()` (`placeholder_file.rs:24`)
  **panics** (`from_os_str` errors on interior NUL) *before* any CfAPI call. **This panic
  unwinds through `fetch_placeholders` (`proxy.rs:135`, `extern "system"`) = UB/abort.**

**NEW vs Wave-1.** Wave-1 (D1/E1) asserted "the platform fails *that entry*". I **refute the
per-entry-isolation assumption for case (b)/(c) and sharpen (a)**: because the crate calls
`CreatePlaceholders::execute` for the **whole batch** (`ticket.rs:148`, `commands.rs:162`) and
**never inspects per-entry `CF_PLACEHOLDER_CREATE_INFO.Result`** (`commands.rs:168` `result()`
is a no-op; `PlaceholderFile::result()` exists but is unused on this path), a single bad child
makes the `CfExecute` return the **first** failing HRESULT, which `cerr()` collapses to
`CloudErrorKind::Unsuccessful` → the *entire directory enumeration* reports failure. So one
weirdly-named sibling can make the **whole folder fail to populate**, not just that file. And
case (c) is strictly worse than Wave-1 said: it is an abort *before* the batch even reaches the
OS, so *no* file in that directory ever appears.

**Citation.** [CRATE] `placeholder_file.rs:20-31, 24` (no validation, `.unwrap()` on NUL);
`commands.rs:162-188` (whole-batch execute, `DISABLE_ON_DEMAND_POPULATION` set);
`proxy.rs:135-155` (extern "system", `.unwrap()` on the fail path). Name rules:
learn.microsoft.com/.../naming-a-file ("Naming Files, Paths, and Namespaces" — reserved chars
`< > : " / \ | ? *`, reserved names CON/PRN/AUX/NUL/COM1…, no trailing space/period).

**Severity.** (a)(b) **BUG** (whole-folder populate failure, very reachable on Drive). (c) **UB**.

**Fix.** Sanitize `display` with the *same* `san()` used for the on-disk path (so the two
agree), reject/skip interior-NUL names *before* `PlaceholderFile::new`, and either create
placeholders one-at-a-time or read back `PlaceholderFile::result()` per entry so one bad name
can't sink the directory. For Drive `/`-in-title, percent-or-fullwidth-escape and keep the true
id in the blob.

---

## 2. Deep path: blob > 4096 bytes AND the MAX_PATH (260) local-FS wall Wave-1 missed

**Input.** A remote tree nested deeply enough that (a) the full remote path UTF-8 > 4096 bytes,
and independently (b) the *local* mirror path `%USERPROFILE%\Smart Explorer\<conn>\<rel>`
exceeds 260 UTF-16 chars.

**Code path.** (a) blob: `cfprovider.rs:123` `.blob(child_remote.into_bytes())` →
`placeholder_file.rs:79` `assert!(blob.len() <= CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH)`.
(b) local path: `populate_to`/`drain` does `std::fs::read_dir(dir)` per level
(`cfprovider.rs:144`) and the app launches `dest` (`app.rs:3133`); `dest` comes from
`local_path_named` joining `san(seg)` per segment (`cfsync.rs:64-67`).

**Predicted outcome.**
- (a) is the Wave-1 blob-assert panic across FFI (still valid; UB). I add the **trigger
  bound**: `CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH` = 4096 *bytes*; with 3-byte UTF-8 CJK or
  4-byte emoji segment names, ~1024-1365 chars of path is enough — well inside Drive limits.
- (b) **NEW — the local filesystem wall.** CfAPI placeholders live on a real NTFS volume and
  the crate/app reach them with plain `std::fs` and `CreateFileW` *without* the `\\?\` long-path
  prefix and **without** declaring `longPathAware` in a manifest. Rust's `std::fs` does add
  `\\?\` for verbatim absolute paths in some cases, but `populate_to`'s incremental `Path::join`
  + the app's `open_path` (`app.rs:3133`, hands a `/`-replaced string to the shell which
  re-parses it) do **not** guarantee the verbatim prefix, and `ShellExecute`/the target editor
  is classically MAX_PATH-bound. So a path > 260 chars: `read_dir` on the deep ancestor returns
  `Err` (silently swallowed by `if let Ok(rd)`, `cfprovider.rs:144`), `fetch_placeholders` for
  that level never fires, the leaf placeholder is never created, `dest.exists()` is false
  (`app.rs:3128`) → user sees the German "Platzhalter wurde nicht erzeugt" error. **Silent
  populate failure that Wave-1's MAX_PATH-free analysis did not predict.**

**Citation.** [CRATE] `placeholder_file.rs:79-84`. MS "Maximum Path Length Limitation"
(learn.microsoft.com/.../maximum-file-path-limitation) — `MAX_PATH` = 260; opt-in required and
the app ships no `longPathAware` manifest entry (verified absent: no manifest sets it; default
is 260).

**Severity.** (a) **UB**. (b) **BUG** (silent failure on deep but legal trees; degrades to the
user-visible error, not a crash — so BUG not UB).

**Fix.** (a) cap/replace the blob with a short id or hash and reconstruct the remote path from
`request.path()` in `fetch_data` (the code *already* falls back to `remote_of(path)` when the
blob is empty, `cfprovider.rs:65` — so the safe fix is: if `child_remote.len() > 4000`, store an
empty blob and rely on the path). (b) emit `longPathAware` in the manifest and prefix `\\?\`.

---

## 3. Google Doc: declared size 0, export produces N>0 bytes — guaranteed broken hydration

**Input.** Open any native Google Doc/Sheet/Slide (e.g. a Google Docs file `Notes`). In
`list_dir`, the Drive API returns **no `size` field** for native Google-Apps types, so
`meta_from_json` sets `size: f["size"].as_str()…unwrap_or(0)` = **0** (`gdrive.rs:267`). The
placeholder is therefore created with `Metadata::file().size(0)` (`cfprovider.rs:107-108`).
`download_name` gives it a `.docx` leaf (`gdrive.rs:518`), and `open_read` **exports** to a real
.docx of N>0 bytes (`gdrive.rs:505-509`).

**Code path.** `fetch_placeholders` → `Metadata::file().size(0)` (`cfprovider.rs:107`,
`metadata.rs:63` sets `FileSize = 0`). Later `fetch_data` (`cfprovider.rs:58-89`):
`required_file_range()` for a logical-size-0 placeholder is `0..0` (`info.rs:29-32`,
`RequiredFileOffset=0`, `RequiredLength=0`). `len = range.end - range.start = 0`
(`cfprovider.rs:85`), so `r.take(0).read_to_end(&buf)` reads **nothing**, and
`ticket.write_at(&[], 0)` writes a zero-length range.

**Predicted outcome — NEW and more severe than Wave-1's "export size differs" framing.**
Wave-1 (B1) assumed the placeholder size is the *real* Drive size and merely *differs* from the
export. **I refute the premise: for Google Docs the declared size is literally 0.** Two distinct
failure modes follow, both worse than "differs":
1. **The likely outcome: a silent EMPTY file.** A size-0 placeholder's required range is `0..0`,
   which *already* "ends on the logical file size" → EoF satisfied immediately with zero bytes
   [MS-OP]. The OS may consider the file fully hydrated at 0 bytes and **never demand the real
   content** — `fetch_data` either isn't called or is called with an empty range. The user opens
   a **0-byte .docx** ("Word cannot open the file"). The export is never delivered.
2. **If the app/Word reads past EoF (size 0):** any read at offset ≥ logical size returns EoF;
   there is nothing to hydrate. To deliver the export the provider would have to *grow* the
   logical size, which requires `RESTART_HYDRATION` with new `FsMetadata.FileSize` [MS-SIZE] —
   **the code never calls it** (no `Update`/`RESTART_HYDRATION` anywhere in `cfprovider.rs`).
   Per [MS-SIZE] the platform path for "actual size ≠ declared size" is exactly
   `CfExecute fails after reaching the expected size`; with expected size 0 it "reaches" it at
   byte 0 and stops. Net: the Doc cannot be hydrated correctly by this provider, ever.

Conversely, for a **regular** file whose Drive size is correct but whose *content length on
read* happens to differ (rare), the [MS-OP] EoF rule means a **short** backend stream leaves the
required range unsatisfied → the user I/O blocks until the 60 s `CANCEL_FETCH_DATA` timeout
(`info.rs:71` `CF_CALLBACK_CANCEL_FLAG_IO_TIMEOUT`); a **long** stream is harmless (over-read is
explicitly allowed [MS-OP]) *except* the bytes beyond declared size past EoF are dropped.

**Citation.** [MS-SIZE] (CfExecute fails on size mismatch; RESTART_HYDRATION is the fix);
[MS-OP] (range must reach logical size; over-read allowed; writes beyond EoF). [CRATE]
`metadata.rs:63`, `info.rs:29-32`. App: `gdrive.rs:267` (size 0), `gdrive.rs:505-509` (export).

**Severity.** **BUG** (Google Docs — a headline feature of the Drive backend — open empty or
fail to hydrate; this is the single most user-visible defect). Wave-1 under-rated it as a
size-*difference* RISK; it is a deterministic *zero-size* BUG.

**Fix.** For export types, `stat`/`HEAD` the export to learn its real byte length before
creating the placeholder, OR create with a large provisional size and call `RESTART_HYDRATION`
(crate `command::Update` / `RestartHydration`, `commands.rs:111-151`) with the true size on
first `fetch_data`. Minimum viable: do not declare size 0 for files that export to non-empty
content.

---

## 4. Partial / mmap read → non-4KB-aligned mid-file required range. REFUTES Wave-1 C3.

**Input.** An app opens the hydrated-on-demand placeholder and does a *partial* read: e.g. a
media player or `CreateFileMapping`/`MapViewOfFile` that touches only the first 512 bytes, or a
ranged read at offset 100000 length 512, on a file whose declared size is, say, 1 MiB.

**Code path.** `fetch_data` (`cfprovider.rs:70`) takes `info.required_file_range()` verbatim and
passes `range.start`/`len` straight to `ticket.write_at` (`cfprovider.rs:88` →
`ticket.rs:71` → `commands.rs:73-84`, unmodified `Offset`/`Length`).

**Predicted outcome — NEW; this is the refutation of Wave-1's unverified C3.**
Wave-1's "works today" rests on the *empirical, untested* claim that the OS always issues
`0..size` for a full open. **Primary source overturns that as a guarantee.** Under
`HydrationType::Full` (what we register, `cfprovider.rs:204`) the platform hydrates the *whole*
file on first access, so in the common double-click case the required range *is* `0..size` and
length reaches EoF → the unaligned single write is exempted by the EoF rule [MS-OP]. **BUT** the
MS Q&A "[Bug Report] CreateFileMapping causes full placeholder hydration ignoring
PROGRESSIVE" confirms the OS *does* issue sub-file required ranges in real workloads, and [MS-OP]
is explicit that for any range **not** ending at EoF, "**both offset and length are 4KB
aligned**" — a hard requirement, not advice. So:
- If the OS ever issues a required range like `100000..100512` (start not 4KB-aligned, end not
  at EoF), our verbatim `write_at(buf, 100000)` violates the alignment rule →
  `CfExecute` returns `STATUS_CLOUD_FILE_INVALID_REQUEST` (`0x8007017C`).
- `proxy.rs:97` then does `command::Write::fail(...).unwrap()`; if that secondary `CfExecute`
  also fails (e.g. the transfer key is already torn down) it **panics across `extern "system"`
  = UB** (Wave-1 O2, but now with a concrete trigger).

So C3 is **not** "OK today" — it is "OK *only* while every consumer reads whole-file and the OS
chooses full hydration." That is an OS-policy bet, and [MS-OP] + the CreateFileMapping bug report
show the bet is not guaranteed. The correct status is **RISK, conditionally reachable today**,
not OK.

**Citation.** [MS-OP] (4KB alignment for non-EoF ranges, verbatim quote above). MS Q&A
"[Bug Report] cfapi: CreateFileMapping causes full placeholder hydration"
(learn.microsoft.com/answers/questions/2103011). [CRATE] `commands.rs:73-84` (no alignment
applied), `proxy.rs:97` (`.unwrap()` on fail). App `cfprovider.rs:70,84-88`.

**Severity.** **RISK** today (BUG/UB the moment a sub-file required range arrives, which the
CreateFileMapping report shows is reachable). Becomes a hard **BUG** if hydration is ever
switched off `Full`.

**Fix.** Align down `start` to a 4KB boundary and align `len` up to a 4KB multiple, clamped to
the logical size (the standard `len = 4096*((len+4095)/4096)` then `min(size-start_aligned)`);
read from the aligned start; write the aligned buffer. Drop the bare `.unwrap()` on the fail
path or wrap handlers in `catch_unwind`.

---

## 5. Two labels that sanitize to the same string — collision on BOTH id and path

**Input.** User saves two connections labelled `My Drive` and `My-Drive` (or `My_Drive`), then
opens a file from each.

**Code path.** `provider_id` maps every non-alphanumeric to `_` (`cfprovider.rs:168-173`) →
both become `SmartExplorer_My_Drive`. `cfsync::san` maps `/\:*?"<>|` and controls to `_`
(`cfsync.rs:30-37`) but **keeps space and `-`**, so `san("My Drive")="My Drive"` while
`san("My-Drive")="My-Drive"` — these *differ*. **The two sanitizers are inconsistent.**

**Predicted outcome — NEW, sharper than Wave-1 G1.** Wave-1 said id *and* path collide. That is
only half right and I correct it:
- **SyncRootId collides** (both `SmartExplorer_My_Drive`, same SID, empty account →
  identical `SyncRootIdBuilder…build()`, `sync_root_id.rs:99-109`). Second connection's
  `is_registered()` returns true (`cfprovider.rs:193`) so it **skips registration and reuses the
  first root's display name/icon/path** — silent identity confusion.
- **But the `local_root` paths do NOT collide** for `My Drive` vs `My-Drive`, because `san`
  keeps space and hyphen. So `ensure_mounted` builds two *different* `local_root`s
  (`conn_root_dir`, `cfsync.rs:40`), inserts two registry entries under two different keys
  (`cfprovider.rs:185-226`), and calls `Session::connect` on the **second** folder using a
  `SyncRootId` that is **already registered to the first folder**. Per MS, a sync root id maps to
  one registered path; `CfConnectSyncRoot` on a folder that isn't the registered path for that id
  yields `STATUS_CLOUD_FILE_NOT_UNDER_SYNC_ROOT` / connect failure. The failure branch then runs
  `sync_root_id.unregister()` (`cfprovider.rs:222`) — which **unregisters the FIRST, still-live
  connection's root** (Wave-1 S1, now with a concrete trigger): the first Drive's placeholders
  stop hydrating mid-session.
- The *truly* colliding case (`My Drive` vs `My_Drive` after `provider_id`, but `san` also
  differs: `"My Drive"` vs `"My_Drive"`) — note **even `My Drive`/`My_Drive` don't collide on
  path** (space vs underscore). I could not construct two labels that collide on the path under
  `san` but not on the id under `provider_id`; the inconsistency means **id-collision without
  path-collision is the reachable bug**, which is *more* damaging (cross-root unregister) than
  the path fight Wave-1 described.

**Citation.** [CRATE] `sync_root_id.rs:99-109` (id build), `:142-148` (is_registered),
`connect.rs` + `session.rs:58-94` (connect to path); error `error.rs:101`
`NotUnderSyncRoot → STATUS_CLOUD_FILE_NOT_UNDER_SYNC_ROOT`. App `cfprovider.rs:168-173, 185-226`,
`cfsync.rs:30-37`.

**Severity.** **BUG.** Two same-`provider_id` labels make the second open tear down the first's
sync root.

**Fix.** Use *one* sanitizer for both id and path, and make the id injective (append a short
hash of the full label, or use the stored connection's stable unique key). Never `unregister()`
a root that this call did not just `register()`; track "did I register this" per call.

---

## 6. Close the app while the editor is open, then save

**Input.** Open a CfAPI placeholder (hydrated), keep the editor open, quit Smart Explorer, then
save in the editor.

**Code path.** On process exit the `registry()` static (`cfprovider.rs:163`) is **not** dropped
(Rust does not run destructors of `static`s at normal exit), but the process teardown closes
handles; if the `Connection` *is* dropped (e.g. on an orderly shutdown that clears the map) its
`Drop` runs `CfDisconnectSyncRoot(...).unwrap()` (`connect.rs:57-66`).

**Predicted outcome — NEW nuance over Wave-1 I1/I2.**
- (a) **Not-yet-hydrated files become permanently inert *for that session*.** After
  disconnect, `filter_from_info` upgrades a `Weak` that is now dead → returns `None` →
  callbacks **silently no-op** (`proxy.rs:295-313`). Any file the editor lazily faults in after
  disconnect (e.g. on save the editor re-reads regions) gets **zero bytes / I/O error**, because
  `fetch_data` never runs. A save that reads-modify-writes a dehydrated region can corrupt.
- (b) **The placeholder folder survives** — disconnect ≠ unregister (`connect.rs:14-15` doc:
  "does NOT mean the sync root will be unregistered"). Files already hydrated remain as normal
  files; dehydrated placeholders remain as 0-content placeholders with **no provider to hydrate
  them** → opening one in Explorer after the app exits shows a stuck/erroring file until the app
  is relaunched.
- (c) **Next launch's `ensure_mounted`:** `is_registered()` is true (root persisted), so it
  **skips** `register()` and goes straight to `Session::connect` (`cfprovider.rs:193-217`),
  re-arming the callbacks → previously-dehydrated placeholders hydrate again. This path is
  fine *only because* registration is persistent; the NEW risk is the **window between exit and
  relaunch** where (a)/(b) bite, plus: if the *exit* happened to drop the Connection while a
  hydration was in flight, `CfDisconnectSyncRoot` can return non-S_OK and `.unwrap()`
  (`connect.rs:59`) **panics during shutdown** (ugly crash dialog), and the `Drop` busy-waits up
  to ~300 ms per `ReadDirectoryChangesW` cycle (`session.rs:177`, `connect.rs:62-64`) — a
  shutdown hang.

**Citation.** [CRATE] `proxy.rs:295-313` (Weak upgrade → no-op), `connect.rs:14-15, 57-66`
(disconnect not unregister; `.unwrap()`; busy-wait), `session.rs:118-210` (watcher join).

**Severity.** **RISK** (data-integrity hazard on save-after-exit; shutdown panic/hang). The save
itself routes through the app's mtime poller, which is gone once the app exits — so **the
edit is silently never uploaded** (see Finding 7) — making this also a **silent data-loss BUG**.

**Fix.** Keep the app alive (tray) while any `remote_edits` entry is outstanding; on exit, flush
pending uploads and either keep the Connection or convert open placeholders to plain files.

---

## 7. Editor save-back on a HYDRATED placeholder vs the NotSupported callbacks — Wave-1's deferred gap

**Input.** Open a placeholder, edit, save. Modern editors save by one of: in-place write;
write-temp-then-`ReplaceFile`/rename-over; or delete-then-recreate.

**Code path.** Save-back is detected purely by the app's mtime poller
`poll_remote_edits` (`app.rs:3226-3274`): it watches `e.temp` (the placeholder path), and on a
stable mtime change spawns `upload_file` (`app.rs:700-710`). The CfAPI filter only overrides
`fetch_data` + `fetch_placeholders`; **`delete`, `rename`, `dehydrate` default to
`Err(CloudErrorKind::NotSupported)`** (`sync_filter.rs:85-111`) and ARE registered
(`proxy.rs:34-77`).

**Predicted outcome — NEW; this is the gap Wave-1 explicitly deferred (N1/M1).**
- **In-place save:** mtime of `e.temp` advances → poller re-uploads. Works.
- **Atomic save via rename/ReplaceFile (Word, many editors):** the editor renames `~tmp` over
  the placeholder. CfAPI fires `NOTIFY_RENAME` → our default `rename` returns
  `NotSupported` → `command::Rename::fail` sets `STATUS_CLOUD_FILE_NOT_SUPPORTED`
  (`commands.rs:376-393`) → **the OS refuses the rename, the editor's save FAILS** with a cloud
  error. The user sees "cannot save"; nothing is uploaded. **This is a hard save-blocking BUG
  for any editor that saves atomically — i.e. most of them.**
- **Delete-then-recreate save:** `NOTIFY_DELETE` → default `delete` → `NotSupported` → the OS
  **blocks the delete** → save fails the same way. Additionally, even if the new file is created
  fresh (outside placeholder semantics), the poller is watching the *old* `e.temp` path; the
  recreated file may not advance the watched inode's mtime as expected.
- **mmap/dehydrate interplay:** if Storage Sense dehydrates the file between edits,
  `NOTIFY_DEHYDRATE` → `NotSupported` → `STATUS_CLOUD_FILE_NOT_SUPPORTED`; combined with the
  in-sync mark (`mark_in_sync`, `cfprovider.rs:121`), a dehydrate request is *refused*, pinning
  hydrated copies on disk (not data loss, but defeats the point of placeholders).

The module doc claims "save-back is handled by the app's edit-watch" — **that assumption only
holds for in-place writes.** For atomic-save editors the OS-level rename/delete refusal happens
*before* any mtime change the poller could see, so save-back is not merely missed, it is
**actively prevented by our own (defaulted) filter**.

**Citation.** [CRATE] `sync_filter.rs:85-111` (delete/rename/dehydrate → NotSupported),
`proxy.rs:227-275` (callbacks registered & wired to `command::*::fail`), `commands.rs:335-393`
(`Delete::fail`/`Rename::fail` set the NotSupported status), `error.rs:100`
(`NotSupported → STATUS_CLOUD_FILE_NOT_SUPPORTED`). App `app.rs:3226-3274` (mtime poller),
`cfprovider.rs:121` (`mark_in_sync`).

**Severity.** **BUG** (atomic-save editors cannot save into the sync root; the headline
"edit in any app, save back automatically" promise fails for Word/Excel/most editors).

**Fix.** Implement `delete`/`rename` to `ticket.pass()` (approve) so the OS performs them, and
detect the resulting file to drive upload; or override them to approve and re-arm the watch on
the new name. At minimum, approve rename/delete so saves aren't blocked.

---

## 8. Directory with thousands of entries: the one-shot DISABLE_ON_DEMAND_POPULATION constraint + 60 s

**Input.** A Drive folder with, say, 50,000 children (Drive allows it). Browse into it.

**Code path.** `fetch_placeholders` calls `list_dir` (paginates 1000/req,
`gdrive.rs:431-468`), builds one `Vec<PlaceholderFile>` (`cfprovider.rs:102-128`), and one
`pass_with_placeholder` (`cfprovider.rs:129`). The crate **always** sets
`CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION` with
`PlaceholderTotalCount = len` (`commands.rs:172-187`).

**Predicted outcome — NEW operational angle.** Wave-1 (J1) noted the "must return all" semantics.
I add the concrete failure: the **whole** enumeration is one synchronous callback that must
finish within the CfAPI **60 s** I/O timeout. For 50k entries, `list_dir` makes ~50 sequential
paginated HTTPS round-trips *plus* per-child `download_name` work (`cfprovider.rs:118`) — and
`download_name` calls `mime_of`, which for any child not already cached issues **another Drive
HTTP GET** (`gdrive.rs:184-195`). On a freshly-listed dir the mimes are cached
(`gdrive.rs:453-458`), so this is usually one network pass, but the build is still O(n) and a
single multi-MB `Vec<PlaceholderFile>` each leaking a boxed blob (`placeholder_file.rs:92`
`Box::leak`) → **n permanent leaks per enumeration** (the leak is reclaimed only in
`PlaceholderFile::Drop`, which *does* run after the slice is consumed — so not a true leak, but
the transient peak is n×(path+struct)). If the listing exceeds 60 s the OS issues
`CANCEL_FETCH_PLACEHOLDERS` with `IO_TIMEOUT` (`info.rs:164`), our in-flight `pass` is moot, and
because the cancel-path sets nothing, the directory is left **empty and marked
DISABLE_ON_DEMAND_POPULATION is *not* applied** (only the success path sets it) → next access
re-fires `fetch_placeholders` and re-attempts the full 50k listing → **repeated 60 s stalls**
every time the user touches the folder.

**Citation.** [CRATE] `commands.rs:172-187` (one-shot, DISABLE flag, total count),
`info.rs:160-174` (cancel/timeout), `placeholder_file.rs:92` (`Box::leak` per blob). App
`cfprovider.rs:102-129`, `gdrive.rs:184-195, 431-468`.

**Severity.** **RISK** (large but legal Drive folders → repeated 60 s hangs; no streaming
fallback because the crate hard-codes DISABLE_ON_DEMAND_POPULATION).

**Fix.** The crate can't stream without a patch; mitigate by reporting progress
(`Request::reset_timeout` if exposed) and/or capping/paging at the app level isn't possible under
DISABLE — so the real fix is a crate patch to drop the DISABLE flag and stream. Short term:
warn on huge dirs and fall back to Temp mode.

---

## 9. `populate_to` swallows a `fetch_placeholders` error → ShellExecute of a nonexistent path

**Input.** Open a deep file while the backend is offline/auth-expired, or while an ancestor dir
errors during `list_dir`.

**Code path.** `populate_to` (`cfprovider.rs:142-160`): `drain` does
`if let Ok(rd) = std::fs::read_dir(dir) { for _ in rd.flatten() {} }`. Then `open_file`:
`if dest.exists() { open_path } else { error_msg }` (`app.rs:3128-3143`).

**Predicted outcome — NEW; partially REFUTES Wave-1 K-note (which called populate_to "valid").**
Wave-1 affirmed `populate_to`. I find the **silent-failure mode they missed**: when
`fetch_placeholders` for an ancestor *fails* (backend offline → `list_dir` `Err` → `cerr` →
`CreatePlaceholders::fail` sets `Unsuccessful`), the OS-side `read_dir` of that ancestor still
*returns* (it returns whatever placeholders already exist, possibly none) — and `populate_to`
**ignores the result entirely** (`for _ in rd.flatten() {}` discards items *and* the per-entry
errors; the outer `if let Ok(rd)` only catches the *open* error, not enumeration errors). So a
failed population is indistinguishable from an empty dir. The leaf placeholder is never created,
`dest.exists()` is false → the code *does* now show the German "Platzhalter wurde nicht erzeugt"
error (`app.rs:3137`) rather than ShellExecuting a missing path — so the **most-recent code
avoids the silent ShellExecute no-op Wave-1 feared in the temp-path days**. BUT: the error
message tells the user "switch to Temp mode" and gives no hint that the *backend is offline* —
because `cerr` collapsed the real `NetworkUnavailable`/`AuthenticationFailed` into
`Unsuccessful` (`cfprovider.rs:29`) and `fetch_placeholders` failures are never surfaced to the
app at all (the app only sees `dest.exists()==false`). So: **not a silent ShellExecute, but a
mis-attributed error** — the user is told to change settings when the real fix is to reconnect.

**Citation.** App `cfprovider.rs:142-160` (drain discards items+errors), `app.rs:3128-3143`
(exists-or-error). [CRATE] `commands.rs:190-212` (`CreatePlaceholders::fail`), `error.rs:29-31`
(`cerr → Unsuccessful`).

**Severity.** **RISK** (mis-attributed error → user disables the feature instead of
reconnecting). Lower than Wave-1's feared silent no-op, because the `dest.exists()` guard now
catches it — credit to the current code; the residual bug is diagnostics, not silence.

**Fix.** Thread the backend error out of population (e.g. stash the last `fetch_placeholders`
error in the filter and read it in `open_file`), and map `cerr` to specific kinds so the user is
told "offline / re-auth", not "switch to Temp."

---

## 10. Re-opening the same file twice quickly

**Input.** Double-click the same Drive file twice within ~1 s.

**Code path.** `open_file` (`app.rs:3096-3144`) each time: `ensure_mounted` (idempotent via the
registry `contains_key`, `cfprovider.rs:185`), then
`self.remote_edits.retain(|e| e.temp != dest)` + push (`app.rs:3116-3127`), then
`populate_to` + `open_path`.

**Predicted outcome — NEW small finding.** No double-mount (registry is keyed by `local_root`,
second `ensure_mounted` short-circuits — correct). The `remote_edits` dedup is correct *only*
because it keys on the exact `dest` PathBuf and `retain`s before push, so the second open
replaces the first entry rather than duplicating — **but it resets `baseline_mtime` to
`i64::MAX`** (`app.rs:3123`). If the user had *already edited and the file's mtime advanced*
between the two opens, the second open's `retain`+push **discards the in-progress edit-tracking
state** (the old `seen_mtime`/`baseline_mtime`) and re-baselines to "arm on first sight." On the
next poll the sentinel branch (`app.rs:3243`) just re-baselines to the *current* (edited) mtime
and `continue`s — so **a save made between the two opens is silently treated as the new baseline
and never uploaded.** Reachable: open, type a char (autosave bumps mtime), double-click to
re-open before the 1.5 s poll → edit lost. Also: two concurrent `fetch_data` for the same file
can run on different CfAPI worker threads (callbacks are arbitrary-threaded, `sync_filter.rs:10`)
and both call `backend.open_read` → two export round-trips; harmless but wasteful, and on a
stateful backend (FTP single connection, `ftp.rs` lock) they serialize.

**Citation.** App `app.rs:3116-3127` (retain+push, baseline reset), `:3243-3247` (sentinel
re-baseline). [CRATE] `cfprovider.rs:185` (registry idempotent), `sync_filter.rs:10-12`
(arbitrary-thread callbacks).

**Severity.** **BUG** (edge): an edit made in the gap between two opens of the same file is
dropped from the upload watch.

**Fix.** On re-open, if a `remote_edits` entry already exists for `dest`, **keep** its
`baseline_mtime`/`seen_mtime` instead of resetting to `i64::MAX`.

---

## 11. NEW (devised): placeholder name ≠ launched name for Google Docs — open finds nothing

**Input.** Open a Google Doc whose title contains a space *and* a `san`-only char, e.g.
`Report: Q3` (Drive titles may contain `:`).

**Code path.** Provider creates the placeholder with `RelativeFileName = download_name(...)` =
`"Report: Q3.docx"` (raw, `cfprovider.rs:118-120`) — which **fails** NTFS validation (`:`), see
Finding 1. The app, meanwhile, computes the launch target with `san(leaf)` =
`"Report_ Q3.docx"` (`cfsync.rs:66`, `:` → `_`). So **even if** the placeholder somehow existed,
the app launches `Report_ Q3.docx` while the provider named it `Report: Q3.docx` → mismatch →
`dest.exists()` false → "Platzhalter nicht erzeugt." The provider-side name and the app-side
name are produced by **two different sanitizers** (`download_name` raw vs `san`) and **cannot
agree** for any name containing a `san`-replaced char.

**Predicted outcome.** For any remote name containing one of `:*?"<>|`/control/`/`/`\`, the
CfAPI open path is **structurally broken**: provider rejects the name (Finding 1) *and* the
launch path wouldn't match even on success. The module doc's own warning
(`cfsync.rs:60-67`, "must match the placeholder name … or the open finds nothing") is
**violated by the code itself**.

**Citation.** App `cfprovider.rs:118-120` (raw display name to `PlaceholderFile::new`),
`cfsync.rs:30-37, 64-67` (`san` on the launch leaf), `app.rs:3106-3133`. [CRATE]
`placeholder_file.rs:20-31` (no sanitization).

**Severity.** **BUG** (any special-char remote name is un-openable in CfAPI mode; the two
name-derivation paths are provably inconsistent).

**Fix.** Apply `cfsync::san` to the display name in `fetch_placeholders` so the provider and the
app use the identical leaf, and keep the true remote path/id in the blob.

---

## 12. NEW (devised): `populate_to` cannot create the leaf for a hidden/locked/error child mid-listing

**Input.** A directory whose `list_dir` succeeds but contains one entry that fails placeholder
creation (Finding 1 case b), positioned *before* the target leaf in the batch.

**Code path.** `pass_with_placeholder` runs the **whole batch** in one `CfExecute`
(`commands.rs:162`); the first failing entry's HRESULT is returned and `cerr`'d to
`Unsuccessful`. Whether later entries (including the target leaf) are created depends on the
platform's batch semantics, which the crate does not inspect.

**Predicted outcome.** Because the crate discards per-entry `Result` (`commands.rs:168`), the app
has no way to know the leaf *was* created even if it was — and if the batch aborts at the bad
entry, the leaf after it is **not** created. `dest.exists()` false → feature appears broken for
a perfectly-named file *because a sibling was badly named*. This compounds Finding 1 into a
"one poisoned sibling breaks unrelated files" class.

**Citation.** [CRATE] `commands.rs:162-188` (batch, `result()` no-op),
`placeholder_file.rs:99-101` (unused per-entry result). App `cfprovider.rs:129`, `app.rs:3128`.

**Severity.** **BUG** (collateral: a bad sibling blocks a good file).

**Fix.** Create placeholders individually (or read back each `result()`), so one bad entry can't
prevent the target leaf.

---

# Ranked "most dangerous" — top 5

1. **Finding 3 — Google Docs hydrate empty / cannot hydrate (declared size 0 vs N-byte
   export).** Deterministic, hits the Drive backend's flagship feature, refutes Wave-1's
   milder "size differs" framing. Users get 0-byte .docx files. **BUG.** [MS-SIZE]/[MS-OP].

2. **Finding 7 — atomic-save editors are BLOCKED by our defaulted `rename`/`delete`
   callbacks (`NotSupported`).** The core promise "edit in any app, save back" fails for Word/
   Excel and most editors because the OS refuses the save's rename/delete. Wave-1 deferred this.
   **BUG.** [CRATE] `sync_filter.rs:85-111`.

3. **Finding 1+11+12 — special-char / NUL remote names break a whole folder (and `/`-in-title
   Drive files), with NUL = UB across FFI; the provider's name and the app's launch name use
   two inconsistent sanitizers so the open can never match.** Very reachable on Drive.
   **BUG + UB.** [CRATE] `placeholder_file.rs:24`.

4. **Finding 5 — label sanitizer inconsistency: two labels collide on SyncRootId but not on
   path, so the second open `unregister()`s the first live root mid-session.** Cross-connection
   teardown + the S1 unregister hazard with a concrete trigger. **BUG.** [CRATE]
   `sync_root_id.rs:99-109`, `cfprovider.rs:222`.

5. **Finding 4 — non-4KB-aligned mid-file required range → `STATUS_CLOUD_FILE_INVALID_REQUEST`,
   then `.unwrap()` panic across FFI = UB.** Refutes Wave-1's *unverified* "OS always sends
   0..size" (C3); the CreateFileMapping bug report shows sub-file ranges are reachable under
   `Full`. **RISK→UB.** [MS-OP] + proxy `.unwrap()`.

(Runner-up: Finding 2b — deep but legal trees silently fail population at MAX_PATH=260 with no
`longPathAware` manifest; a clean NEW finding Wave-1's path analysis missed.)
