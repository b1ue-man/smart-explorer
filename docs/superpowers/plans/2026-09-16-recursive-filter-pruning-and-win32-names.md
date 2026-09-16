# Filter-pruned recursive scans, Win32-hostile names, concurrent transfers and Rooms — 2026-09-16

> Executed inline by the main agent under `AGENTS.md`: no subagents, no local
> builds or tests; every milestone's expected result is verified once by the
> single remote task suite `native/test-filter-transfer-task.sh`
> (`.github/workflows/filter-transfer-task.yml`).

## Explicit batch goal

1. **Recursive mode discards non-matching entries at scan time.** When a filter
   is active while a recursive scan runs, entries that cannot appear in the
   view are not retained, not counted against the bounded 1 M-entry / 128 MiB
   scan budget and not sorted. Only matches and the directories needed to show
   them in the tree are kept. A filter change that could admit entries the
   pruned listing never kept (or a listing the budget truncated) restarts the
   scan with the new filter; a narrower filter only refilters the retained
   entries. Refresh (F5 / Rekursiv toggle) keeps the typed name filter.
2. **Win32-hostile names are visible, filterable and deletable.** Files or
   folders whose names Win32 cannot address through ordinary paths — reserved
   device names (`NUL`, `CON`, `AUX`, `PRN`, `COM0-9`, `LPT0-9`, `COM¹²³`,
   `LPT¹²³`, with or without an extension), names ending in a dot or space and
   names with characters Win32 rejects — are listed with their real metadata,
   can be selected with a dedicated filter ("Nur problematische Namen"),
   deleted (Papierkorb via a safe rename first; endgültig via verbatim paths)
   and renamed. Explorer cannot do any of this.
3. **Transfers run concurrently.** Several uploads, downloads or
   remote-to-remote copies (same peer, several Direct peers, a Room) run at the
   same time and share bandwidth instead of waiting behind one slot.
4. **Rooms keep working and are proven.** Room creation, joining by code,
   membership, Room exports, every file transaction and concurrent transfers
   over a Room are exercised by the automated Share end-to-end script.

## Stage one: evidence

- `scanner/os/walk.rs` streams every entry; `app/core/view_selection.rs`
  filters afterwards from `App::entries` (the "cache"). Tree mode shows a
  directory only when a descendant file matches (`has_match`), so directories
  are structure, files are results.
- `app/core/scanning.rs::start_scan_navigated` clears the name filter on every
  scan start, including `rescan()`, so toggling Rekursiv wipes the typed
  filter today.
- `scanner`, `rscan::walk_state` and `vfs::LocalBackend::list_dir` call
  `std::fs::symlink_metadata(dir.join(name))`. On Windows this reopens each
  child by path; for `C:\dir\NUL` Win32 resolves the *device*, so the real
  file's metadata is lost or the listing aborts. `DirEntry::metadata()` uses
  the enumeration record instead (no extra open on Windows; `lstat` on Unix).
- `vfs/os/windows/local_platform.rs::to_os` only swaps separators. Rust's std
  passes paths shorter than 248 UTF-16 units to Win32 unchanged, so
  `DeleteFileW("C:\dir\NUL")` hits the device. Only a `\\?\` verbatim path
  addresses the file. `analytics/os/windows/paths.rs` already builds verbatim
  roots for the storage scanner (kept as is; the VFS gets its own bounded
  helper because `vfs` must not depend on `analytics`).
- `app/core/delete_actions.rs` recycles through `trash::delete` (Windows
  `IFileOperation`), which fails on such names exactly like Explorer.
- `mount/core/path.rs` and `mount/os/windows/wide.rs` each carry a private
  reserved-name check for *outgoing* mount names; they stay untouched.
- `bin/bench.rs` compiles `scanner` and `types` as private modules, so the new
  scan-retention contract must live in those two modules only.
- Share transport is already concurrent: one cached QUIC connection per peer
  relation, one bi-stream per operation (`share/core/node_sessions.rs`), the
  server spawns a task per accepted stream with a 32-permit blocking pool
  (`server.rs`, `blocking.rs`), and the daemon IPC multiplexes requests
  (`agent/core/mux.rs`). The only serialization point was the GUI's single
  transfer slot (`upload_rx`/`transfer_*` in `app/core/state.rs`, guarded by
  "Es läuft bereits eine Übertragung").
- `native/test-share-lifecycle-e2e.sh` drives four CLI clients against a local
  Share server but never touched Rooms (and its Direct inbox expectations
  predate automatic request decisions, so it cannot host new coverage); `se share room create`,
  `se connections add-room --code`, `se share export add --room` and
  `share://room/<room>/<device>/…` targets exist in the CLI.

