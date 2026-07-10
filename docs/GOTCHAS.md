# Gotchas & Dead Ends (read before "improving" these)

Hard-won, verified findings. Each cost real debugging. Don't re-tread them.

## Build / toolchain

- **GNU toolchain only.** Built with `x86_64-pc-windows-gnu` + Strawberry GCC as
  linker (not MSVC). `export PATH="$USERPROFILE/.cargo/bin:/c/Strawberry/c/bin:$PATH"`.
- **comctl32 `TaskDialogIndirect` = instant crash.** Do NOT use
  `rfd::MessageDialog` — it statically imports `comctl32!TaskDialogIndirect`,
  which only exists in comctl32 v6. Without an embedded v6 manifest (the GNU
  toolchain doesn't add one), the loader binds v5, the import is unresolved, and
  the process dies at load with exit `0xC0000139` (STATUS_ENTRYPOINT_NOT_FOUND)
  **before any Rust runs** (so no crash.log). Use the `confirm_yes_no` helper
  (`MessageBoxW`) instead. `rfd::FileDialog::pick_folder` is fine (no TaskDialog).
  Diagnose load-time "entry point not found" by diffing `objdump -p` imports of
  the broken vs last-working exe.
- **`russh` crypto backend.** Its default is `aws-lc-rs` (needs NASM/CMake, breaks
  on GNU). Use `default-features = false` + `ring`. Still verify it compiles
  before building on top of it — see REMOTE_LAYER_PLAN §5.
- **PowerShell 5.1 + cargo.** cargo writes progress to stderr, which PS 5.1 turns
  into error records → trips `throw` in scripts even on success, and the tool may
  report failure on exit 0. Run cargo via the Bash tool (`2>/dev/null`); do
  file/version/makensis steps as separate simple PS calls. Quote makensis args:
  `& $makensis "/DVERSION=x.y.z" "installer.nsi"`.
- **Bundled SQLite for sync state.** `rusqlite` is built with `bundled` so
  incremental mirror state does not depend on a system SQLite DLL. Keep the GNU C
  toolchain available for release builds, and do not drop `bundled` unless the
  Windows and WSL feed build both prove the replacement path.
- **cargo-audit quick-xml advisories via plist.** `quick-xml 0.39.2` is currently
  pinned by `plist 1.9.0` through `netdev`/`iroh` and by Wayland build tooling.
  CI explicitly ignores only `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195` on the
  main native lockfile after attempting the available `iroh`/`netdev` updates.
  Remove those ignores as soon as the transitive path can move to
  `quick-xml >=0.41.0`; do not broaden the audit suppression.

## egui / UI

- **Ctrl+C / Ctrl+X / Ctrl+V are NOT delivered as key events.** egui's winit
  backend turns them into semantic `Event::Copy/Cut/Paste` — and for a FILE
  clipboard (CF_HDROP, no text) it emits NEITHER a paste event NOR a key event,
  and when idle triggers no repaint at all. So `consume_key(V)` and in-frame
  polling both fail. Fix (in `app.rs`): a dedicated background thread polls
  `GetAsyncKeyState` ~30×/s, gated to our foreground window, and wakes the UI.
  Clipboard keybindings can't be unit-tested — verify in the running GUI.
- **`ui.columns` does not clip.** The wide table bled into the other split-screen
  pane. Use per-pane `allocate_ui_at_rect` + `ui.set_clip_rect(rect)` + a painted
  divider (see `ui_central`).
- **Tabs use a swap model.** The active tab's state lives in the `App` fields;
  inactive tabs are parked in `TabState` and `mem::swap`'d in/out
  (`swap_with_tab`). Split-screen renders the non-focused pane by swapping its
  tab in around the `ui_table` call, then swapping back. Any new per-view state
  must decide: per-tab (add to `TabState` + `swap_with_tab`) or global.

## Windows shell integration

- **You cannot replace other apps' Open/Save dialogs system-wide.** No registered
  default file picker exists; the dialogs are created in-process per app. Only
  DLL injection could do it (unsupported, breaks on UWP/sandboxed, AV-flagged).
  Directory Opus refuses to. Out of scope, permanently.
- **Registry default-file-manager override does NOT redirect folder double-clicks
  on Win11.** Writing `HKCU\Software\Classes\Directory\shell\open\command` (with
  no DelegateExecute/ddeexec) was supposed to shadow the Folder-class handler.
  **Live test disproved it**: with the exact keys written by hand, ShellExecute
  "open" on a folder still launched Explorer. Win11 routes folder activation
  through the Folder class's `DelegateExecute` COM handler, which wins; in-window
  navigation never consults the verb at all. The toggle shipped in 0.3.4 and was
  REMOVED in 0.3.5 (with a startup self-heal). The ONLY thing that actually
  redirects double-clicks is a background window-hook (FileExplorerInterceptor
  style) — invasive, flashy, declined. `shell_register.rs` keeps the reversible
  context-menu verb + the (proven-correct, reversibility-tested) registry
  helpers, but the default-manager feature is gone.
- **Win11 MAIN/modern context menu needs a SIGNED package.** A legacy registry
  verb (our `OpenInSmartExplorer`) only ever appears under "Show more options".
  Reaching the main menu requires an `IExplorerCommand` COM handler **plus**
  package identity (sparse/MSIX) **signed with a cert the machine trusts**. A
  self-signed cert means asking every user to trust it (non-starter / needs
  admin). Not shippable for an unsigned per-user app without buying a
  code-signing cert (e.g. Azure Trusted Signing). The Rust COM DLL itself is
  feasible; the signing+packaging is the wall. **Update (0.5.2):** the COM half
  is now BUILT — `native/explorer-command/` is a cdylib that implements
  `IExplorerCommand` + `IClassFactory` and exports `DllGetClassObject` /
  `DllCanUnloadNow`; it compiles + links to a real PE DLL on windows-gnu. The
  sparse-package `AppxManifest.xml` is written too. Everything up to **signing**
  is done; signing remains the wall. See `docs/WIN11_CONTEXT_MENU.md`.

## Updater

- Per-user install under `%LOCALAPPDATA%\Programs\Smart Explorer\`; app data in
  `%APPDATA%\smart_explorer\`. A normal check downloads the OS-specific
  app/updater/`se` trio to app data, verifies every SHA-256, and atomically
  persists a staging manifest. It does **not** replace an installed file or stop
  a process. The update dialog requires explicit consent; **Later** retains the
  same verified staging across restart, while **Discard** removes it.
- Applying is a hash-bound transaction run by the separate updater helper. The
  helper revalidates itself and all staged payloads, waits for the exact parent
  PID to exit, requests a graceful daemon stop, and refuses to continue while a
  matching Smart Explorer binary is still running. Do not reintroduce force-kill
  behavior to make an update appear successful.
- Before replacement, the helper verifies and durably archives the outgoing app
  with a SHA-256 sidecar. It prepares checked sibling files, then replaces the
  updater, `se`, and app as one rollback-capable transaction. A failed replace or
  failed launch restores the previous targets and attempts to relaunch the
  verified old app. Length checks alone are not an acceptable replacement for
  the immediate SHA-256 revalidation at each process/replacement boundary.
- Release publication is also transactional: build into isolated staging,
  verify all six Windows/Linux feed payloads and hashes plus the Windows build
  manifest, promote with rollback backups, and move `version.txt` last. A
  Windows-only partial build must never mutate the shared feed or ancillary
  release files; direct runs require explicit, separate `-Feed` and
  `-ReleaseOutput` paths.
- **Rollback** archives the running version on startup and again as a mandatory
  precondition to replacement, but archives still accumulate only **going
  forward**. Remote rollback discovery uses GitHub Releases first and a legacy
  `release/vX.Y.Z` branch fallback. The atomic `update_pinned.txt` file pauses
  automatic forward checks after manual rollback; applying a newer staged update
  clears the pin only after the new app has launched successfully.
