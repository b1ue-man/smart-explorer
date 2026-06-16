# Editing remote files in place — strategy decision (#23)

Question: when you open a remote file, edit and save in its normal app, the save
should land back on the remote — ideally with the path *being* the remote, no
temp juggling. What's the right mechanism? (Researched, then decided.)

## Options researched

1. **Temp clone + watch + upload-on-save** (WinSCP "Edit", Sublime SFTP, my
   first cut). Download to `%TEMP%`, launch the app, watch the temp file, upload
   on change.
   - ✅ Works with *any* app and *any* backend; simple; testable.
   - ❌ The path is `%TEMP%\…`, not the remote; client-side change-tracking;
     stale-copy/race risk; "fumbling between multiple remotes" — exactly the UX
     the maintainer dislikes.

2. **Remote agent (the VS Code Remote-SSH model).** VS Code installs a **server
   on the remote host**; the editor/extensions run *there*, the UI is local, so
   "the path is the remote" because file I/O happens on the remote machine.
   - ✅ Gold standard *within its own editor*; great for browse/search (this is
     our earlier "peer-agent Backend" idea).
   - ❌ Does **not** let arbitrary local apps (Word, Photoshop, a PDF viewer)
     open a remote path — it runs *its own* editor remotely. Wrong tool for
     "open in the file's associated app."

3. **Filesystem mount — WinFsp / rclone / sshfs-win.** Mount the remote as a real
   drive letter; any app opens the real path; save-back is transparent.
   - ✅ Truly seamless for any app.
   - ❌ Requires installing **WinFsp** (a third-party kernel-mode filesystem
     driver) + a mount engine — **not barrier-free** (the maintainer's goal),
     and a heavy external dependency to bundle.

4. **Windows Cloud Files API (CfAPI) — placeholder "sync root".** The native
   Windows mechanism (since 10/1709) behind OneDrive's on-demand files. We
   register a sync root; the remote shows up as **real local paths** under a
   folder; the OS minifilter `cldflt.sys` calls our engine to **hydrate on open**
   and notifies us to **upload on change**.
   - ✅ Native — **no third-party driver, no install** (barrier-free); the path
     is a real local path that *is* the remote item; on-demand hydrate; save-back
     via change notifications; no temp juggling; works for any app. Maps cleanly
     onto our `vfs::Backend` (hydrate = `open_read`, upload = `open_write`).
   - ❌ Windows-only (fine — we're Windows-targeted); COM/Win32-heavy; a large
     implementation; **can't be exercised in the headless Linux build env**.

## Decision

**Reject temp-clone-watch as the strategy.** Adopt the **Windows Cloud Files API
placeholder sync root** as the proper, barrier-free, "path-is-the-remote, save
flows back, no tracking" implementation. It's the same model OneDrive uses, needs
no extra install (unlike WinFsp), and unifies with our backends — it even
subsumes "browse the remote as if local." The VS Code agent model is kept as a
separate future feature for *fast remote browse/search* (the peer-agent idea),
not for opening files in local apps.

### Shape of the CfAPI implementation (`native/src/cfsync.rs`, Windows-only)
- `CfRegisterSyncRoot` for a per-connection root under e.g.
  `%USERPROFILE%\Smart Explorer\<label>`; `CfCreatePlaceholders` for the listing
  (lazy — directories first, files as placeholders with size/mtime).
- A `CfConnectSyncRoot` callback table:
  - `CF_CALLBACK_TYPE_FETCH_DATA` → stream `backend.open_read(remote)` into the
    placeholder via `CfExecute(TRANSFER_DATA)` (hydrate on open).
  - `CF_CALLBACK_TYPE_VALIDATE_DATA` / dehydrate as needed.
- Watch the sync root with `ReadDirectoryChangesW` for **modified/new** files →
  `backend.open_write(remote)` (upload on save), then mark in-sync; deletes/renames
  mirrored to the backend.
- Unregister + cleanup on disconnect (reversible, like our shell integration).

### Honesty / risk
This is a substantial native effort and **cannot be tested in this environment**
(no Windows, no `cldflt.sys`). It will compile for `x86_64-pc-windows-gnu`, but
the placeholder/hydration/callback lifecycle needs a real Windows smoke-test.
Gated/opt-in so existing users are unaffected.

### Interim?
If a working "edit & save back" is wanted *before* CfAPI lands, the temp-watch
(option 1) is the only thing shippable today — but it carries the UX downsides
above. Recommendation: go straight to CfAPI rather than ship-then-replace.

---

## SHIPPED STATUS (0.5.25)

Both open modes exist and are **user-toggleable** (Einstellungen → REMOTE-DATEIEN
ÖFFNEN); save-back works in both:

- **Temp-Kopie** — `open_temp_path` in `%TEMP%`, watched by `RemoteEdit` /
  `poll_remote_edits`, re-uploaded via `upload_file` on save (1.5 s debounce).
  Ephemeral, universal.
- **CfAPI / Platzhalter** — `cfsync::local_path`: a **persistent per-connection
  sync folder** `%USERPROFILE%\Smart Explorer\<conn>\<remote layout>`, hydrated
  eagerly and watched by the same save-back mechanism. The path is stable and
  mirrors the remote.

**Still to land (needs a real Windows test):** the *native* Cloud Files API layer
on top of that folder — `CfRegisterSyncRoot` + `CfConnectSyncRoot` FETCH_DATA
hydration (`CfExecute` TRANSFER_DATA) + OS save notifications — so files become
true on-demand **placeholders** (download lazily, show the OneDrive-style status)
instead of eager copies. The `cfsync` folder is exactly the sync-root location
that layer will register; it's gated behind the toggle and Windows-only.