## Research

- Win32 name resolution (`RtlIsDosDeviceName_U`): the final component is
  checked after stripping one trailing `:`, trailing dots/spaces, and text
  from the first `.`/`:`; `COM`/`LPT` accept one digit (Microsoft's current
  naming guidance also lists `0` and the superscripts `¹ ² ³`). Verbatim
  (`\\?\`) paths skip this resolution, keep trailing dots/spaces and never
  fold `.`/`..`, so the caller must fold them and use backslashes.
- Rust std: `get_long_path` returns paths shorter than 248 units untouched
  (drive-rooted or UNC), so verbatim conversion must be explicit.
- `DirEntry::metadata()` on Windows is documented as needing no extra system
  call and never following symlinks; on Unix it is `lstat`.
- `trash` 5.2.5 uses the shell (`IFileOperation`) on Windows; the shell cannot
  parse reserved names, so a safe rename must precede recycling.

## Stage two: milestones

| # | Milestone | Files / boundary | Expected result (remote suite) |
| --- | --- | --- | --- |
| M1 | Win32 name rules (pure) | new `types/core/win32_names.rs`, `types/mod.rs` | `win32_name_issue` flags `nul`, `NUL.txt`, `con.tar.gz`, `com1`, `LPT¹`, `aux .`, trailing dot/space, `a<b`; accepts `null`, `com`, `com10`, `nul_x`; `win32_safe_name` yields `_nul.txt`, `foo`, `a_b`, `unbenannt`. |
| M2 | Filter flag and scope comparison | `types/core/types.rs`, `filter/core/filter.rs`, new `filter/core/scope.rs`, `filter/mod.rs` | `problem_names_only` keeps only hostile names; `is_at_least_as_narrow_as` follows the conjunction rules (substring prefix, extension subset, range containment, flag implication, regex/glob equality); `scan_restart_needed` restarts for broader filters or truncated listings only. |
| M3 | Local scanner retention | new `scanner/core/retention.rs`, `scanner/os/shared.rs`, `scanner/os/walk.rs`, `scanner/os/collect.rs`, `scanner/mod.rs`, `bin/bench.rs` | Filtered `start_scan` emits root, matches and their ancestor chain only, `scanned` still counts every visited entry, `descend=false` stops traversal; unfiltered scans are unchanged; `ScanHandle::truncated` exists; the walker uses `DirEntry::metadata()`. |
| M4 | Remote scanner retention | `rscan/os/shared/{rscan,walk_state,parallel,search}.rs`, tests, `gdrive/core/gui_task_tests.rs`, `app/core/gui_design_task_ui.rs` | Same contract through `start_scan_backend(..., retention, ...)` for serial and parallel walks; listing preflight is skipped while pruning. |
| M5 | Local VFS addresses hostile names | new `vfs/os/windows/verbatim.rs`, `vfs/os/windows/local_platform.rs`, `vfs/os/shared/local.rs`, `vfs/mod.rs` | (Windows) `to_os("C:/data/nul")` = `\\?\C:\data\nul`, ordinary paths unchanged, UNC → `\\?\UNC\`; `list_dir` of a folder holding a real `nul` and `trailing.` file reports both with real sizes; `stat`, `rename_no_replace` and `remove_entry` succeed on them. |
| M6 | App: pruned scans, restart rule, labels | `app/core/{state,app_models,init,prefs_tabs,landing,scanning,drains_connect,filterbar,omni_accel,shell_commands,view_selection}.rs`, new `app/core/filter_scope.rs` | Recursive scans pass the active filter as retention; refresh keeps the name filter; filter edits call `filter_changed` (restart vs. refilter); truncated flag captured at Done; counter shows "Treffer / durchsucht" while pruned; tree mode shows a directory that itself matches. |
| M7 | Delete and rename hostile names | new `app/core/delete_hostile_names.rs`, `app/core/{delete_actions,delete_lifecycle,delete_drain}.rs`, `app/os/{windows.rs,linux_os.rs}`, `app/os/shared/file_actions.rs`, `app/core/table.rs` | Papierkorb renames a hostile-named local target to a safe unique sibling first and reports the count; endgültig löschen works through M5; renaming *to* a hostile name is refused on Windows; hostile local names are painted in the warning color. |
| M9 | Concurrent remote transfers | new `app/core/transfer_jobs.rs`, `app/core/{transfer_lifecycle,state,init,frame_update,shutdown,status_errors}.rs`, `app/os/shared/{clipboard_upload,drag_drop,copy_paste_task_tests}.rs` | Uploads, downloads and remote-to-remote copies are admitted into a lane of up to six concurrent workers with a FIFO queue; each has its own progress and cancel, the status bar lists them and the queue depth; finished transfers refresh the remote view once; the lane's admission, queueing, lost-worker reporting and shutdown are unit-tested. |
| M10 | Room lifecycle end to end | new `native/test-share-room-e2e.sh` (standalone; the Direct part of `test-share-lifecycle-e2e.sh` is stale against auto-accepted requests, see `docs/TODO.md` H1), new `agent/core/agent_error.rs` (the agent protocol forwards only error text, so the client rebuilds `NotFound`/`AlreadyExists`/`PermissionDenied` from the forwarded OS error; without it `se cp` into a new Room path failed with "cannot determine destination type", run 35091905058) | With the local Share server: C creates a Room (`se share room create`), D joins with the printed code, both see one member; Room-only exports; `ls`/`cat`/`stat`/`cp`/`cp -r`/`mkdir`/`mv`/`search`/`rm` over `share://room/<room>/<device>/…`; three concurrent downloads across both directions complete byte-exact; after `remove-room` neither side reaches the other's Room export. |
| M8 | Suite, docs, graph | `native/test-filter-transfer-task.sh`, `.github/workflows/filter-transfer-task.yml`, `README.md`, `docs/TODO.md`, `docs/GOTCHAS.md`, `graphify-out/` | One dispatch runs the Linux job (all `recursive_filter_task_` tests, affected integrations, the Share E2E incl. Rooms, per-file rustfmt, clippy on host and Windows target) and the Windows job (the same tests plus the `#[cfg(windows)]` real-file cases). |

## Second research pass / resolved gaps

- Retention is a trait object (`ScanRetention { retain, descend }`) defined in
  `scanner`; the app implements it over `CompiledFilter`. This keeps `scanner`
  free of `filter` (bench.rs constraint) and lets `rscan` share the type.
- Ancestor emission: each pending directory carries an `Arc<Lineage>` (entry +
  `AtomicBool emitted` + parent). The first retained descendant walks the chain
  upward and emits every not-yet-emitted ancestor into the same batch; the
  atomic swap makes concurrent sibling tasks emit each ancestor once. View
  building is order-independent, so late ancestors are fine.
- Budget: only emitted entries claim budget; `scanned`/`bytes` keep counting
  every visited entry so progress stays honest.
- The restart rule treats a filter as a conjunction of independent
  constraints; anything not provably narrower restarts. Regex/glob text only
  compares equal; substring text narrows when the new text starts with the old
  text and neither contains group separators.
- `rescan()` keeps the name filter; navigation into another folder still
  clears it (existing intent).
- The Papierkorb path renames first (`MoveFileExW` with a verbatim source via
  `LocalBackend::rename_no_replace`) because the shell cannot parse the name;
  the renamed item lands in the Recycle Bin and the success notice reports the
  rename count. Permanent deletion needs no rename.
- Painting and rename refusal follow an OS fact (`local_names_follow_win32_rules`)
  supplied by `app/os`, so `app/core` stays free of `cfg(windows)`.

## Status

M1–M10 are implemented (commits on `main` ending in `[task candidate]`). The
single suite `native/test-filter-transfer-task.sh` maps every milestone; its
remote runs and the terminal release are recorded on `docs/TODO.md` (F1).
