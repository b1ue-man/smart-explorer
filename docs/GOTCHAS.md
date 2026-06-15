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
  `%APPDATA%\smart_explorer\`. Self-update = "rename dance" (works on a running
  exe without admin). Feed = a folder with `version.txt` + `Smart Explorer.exe`;
  publish exe FIRST, then version.txt.
- **Rollback** archives the outgoing binary to `<install>/versions/` and also
  archives the running version on startup. It only accumulates **going forward** —
  jumping from a pre-rollback version straight to latest leaves nothing to roll
  back to (no copies of old binaries exist to seed it). The pin file
  (`update_pinned.txt`) pauses auto-update after a manual rollback; "update to
  latest" clears it.
