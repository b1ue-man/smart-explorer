# Android → Rust Transfer Contract (facts as of 2026-09-28)

Purpose: document exactly how the Android app today drives copy/move/paste/download/upload/
remote-to-remote transfers through the mobile JSON facade, as input to redesigning
`native/src/transfer/`. Facts only, with file:line citations; design is not judged here.

## Files actually read
Rust (fixed set): `native/src/mobile/mod.rs`, `core/events.rs`, `core/entry.rs`, `core/error.rs`,
`core/location.rs`, `os/shared/runtime.rs`, `os/shared/edits.rs`.
Rust (selected from `os/shared/domains/`, by name, after `ls` of that folder): `mod.rs` (dispatch
table — needed to see which prefixes route where), `share_exec.rs`, `share_peers.rs`,
`share_requests.rs`, `share_state.rs`, `share_status.rs`, `background.rs`.
Docs: `docs/superpowers/plans/2026-09-25-android-apk/api.md` §2–§4.5 (lines 56–190) and §4.8–§5
(lines 274–363).
Android (fixed set): `core/Core.kt`, `core/NativeBridge.kt`.
Android (selected from `ui/files/`, by name, after `ls` of that folder): `FileActions.kt`,
`FilesViewModel.kt`, `FilesModels.kt`, `FilesOverlays.kt`, `FileDialogs.kt`, `FilesTopBar.kt`,
`FilesBars.kt`.

## 1. Facade methods that start transfers

All copy/move go through **one** JSON method, `fs.transfer`, regardless of local/remote/share
direction — there is no separate "download"/"upload"/"paste" verb; direction is implicit in
whether `sources`/`targetDir` resolve to local or remote backends:

- `fs.transfer {sources:[location], targetDir, mode:"copy|move", conflict:"skip|replace|keepBoth", filter:Filter?, baseDir:String?}` → `{taskId}` (api.md:136–140). `move` only local→local, else `unsupported` ("Verschieben von/zu Remote wird nicht unterstützt"); `conflict` only affects local→local (remote targets always auto-number, "Name (2)").
- `fs.conflicts {sources:[location], targetDir}` → `{names:[String], choosable:Boolean}` (api.md:133–135), a pre-check called before `fs.transfer`, not a recursive scan.
- `fs.import {files:[{fd:Int, name, size:Long?}], targetDir}` → `{taskId}` (api.md:163–166): the Android "receive shared content" path — Kotlin hands over `ParcelFileDescriptor.detachFd()` descriptors, Rust owns/closes them and streams straight into the target with progress.
- `fs.materialize {locations}` → `{taskId}`, `result={paths}` (api.md:152–153): downloads remote files into `<cache>/share/` for Android's outbound "Share to…" intent.
- `fs.fetch {location}` → `{taskId}`, `result={localPath, mime, editId}` (api.md:147–151): downloads one remote file for local open/edit.
- `fs.uploadEdit {editId, mode:"overwrite|copy", force}` → `{taskId}` (api.md:155–161): uploads a changed local copy back; `mode:"copy"` re-uses the transfer engine.
- `fs.extract`/`fs.delete`/`fs.discardEdit` also return `{taskId}` (api.md:141–142, 162, 167–169) but are not transfers.

Kotlin call sites: `FilesApi.transfer(sources, targetDir, mode, conflict, filter, baseDir)` from
`FileActions.kt:218–225` (`transferNow`, reached via `startTransfer`/`runTransfer`,
`FileActions.kt:204–215`); `FilesApi.conflicts` at `FileActions.kt:205`; `FilesApi.materialize` at
`FileActions.kt:98`; `FilesApi.fetch` at `FileActions.kt:72`; `FilesApi.uploadEdit` at
`FileActions.kt:419`; `ShareIntentHandler.importInto(targetDir)` (→ `fs.import`) at
`FileActions.kt:437`. `FilesApi.*` itself (the Kotlin→JSON method-name/arg mapping) is under
`android/.../api/FilesApi.kt`, outside the permitted `core/`+`ui/files/` set — **unresolved**, see
below; the JSON shapes above come from api.md, not from reading that file.

Routing on the Rust side: `mobile/mod.rs:106–117`'s `call()` always dispatches through
`dispatch::dispatch(rt, method, &args)` (`os/shared/dispatch.rs`). That file, plus
`os/shared/transfer.rs`, `os/shared/drive.rs`, `os/shared/import.rs`, `os/shared/delete.rs`,
`os/shared/fs_list.rs`, `os/shared/fs_edit.rs` are declared as siblings of `domains/` directly
under `os/shared/` (`mobile/mod.rs:17–68`) — **not inside `os/shared/domains/`** and not
individually named in my brief, so they are out of the permitted reading scope. Confirmed from
`os/shared/domains/mod.rs:55–70`: its `dispatch()` only matches `sync|bg|conn|gdrive|share|analyze|
reclaim|update` prefixes — `fs.*` is not among them, so `fs.transfer`/`fs.import`/`fs.delete`/
`fs.extract` must be handled earlier, in the out-of-scope `os/shared/dispatch.rs` (likely calling
into `os/shared/transfer.rs`, `import.rs`, `delete.rs`). **This means the exact Rust functions
behind `fs.transfer` itself (`TransferLane`, `download_paths_progress`, `copy::start_copy_*`, or a
"local copy engine") could not be confirmed within the permitted file set — unresolved.**

