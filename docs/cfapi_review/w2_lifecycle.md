# W2 — Save-back, Lifecycle & File-Ops Integration of the CfAPI Remote-File Feature

Wave-2 audit. Builds on `w1_crate_map.md` (crate internals: the 13 callbacks +
`Connection` lifetime). This wave covers the parts the data/placeholder audits
touch least: **save-back mechanics**, **mtime-watch correctness**, **connection
lifetime across the whole app**, **stale sync roots / uninstall**, the
**Temp↔CfApi toggle**, and **many-files-one-connection**.

Code under audit:
`native/src/cfprovider.rs`, `native/src/app.rs`, `native/src/cfsync.rs`,
`native/src/daemon.rs`, `native/src/main.rs`, `native/src/shell_register.rs`.
Crate: `cloud-filter-0.0.6` (`filter/proxy.rs`, `filter/sync_filter.rs`,
`root/connect.rs`).

Severity tags: **BUG** (will misbehave), **RISK** (works today, fragile /
latent), **FIDELITY** (behaves but diverges from a true OneDrive-style provider /
user expectation), **OK** (correct/acceptable).

### Ground truth from Microsoft docs (cited throughout)

- **There is no "data changed / file dirtied" callback.** The complete
  `CF_CALLBACK_TYPE` enum is `FETCH_DATA, VALIDATE_DATA, CANCEL_FETCH_DATA,
  FETCH_PLACEHOLDERS, CANCEL_FETCH_PLACEHOLDERS, NOTIFY_FILE_OPEN_COMPLETION,
  NOTIFY_FILE_CLOSE_COMPLETION, NOTIFY_DEHYDRATE(+_COMPLETION),
  NOTIFY_DELETE(+_COMPLETION), NOTIFY_RENAME(+_COMPLETION), NONE`. None of these
  fires when an app *writes* a hydrated placeholder. The provider must detect
  local edits itself.
  ([CF_CALLBACK_TYPE](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type))
- **The Cloud Mirror sample detects local changes by watching the directory**
  (`CloudProviderSyncRootWatcher` → `DirectoryWatcher`), not by a callback; and
  it explicitly lists "when the local client file changes, the local sync client
  must notify the cloud service" as the provider's job, *not* the platform's.
  ([Build a Cloud Sync Engine](https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine))
