# Editing remote files in place — strategy decision (#23)

> **Status note:** the strategy discussion below is historical. The current
> source tree ships remote open/save-back through temp-copy + mtime watch. There
> is no active native Cloud Files provider (`cfprovider.rs`/`cfsync.rs`) in
> `native/src`; the CfAPI work is retained as research and would need to be
> revived as a new feature with the safety fixes in `docs/CFAPI_REVIEW.md`.

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

### Original CfAPI shape considered (`native/src/cfsync.rs`, Windows-only)
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

### Original interim note
If a working "edit & save back" is wanted *before* CfAPI lands, the temp-watch
(option 1) is the only thing shippable today — but it carries the UX downsides
above. Historical recommendation at the time: go straight to CfAPI rather than
ship-then-replace. Current shipped behavior is the temp-watch path described
below.

---

## SHIPPED STATUS (current)

Remote open/save-back is implemented through **Temp-Kopie**:
`open_temp_path` in `%TEMP%`, watched by `RemoteEdit` / `poll_remote_edits`, and
re-uploaded via `upload_file` on save (1.5 s debounce). This is universal across
SFTP/FTP/WebDAV/Drive/peer backends and remains the active code path.

The native Cloud Files API provider is **not currently shipped**. The old
provider/mirror notes below and in `docs/CFAPI_REVIEW.md` are historical
research; reviving true on-demand placeholders would require a new Windows-only
feature with real `CfRegisterSyncRoot`/`CfConnectSyncRoot` lifecycle handling,
safe FETCH_DATA chunking, rename/delete callback coverage, startup reconnect,
and a real Windows smoke test.