The one handler I *could* read in full, `os/shared/edits.rs` (explicitly allowed), shows the real
`crate::transfer` call shape for the fetch/upload-copy paths — see §5.

## 2. Android in-app clipboard (copy/cut → paste)

- `copyToClip(tab, entries, move)` (`FileActions.kt:121–134`) stores only location strings plus
  flags in a `Clip` (`FilesModels.kt:38–44: sources, move, local, filter, baseDir`) — **no
  download, no scan** happens at copy/cut time.
- `paste(tab)` (`FileActions.kt:137–149`) builds a `TransferRequest` (`FilesModels.kt:47–55`) and
  calls `startTransfer`, which does one `fs.conflicts` name-existence check
  (`FileActions.kt:205`), then either shows a conflict dialog or calls `transferNow` →
  `FilesApi.transfer` (`fs.transfer` task) directly (`FileActions.kt:204–231`). No remote content
  is pre-downloaded and no recursive pre-scan of the transfer runs before the task starts.
- Move guard, client side: `paste()` refuses when `current.move && (!current.local ||
  tab.listing?.isLocal != true)` with "Verschieben zu Remote wird nicht unterstützt – bitte
  kopieren" (`FileActions.kt:144–147`) — matches the server-side restriction in api.md:136–140.
  **Move is not supported for any remote endpoint on Android (same as desktop).**
- Copy is not restricted this way: `sources`/`targetDir` are independent `location` strings, and
  `Loc::parse` treats `share://…` exactly like `sftp://`/`ftp://`/`webdav://`/`smb://` — a scheme
  parsed into `LocKind::Share` via the same `authority`+`path` logic (`core/location.rs:87–108`),
  reused through the same `resolve_loc`/`resolve_live` backend pool (`os/shared/runtime.rs:177–
  234`). No share-specific transfer code was found in the `share_*.rs` domain files read — their
  `share.*` JSON methods (`os/shared/domains/mod.rs:134–166`) cover pairing, device/room/export
  management, status polling and remote **command exec** (`share.exec`, a shell command on a peer,
  `share_requests.rs:155–225`; not a file copy), never a transfer verb. This indicates **peer-share
  copy reuses the generic `fs.transfer` path** with a `share://` location — copy-to/from/between
  peer shares should work the same as any other remote pair; only local↔local move is special-cased.
  (The generic `fs.transfer` handler itself is out of scope, so this is inferred from the location
  model + absence of a share-transfer verb, not directly observed.)
- `outermost()` (`FileActions.kt:176–201`) deduplicates a selection against an already-fetched
  `ScanWindow` (recursive/filtered view) so nested selected rows aren't transferred twice — it reads
  already-scanned rows (`window.fetchAll`), it does not scan the transfer itself.
- `requestTransferTo` (`FileActions.kt:152–169`, "Kopieren/Verschieben nach…") is the same flow via
  a location picker instead of clipboard paste; `onPicked` (`FileActions.kt:407–413`) starts the
  same `TransferRequest`.

## 3. Task model and progress events (exact JSON field names)

`Task` (api.md:83–86): `id, kind, title, state:"queued|running|done|failed|canceled", doneBytes:
Long, totalBytes:Long, doneItems:Long, totalItems:Long, rateBps:Long, message:String?,
errors:[{path,message}], result:Json?, startedMs:Long, finishedMs:Long?`. `kind` includes
`transfer, delete, scan, properties, open, upload, materialize, extract, index, sync, mirror,
analyze, reclaim, trash, oauth, share, exec, update` (api.md:87–88).

Events via `pollEvents` (api.md:97–107): `{"type":"task","task":Task}` (bundled progress, ≤4/s per
task, and terminal state), plus `share`, `shareRequest{count}`, `edits`, `jobs`,
`openUrl{url}`, `error{action,message}`, `volumes`.

