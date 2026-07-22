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
- **Release builds must stay inside the memory budget.** The desktop crate once
  exhausted an 8-GiB host with full LTO and displaced unrelated processes.
  Keep ThinLTO, eight codegen units, `CARGO_BUILD_JOBS=1`, and non-incremental
  builds pinned in every canonical release leaf. The top-level wrapper also routes the
  large Linux tree through `native/run-release-memory-bounded.sh`, which uses a
  3-GiB high/4-GiB hard cgroup limit plus at most 1 GiB of swap when systemd
  scopes are available. Do not weaken or make these limits ambient overrides;
  ordinary diagnostic builds may use separate Cargo settings.
- **Embedded SSH-agent binaries retain source paths.** Unmapped Cargo registry
  paths make an otherwise identical static-musl agent differ between a local
  build and GitHub's `/home/runner` build, tripping the intentional exact-byte
  freshness guard. Keep the encoded repository/`CARGO_HOME` path remaps and the
  forced target directory in `native/build-agent-bundles.sh`; it intentionally
  rejects ambient `RUSTFLAGS` variables. Rebuild and commit both
  `native/agent-bin/` payloads instead of weakening the CI comparison.
- **Bundled SQLite for sync state.** `rusqlite` is built with `bundled` so
  incremental mirror state does not depend on a system SQLite DLL. Keep the GNU C
  toolchain available for release builds, and do not drop `bundled` unless the
  Windows and WSL feed build both prove the replacement path.
- **Linux Exec talks to systemd through target-specific `zbus`.** Keep `zbus`
  under the Linux Cargo target section with `default-features = false` and the
  Rust-only Tokio/blocking API features. The Exec provider calls
  `StartTransientUnit` directly; replacing it with `systemd-run` would trust a
  shell/PATH boundary, while moving `zbus` to shared dependencies would burden
  and risk the Windows GNU build.
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

## Windows remote-drive integration

### Dokany remote-drive boundary

- **This is a real user-mode filesystem, not CfAPI.** The selected backend root
  is exposed as a Windows drive letter through Dokany callbacks. Do not add a
  Cloud Files sync-root registration, placeholders, hydration callbacks, or
  `cldflt.sys` state to this path. Cryptomator is the useful UX analogy; Smart
  Explorer owns the backend proxy and whole-file cache rather than a vault.
