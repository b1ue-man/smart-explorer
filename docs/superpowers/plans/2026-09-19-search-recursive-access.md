# Search, recursive navigation and access repair

Status: implementation complete; focused remote acceptance pending. No local
builds or test execution.

## Goal and deliverables

Repair unreliable extension searches and empty/stalled recursive results, add
collapsible recursive folders without changing selection or transfer contents,
preserve relative structure when copying filtered recursive results, and handle
denied local access inside the existing application, including storage analysis.
Commit coherent changes, run one focused suite on remote CI, then perform the
single complete remote release transaction.

## Stage one: repository evidence and approach

Inspected on 2026-09-19 at `ba5f1e0`; the worktree initially contained only
untracked generated Graphify material. Graphify query preceded source inspection.

- `app/core/prefs_tabs.rs::root_prefix` removes every trailing separator, while
  `view_selection.rs::recompute_view` requires an exact root entry. `C:/`, `/`
  and other roots with a trailing separator can therefore produce an empty tree
  even when scanning returned entries.
- `filter/core/filter.rs` exempts directories from extension matching, so an
  extension-only search treats every directory as a match. Extension parsing in
  `omni_accel.rs` does not accept `*.ext` or semicolon-separated lists; matching
  only the last extension cannot express compound suffixes such as `tar.gz`.
- `FilterRetention::descend` requires `include_dirs`, and the tree drops hidden
  directory rows together with their descendants. Selecting files only therefore
  prevents discovery/display of files in subdirectories.
- Local and remote scans have entry/text/depth budgets. The remote scanner can
  reject a complete large directory before emitting any child. A filter changed
  during an unpruned scan can miss unseen matches when that scan later truncates.
  Local deferred ancestor emission marks an ancestor emitted before success.
- There is no recursive collapse state. Rendering, selection-all and summaries
  use the same `view`; folding that vector directly would change their meaning.
- Filtered clipboard preparation re-expands each selected directory, including
  overlapping ancestor/descendant selections, and files-only clipboard copies
  flatten their names. Those paths need one deduplicated result plan.
- Storage access currently launches a separate elevated analysis window from
  `analytics/os/windows/elevation.rs`. The Windows enumerator already supports
  backup-read privileges and handles provider failures without abandoning a scan.
  Reuse that machinery through a narrow read capability in the original process.

Stage-one approach: separate complete result membership from row presentation;
make filter parsing consistent with scan retention and transfers; retain partial
results with explicit diagnostics; replace the separate admin UI with a scoped,
authenticated headless read helper whose results/handles stay in the current UI.

## First research pass (2026-09-19)