Producer side (`os/shared/runtime.rs`, in scope): `TaskCtx` (struct at 392–397) exposes
`id()` (399–402), `cancelled()`/`cancel_flag()` (404–410, backed by a shared `Arc<AtomicBool>`),
`progress(done_bytes, total_bytes, done_items, total_items)` (412–423 → maps 1:1 to
`doneBytes/totalBytes/doneItems/totalItems`), `message(text)` (425–427 → `message`),
`error(path, message)` (429–431 → appends to `errors`), and `set_failure_result(value)` (433–437,
e.g. `{"conflict":true}` for a failed `fs.uploadEdit`, api.md:155–161). `finish_task`
(356–374) turns the closure's `Result` plus `cancelled()` into `TaskState::Done/Failed/Canceled`
and the final `message`/`result`. Cancellation itself (`task.cancel {id}` flipping the
`AtomicBool`) lives in `core/tasks.rs` (`TaskTable`/`TaskRecord`), which is **not** one of the four
named core files (`events.rs, entry.rs, error.rs, location.rs`) — out of scope, so the exact
cancel-propagation code was not read; only the `TaskCtx` consumer side is confirmed.

Consumer side, Kotlin: `Core.pumpEvents()` (`Core.kt:176–202`) polls `NativeBridge.pollEvents`
every ~1 s, parses batches, and for a `CoreEvent.Task` upserts into a `TaskStore`
(`Core.kt:188`) exposed as `Core.tasks: StateFlow<List<TaskInfo>>` (`Core.kt:57–58`).
`reportTask(task, success, failure)` (`FileActions.kt:33–42`) requires `task.state` (`"done"` /
`"canceled"` / other), `task.errors` (checks `.isEmpty()`/`.size`), and `task.message`; a
"Details" action opens the transfers sheet (`AppNav.send(NavRequest.ShowTransfers)`,
`FileActions.kt:35`, wired to `ui.transfers.TransfersSheet` in `FilesOverlays.kt:127` — that
package is outside `ui/files/`, so its exact per-row rendering is unresolved, see below).
`OpenProgressDialog` (`FileDialogs.kt:296–327`) reads `task.doneBytes`/`task.totalBytes` through
`Format.fraction`/`Format.size` (312–319), falling back to an indeterminate bar when the fraction
is null (e.g. `totalBytes==0`) — so Kotlin tolerates a zero/unknown total, it does not require a
non-zero total up front. `PasteBar` (`FilesBars.kt:34–53`) and `ConflictDialog`
(`FileDialogs.kt:211–240`) are UI around the transfer *start*, not its progress.

**Unresolved:** the requested `TransferMsg`/`TransferProgress` names (presumably `crate::transfer`'s
own progress-message type) and their exact field-by-field mapping onto
`TaskCtx::progress/message/error` happen inside `os/shared/drive.rs`'s `run_transfer` helper and/or
`os/shared/transfer.rs` — both out of scope. Kotlin's own `TaskInfo`/`CoreEvent`/`CoreEvents`/
`TaskStore` classes (mirroring the JSON above) live under `core/` but are not `Core.kt` or
`NativeBridge.kt`, so they were not read either; field names above are reconstructed only from call
sites (`task.state`, `task.errors`, `task.message`, `task.doneBytes`, `task.totalBytes`, `task.id`).

## 4. Android-specific constraints

- **Content-provider stream uploads / share-receive:** `fs.import` (api.md:163–166) is the
  mechanism — Kotlin detaches file descriptors via `ParcelFileDescriptor.detachFd()` and Rust reads
  them as a progress-reporting task directly into the (local or remote) target, numbering occupied
  names. Kotlin trigger: `ShareIntentHandler.importInto(targetDir)` called from
  `FilesViewModel.importShared` (`FileActions.kt:436–440`) and surfaced by `ReceivePicker`
  (`FilesOverlays.kt:146–168`). **Unresolved:** `ShareIntentHandler.kt` lives under
  `android/.../system/`, not `ui/files/` or `core/`, so it is out of scope; the Rust-side reader
  (presumably `os/shared/import.rs`, declared at `mobile/mod.rs:39–40`, and the specific
  `upload_reader_progress` function named in my brief) is also out of scope — neither was located
  or confirmed.
- **App trash exclusion:** `Runtime::set_volumes` (`os/shared/runtime.rs:163–175`) calls
  `crate::apptrash::set_volumes(paths)` every time storage volumes are (re)configured, including
  from Kotlin's `sys.volumes` push (`Core.kt:204–211`, `pushVolumes`). The specific
  `apptrash::excluded_name` helper named in my brief was **not** encountered in any permitted file
  (likely in `os/shared/delete.rs` or `os/shared/trash.rs`, both out of scope) — unresolved.
- **Foreground service:** no Service class or foreground-service handling was found in the
  permitted files. `Core.kt` runs `init` and the event pump on plain daemon threads (`Core.kt:76,
  159, 197`, `thread(..., isDaemon = true)`), independent of any Activity/Service lifecycle.
  Whether a separate foreground Service keeps the process alive during a long transfer is
  **unresolved** — no such file exists under `ui/files/`, and it is not one of the two named
  `core/` files.