- **Dokany is exact-versioned but the reviewed MSI is installer input.** The
  supported runtime is the official Dokany 2.3.1.1000 release. Its library
  header defines `DOKAN_VERSION 231`, while its kernel interface header defines
  the distinct `DOKAN_DRIVER_VERSION 0x0000190` (decimal 400).
  Delay-load only `%WINDIR%\System32\dokan2.dll` with
  `LOAD_LIBRARY_SEARCH_SYSTEM32`, resolve the bounded symbol table, require
  `DokanVersion()` to return DLL API 231, and require `DokanDriverVersion()` to
  return kernel protocol 0x190/400. Never
  search the application directory, current directory, `PATH`, registry, or a
  caller-controlled path, and never silently accept a merely ABI-compatible
  older/newer driver. There is no link-time Dokany import and no DLL or driver
  beside the app. The recommended NSIS installer does, however, embed the
  reviewed official `Dokan_x64.msi` as a standard-selected optional offline
  component; portable/auto-updated users obtain the same pin through the GUI or
  `se drive install-runtime`. Keep the base install per-user and invoke UAC only
  for the system-wide MSI. Plain `/S` must skip it; only
  `/S /INSTALLDOKANY=1` opts a silent setup in. Never uninstall the shared
  Dokany runtime with Smart Explorer. The pin in `native/dokany-runtime.nsh` is
  version `2.3.1.1000`, DLL API 231, driver protocol 0x190/400,
  `https://github.com/dokan-dev/dokany/releases/download/v2.3.1.1000/Dokan_x64.msi`,
  9,269,248 bytes, SHA-256
  `69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465`.
  The official project states that signed release drivers are provided; use of
  that official runtime needs neither Developer Mode nor `TESTSIGNING`.
  Primary sources checked 2026-07-22:
  [2.3.1.1000 release](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000),
  [tagged README](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/README.md),
  [tagged API header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/dokan.h),
  [tagged driver header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/sys/public.h),
  [version-query implementation](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/version.c),
  [Dokan API](https://dokan-dev.github.io/dokany-doc/html/group___dokan.html).
- **Treat the Dokany installer as hostile until every boundary passes.** Rust
  permits HTTPS, at most two redirects, only the exact source URL or GitHub's
  `release-assets.githubusercontent.com` final host, and at most pinned-size +
  one byte. Require exact size/SHA-256; open only a non-reparse regular file
  without write/delete sharing; hash that locked handle and pass the same handle
  to `WinVerifyTrust` with chain revocation. Open each normalized parent below
  the stable volume/share root as a direct non-reparse directory handle without
  share-delete, and hold the file plus parent chain across launch so an attacker
  cannot swap a pathname component before the elevated reopen. Then launch
  System32 `msiexec.exe` through UAC `runas` with `/passive /norestart
  ADDLOCAL=DokanDriverFeature INSTALLDEVFILES=0`. Exit 0 still requires the exact
  DLL-API-231 and driver-protocol-0x190/400 postcheck. The automatic GUI/CLI path
  runs only for an absent runtime or unavailable driver; it must not replace or
  downgrade a genuinely incompatible installed shared runtime. Release fetchers
  consume the same manifest, NSIS
  embeds the verified MSI, and the release wrapper must extract and compare it.
  Keep `third-party/dokany/` notices and GPL/LGPL/MIT texts installed, but do not
  publish the MSI as a separate feed or Release asset.
- **Keep the daemon/host authority split.** The daemon resolves the saved
  connection, credentials, active fallback and exact root. `RootedBackend`
  rejects traversal and link-like ancestors, strips provider object IDs and
  sanitizes errors. The daemon spawns its current executable with the exact
  private `--mount-host <MountId>` argument; that isolated process receives a
  rooted backend proxy over loopback using distinct one-use launch/backend
  capabilities plus a session capability; it must never receive the daemon's
  global token, account, endpoint, credential material, or unrestricted
  backend root. Keep `env_clear`, loopback validation, bounded frames and
  fail-closed EOF behavior.
- **Peer roots require a live daemon probe and a connection-bound lease.** Offer
  concrete Share roots as `/Label` and `/Verbindungen/<connection>`; the
  aggregate `/` is synthetic and read-only. GUI discovery must ask the daemon's
  active remote `PeerBackend`, not infer guarantees from local labels. Mount
  admission obtains a lease bound to the authenticated QUIC connection and the
  exact root. Direct/relay route changes inside that connection preserve it;
  replacing the connection invalidates it and requires Retry/remount. Never let
  a new connection inherit an old root capability. Stopping a share or
  revoking/changing its authorization is a synchronous admission barrier: new
  operations fail closed, active leases become invalid and a later re-share
  requires Retry/remount. One already admitted operation may finish;
  multi-stage writes must recheck before flush and promotion.
- **Root authority and read/write mode are separate gates.** Strict mode is the
  default even for read-only mounts. The deployed Linux Agent must launch with
  the exact root via `--serve-root`, bind every root component with
  `openat2(RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS)`, and enter Landlock ABI
  3+ before worker threads start; ABI 2 cannot constrain `truncate`/`O_TRUNC`.
  Google Drive's parent-ID hierarchy is also technically confined. Plain SFTP,
  Local/UNC, WebDAV and FTP need explicit trusted-root admission. Peer/Share
  propagates the concrete remote backend/root result: an Agent-confined export
  can remain `Enforced`, while Local/UNC/plain SFTP remains `Unverified` through
  Peer because its check-to-operation race is unchanged. Trusted mode keeps
  serialized validation but cannot close that external symlink/junction race.
  A fallback must be reevaluated for this capability as well as write semantics.
  Sources checked 2026-07-21:
  [Landlock API](https://docs.kernel.org/userspace-api/landlock.html),
  [`openat2(2)`](https://man7.org/linux/man-pages/man2/openat2.2.html).
- **Strict SFTP mounts never inherit the browsing fallback.** A saved SFTP
  connection must have Agent enabled, and deployment plus the protocol-v9
  `--serve-root` handshake must succeed. Surface any failure; never silently
  construct a plain-SFTP mount. Ordinary browsing may retain the established
  SFTP backend only when Agent deployment itself fails; after a successful
  handshake an operation error is surfaced, not replayed blindly. Address
  candidates are deduplicated and interleaved
  IPv6/IPv4 with 250-ms staggering while known-host identity remains the
  original hostname. Keep typed connect/handshake/channel/exec timeouts and
  stdout/stderr capture independently bounded to 64 KiB each. Keep one SFTP
  subsystem for bootstrap, verification and fallback; exec channels run probes
  and the agent.
- **Agent identity is content-addressed, not merely version-addressed.** The
  remote path includes protocol v9 plus the full SHA-256 of the embedded bytes.
  Upload to an exclusive random temporary name, verify its SHA-256 through the
  same SFTP subsystem, then promote/chmod. Re-read and hash the installed file
  before every exec, including reconnect; a matching `--version` string alone
  is insufficient.
- **RW is a three-part contract, not a generic “backend can write” boolean.** A
  mounted root must prove `create`, `replace`, and `namespace_replace` before a
  read-write host starts. `create` means atomic exclusive ownership through
  `open_write_new`, never stat-then-create. Local/UNC and the SSH Agent
  implement all three write primitives, although Local/UNC still require the
  root trust opt-in above.
  Plain SFTP deliberately reports no complete RW staged capability. Its single
  SFTP-v3 subsystem uses standard `SSH_FXP_RENAME`, which is no-replace; do not
  open a second extension subsystem or upgrade a stat+rename race or shell
  command into an atomic guarantee. The aggregate Share root `/` is read-only;
  concrete `/Label` and `/Verbindungen/<connection>` exports delegate capability
  discovery to their exact backend/root. A fallback can therefore change the
  safe answer and must make RW fail conservatively rather than weaken it
  silently; RO remains possible only if the independent root
  gate above also passes or trusted-root mode was explicit. Source checked
  2026-07-21:
  [SFTP-v3 rename](https://datatracker.ietf.org/doc/html/draft-spaghetti-sshm-filexfer#section-6.5),
  [russh-sftp API surface](https://github.com/AspectUnk/russh-sftp).
- **Application writes are whole-file transactions.** Materialize a regular
  file into the mount-specific local spool; persist the dirty record and sync
  it before the first local mutation; on flush upload to a unique staging path;
  then promote with the declared backend primitive. This is what makes repeated
  Obsidian-style truncate/write/flush/close cycles and editor
  temp-file-to-replacing-rename saves viable. A flush after every edit means a
  complete remote upload after every edit. Never stream a partial editor write
  directly onto the destination object.
- **Conflict checks reduce risk; they are not universal CAS.** Compare the
  opening baseline (provider ID, size, mtime, and content MD5 when supplied)
  before staging and again immediately before promotion. Revalidate both
  source and destination for replacing rename. Unless a backend exposes a
  conditional commit, a small TOCTOU window remains between the last stat and
  promotion. Do not document or code this as perfect concurrent-write
  prevention. Never retry an ambiguously dispatched promotion: once a remote
  namespace change may have committed, return filesystem success where Windows
  requires it, surface `Conflict`, and retain the recovery journal/spool.
- **Never unlink an ambiguously owned staging spelling.** After an exclusive
  create crosses Agent/daemon/Peer boundaries, a lost Ready/final ACK or failed
  promotion may leave a hidden stage. A check-then-unlink is still vulnerable
  if another actor moves the owned object and reuses that name. Retain it until
  a future stable-ID/lease garbage collector can prove identity; leaking one
  hidden stage is preferable to deleting foreign content.
- **Recovery state outranks tidy shutdown.** Persisted state is explicitly
  `Clean`, `Required`, or `Unknown`; legacy records without trustworthy status
  map to `Unknown`, never clean. Under the exclusive cache lease, audit local
  journal/spool state at daemon startup and again in the host before opening the
  remote backend. A connection timeout therefore cannot hide local recovery.
  Dirty/conflicting writes and
  quarantined deletes remain under the mount ID in `mount-cache`; startup/Retry
  replays only provably safe work. An eject or host exit must not erase a spool
  merely because the callback returned. Preserve journal rotation/torn-tail
  validation, the exclusive reparse-safe cache lease, and path-wide
  delete-on-last-handle semantics.
- **A host exit code is not an actionable diagnosis.** Preserve the mount
  host's terminal recovery/conflict status and append only bounded stderr/process
  context. Surface DLL API and driver-protocol mismatches independently, name
  strict-root or staged-RW admission failures, map an invalid Peer lease to
  Retry/remount, and say when recovery data must remain. Only a provably clean
  drive-manager entry may be offered for removal.
- **Portable `open-temp` recovery must survive Electron launchers.** Atomically
  write a marker only after a successful download. Delete only complete empty
  legacy manifests with no payload; malformed/truncated state fails closed, and
  genuine sessions are startup notices rather than app errors. Never interpret
  a finished `ShellExecute` launcher as editor completion: Obsidian may forward
  to its existing single-instance process. A declared file remains recoverable
  while temporarily absent during atomic save/delete/rename, and any `NotFound`
  in that mutation path retains the prior marker.
- **Remote latency is application latency.** Reads first materialize an entire
  file, and each changed flush uploads it entirely. Long callbacks use
  `DokanResetTimeout` every 30 seconds with a five-minute request timeout; the
  manager's stop grace exceeds that boundary. The reported free space is the
  local spool's lower bound, not the remote quota. Keep unrelated files
  parallel but serialize one file/namespace mutation.
- **The surface is intentionally narrower than NTFS.** Windows file attributes
  and timestamps cannot be set, ACL/security writes, alternate data streams,
  open-by-ID and reparse-point access are unsupported, and remote symlinks are
  hidden/rejected rather than followed. `GetFileSecurity` returns
  `STATUS_NOT_IMPLEMENTED` so Dokany synthesizes a current-user descriptor.
  Never fabricate durable remote semantics for metadata the backend cannot
  preserve.

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
- **The downloaded helper must understand the previous app's launch protocol.**
  v0.5.119 installs the new feed helper and invokes it with only the legacy app
  target/staging/status fields plus the staged app hash; the normally installed
  helper path carries no helper-hash argument. Removing the explicit legacy
  parser makes updates fail before replacement with a missing SHA argument. The
  bridge must stay fail-closed: require the staged SHA, never downgrade a request
  containing any modern-only flag, and never UAC-relaunch an updater. This also
  applies to the modern protocol: an elevated helper would otherwise launch the
  GUI with administrator privileges. Require the installer when cleanup or
  replacement needs elevation.
  The first helper must wait for the exact v0.5.119 parent without a fixed
  timeout and serialize duplicate workers for the same installed target. Retire
  an older/equal request only when a target-key/version/app-SHA completion
  receipt proves the winner; rebase a newer request on that verified target and
  attempt it without downgrading. If the launched winner blocks a queued newer
  worker, fail visibly and retain the payload. Before changing the target,
  durably create an exclusive target-keyed intent that binds the requested
  version, baseline SHA, and staged SHA. If a helper dies after replacement but
  before status/receipt publication, a later stale worker must recover that
  exact intent or fail closed; it must never adopt the unproven binary as a new
  baseline. The replacement app syncs a private receipt sibling after its first
  GUI frame, atomically publishes it, and completes a nonce-bound two-way
  loopback handshake. Receipt publication is the irreversible commit point;
  the helper retains rollback state and the intent until then. Do not run
  recovery launch while the exact target process is already running without a
  receipt; wait for its receipt and then fail closed rather than creating a
  duplicate. Do not run abandoned-staging cleanup in that acknowledged launch;
  a queued v0.5.119 worker can still own another verified payload.
  Visible update status/error state must be prepared before launching the new
  or restored app. Preserve the exact-v0.5.119 argv, prelaunch-state,
  durable-intent, receipt, serialization/rebase, rollback, path-alias, and
  target-drift regressions.
- Before replacement, the helper verifies and durably archives the outgoing app
  with a SHA-256 sidecar. It rejects aliases between executable, staging, and
  bookkeeping paths and prepares checked sibling files. Existing targets must
  remain continuously addressable: Windows uses `ReplaceFileW` with a verified
  backup, while Linux creates a verified backup and atomically renames over the
  target. Replace updater, `se`, then app so an interruption always leaves a
  runnable old or new GUI. A failed replace or failed acknowledged launch
  restores the previous targets and status, then attempts to relaunch the
  verified old app.
  Length checks alone are not an acceptable replacement for the immediate
  SHA-256 revalidation at each process/replacement boundary.
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