- **NOTIFY_DELETE and NOTIFY_RENAME block the user app and expect a response**
  ("The user application that performs the rename/move is blocked. A response is
  expected from the sync provider.").
  ([CF_CALLBACK_TYPE](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type))
- **After `CfDisconnectSyncRoot` (or a provider crash) the platform fails any
  operation that depends on the callbacks**, and on crash "the platform will
  detect this and perform the necessary cleanup."
  ([CfDisconnectSyncRoot](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfdisconnectsyncroot))
- **Writing a placeholder dirties it; the provider must re-mark in-sync** after
  uploading via `CfSetInSyncState(CF_IN_SYNC_STATE_IN_SYNC, …)`, ideally guarded
  by the USN to avoid a race.
  ([CfSetInSyncState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetinsyncstate))
- **Every callback has a fixed 60-second timeout.**
  ([CF_CALLBACK_TYPE remarks](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type))

---

## 1. SAVE-BACK MECHANICS

### What actually fires on save

In CfApi mode the app does **not** rely on any CfAPI save callback. After the
placeholder hydrates, `open_file` registers a `RemoteEdit` and the app's own
mtime poller (`poll_remote_edits`, `app.rs:3226`) re-uploads via the backend
when the on-disk mtime advances and is stable. This is the only save-back path —
`cfprovider.rs:57-132` implements **only** `fetch_data` and
`fetch_placeholders`; everything else takes the crate default.

This matches the platform model: there is **no** CfAPI callback for "file data
was modified", so a provider must watch the file itself
([CF_CALLBACK_TYPE](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type);
[Cloud Mirror's DirectoryWatcher](https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine)).
So the *architecture* (watch + re-upload) is correct. The problems are in which
filesystem ops the editor performs and in the placeholder's CfAPI state.

### Which filesystem ops a "save" performs — and the consequences

Editors save in three different ways, and they interact very differently with a
CfAPI placeholder:

- **In-place rewrite** (Notepad's classic mode, many simple editors:
  `CreateFile(OPEN_EXISTING)` → `WriteFile` → `SetEndOfFile`): the existing
  placeholder/hydrated file is opened and overwritten in place. The mtime
  advances; `poll_remote_edits` sees it and uploads. **This is the only pattern
  the mtime watcher handles cleanly.** OK.

- **Write-temp-then-rename-over** (Word, Excel, VS Code, Notepad on Win11,
  atomic-save editors: write `~tmpXXXX`, then `ReplaceFile`/`MoveFileEx` over the
  target). The rename of the *new* temp file **onto the placeholder path** is a
  rename/replace **inside the sync root**. Per the docs this raises
  `CF_CALLBACK_TYPE_NOTIFY_RENAME`, the user app is **blocked**, and a response is
  expected. The crate's default `rename` returns
  `Err(CloudErrorKind::NotSupported)` (`sync_filter.rs:104-111`), which
  `proxy.rs:258-274` reports to the OS as `STATUS_CLOUD_FILE_NOT_SUPPORTED`.
  **The save can therefore fail or the editor can report "cannot save / access
  denied", or the placeholder is left in an inconsistent state.** Even where the
  OS lets the replace proceed (replacing a placeholder with a brand-new
  *non-placeholder* file outside the engine's blessing), the result is a normal
  file that **no longer carries our `FileIdentity` blob** (see desync below).
  — **BUG (save-as-rename-over is the single most common save pattern and the
  default `rename`=NotSupported actively fights it).**

- **Delete + create** (a few editors, and "Save As" to the same name): delete the
  placeholder then create a new file. Delete inside the sync root raises
  `CF_CALLBACK_TYPE_NOTIFY_DELETE` (blocking, response expected); the crate
  default `delete` is `Err(NotSupported)` (`sync_filter.rs:85-92`) →
  `STATUS_CLOUD_FILE_NOT_SUPPORTED`. **The delete (and thus the save) can be
  refused.** — **BUG/RISK** (less common than rename-over but same root cause).

### Does writing convert the placeholder to a normal/dirty file, and does it desync the blob?

Yes, and this is the critical correctness issue:

- Writing to a hydrated placeholder transitions it out of the **in-sync** state
  (it becomes "dirty"); the provider is expected to re-establish in-sync after
  uploading via `CfSetInSyncState(CF_IN_SYNC_STATE_IN_SYNC, …)`
  ([CfSetInSyncState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetinsyncstate)).
  **The CfApi path never calls `CfSetInSyncState` after a save-back upload.**
  (`cfsync::mark_in_sync` exists at `cfsync.rs:107-143` but is only wired into
  the *non-CfAPI* mirror path, never called from `cfprovider.rs` or from
  `poll_remote_edits`/`drain_edit_saves`.) Result: a saved file stays showing the
  "sync pending" overlay forever, even after a successful upload. — **FIDELITY**
  (cosmetic: wrong overlay; functionally the upload still happened).

- **Blob desync — the dangerous case.** For an **in-place rewrite**, the file
  keeps its placeholder identity and the `FileIdentity` blob (the remote path we
  stored at `cfprovider.rs:123`) survives, so a later `fetch_data` still resolves
  the correct remote path (`cfprovider.rs:64-69`). But for **rename-over /
  delete+create** saves, the file that ends up at the path is a **fresh,
  full (non-placeholder) file with NO `FileIdentity` blob**. If that file is
  later dehydrated and re-hydrated, `fetch_data` finds `blob.is_empty()` and
  falls back to `remote_of(&request.path())` (`cfprovider.rs:65-66`), i.e. it
  derives the remote path from the *local on-disk path*. That fallback is only
  correct when the local leaf name equals the remote name. For Google-Docs /
  transform backends the placeholder leaf was renamed by `download_name`
  (`.docx`/`.xlsx`, `cfprovider.rs:118`, `app.rs:3106`), so the derived remote
  path is **wrong** → a later fetch downloads the wrong item or 404s. — **BUG
  (latent): rename-over save drops the identity blob, and the no-blob fallback
  mis-resolves transformed-name files.**

### Severity-tagged issues

- **BUG** — Atomic "write-temp-then-rename-over" saves (Word/Excel/VS Code/Win11
  Notepad) hit `NOTIFY_RENAME` → default `NotSupported` → save can be blocked or
  corrupt the placeholder. `cfprovider.rs` does not implement `rename`.
- **BUG/RISK** — Delete+create saves hit `NOTIFY_DELETE` → default
  `NotSupported` → delete refused.
- **BUG (latent)** — Rename-over/delete+create strips the `FileIdentity` blob; the
  empty-blob fallback (`cfprovider.rs:65-66`) mis-derives the remote path for
  `download_name`-renamed (Google-Docs) files.
- **FIDELITY** — No `CfSetInSyncState` after save-back upload → permanent
  "sync pending" overlay on edited files. `cfsync::mark_in_sync` is unused by the
  CfApi path.

### Recommendations

- Implement `SyncFilter::rename` and `SyncFilter::delete` to **approve** the
  operation (use the ticket's pass/approve path) rather than letting them default
  to `NotSupported`, so atomic-save editors and deletes work. At minimum, approve
  in-place same-name replace.
- After a successful save-back upload (`drain_edit_saves`, `app.rs:3287`), call
  `CfSetInSyncState(IN_SYNC)` (reuse/extend `cfsync::mark_in_sync`) **USN-guarded**
  per the docs, to clear the dirty overlay.
- Harden the no-blob fallback: when `blob.is_empty()`, refuse rather than guess,
  or re-attach the blob via `CfConvertToPlaceholder` after save-back so identity
  survives a rename-over. Document that only in-place-rewrite editors are fully
  supported until `rename`/`delete` are implemented.

---

## 2. mtime WATCH CORRECTNESS

### The mechanism

`RemoteEdit` (`app.rs:682-692`) carries `baseline_mtime` (last
uploaded/downloaded mtime; a change above it = a save) and `seen_mtime` (a
one-cycle debounce so we don't upload mid-write). `poll_remote_edits`
(`app.rs:3226-3274`) runs at most every 1500 ms (`app.rs:3230`).

For CfApi mode the edit is registered with `baseline_mtime: i64::MAX` as a
**sentinel** (`app.rs:3123`) because there is no download step that could set a
real baseline (the file hydrates lazily through CfAPI, not through
`download_to`). The poller arms the baseline the first time it actually sees the
hydrated file (`app.rs:3243-3247`): on first sight it sets
`baseline = seen = m` and `continue`s — it does **not** treat the hydrated
content as an edit. Good.

### Trace: hydrate → save

1. Hydrate: `fetch_data` writes the content (`cfprovider.rs:88`), mtime = T0.
2. Poll cycle A sees mtime T0, `baseline_mtime == i64::MAX` → arm:
   `baseline = seen = T0`, continue. (No upload — correct, avoids re-uploading
   freshly hydrated content.)
3. User saves → mtime T1 (> T0).
4. Poll cycle B: `m == T1`, `T1 != baseline(T0)`, `T1 != seen(T0)` → else-branch:
   `seen = T1` (debounce; no upload yet).
5. Poll cycle C: `m == T1` still, `T1 == seen(T1)` → upload fires once;
   `uploading = true`, `baseline = T1`, launch upload (`app.rs:3251-3254`).
6. `drain_edit_saves` clears `uploading` on completion (`app.rs:3285`).

**Upload fires exactly once per save.** The debounce (one cycle where the mtime
is stable) is what makes this single-shot. OK.

### Failure modes found

- **Save during an in-flight upload is dropped.** `poll_remote_edits` filters to
  `!e.uploading` (`app.rs:3235`) and sets `baseline = T1` *before* the upload
  completes (`app.rs:3253`). If the user saves again (mtime T2) while the upload
  of T1 is still running, the poller skips that edit (uploading==true). When the
  upload finishes, `uploading` is cleared but `baseline` is already T1; if T2 was
  observed only while uploading, the next poll compares T2 to baseline T1 (T2 >
  T1) and re-arms via `seen` — so it is **eventually** caught. But if the editor
  writes T2 *equal-ish* fast and the mtime granularity collapses T1==T2, the
  second save is **missed**. Filesystem mtime is ms-resolution here
  (`file_mtime_ms`, `app.rs:671-678`); for a backend with coarse mtime two quick
  saves can share a timestamp. — **RISK (missed-save under rapid re-save).**

- **mtime regression after upload error.** On upload failure `drain_edit_saves`
  sets `baseline_mtime = 0` (`app.rs:3294`) to allow retry. With baseline 0, the
  next poll sees `m != 0` and `m != seen`? — actually `seen` still holds the last
  value, so it may upload on the next stable cycle. Acceptable, but baseline 0 is
  a magic value that also collides with `file_mtime_ms`'s "couldn't read" return
  of 0 (`app.rs:677`); a transiently-unreadable file (locked during save) returns
  0 and is `continue`d (`app.rs:3237`), so no false upload. OK, but the `0`
  overloading is fragile. — **RISK (minor).**

- **Chunked hydration / multiple mtime bumps.** Our `fetch_data` does a
  **single** `write_at` for the whole required range (`cfprovider.rs:84-88`), so
  hydration produces essentially one mtime bump, and the sentinel arms on it.
  **However**, if hydration is slow, the *first* poll after open can run **before**
  hydration finishes: the placeholder may already exist (mtime set by CfAPI at
  placeholder creation) with size 0 or partial. The sentinel would then baseline
  to that *pre-hydration* mtime, and when hydration completes and bumps the
  mtime, the poller would interpret the **hydration write as a user edit** and
  **re-upload the just-downloaded content back to the remote** (a no-op-ish but
  wasteful, and racy round-trip that can clobber a concurrent remote change).
  This is the classic "immediate re-upload of hydrated content" bug. It is *not*
  fully prevented: the sentinel only protects against treating the **first seen**
  mtime as an edit, not against a later hydration-completion bump.
  — **RISK→BUG (re-upload of hydrated content if the poll observes the
  placeholder before hydration completes).** The Temp-mode path avoids this
  because `drain_file_open` baselines to the **downloaded** mtime only *after* the
  download future completes (`app.rs:3211-3219`); the CfApi path has no equivalent
  "hydration complete" signal.

- **Note: the 1500 ms poll vs the 60 s callback timeout.** Hydration
  (`fetch_data`) must complete within CfAPI's fixed 60 s callback budget
  ([CF_CALLBACK_TYPE remarks](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type)).
  A slow backend (large file over SFTP) buffering the whole file in one
  `Vec`/`write_at` (`cfprovider.rs:85-88`, and W1 §8) can blow the 60 s budget →
  hydration fails → the file shows as never-hydrated even though the poller is
  watching. — **RISK (cross-references W1's no-chunking finding).**

### Severity-tagged issues

- **RISK→BUG** — No "hydration complete" baseline for CfApi; if a poll observes
  the placeholder before hydration's mtime bump, the hydration write is mistaken
  for a user edit and re-uploaded.
- **RISK** — Rapid re-save during an in-flight upload can be missed when two
  saves collapse to the same ms mtime.
- **RISK (minor)** — `baseline_mtime = 0` magic value overloads
  `file_mtime_ms`'s error sentinel.

### Recommendations

- Give the CfApi path a real "hydration done" baseline: implement
  `SyncFilter::opened`/`closed` (already wired in `proxy.rs:169-191`, both no-op
  in `cfprovider.rs`) or have `fetch_data` record the post-hydration mtime, and
  baseline the `RemoteEdit` to that, instead of relying on "first sight" in the
  poller. Alternatively, only arm the watch after the file's size matches the
  placeholder's declared size.
- Use a monotonically increasing **content hash or size+mtime tuple**, not bare
  ms-mtime, to detect rapid re-saves.
- Replace the `0` baseline-reset magic with an explicit `armed: bool` /
  `Option<i64>`.

---

## 3. CONNECTION LIFETIME ACROSS THE APP

### How it connects

`ensure_mounted` (`cfprovider.rs:178-228`) connects **lazily on first
`open_file`** in CfApi mode (`app.rs:3097`). It registers the sync root if not
already registered (`cfprovider.rs:193-211`), then `Session::connect`
(`cfprovider.rs:217`) and stores the `Connection<RemoteFilter>` in a
process-lifetime `static` registry keyed by local-root path
(`cfprovider.rs:163-166,226`). Per W1's lifetime section, the `Connection` holds
the **only strong `Arc<RemoteFilter>`**; dropping it calls
`CfDisconnectSyncRoot` and frees the filter, after which callbacks no-op
(`connect.rs:57-66`, `proxy.rs:295-313`). So **hydration works only while the GUI
process runs.**

`App::new` (`app.rs:797`) only *loads the mode preference* (`app.rs:985`); it
never calls `ensure_mounted`. **There is no startup mount and no reconnect of
known roots.** `main.rs` likewise establishes no CfAPI connection at startup
(`main.rs:44-111`).

### (a) Open the placeholder from Explorer after the app is closed

The sync root **registration persists** (WinRT `Register`, never unregistered on
exit — see Q4), so the folder still appears in Explorer with placeholders on
disk. But the **`Connection` is gone** (process exited → registry static
dropped → `CfDisconnectSyncRoot`; or process crashed → platform auto-cleanup).
Opening an **un-hydrated** placeholder now has **no provider to answer
`fetch_data`**. Per the docs, "after a call to `CfDisconnectSyncRoot` returns,
the sync provider will no longer receive callbacks and **the platform will fail
any operation that depends on said callbacks**"
([CfDisconnectSyncRoot](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfdisconnectsyncroot)).
So Explorer/the app shows the placeholder, but opening it **fails** (the OS
returns a cloud-file error such as `ERROR_CLOUD_FILE_*` /
`STATUS_CLOUD_FILE_NOT_RUNNING`-class) instead of hydrating. Already-hydrated
("full"/pinned) files still open from local disk. — **BUG/FIDELITY (offline
placeholders are dead until the GUI is relaunched *and* the user re-opens that
file through the app to re-mount).**

### (b) Does it reconnect on next launch?

Not at startup, and not for "all known roots." It reconnects **only** when the
user next does `open_file` in CfApi mode for a connection whose
`ensure_mounted` runs again (`app.rs:3097` → `cfprovider.rs:185` finds no
registry entry → reconnects). So a placeholder browsed in Explorer (outside the
app) still won't hydrate until the user routes the open back through Smart
Explorer. **It should reconnect all registered Smart Explorer sync roots at
startup** so Explorer-initiated hydration works. — **RISK/FIDELITY.**

### (c) The `--sync-daemon` background process

`smart_explorer.exe --sync-daemon` runs `daemon::run_daemon` (`main.rs:57-60`,
`daemon.rs:146-183`). It **only runs bisync jobs** (`run_one` →
`bisync::run`, `daemon.rs:102-142`); it **never calls `cfprovider::ensure_mounted`
or touches CfAPI** (confirmed: no `cfprovider`/`ensure_mounted` reference in
`daemon.rs`). So the persistent background process does **not** serve
placeholders. Only the GUI mounts/serves CfAPI. The daemon is the natural home
for a long-lived provider but doesn't do it today. — **FIDELITY (a true
OneDrive-style provider would hydrate even when the GUI is closed; ours can't).**

### Severity-tagged issues

- **BUG/FIDELITY** — Placeholders are non-functional whenever the GUI isn't
  running; opening one from Explorer fails per `CfDisconnectSyncRoot` semantics.
- **RISK/FIDELITY** — No startup reconnect; re-mount only happens on the next
  in-app `open_file`, never for Explorer-initiated opens.
- **FIDELITY** — `--sync-daemon` does not serve CfAPI; no headless provider.

### Recommendations

- At GUI startup (and ideally in the daemon), enumerate registered Smart Explorer
  sync roots and `ensure_mounted` each, so Explorer-driven hydration works
  without re-opening through the app.
- Move (or also run) the CfAPI provider in `--sync-daemon` so placeholders
  hydrate when the GUI is closed; that is the OneDrive model the UI text promises
  ("wie OneDrive", `app.rs:6726`).
- If a true background provider is out of scope, **soften the promise** in the
  settings UI and surface a clear "provider offline — reopen via Smart Explorer"
  message when an Explorer open fails.

---

## 4. STALE SYNC ROOTS

### What persists

`SyncRootId::register` (WinRT `StorageProviderSyncRootManager::Register`,
`cfprovider.rs:199-210`) persists the sync root **across reboots and across app
exit**. The code **only ever unregisters in one place**: the connect-failure
cleanup in `ensure_mounted` (`cfprovider.rs:222`,
`let _ = sync_root_id.unregister();`). **On normal exit nothing unregisters**
(the registry static just drops, which disconnects but does **not**
unregister — `connect.rs` Drop comment: "this does **NOT** mean the sync root
will be unregistered").

### What accumulates

- One **registered sync root per distinct connection label** (the provider id is
  `SmartExplorer_<sanitized label>`, `cfprovider.rs:168-174`). Open files on N
  connections over the app's life → N persistent sync-root registrations that
  outlive every run and never get cleaned up.
- Each leaves a branded node in Explorer's navigation pane and a registry key
  under the StorageProvider sync-root hive (per
  [Build a Cloud Sync Engine → sync-root registration](https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine)).

### Does uninstall clean them up?

**No.** The uninstall hook is `main.rs:65-69` (`--unregister` →
`shell_register::unregister_all`), and `unregister_all` (`shell_register.rs:213-216`)
only removes the **context-menu verb** and the **default-manager** keys. It does
**not** call `SyncRootId::unregister`, `CfUnregisterSyncRoot`, or
`StorageProviderSyncRootManager::Unregister` for any CfAPI root (grep confirms the
only `unregister` of a sync root in the whole tree is `cfprovider.rs:222`). So
**uninstalling Smart Explorer leaves every CfAPI sync root it ever created
registered**, pointing at `%USERPROFILE%\Smart Explorer\<conn>` folders whose
provider will never connect again.

This is exactly the failure mode behind the earlier **`-2145452027`
(`0x8007016B` ERROR_CLOUD_FILE_PROVIDER_NOT_RUNNING)** / invalid-name class of
issue: a **registered-but-unconnected** root makes Explorer attempt to route the
folder through a dead provider. The Cloud Mirror docs warn about the same shape —
"If the sample crashes, it's possible that the sync root will remain registered.
This will cause File Explorer to relaunch every time you click on anything … If
this occurs, uninstall the … application."
([Build a Cloud Sync Engine → Use the sample](https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine)).
Our app reproduces that hazard and never cleans it on uninstall. — **BUG
(orphaned sync roots survive uninstall and can break Explorer for those
folders).**

### Severity-tagged issues

- **BUG** — No unregister of CfAPI sync roots on uninstall (`unregister_all` omits
  them); orphaned registered-but-unconnected roots accumulate and can break
  Explorer (the `-2145452027` class).
- **RISK** — Roots accumulate one-per-label across the app's lifetime with no GC
  even during normal use.

### Recommendations

- In `unregister_all` (`shell_register.rs:213`), enumerate Smart Explorer sync
  roots (provider ids prefixed `SmartExplorer_`) and call
  `StorageProviderSyncRootManager::Unregister` / `SyncRootId::unregister` for each
  before deleting the `%USERPROFILE%\Smart Explorer` tree.
- Track created roots (e.g. a list in `%APPDATA%`) so uninstall and a "reset
  cloud roots" maintenance action can find and remove them all.
- Consider unregistering a root when its connection is intentionally torn down
  (mode switch to Temp / connection removed), not only on connect failure.

---

## 5. TOGGLE & MODE (Temp ↔ CfApi)

### How the toggle works

`RemoteOpenMode` (`app.rs:588-591`) is persisted as plain text in
`%APPDATA%\smart_explorer\remote_open_mode.txt`
(`load_remote_open_mode`/`save_remote_open_mode`, `app.rs:602-618`). The settings
radio (`app.rs:6714-6736`) writes the file and updates the in-memory mode; the
change takes effect on the **next** `open_file`.

### Is the root cleaned up when switching CfApi → Temp?

**No.** Nothing in the toggle handler (`app.rs:6732-6736`) touches CfAPI: it does
not disconnect the live `Connection`, does not `unregister` the sync root, and
does not delete the mirror folder. A user who opened a file in CfApi mode (thus
`register` + `connect` a root) and then switches to Temp leaves the **sync root
registered and connected** (the registry static still holds the `Connection` for
the process lifetime). — **RISK** (leaked root + provider for the rest of the
session; combined with Q4, also leaked across uninstall).

### Folder conflict between the two modes

Both modes can target the **same** `%USERPROFILE%\Smart Explorer\<conn>` tree:

- CfApi: `cfsync::conn_root_dir(label)` = `<base>/<conn>` is the **sync root**
  (`cfprovider.rs:183`, `cfsync.rs:40-42`), and files land at
  `local_path_named(...)` under it (`app.rs:3107`, `cfsync.rs:64-67`).
- Temp on **non-Windows** uses `cfsync::local_path(...)` under the **same**
  `<base>/<conn>` tree (`app.rs:3166`). On Windows, Temp uses
  `%TEMP%\smart_explorer_open` instead (`open_temp_path`, `app.rs:664-668`), so
  on Windows the two modes don't share the folder — **but the CfApi *registration*
  on that folder remains**.

If a user toggles modes on Windows, the registered sync root still claims the
`<base>/<conn>` folder as a CfAPI sync root, while Temp mode writes elsewhere —
no direct file collision, but the folder is still a half-owned, registered sync
root with a possibly-disconnected provider (a stale-root hazard, Q4). On
non-Windows both modes write into the same mirror tree; since non-Windows has no
real CfAPI it's just a shared mirror (acceptable). — **RISK** (Windows: a
registered CfAPI root persists even though the user moved to Temp mode; the
folder keeps OneDrive-style behavior the user thinks they turned off).

### Severity-tagged issues

- **RISK** — Switching CfApi → Temp does not disconnect/unregister the CfAPI root
  or delete its folder; the root and its in-process `Connection` leak for the
  session and the registration persists indefinitely (feeds Q4).
- **OK** — No raw file-content collision on Windows (Temp writes to `%TEMP%`); on
  non-Windows the shared mirror is benign (no CfAPI there).

### Recommendations

- On switching **away** from CfApi, disconnect the connection (drop the registry
  entry) and offer/auto-do `SyncRootId::unregister` for that label, so the folder
  reverts to a plain directory.
- Document (or enforce) that the `%USERPROFILE%\Smart Explorer\<conn>` tree is
  owned by exactly one mode at a time.

---

## 6. MULTIPLE FILES / DIRS (one sync root, many placeholders)

### Design

One connection (label) = one sync root = one `Connection<RemoteFilter>` in the
registry (`cfprovider.rs:163-166`). `ensure_mounted` is idempotent: a second
`open_file` on the same label finds the existing registry entry and returns the
existing local root without reconnecting (`cfprovider.rs:185-186`). All
placeholders under that root are served by the **same** `RemoteFilter`, which
maps each placeholder's blob/path back to a remote path
(`cfprovider.rs:41-55,64-69,98-104`). This is the intended CfAPI model — one
provider serves an entire sync-root subtree — and is fine.

### Concurrency considerations

- Callbacks run on **arbitrary CfAPI worker threads, possibly concurrently**
  ([CF_CALLBACK_TYPE remarks](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type);
  W1 §5/§6). Opening several files at once → concurrent `fetch_data` calls into
  the **single shared backend**. The SFTP/FTP backends serialize internally
  (locks, per W1 §threading), so this is correct but **serializes** concurrent
  hydrations (perf, not correctness). — **RISK(perf), OK(correctness).**
- The `RemoteEdit` watch list is capped at 50 (`app.rs:3117,3169`); opening a
  51st file silently **does not register a save-back watch** (the placeholder
  still opens and hydrates, but edits to it won't upload). With one connection
  and many files this cap is reachable. — **RISK (silent loss of save-back past
  50 open edits).** The list is only pruned by exact-path replace
  (`retain(|e| e.temp != dest)`, `app.rs:3116,3168`), never by "file closed", so
  it monotonically fills.
- `populate_to` (`cfprovider.rs:142-160`) re-walks ancestor directories on every
  open to force population; with many files in deep trees this repeats work but is
  correct. — **OK.**
- Per-placeholder creation errors are swallowed: `pass_with_placeholder` returns a
  single `Ok` for the whole batch and the code never inspects per-item
  `.result()` (W1 §9). With many siblings in one directory, a single bad name
  (NUL / >4 KiB blob → panic across FFI, or a duplicate) can drop or crash the
  whole listing. — **RISK (inherited from W1 §6/§7/§9).**

### Severity-tagged issues

- **OK** — One `Connection`/`RemoteFilter` serving many placeholders is the
  correct CfAPI model; idempotent mount is sound.
- **RISK** — 50-entry `remote_edits` cap silently drops save-back for further
  files; the list never shrinks on close.
- **RISK(perf)** — Concurrent hydrations serialize on the shared backend's lock.
- **RISK** — Batch placeholder errors/panics (W1 §6/§7/§9) scale with directory
  size.

### Recommendations

- Evict `remote_edits` on file close (wire `SyncFilter::closed`,
  `proxy.rs:181-191`, currently no-op) or use an LRU keyed by path, instead of a
  hard 50 cap that silently drops watches.
- Surface a notice when the watch cap is hit so the user knows edits to that file
  won't upload.
- (Cross-ref W1) guard placeholder blob length and filename NUL before
  `pass_with_placeholder` to avoid a single bad sibling taking down a whole
  directory listing.

---

## Cross-cutting severity summary

| # | Area | Top issue | Severity |
|---|------|-----------|----------|
| 1 | Save-back | Atomic rename-over save → `NOTIFY_RENAME`=NotSupported blocks/corrupts save | **BUG** |
| 1 | Save-back | Rename-over strips `FileIdentity` blob → wrong remote path on later fetch | **BUG (latent)** |
| 1 | Save-back | No `CfSetInSyncState` after upload → permanent "pending" overlay | **FIDELITY** |
| 2 | mtime watch | No "hydration complete" baseline → hydration write can be re-uploaded | **RISK→BUG** |
| 2 | mtime watch | Rapid re-save during in-flight upload can be missed | **RISK** |
| 3 | Lifetime | Placeholders dead whenever GUI is closed (Explorer open fails) | **BUG/FIDELITY** |
| 3 | Lifetime | No startup reconnect of known roots | **RISK/FIDELITY** |
| 3 | Lifetime | `--sync-daemon` doesn't serve CfAPI | **FIDELITY** |
| 4 | Stale roots | Uninstall never unregisters CfAPI roots → orphaned roots break Explorer | **BUG** |
| 5 | Toggle | CfApi→Temp switch doesn't disconnect/unregister the root | **RISK** |
| 6 | Many files | 50-entry watch cap silently drops save-back; never shrinks | **RISK** |

The two must-fix items: **(Q1) implement `rename`/`delete` + re-mark in-sync so
real editors can save**, and **(Q4) unregister CfAPI sync roots on uninstall**
(plus reconnect at startup, Q3). Everything else is hardening on top of a sound
watch-and-upload architecture.