- Rust [`read_dir`](https://doc.rust-lang.org/stable/std/fs/fn.read_dir.html)
  documents errors both when opening a directory and while advancing it. Handle
  an individual failure without discarding previous entries or unrelated branches.
- Rust [`Path::extension`](https://doc.rust-lang.org/stable/std/path/struct.Path.html#method.extension)
  returns the last suffix. Explicit compound-extension filtering must compare a
  dot-delimited filename suffix, independently of OS associations or known types.
- [`globset::GlobBuilder`](https://docs.rs/globset/latest/globset/struct.GlobBuilder.html)
  defaults to case-sensitive matching; choose case-insensitive matching explicitly
  for the application's name search and report invalid patterns rather than
  silently accepting every entry.
- Windows [backup privilege](https://learn.microsoft.com/en-us/windows/win32/secauthz/privilege-constants)
  grants read access with backup intent; it is not a blanket filesystem write
  grant. Keep the GUI unelevated and restrict the helper to requested reads.
- Windows [named-pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)
  and [peer process IDs](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getnamedpipeclientprocessid)
  support a local IPC channel bound to the launched helper and requesting process.

## Stage two: milestones and expected results

1. **Search semantics.** `filter/core`, `app/core/omni_accel.rs`, filter UI.
   Normalize bare/dotted/wildcard-prefixed, comma/semicolon/space-separated
   suffixes; arbitrary Unicode and compound suffixes work consistently; extension
   filters reject directories as direct hits. Invalid name patterns produce a
   visible validation message and no accidental all-match scan/copy.
   Acceptance: mixed-case uncommon/compound endings match exactly; empty folders
   do not appear; directory-only/name searches retain their intended behavior.
2. **Recursive result integrity.** `app/core/view_selection.rs`, a small pure
   recursive-tree module, `filter/core/scope.rs`, local/remote scan orchestration.
   Use scanner root identity and component-safe parent linkage; traverse folders
   even when their rows are hidden; retain bounded partial results and diagnostics;
   refresh once when an in-flight broad scan truncates after a narrower filter.
   Acceptance: roots, trailing separators, files-only mode, large directories,
   missing ancestors, access errors, cancellation and filter transitions preserve
   every discovered relevant file without a restart loop or stale-tab crossover.
3. **Collapsible folders.** Per-tab recursive presentation state, table input,
   keyboard navigation, summaries and selection.
   Acceptance: folding/unfolding is immediate, survives sorting/filter changes
   within a location, stays isolated per tab, preserves existing selections, and
   select-all/invert/copy operate on the complete filtered result set.
4. **Filtered transfers.** Clipboard and copy planning at local/remote boundaries.
   Acceptance: overlapping selections deduplicate; files-only and folded recursive
   selections retain paths relative to the displayed root; nonmatching siblings
   and empty structure folders are excluded; failures do not silently publish an
   incomplete clipboard or delete uncopied source files.
5. **Read access in the existing application.** A typed local-access module,
   Windows headless helper/IPC, scanner and analytics adapters, access UI, file
   read/copy adapters. Reuse existing backup-read and image-lock primitives.
   Acceptance: only an actual denial offers consent; cancellation/failure preserves
   current results; successful consent retries the original operation in the same
   view; a granted read capability is reusable for the authorized local root during
   the session; remote paths receive provider-specific errors, never local UAC;
   no separate admin window, ACL rewriting or unrestricted elevated command API.
6. **Integration and delivery.** Refresh the root AST graph, update README and
   affected access/transfer docs and the live TODO board; add exactly one checked-in
   task suite and one exact-SHA remote workflow following established patterns.
   Acceptance: focused portable and real Windows cases pass remotely, including
   scanner-to-view-to-copy and authenticated helper boundaries. The candidate is
   pushed with a `[task candidate]` head. Only afterward dispatch the existing
   `build.yml` complete-release transaction and inspect publication at intervals
   of at least 30 minutes.

Dependencies: 1 precedes 2/4; 2 precedes 3; 5 shares enumerator/file-read adapters
with 2/4; all implementation precedes the single suite invocation. Add fixtures
only after implementation. No per-milestone builds, tests, releases or version bumps.

## Second research pass / final-plan gate

Completed before source edits on 2026-09-19:

- Use a headless helper to open **read-only file/directory handles**, then transfer
  them into the original process with
  [`DuplicateHandle`](https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-duplicatehandle).
  Do not transfer an administrator token or relaunch the GUI. Keep the executable
  and its ancestors locked through UAC and verify its SHA-256 in the helper.
- Create one random, local-only pipe instance; check the client PID against the
  process returned by `ShellExecuteExW`, and check the server PID/image from the
  helper. Restrict the wire protocol to a root-bound read operation enum. Bound
  frames and waits, serialize calls per connection, close on protocol/peer failure,
  and stop the helper when the parent exits. The pipe's default ACL plus these
  peer checks must never authorize an unrelated process.
- Follow the [`CreateFileW`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
  backup-intent and reparse semantics. Lock and validate each ancestor while
  acquiring the requested handle; reject link traversal and paths outside the
  authorized root. Do not change ACLs, ownership or permit writes through the
  helper. Reuse granted reads for enumeration, metadata and copying source bytes.
- Share the existing Windows handle-based enumerator with the explorer scanner;
  carry full timestamps/attributes so extension/date/hidden filters agree with
  ordinary scans. Keep OS-specific code in adapters. Preserve copy source-identity
  and conflict checks when substituting a granted read handle.
- Store complete result rows and collapsed folder keys in one per-tab presentation
  object; `view` remains the displayed row list. Selection-all, inversion, totals
  and filtered recursive copy use complete result membership. Link parents by
  normalized component boundaries, with a virtual root fallback for interrupted
  streams; never require a spelling-identical root entry to render children.
- Keep memory ceilings explicit rather than replacing them with an unbounded walk.
  Emit large remote directories incrementally up to the budget; isolate depth and
  per-directory errors; keep partial results visible and offer a narrower refresh.
  If the user narrowed a still-running scan, do that refresh automatically once
  on truncation, not on every frame. Invalid filters retain no results and cannot
  be used as evidence that a previous pruned scan covered a later valid filter.
- Use a deduplicated snapshot of matching recursive files for recursive copy,
  including selected folded subtrees, with relative paths from the displayed root.
  Flat folder copy still expands the explicitly selected folders off the GUI
  thread. A stale selection cannot carry a now-filtered-out file into a copy.

Routine field/module wiring in existing near-limit application files is permitted
for these extracted responsibilities; new behavior lives in focused modules below
500 lines. No exception permits adding a new feature body to an oversized file.

## Implementation refinements and acceptance entrypoint

Inspection during integration also found that the recursive-mode flag was shared
across otherwise independent tabs. It now follows the tab's scan/filter state.
Wide local listings descend in bounded directory batches, so a pruning filter
does not accumulate every pending directory before producing its first hit.
Per-directory panic isolation and a flushing result sink preserve earlier rows.
The helper compares filesystem identities at the consented root in addition to
textual containment, covering case-sensitive NTFS sibling roots.

`FILEDESCRIPTORW` has a fixed relative-name buffer. Long snapshots are materialized
through the same existing download/copy adapter instead of truncating names.
Normal virtual files stream lazily from shared read handles; COM clones maintain
separate seek cursors. Cut and external folder drag keep their prior whole-folder
semantics. The read helper intentionally grants no protected destination writes
or deletes, and does not change remote-provider authentication.

The one suite is `native/test-search-recursive-access-task.py`, dispatched for an
exact candidate through `search-recursive-access-task.yml`. Its search/tree/copy
cases cover M1-M4, including real wide trees and long snapshot paths. M5 includes
protocol rejection, preserved results on failed consent, Windows deny-ACL/token
restoration, same-process reuse of handles supplied by an authenticated child,
helper shutdown and read-only COM streams. Existing directly affected directory,
scanner and analytics cases are selected into the same binary invocation. M6
records candidate/hash-bound acceptance on Linux and Windows before release.