- **SAF/MediaStore/FUSE rename chain (`android_fs`):** not referenced anywhere in the permitted
  file set. **Unresolved** — presumably under `native/src/mobile/os/` (an Android path adapter) or
  `native/src/os/android/`, neither in scope.
- **ZIP-sourced edits:** confirmed as a related edge case — uploading an edited copy that came from
  inside a ZIP fails immediately with `permission` (api.md:161; matches the `is_app_internal`
  rejection pattern in `core/location.rs:153–155, 187–192`, used e.g. by `fs_list::reject_trash`-
  style guards called from `edits.rs:51, 180` for trash, though the ZIP-specific check itself sits
  in the out-of-scope `fs.uploadEdit` handler).

## 5. Public `crate::transfer` items confirmed used by mobile code

Only `os/shared/edits.rs` and `os/shared/runtime.rs` (both explicitly in scope) show real call
sites; everything else routes through out-of-scope files (§1, §3, §4):

- `crate::transfer::MAX_ACTIVE_TRANSFERS` — `os/shared/runtime.rs:130`, bounds the `Slots`
  semaphore used by `spawn_transfer_task` (`runtime.rs:296–302`) so only a limited number of
  transfer-kind tasks run concurrently (other task kinds use unbounded `spawn_task`).
- `crate::transfer::TransferRequest` — imported `os/shared/edits.rs:13`. Variants observed:
  `TransferRequest::Download { backend, files, dest_local, filter }` (`edits.rs:116–122`, used by
  `fs.fetch`/`fs.materialize`'s `download_one`) and `TransferRequest::Upload { paths, backend,
  dest_root }` (`edits.rs:356–360`, used by `fs.uploadEdit` mode `"copy"`). Both are run through a
  local wrapper, `super::drive::run_transfer(ctx, request)` (imported `edits.rs:6`, called
  `edits.rs:115, 354`), whose own body is in the out-of-scope `os/shared/drive.rs` — so I can
  confirm the `TransferRequest` shape but not what `run_transfer` does with it internally (progress
  loop, cancellation wiring, retry) or whether `fs.transfer` itself reuses `TransferRequest` or a
  different request type.
- `crate::transfer::remove_owned_tree(root: &Path, dir: &Path) -> io::Result<()>` —
  `edits.rs:82, 130, 166, 270, 394`, used to clean up cached download folders under the edits
  cache root (on eviction, failed download, orphan sweep, and explicit discard).
- `crate::transfer::upload_file(backend: &dyn Backend, local_path: &Path, remote_path: &str)` —
  `edits.rs:333` (`fs.uploadEdit` mode `"overwrite"`), mapped through `ApiError::internal` on
  failure, so it returns a `Display`-able error type (exact signature not fully visible from the
  call site alone).

**Unresolved:** `TransferLane`, `download_paths_progress`, `copy::start_copy_*`, and whatever
`fs.transfer`/`fs.delete`/`fs.extract` actually call — none of their call sites fall inside the
permitted file set; they are presumably in `os/shared/transfer.rs`, `delete.rs`, `trash.rs` (all
out of scope, siblings of `domains/` under `os/shared/`, not individually named in the brief).

## Summary of unresolved items (with reasons)

1. Exact Rust functions/types behind `fs.transfer` (and `fs.delete`/`fs.extract`) — handler is
   `os/shared/transfer.rs` (+ `dispatch.rs`), a sibling of `domains/`, not inside it and not named.
2. `TransferMsg`/`TransferProgress` → `TaskCtx` field mapping — inside `os/shared/drive.rs`
   (`run_transfer`) / `transfer.rs`, out of scope.
3. `fs.import`'s Rust reader and `upload_reader_progress` — likely `os/shared/import.rs`, out of
   scope; its Kotlin caller `ShareIntentHandler.kt` is under `system/`, also out of scope.
4. `apptrash::excluded_name` — not found in any permitted file; likely `os/shared/delete.rs` or
   `trash.rs`, out of scope.
5. Foreground-service handling during transfers — no such code found in `core/Core.kt`,
   `NativeBridge.kt`, or `ui/files/*`; may live in an Android `Service` class elsewhere.
6. `android_fs` SAF/MediaStore/FUSE rename chain — not referenced anywhere in the permitted set.
7. `core/tasks.rs` (`TaskTable`/`TaskRecord`, incl. `task.cancel` propagation) — not one of the four
   named core files, so its internals (e.g. `rateBps` computation) are unconfirmed; only the JSON
   shape (api.md) and the `TaskCtx` producer API (`runtime.rs`) are confirmed.
8. Kotlin `FilesApi.kt`, `TaskInfo.kt`, `CoreEvent.kt`/`CoreEvents.kt`, `TaskStore.kt`,
   `ui/transfers/TransfersSheet.kt` — none are `core/Core.kt`/`NativeBridge.kt` or under
   `ui/files/`; field names used above are reconstructed only from call sites in the files read.
