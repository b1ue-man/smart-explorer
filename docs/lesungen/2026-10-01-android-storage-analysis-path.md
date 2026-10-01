# Android storage analysis and duplicate finder: the current path from screen to syscall (facts as of 2026-10-01)

Purpose: input for optimizing the Android storage analyses after a user report ("the analysis
stops early at `data/` because of permission issues; no further permissions can be granted;
Android 11+ blocks other apps' `Android/data` and `Android/obb` even with all-files access").
Facts only, with file:line citations; the design is not judged. What the Android file system
layer returns for `Android/data` / `Android/obb` is **not verifiable from the repository** and is
marked wherever it matters. Read-only reading; nothing was built, run or tested.

Date: 2026-10-01. Citation convention: bare file names are unique inside the surface read (list
below); `mod.rs` is always qualified (`analytics/mod.rs`, `reclaim/mod.rs`, `domains/mod.rs`,
`apptrash/mod.rs`, `local_access/mod.rs`).

## Key facts in five lines

1. Android analysis = `analyze.start` -> task `analyze` -> `crate::analytics::scan` (rayon, 2 workers by
   default) -> tree kept in memory -> pull-based `analyze.node` / `analyze.issues`. Duplicates =
   `reclaim.start` -> `crate::analytics::scan_reclaim` (single thread) -> `reclaim.groups`.
2. A child directory or entry that cannot be read (EACCES/EPERM or any I/O error) never ends the
   analysis scan: one issue is recorded (64 kept, the rest only counted), the directory becomes an
   empty node of size 0, the walk continues, the outcome is `Partial` (tree + "n Pfade nicht lesbar
   [Bericht]"). Only an unreadable **scan root** (or a cancel) ends it: `Failed`, no tree, error card.
3. No code on the chain special-cases `Android/data` / `Android/obb`. The helper
   `apptrash::hidden_app_folders_in` exists (its doc says Android lists the package folders there but
   refuses to open them) but is not called from `analytics/`, `local_access/` or `analyze.rs`.
4. Per entry the analysis walker does one `lstat`-equivalent (`DirEntry::metadata`) for files **and**
   directories; the duplicate walker stats only files. Progress to Kotlin = two atomics polled every
   250 ms (`files`, `bytes`); the current directory, `dirs` and the hashing counters are not forwarded.
5. Duplicates keep only the 200 largest files >= `minSize` as candidates; the sticky walk budget
   (1,000,000 entries / 128 MiB path text / depth 512) ends the whole duplicate walk; the mobile
   bridge never reads `scan_limit` or the truncation counters, and the errors it forwards to the task
   record are not read by Kotlin (none of this reaches the UI).

## Files actually read

Kotlin (all complete), under `android/app/src/main/java/app/smartexplorer/android/`:
`ui/analytics/{AnalysisScreen,AnalysisViewModel,DuplicatesScreen,DuplicatesViewModel,ScanPages,Treemap,TreemapView}.kt`,
`api/AnalyzeApi.kt`, `system/Storage.kt`, `core/Dtos.kt`.

Rust mobile facade, under `native/src/mobile/os/shared/` (complete): `domains/analyze.rs`,
`dispatch.rs`, `scan.rs`, `places.rs`, `runtime.rs`. Callees of `analyze.rs` outside the named list, only
the cited lines: `domains/mod.rs` lines 66 and 170-174 (dispatch arms, found by grep),
`domains/locations.rs` lines 7-51 (`is_local`, `location_for`, `join_segments`), `domains/args.rs`
lines 9-23 and 77-84 (`invalid`, `canceled`, `io_error`, `reject_app_internal`).

Rust analytics, under `native/src/analytics/`: `mod.rs`, `os/mod.rs`; `os/shared/{analytics.rs,
analytics_budget.rs, analytics_outcome.rs, analytics_backend.rs, remote.rs}` and `core/{progress.rs,
analysis_report.rs}` (complete); `core/analysis_transfer.rs` (lines 1-60), `core/tree_transfer.rs`
(1-40), `core/windows_analysis_transfer_task_tests.rs` (1-40), `os/shared/analytics_tests.rs` (1-220
plus the test-name list); `os/shared/reclaim/{mod,types,local,duplicates,budget,retention,util,cleanup,
backend,backend_duplicates}.rs` (complete). Not read: `reclaim/verify.rs` (GUI trash plan),
`reclaim/*_tests.rs` bodies. `analytics/os/windows/` is an empty directory.

Walker callees on the chain (functions only): `native/src/local_access/mod.rs` and
`local_access/os/linux_os.rs` (complete), `native/src/apptrash/mod.rs` (1-130) and
`apptrash/os/shared/store_tests.rs` (180-225), `native/src/agent_proto/os/shared/fs.rs` (40-100,
`is_pseudo_dir`).

Docs: `docs/superpowers/plans/2026-09-25-android-apk/api.md` §4.9 (lines 277-284, plus a heading grep),
`docs/ANALYTICS_ACCESS.md` (complete).

Greps inside the surface only (results quoted where relevant): `hidden_app_folders_in`, `Android/data`,
`Android/obb`, `statvfs`, `StorageStats`, `MediaStore`, `available_parallelism`,
`SMART_EXPLORER_ANALYTICS_THREADS`, `rayon`, `can_request_access`, `permission_denied`.

## 1. Call chain from the analysis screen to the scan

### 1.1 Kotlin

- `AnalysisScreen(vm, onClose)` switches on `vm.phase` (`ScanPhase.Setup|Scanning|Result`,
  AnalysisViewModel.kt:19; AnalysisScreen.kt:50-73). Setup page = `ScanSetupPage` (ScanPages.kt:40-93):
  the place card opens `LocationPickerDialog` (ScanPages.kt:81-92; its source is outside the surface);
  [Analysieren] calls `vm.start()` (AnalysisScreen.kt:59-60). Hint text: "Lokale Speicher, Verbindungen und
  Share-Geräte lassen sich analysieren." (AnalysisScreen.kt:62).
- `AnalysisViewModel.start()` (AnalysisViewModel.kt:58-99): cancels a still running previous task
  (60-63), resets state and sets `phase = Scanning` (64-69), then in `viewModelScope`:
  `AnalyzeApi.start(target)` (72) -> `taskId`; `FilesApi.awaitTask(id)` (74) -> `TaskInfo`;
  `when (task.state)` (75-93). `location` is a free string (AnalysisViewModel.kt:26).
- Pre-filling: `preselect(current)` suggests "the place shown in Dateien" (47-50); `request(target, start)`
  lets another screen start an analysis (52-56). Their callers are outside the surface.

### 1.2 JSON methods (api.md:277-284; AnalyzeApi.kt:43-70; arms at domains/mod.rs:170-174)

| JSON method | Rust handler | Answer |
|---|---|---|
| `analyze.start {location}` | `start_analysis` analyze.rs:150-159 | `{taskId}` (progress: `doneItems` files, `doneBytes`) |
| `analyze.node {taskId, path:[String]}` | `node` analyze.rs:227-272 | `{name,size,isDir,children[{name,size,isDir,childCount}],location}` |
| `analyze.issues {taskId}` | `issues` analyze.rs:274-294 | `{count, text}` |
| `reclaim.start {location, minSize}` | `start_reclaim` analyze.rs:296-309 | `{taskId}` |
| `reclaim.groups {taskId}` | `groups` analyze.rs:379-403 | `[{size, items:[{location, mtimeMs}]}]` |

Task control/progress go through `FilesApi.awaitTask` / `FilesApi.cancelTask` /
`rememberTask(taskId)` (AnalysisViewModel.kt:74,150; ScanPages.kt:98); the JSON method names behind
them were not read (outside the surface).

Routing: `dispatch.rs:7-17` handles only `sys|task|loc|fs|scan|index|trash` itself; `analyze` and
`reclaim` fall through to `domains::dispatch` (dispatch.rs:17), whose arm is `"analyze" | "reclaim"
=> analysis_method(...)` (domains/mod.rs:66). `scan.rs` (`scan.*`: validate/start/view/issues,
scan.rs:76-84) is the filtered recursive **search**; `analyze.rs` imports nothing from it
(analyze.rs:9-14). Search walks `crate::scanner` / `crate::rscan` (scan.rs:126-139), analysis walks
`crate::analytics`.

### 1.3 Location handling

- `checked_location` (analyze.rs:134-141): `location` required, `zip://` and `trash://` rejected
  (`reject_app_internal`, args.rs:77-84; runtime.rs:278-281), blank rejected.
- `is_local(location) = !location.contains("://")` (locations.rs:7-10). Local path -> local walker
  (analyze.rs:163-172, 183). Anything else -> `resolve_remote` (analyze.rs:145-148: `Runtime::resolve`
  + `vfs::sync_backend`, i.e. the uncached backend) and `scan_backend` (analyze.rs:181; see 2.7).

### 1.4 Root paths that are offered

- Rust `loc.roots` (places.rs:16, 43-123): `storage` entries are `rt.volumes()`
  (places.rs:44-63): `{id:"storage:<path>", label, subtitle:<path>, location:<path>, kind:"storage",
  removable}`; plus favorites, recent, connections, gdrive, devices, rooms, trash (places.rs:113-122).
  There is no "/" root and no capacity (total/free) field in a root (places.rs:23-32).
- Volumes come from Kotlin `Storage.volumes(context)` (Storage.kt:12-26): `StorageManager.storageVolumes`
  filtered to `MEDIA_MOUNTED` / `MEDIA_MOUNTED_READ_ONLY` (15), path = `volume.directory.absolutePath`
  (17-19; `StorageVolume.getDirectory()` is an API-30 call, no fallback in this file), primary volume
  first (25). They enter the core as `VolumeInfo{path,label,primary,removable}` (Dtos.kt:102-109) in the
  init config and `sys.volumes`; Rust stores them in `Runtime::set_volumes` (runtime.rs:160-172), which
  also hands them to `apptrash` (runtime.rs:161-166).
- Which roots `LocationPickerDialog` lists, and whether a user can navigate to "/" or into
  `Android/data`, is not in the surface (open point). Verified: `analyze.start` accepts any local path
  string (analyze.rs:134-141, 163-172).

### 1.5 How results, progress and partial outcomes return to Kotlin

- Progress: the task thread reports `ctx.progress(progress.bytes, 0, progress.files, 0)` (analyze.rs:185-192);
  `TaskInfo.doneBytes/doneItems` (Dtos.kt:72-94). `totalBytes/totalItems` stay 0, the bar is
  indeterminate (ScanPages.kt:105). Text shown: `"{doneItems} Dateien · {size(doneBytes)}"`
  (ScanPages.kt:118-121) plus `task.message` if non-blank (ScanPages.kt:107): "Analysiere…"
  (analyze.rs:173) or "Verbinde…" for remote (analyze.rs:166). Note on the page: "Der Scan läuft weiter,
  wenn diese Seite verlassen wird." (ScanPages.kt:109-113).
- Completion: `awaitTask` returns `TaskInfo` with `state` (Dtos.kt:78). `done` -> `phase = Result`,
  `loadNode(emptyList())`, then `issues = AnalyzeApi.issues(id)` (null on `CoreException`)
  (AnalysisViewModel.kt:76-84). `canceled` -> back to Setup + Snackbar "Analyse abgebrochen" (85-88).
  anything else -> Setup with `error = task.message ?: "Analyse fehlgeschlagen"` (89-92); a
  `CoreException` from `start` -> `error = e.message ?: e.kind` (94-97).
- The task `result` is `{files,dirs,bytes,issues}` (analyze.rs:194-199, 224); Kotlin does not read
  `task.result` (AnalysisViewModel.kt:75-93).
- Tree: pulled per folder with `analyze.node` (AnalysisViewModel.kt:129-145). Partial outcome: see section 4.
- Finished results are kept in the process-wide `RESULTS` queue: `MAX_RESULTS = 4`, oldest finished slot
  evicted first (analyze.rs:17, 39, 46-64); a failed or canceled scan frees its slot (analyze.rs:75-80).
  `AnalyzeApi` has no release call (AnalyzeApi.kt:43-70); leaving the Result screen drops only Kotlin state
  (AnalysisViewModel.kt:118-127).

## 2. The analysis walker

### 2.1 Entry point and platform selection

- Local: `crate::analytics::scan(Path::new(&local_root), &scan_progress)` (analyze.rs:183) =
  `scan_with_guard(root, p, None)` (analytics.rs:53-55, 57-95). `scan_with_guard` is `pub(crate)` (re-export
  analytics/mod.rs:16); its only other user in the surface is the Windows remote-task test (analytics_tests.rs:14).
- Adapter: `analytics/os/mod.rs:1-3` re-exports `display_path, normalize_scan_root,
  parallel_scan_allowed, read_directory, EntryKind, LocalEntry` from `crate::local_access`;
  `local_access/mod.rs:4-9` selects `os/windows/mod.rs` under `cfg(windows)` and `os/linux_os.rs`
  otherwise (Android uses the Linux file). Windows-only: handle-based directory reader
  (`local_access/os/windows/directory.rs:96`), backup-read retry and UAC helper
  (ANALYTICS_ACCESS.md:3-24, 99-112), `parallel_scan_allowed` from token state
  (`local_access/os/windows/privilege.rs:29`), verbatim `\\?\` roots
  (`local_access/os/windows/paths.rs:22,135`). `analytics/os/windows/` is empty. None of this runs on Android.
- Not on the Android local chain: `analysis_transfer.rs` / `tree_transfer.rs` (completed-result transport
  for Direct Share and the GUI worker bridge, analysis_transfer.rs:1; limits tree_transfer.rs:5-11) and
  `remote.rs` `scan_remote` (analyze.rs imports only `DuplicateGroup, ScanOutcome, ScanStatus, SizeNode`,
  analyze.rs:13).

### 2.2 Threading model

- Thread count: `local_scan_threads()` = env `SMART_EXPLORER_ANALYTICS_THREADS` else **2**, clamped to
  1..=4 (analytics.rs:97-103). No CPU-count lookup anywhere in `analytics/`, `analyze.rs`, `runtime.rs`,
  `local_access/` (grep `available_parallelism|num_cpus`: no hit). The env var is set nowhere in the
  surface (grep: only the read at analytics.rs:98).
- `parallel_scan_allowed()` is constant `true` on Linux/Android (linux_os.rs:4-6), so a dedicated rayon pool
  with `threads` workers and 64 MiB stacks is built per scan (`SCAN_THREAD_STACK_BYTES`,
  analytics.rs:44, 71-79); a failed build makes the scan strictly serial (analytics.rs:80-88).
- Work split: every directory visits its sub-directories with `into_par_iter().map(visit).collect()` when
  there is more than one, else serially (analytics.rs:300-314); the files of a directory are handled by the
  thread that lists it.
- Other threads involved: task thread `task-analyze` (runtime.rs:303-305) runs `analysis_task`; it
  spawns the scoped `storage-scan` thread with a 64 MiB stack (analyze.rs:115-118) that runs
  `pool.install(visit)` (analytics.rs:90-93) while the task thread polls every 250 ms (analyze.rs:120-126).
  Virtual stack reservation per run: 64 MiB x (1 + pool size).

### 2.3 Syscalls per entry (Linux/Android adapter)

`read_directory` (linux_os.rs:8-45):

- per directory: `std::fs::read_dir(path)?` (line 11; open + getdents loop + close inside std).
- per entry: `entry.file_type()` (line 16; std docs: free where the file system supplies `d_type`, otherwise an
  `lstat`; std behaviour, not repo-verified); then **unconditionally** `entry.metadata()` (line 26; std
  docs: on Unix the equivalent of `symlink_metadata`, i.e. one stat per entry). The size is used only
  for regular files, directories get 0 (lines 27-31): directories are stat-ed but their stat result is
  discarded except for the `LocalEntry` time fields.
- additionally computed per entry, not used by the analysis: `modified()`, `created()` (lines 39-40),
  `hidden` from a second `entry.file_name()` (line 41; `file_name()` is also called at line 33).
- `entry.path()` is only built in the error closure (linux_os.rs:13-15).
- Symlinks and non-regular entries (`EntryKind::Link|Other`, or `is_link_like`) are skipped silently,
  no issue recorded (analytics.rs:218-220).

Reported sizes are logical `st_size` (`metadata.len()`, linux_os.rs:28), see ANALYTICS_ACCESS.md:181-182.

### 2.4 Ordering

- Directory entries are processed in `read_dir` order (unsorted); sub-directories are visited in that
  order, `par_iter().collect()` keeps it (analytics.rs:311).
- Tree order inside a node: directory nodes, then retained file nodes, then one aggregate node
  `"… N weitere Eintraege"` (analytics.rs:328-338). Files are sorted by size desc then name only when a
  directory has more than 4096 of them (analytics.rs:271-273); otherwise `read_dir` order.
- Presentation order is applied at query time: `analyze.node` sorts children by size desc then name and
  returns at most `MAX_NODE_CHILDREN = 500` (analyze.rs:18, 250-254); finding the node walks `path`
  segments with a linear `find` per level (analyze.rs:243-249).

### 2.5 Memory model and bounds

- The whole retained tree stays in memory: `SizeNode{ name: Box<str>, size, is_dir, children: Vec }`
  (analytics.rs:35-40; own estimate "name + ~48 bytes per node", analytics.rs:7-10), kept in
  `Stored::Analysis` (analyze.rs:21-26) until evicted (4 slots, 1.5).
- Transient per directory: `files: Vec<(Box<str>, u64)>` holding **all** file names of the directory until
  folding, `subdirs: Vec<(PathBuf, Box<str>, Retention)>` (analytics.rs:192-193, 269-295).
- Bounds (analytics_budget.rs): retained nodes `MAX_ANALYTICS_NODES = 6_000_000` (11); retained name text
  `MAX_ANALYTICS_TEXT_BYTES = 768 MiB` (12; counted per node as the **name** length only: analytics.rs:70,
  245, 281); recursion depth `MAX_ANALYTICS_DEPTH = 2048` (15); files kept per directory
  `MAX_RETAINED_FILES_PER_DIRECTORY = 4096`, largest first (18; analytics.rs:271-295).
- Diagnostics bounds: 64 issues (analytics_outcome.rs:5), 16 notes (analytics_outcome.rs:124).

### 2.6 What happens when a bound is hit

- Node or text bound: `claim` flips a one-way `aggregating` flag, adds one note ("Detailansicht ab {path}
  zusammengefasst: das Limit fuer node count|retained name text|depth ist erreicht; Groessen und Zaehler
  bleiben vollstaendig", analytics_budget.rs:82-104) and returns `Retention::Aggregate` from then on:
  counting continues, sub-trees are still walked, but their nodes are dropped into the parent's
  aggregate node (analytics.rs:316-338). The outcome stays `Complete` (test analytics_budget.rs:119-166).
- Per-directory file bound: files beyond the 4096 largest are folded into the aggregate node;
  `aggregated_files` counts them (analytics.rs:274-298; test analytics_tests.rs:120).
- Depth bound: `scan_dir` at depth > 2048 records the issue "Verzeichnistiefe ueber 2048 wird nicht weiter
  erfasst" and returns an empty node (analytics.rs:142-152) -> `Partial`.
- Documented intent: "Nothing short of cancellation ends the scan early" (analytics.rs:49-52;
  ANALYTICS_ACCESS.md:264-288).

### 2.7 Non-local analysis on Android (for completeness)

`analyze.rs:181` calls `scan_backend(&**backend, root, ...)` directly, not `scan_remote` (remote.rs:6,
which first tries `backend.scan_storage`). `scan_backend` (analytics_backend.rs:11-39): serial when
`backend.parallelism() <= 1` (254-328), otherwise a level-by-level breadth-first walk in a rayon pool of
`parallelism().clamp(2, 16)` threads that collects all listings in a `HashMap` and builds the tree at the
end (152-251). A `list_dir` error, an invalid child name or a duplicate child name fails **that
directory** (analytics_backend.rs:76-87) and is recorded via `record_io` (199, 285).

## 3. Error handling

### 3.1 Analysis walker, failure by failure

| Failure | Handling | Citation |
|---|---|---|
| `read_dir(dir)` fails (EACCES, EPERM, ENOENT, ...) for a **non-root** directory | `record_io(display_path(dir), err, false)`; the directory becomes an empty node of size 0; siblings continue | analytics.rs:260-262, 339-344; linux_os.rs:11 |
| same for the **scan root** | `record_io(..., is_root=true)` sets `root_failed` -> status `Failed`, `tree = None` | analytics_outcome.rs:156-160, 172-185 |
| `readdir` item error, or `file_type()` / `metadata()` of one entry fails | entry skipped (a directory entry is **not descended into and not counted**), issue recorded on the **parent** path, error text starts with the entry path for `file_type`/`metadata` errors; listing continues | analytics.rs:204-214; linux_os.rs:13-16, 26 |
| `PermissionDenied` kind | additionally counted in `permission_denied` | analytics_outcome.rs:149-154 |
| more than 64 issues | stored 64, the rest only counted in `suppressed_issues` | analytics_outcome.rs:5, 156-170 |
| panic inside one directory | `catch_unwind` per directory: one issue, empty node, scan continues | analytics.rs:153-178 |
| cancel | checked at directory start, per entry, before recursion; result `Canceled`, no tree | analytics.rs:139-141, 215-217, 308; analytics_outcome.rs:172-175 |
| depth > 2048 | see 2.6 | analytics.rs:142-152 |

Status mapping (analytics_outcome.rs:172-195): root failed -> `Failed`; no issues -> `Complete`;
else `Partial`. Tests that pin this contract: missing root is `Failed` (analytics_tests.rs:61), a first
entry error keeps the readable sibling (72), erroring entries never end the directory (161), a denied
child directory keeps the readable sibling (analytics_tests.rs:6-35), and 70 denials stay `Partial` with 64
retained, 7 suppressed and the tree intact (analytics_outcome.rs:26-43).

Std behaviour, not repo-verified: `std::fs::ReadDir` stops yielding after a `readdir` error, so entries
after it in that directory are not listed (the entries read before the error are kept).

### 3.2 Code paths that end the whole scan or stop further directories

- `ScanStatus::Failed`: only when the scan root's own listing (or guard/panic/depth at depth 0) fails.
  `is_root` is true only for the first `scan_dir` call (analytics.rs:89); the recursion passes `false`
  (analytics.rs:304); entry-level errors inside the root directory also use `is_root = false`
  (analytics.rs:207-211). analyze.rs:202-209 turns `Failed` into
  `ApiError("internal", "<first issue path>: <first issue detail>")`.
- Cancellation (section 5).
- Infrastructure: scan thread cannot be spawned (`io_error("Scan starten", ...)`, analyze.rs:119) or the
  scan thread panics outside the per-directory `catch_unwind` -> `ApiError("internal", "Der Scan ist
  unerwartet abgebrochen.")` (analyze.rs:128-130).
- No budget stops the analysis walk (analytics_budget.rs:1-6). **The duplicate walk does have a sticky stop,
  see 6.3.**

### 3.3 Trace for `/storage/emulated/0/Android/data` and `.../Android/obb`

The repository's own statement about these folders: "Android lists the package folders and files such as
`.nomedia` there but refuses to open them even with all-files access" (apptrash/mod.rs:46-50). No
device evidence is in the repo, so each branch below is the code path for the corresponding outcome of
the OS call.

1. If the parent `.../Android` lists normally and `file_type()` / `metadata()` of `data` and `obb` succeed
   (the OS outcome the apptrash comment implies): each becomes `EntryKind::Directory` -> `budget.claim` ->
   pushed to `subdirs` (analytics.rs:226-246).
2. `scan_dir(.../Android/data)`: `enter_directory` (analytics.rs:157) then `read_directory`
   (analytics.rs:166).
   - If `read_dir(Android/data)` succeeds (as the apptrash comment states): its entries are the package
     folders (directories, each queued for recursion) and files such as `.nomedia` (counted as files).
     `scan_dir(<package folder>)` -> `read_dir` fails -> **one issue per package folder** (path +
     "Permission denied (os error 13)" style text from `io::Error::to_string`), `permission_denied += 1`,
     empty size-0 node (3.1 row 1). With many installed apps, the 64 retained issues are quickly full;
     the remainder is only counted (`suppressed_issues`). `Android/data` and `Android/obb` both repeat this.
   - If `read_dir(Android/data)` itself fails: exactly one issue for `.../Android/data`, empty node.
   - If `lstat` of the entry `data` / `obb` fails in `read_directory`: the entry is skipped before any node
     exists; one issue on the parent `.../Android` whose text starts with `.../Android/data: ...`
     (linux_os.rs:13-15, 26; analytics.rs:204-214).
3. In every branch the walk continues with `obb`, `media` and all siblings; the final status is `Partial`
   (analytics_outcome.rs:172-185), the task ends `done` with the message "N Pfade nicht lesbar"
   (analyze.rs:210-213, 224). In the Android UI `Android/data` and `Android/obb` then show, per branch, as
   directories whose children are 0 B package folders (first branch; a listed `.nomedia` is a normally
   empty marker file), as childless 0 B directories (second branch), or not at all (third branch, the entry
   is skipped), next to the global "N Pfade nicht lesbar [Bericht]" row (section 4).
4. Only if `Android/data` (or any unreadable folder) is itself the **scan root** (for example started from the
   place shown in "Dateien", AnalysisViewModel.kt:47-56) does the root rule apply: `Failed`, no tree,
   error card "Nicht abgeschlossen" with `"<path>: Permission denied (os error 13)"` (analyze.rs:202-209;
   AnalysisViewModel.kt:89-92; ScanPages.kt:77).
5. Duplicates: same branches via `reclaim/local.rs:125-131` (see 6.4); there the root rule yields
   `ApiError("not_found", "<path>: <error>")` (analyze.rs:354-356).

### 3.4 Special-casing of names on the chain

- Analysis walk skip rules: link-like and non-regular entries (analytics.rs:218-220), the app trash folder
  name `.SmartExplorer-Papierkorb` while `apptrash` volumes are set (analytics.rs:223-225;
  apptrash/mod.rs:26, 30-44), directories under `/proc`, `/sys`, `/dev`, `/run` (analytics.rs:241-243;
  agent_proto/os/shared/fs.rs:60-68), Windows-only `unreachable` names (analytics.rs:226-239; Linux sets it
  to false, linux_os.rs:38).
- Duplicate walk skip rules: symlinks, trash name, pseudo directories (reclaim/local.rs:169-174), and
  detail suppression below directories named like cleanup targets (6.5).
- **No rule mentions `Android`, `data`, `obb`** in `analytics/`, `local_access/` or `analyze.rs` (grep for
  `Android/data|Android/obb|hidden_app_folders_in` inside these paths: no hit).
- The helper that does: `apptrash::hidden_app_folders_in(dir)` (apptrash/mod.rs:46-68), true when `dir` is
  `<volume>/Android/data` or `<volume>/Android/obb` for a volume registered with `set_volumes`; unit test
  `android_task_apptrash_hidden_app_folders_only_below_volume_roots` (store_tests.rs:185-217). Its doc: "Sync
  walks omit them like the trash, as protected omissions. Inert while no volumes are set"
  (apptrash/mod.rs:49-50). Inside the surface it has only the definition and the test; other callers
  were not searched (outside the surface).

## 4. How partial and omitted results reach the Android UI

Rust side:

- `Partial` -> task `done`, `ctx.message("{n} Pfade nicht lesbar")` with n = retained + suppressed issues
  (analyze.rs:210-213), tree stored (216-224). The message is set only at the end of the scan; the progress
  page that displays `task.message` is replaced by the result page once `awaitTask` returns
  (ScanPages.kt:107; AnalysisViewModel.kt:76-78).
- `analyze.issues` (analyze.rs:274-294): `count = issues.len() + suppressed_issues` (290); `text` = retained
  issues as `"<path>: <detail>"` (<= 64), then `"… N weitere"` (285-287), then the notes (<= 16, e.g. the
  aggregation note) (288).
- Not forwarded: `permission_denied` (analytics_outcome.rs:75) and `aggregated_files`
  (analytics_outcome.rs:80); analyze.rs has no reference to either (grep `permission_denied`: no hit).

Kotlin side:

- `issues` is loaded once after `done`; a failing call leaves it `null` silently (AnalysisViewModel.kt:79-84).
- `NodeList` shows, at the top of every drill-down level, `"{count} Pfade nicht lesbar"` + [Bericht] only if
  `issues != null && issues.count > 0` (AnalysisScreen.kt:135-145); the count is global, not scoped to the
  shown folder. [Bericht] opens `TextReportDialog("Leseprobleme", issues.text)` (AnalysisScreen.kt:117-120).
  Consequence: an outcome with notes but zero issues (aggregation) shows no row and the notes are not
  reachable.
- Error state instead of the tree only for `Failed` / thrown errors: Setup page with
  `ErrorCard(message, title = "Nicht abgeschlossen")` (AnalysisViewModel.kt:89-97; ScanPages.kt:77).
- Presentation of unreadable folders: ordinary directory nodes of size 0 (analytics.rs:260-262, 339-344);
  rows "0 B · 0 % · 0 Einträge" (AnalysisScreen.kt:155-179); absent from the treemap (`size > 0` filter,
  TreemapView.kt:137-138; at most `MAX_TILES = 60` tiles plus one "n weitere" tile, TreemapView.kt:41,
  139-143).
- The header "n Einträge" is `node.children.size`, i.e. at most the 500 returned children, while the size is
  the true total (AnalysisScreen.kt:131-134; analyze.rs:254, 260). The page title is the node name, for a
  root the last path segment, e.g. "0" for `/storage/emulated/0` (analytics.rs:62-65; AnalysisScreen.kt:81).

## 5. Progress cadence, per-entry overhead, cancellation

Cadence:

- Task thread: `run_watched` loops `ctx.progress(bytes, 0, files, 0)` then sleeps `PROGRESS_TICK = 250 ms`
  (analyze.rs:19, 106-132, 185-192), once more after the scan ends (analyze.rs:127). `ctx.progress` takes the
  hub mutex, updates the task record and `notify_all`s waiters (runtime.rs:384-395, 411-417, 53-57). How
  `TaskRecord::progress` throttles or emits events is in `tasks.rs`/`events.rs` (outside the surface).
- Walker counters (shared `Arc<AtomicU64>`, `Relaxed`, progress.rs:65-70): files/bytes flushed every 128
  files of a directory and once after its listing (analytics.rs:251-256, 265-266); `dirs` is added after the
  parent's listing (analytics.rs:267) and counts discovered, not finished directories.
- Forwarded to Kotlin: only `bytes` and `files` (analyze.rs:187-190). Not forwarded: `dirs`, the current
  directory (`Progress::snapshot().current`, progress.rs:104-144), phase, and for duplicates
  `fingerprinted`, `hashed`, `candidates` (types.rs:26-35; no reference in analyze.rs).
- No JSON is produced during the walk; JSON appears only for `analyze.node` / `analyze.issues` /
  `reclaim.groups` calls (analyze.rs:227-294, 379-403).

Per-entry / per-directory overhead, analysis walker:

- per entry: `file_name()` twice and `modified()/created()` in `read_directory` (linux_os.rs:33, 39-41);
  `name.to_string_lossy().into_owned().into_boxed_str()` (analytics.rs:221); cancel flag load (215);
  `apptrash::excluded_name` string compare, lock only on a name match (223; apptrash/mod.rs:41-44).
- per directory entry: `dir.join` allocation (240), `is_pseudo_dir` on `to_string_lossy` (241), `claim`
  = flag load + two CAS loops (244-246; analytics_budget.rs:75-113).
- per file: `Vec` push (250); for each of the first 4096 files a `dir.join(file_name)` path allocation
  passed to `claim` (278-283) plus the same two CAS loops.
- per directory: `display_path` string, `visible_path` called twice (`enter_directory` and `set_phase`),
  `Progress.state` mutex lock, `catch_unwind`, `Vec` allocations, a rayon fork when more than one
  sub-directory (analytics.rs:157, 300-314; progress.rs:89-122; linux_os.rs:51-53).
- Contention points: `Progress.state` mutex per directory (progress.rs:71, 104-118); `Diagnostics` mutexes
  only when an issue/note occurs (analytics_outcome.rs:128-143, 164).

Cancellation:

- Kotlin `vm.cancel()` -> `FilesApi.cancelTask(id)` (AnalysisViewModel.kt:101-103, 147-155);
  `run_watched` sees `ctx.cancelled()` at the next tick and stores `progress.cancel` (analyze.rs:121-123);
  the walker checks the flag as in 3.1; result `Canceled` -> `Err(canceled("Analyse abgebrochen"))`
  (analyze.rs:201) -> task state `Canceled` (runtime.rs:337-339) -> Snackbar (AnalysisViewModel.kt:85-88).
  Latency = up to one tick (250 ms) plus the currently blocking `read_dir`/stat call (no interruption of a
  syscall).
- `start()` while scanning cancels the previous task (AnalysisViewModel.kt:60-63); closing the page does
  not (ScanPages.kt:109-113).

## 6. The duplicate finder on Android

### 6.1 Flow and parameters

- `reclaim.start {location, minSize}`: `minSize` must be >= 0, forced to >= 1 (analyze.rs:298-301); options
  `ReclaimOptions { duplicate_min_bytes: minSize, ..Default }` (analyze.rs:318-321) = `large_min_bytes 1 GiB`,
  `stale_days 365`, **`max_items 200`**, `partial_fingerprint_bytes 64 KiB` (types.rs:14-23).
- Kotlin offers `minSize` chips 100 KiB, 1 MiB (default), 10 MiB, 100 MiB (DuplicatesViewModel.kt:176-178;
  DuplicatesScreen.kt:78-88). After `done`: `reclaim.groups`, groups with more than one item, sorted by
  `size * (n - 1)` descending (DuplicatesViewModel.kt:70-77). Hint: "Findet Dateien mit gleichem Inhalt.
  Remote-Orte ohne Papierkorb werden nur angezeigt." (DuplicatesScreen.kt:64).
- Local: `scan_reclaim(Path, &progress, &opts)` (analyze.rs:338-340; reclaim/local.rs:36-94) on the same
  `storage-scan` thread wrapper (analyze.rs:106-132). Remote: `scan_reclaim_backend` (analyze.rs:335-337; 6.6).
- `reclaim.groups` forwards only `size` and `items[{location, mtimeMs}]`; hash, evidence, reclaimable and
  confidence stay in Rust (analyze.rs:385-399).

### 6.2 Walker (local)

- Single-threaded recursive `scan_dir` (reclaim/local.rs:97-228); no rayon or thread use in `reclaim/` except
  the remote agent walk worker (grep: only `reclaim/backend.rs:109`).
- Syscalls: `std::fs::read_dir(dir)` (125); per entry `entry.path()` and `entry.file_name()` allocations (150-151);
  `entry.file_type()` (158) decides the kind; **no stat for directories**; `entry.metadata()` (191) once per
  regular file.
- Per entry also: `budget.claim` with plain counters (153), `apptrash::excluded_name` (170),
  `is_pseudo_dir` for directories (171), `progress.files` atomic add per file (190) and a `fetch_update` on
  `progress.bytes` per file (200-204). For non-suppressed files: `to_fwd(&path)` and `name.to_string_lossy()
  .into_owned()` string allocations, `ReclaimItem::new` and `record_file` (209-217, 230-292), which also
  evaluates large (>= 1 GiB), stale (> 365 days), empty and `.log` categories and `file_cleanup_reason`
  (`to_ascii_lowercase` allocation, cleanup.rs:11-21). Per directory `dir_cleanup_reason`
  (`to_ascii_lowercase`, cleanup.rs:23-26, 40) and `progress.dirs` add (local.rs:117-118). The Android bridge
  reads only the duplicate groups of the report (analyze.rs:354-376).
- Order: `read_dir` order, depth first, sequential.
- Progress to Kotlin: `files` and `bytes` of the walk (analyze.rs:342-349); during fingerprinting and
  hashing these counters do not change; the message stays "Suche Duplikate…" (analyze.rs:328).

### 6.3 Bounds and the sticky stop

- `ReclaimBudget` (reclaim/budget.rs:3-5, 53-90): 1,000,000 entries, 128 MiB of path+name text (counted per
  entry as `path.len() + name.len()`, local.rs:152), depth 512. On the first excess `stopped` is set
  permanently (budget.rs:83-86): `record_limit` stores `scan_limit` plus one error "<root>: reclaim scan
  stopped at bounded entry count|path/name text|depth limit" (local.rs:153-157, 324-335), the entry loop
  breaks and every later `scan_dir` returns immediately (local.rs:108-110, 134-137, 221-223). Remaining
  directories are not visited.
- `scan_limit`, `duplicate_candidates`, `duplicate_candidates_retained` and `result_counts` are fields of the
  report (types.rs:167-189, 192-205) that `reclaim_task` does not read (analyze.rs:354-376; grep of analyze.rs
  for `scan_limit|duplicate_candidates|has_truncated|result_counts`: no hit).
- **Candidate cap**: while walking, `record_file` keeps only the `max_items` (200) largest files with
  `size >= duplicate_min_bytes` in `acc.files` via `retain_best` (local.rs:276-291; retention.rs:8-28);
  `duplicate_groups` applies the cap again (duplicates.rs:26-38). Every other file >= `minSize` is counted
  (`duplicate_candidates`) but never fingerprinted or hashed. Result groups are capped at 200 as well
  (duplicates.rs:113-127; `total_groups` counts all, 112).

### 6.4 Error handling

- `read_dir(dir)` fails: `push_error(.., dir == root)`, `DirScan::default()` (incomplete), the caller keeps
  walking (local.rs:125-131, 187-188). Root: `root_error` set (337-342) -> `ApiError("not_found",
  "<path>: <error>")`, task `failed`, Kotlin error card from `task.message` (analyze.rs:354-356;
  DuplicatesViewModel.kt:82-86).
- entry error / `file_type()` / `metadata()` failure: error recorded, entry skipped, loop continues
  (local.rs:138-149, 158-165, 191-198).
- Errors are bounded: 64 stored, rest counted (`MAX_RECLAIM_ERRORS`, util.rs:3-11). The bridge forwards at
  most 100 to the task record (`ctx.error("", error)`, analyze.rs:357-359) and the count in the summary
  (analyze.rs:367). Kotlin does not read `task.errors` or `task.result` for the scan
  (DuplicatesViewModel.kt:70-86), so scan-time errors, `scan_limit` and truncation are not shown.
- Fingerprint/hash open or read failures (EACCES at `File::open`): "Fingerprint <path>: <error>" /
  "Hash <path>: <error>" bounded errors, the file leaves the grouping, the run continues
  (duplicates.rs:62-73, 85-96).
- Cancel: flag checked per entry, per candidate and per 1 MiB chunk (local.rs:108, 134; duplicates.rs:54-82,
  150-156, 172-174); analyze.rs:351-353 -> `canceled("Duplikatsuche abgebrochen")`.
- No `Android/data` / `Android/obb` special-casing here either (3.4).

### 6.5 Detail suppression below cleanup-named directories

A directory whose name matches `.git`, `node_modules`, `target`, `build`, `dist`, `cache`, `caches`, `.cache`,
`log`, `logs`, `__pycache__`, `.pytest_cache`, `.mypy_cache`, `.gradle` (compared lower-case, any depth except
the root itself) sets `skip_detail` for its whole subtree: files are still counted into `files`/`bytes` but are
not recorded as duplicate candidates (local.rs:118-119, 206-208; cleanup.rs:32-88).

### 6.6 Grouping, hashing, I/O (local)

`duplicate_groups` (duplicates.rs:19-136), all sequential:

1. Candidates (<= 200 largest) grouped by exact size in a `BTreeMap`, sizes processed descending, sizes with
   one candidate dropped (46-53).
2. Partial fingerprint per candidate (`partial_fingerprint`, 138-165): SHA-256 over `size` (8 bytes BE) + first
   `sample` bytes + (if `size > sample`) last `sample` bytes, `sample = min(64 KiB, size, 1 MiB)` (145);
   one `vec![0u8; sample]` allocated per file (148), two `read_exact` and one `seek`.
3. Candidates with equal (size, fingerprint) are fully hashed with SHA-256 (`sha256_file`, 167-182): a
   `vec![0u8; 1 MiB]` per file (170), `File::read` loop of up to 1 MiB, cancel check per chunk.
4. Equal full hashes -> `DuplicateGroup{ hash: Sha256, evidence: LocalSha256, items sorted newest first }`,
   `reclaimable = size * (n - 1)` (98-127). No byte-for-byte comparison in this path (`bytes_equal`,
   184-214, is a separate function not called here).

Remote (non-local) duplicates: `scan_reclaim_backend` (reclaim/backend.rs:45-94) uses an agent-side hashed walk
when `backend.supports_walk_hashed()` (96-181, evidence `AgentMd5` at line 145), else a listing walk (189-324).
Only entries that carry a 32-hex-digit `content_md5` become candidates (backend.rs:327-402, filter at 377-401);
grouping is by `(size, md5, evidence)` (backend_duplicates.rs:11-78); neither file calls `open_read` (grep: no
hit), so no content is read. The same `max_items = 200` candidate cap (backend.rs:377-401; backend_duplicates.rs:16-25,
69), the same sticky budget and `scan_limit` apply (backend.rs:132-136, 265-272).

## 7. Android platform storage APIs for totals and free space

- Present: `StorageManager.storageVolumes` for mount points, state, description, primary/removable flags and a
  `registerStorageVolumeCallback` for state changes (Storage.kt:13-24, 32-40). Nothing else from the platform
  storage APIs is used in the surface.
- Not present anywhere in the surface (grep `StorageStats|MediaStore|statvfs|freeSpace|totalSpace|usableSpace`
  in the allowed Kotlin, and `statvfs|fs2|available_space|free_space|total_space|disk_space|StorageStats|
  MediaStore` in the allowed Rust): no `StorageStatsManager`, `MediaStore` or `statvfs`/`StatFs` use for volume
  totals or free space. `VolumeInfo` and `Root` carry no capacity fields (Dtos.kt:35-44, 102-109;
  places.rs:23-32).
- The numbers shown by the analysis are sums of logical file sizes found by the walk
  (analytics.rs:247-266, 316-327; analyze.rs:197; AnalysisScreen.kt:131-134); ANALYTICS_ACCESS.md:181-182
  states that logical totals do not claim to include file system bookkeeping, snapshots or all allocated space.
- Permission wording in the Linux/Android adapter: `can_request_access` is `false` and `request_access` returns
  "Zusätzliche Leserechte müssen unter Android in den Einstellungen als „Zugriff auf alle Dateien“ gewährt
  werden" (linux_os.rs:55-65); neither is referenced from `analyze.rs` or `analytics/` (grep: no hit).

## Offene Punkte

1. `LocationPickerDialog` (`ui/picker/`): which roots it lists, whether "/" or `Android/data` can be chosen or
   navigated to. Not in the surface; only `loc.roots` (places.rs:43-123) and `Storage.volumes` could be read.
2. Device behaviour on Android 11-15 is not verifiable from the repo: whether `opendir`, `readdir` and `lstat`
   on `<volume>/Android/data|obb` and on `<package>` folders succeed or fail with EACCES/ENOENT, whether `d_type`
   is filled by the FUSE layer, and what MediaProvider answers per syscall. The only repo statement is the
   comment at apptrash/mod.rs:46-50.
3. Other callers of `apptrash::hidden_app_folders_in` (the doc says sync walks) were not searched; inside the
   surface only the definition and its test exist.
4. Transport and cadence of progress outside the surface: `Core.request` / `NativeBridge`, `FilesApi.awaitTask`
   and `cancelTask` (poll or event, timeouts), `rememberTask`, and on the Rust side `tasks.rs`
   (`TaskRecord::progress`, `push_error` storage) and `events.rs` (event throttling).
5. Rust std internals on Android that the numbers depend on: syscall used by `DirEntry::metadata`
   (`fstatat64` vs `statx`), whether `created()` works there, whether `ReadDir` ends after an error.
6. Whether anything sets `SMART_EXPLORER_ANALYTICS_THREADS` on Android (`Core.kt`, native bridge, manifest not in
   the surface); `Cargo.toml` rayon features.
7. Callers of `AnalysisViewModel.request` / `preselect` and `FilesLocation` (which screens start an analysis from
   which place); `ErrorCard`, `TextReportDialog`, `Format`, `LoadingBar` internals (presentation only).
8. `sys.rs` / `fs.properties` and `AndroidManifest.xml` (MANAGE_EXTERNAL_STORAGE declared or used, any free/total
   space call behind another facade method): not read; section 7 states only what is absent inside the surface.
9. `reclaim/verify.rs` (`prepare_reclaim_trash_plan`) and the `*_tests.rs` bodies of `reclaim/` were not read;
   Android deletes duplicates through `fs.delete` (api.md:284), not through that plan.
10. `domains/mod.rs`, `domains/locations.rs`, `domains/args.rs`, `apptrash/mod.rs`, `store_tests.rs` and
    `agent_proto/.../fs.rs` were read only at the cited lines/functions because they are callees on the chain
    but lie outside the named reading surface.
