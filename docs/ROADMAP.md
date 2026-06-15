# Smart Explorer — Roadmap & Status

Native Windows file explorer (Rust + eframe/egui), GNU toolchain. Current
release: **0.4.1**. Distribution: per-user NSIS installer + self-update from a
feed — a local/UNC folder **or an http(s)/git URL** (see
[`native/README.md`](../native/README.md)).

## Done (shipped)

- Deep recursive filtering (name/regex/glob, size, date-via-calendar, type);
  tree-structured recursive view; filtered copy preserving folder structure.
- Native Windows shell icons (extension-keyed, off-thread; `icons.rs`).
- Collapsible filter panel; summary panel; fuzzy folder search with a
  live-updating index (`folder_index.rs` + notify watcher).
- Tabs (swap-based, `TabState`); **split-screen** two-pane view (F6).
- Full keyboard map; rubber-band + ctrl/shift selection; type-to-jump.
- Clipboard: CF_HDROP (`shell_clipboard.rs`) + filter-aware **virtual files**
  (`virtual_clipboard.rs`); OS-level Ctrl+C/X/V detection (egui swallows them —
  see GOTCHAS). Native shell context menu (`shell_menu.rs`).
- Self-update from a feed with restart prompt **and rollback** to previously
  installed versions (`updater.rs`); versions archived to `<install>/versions/`.
- **Git/HTTPS update source** (0.3.9): the update feed can be an http(s) URL or a
  GitHub repo link — the app self-updates straight from the repo's
  `release-native/update-feed/` over `raw.githubusercontent.com`. Same
  `version.txt` + `smart_explorer.exe` layout as the folder feed; transport is
  the only difference (`Feed` enum in `updater.rs`, `ureq`/rustls-ring). A
  `.github/workflows/build.yml` cross-compiles the Windows exe on every push.
- Shell integration (`shell_register.rs`): per-user, reversible
  "Open in Smart Explorer" context-menu verb + launch-path argument.

## Next up — Remote layer (in progress, design done, not yet implemented)

The big one. Full implementation plan: **[REMOTE_LAYER_PLAN.md](REMOTE_LAYER_PLAN.md)**.

Target spec (from the project owner):
- **SFTP and FTP** with username/password **or** keyfile login.
- **Network drives** reachable in the local network and by address (UNC).
- A **single standardized network layer / interface** that Smart Explorer talks
  to, on top of which all further protocols are built.
- Explicitly **out of scope for now**: cloud (Google Drive etc.) and syncing —
  but the interface must be designed so they can be added later.

To-do, in order:
1. ✅ **`vfs.rs` — the `Backend` interface + `LocalBackend`** (0.4.0). Shipped as
   a self-contained, host-tested module: `Scheme`, `VfsMeta`, the blocking
   `Backend` trait, `LocalBackend` (mirrors today's `std::fs`), `backend_for()`
   dispatch, `is_remote_root()`. Kept isolated from the hot local scan/copy loops
   on purpose (one-line `mod vfs;` edit); the remote scan/copy paths route
   through it as each backend lands. `FileEntry.scheme` is added when the first
   remote backend is wired (step 2) so it can be tested against a real backend.
2. ✅ **SFTP backend** (`sftp.rs`) — 0.4.1. `russh 0.61` + `russh-sftp 2.3`
   (ring crypto, verified no aws-lc; cross-compiles windows-gnu), password +
   keyfile auth, host-key TOFU (`known_hosts_sftp.txt`). Async↔sync bridge via a
   private multi-thread tokio runtime + `block_on` per op; `File` adapted to
   `std::io::{Read,Write}`. Implements `vfs::Backend`; `backend_for` routes
   `sftp://`. Standalone module — only existing-file edits are `mod sftp;` and
   one `backend_for` arm. URL parsing + refusal-without-credentials unit-tested
   (5 tests); live network exercised by the API (no sshd in the build sandbox).
   Browsing wires up in the connect-UI step (5).
3. **FTP/FTPS backend** (`ftp.rs`) via `suppaftp`.
4. **Network drives**: UNC `\\server\share` already works through `LocalBackend`
   (std::fs); add authenticated connect-by-address (WNetAddConnection2W). Local
   network *discovery* is unreliable on Win11 — see plan.
5. **Connect UI** (protocol/host/port/user/auth) + credential storage
   (`keyring` → Windows Credential Manager).

## Later (not planned in detail)

- Cloud backends (Google Drive / OneDrive / Dropbox / S3 / WebDAV) on the same
  `Backend` interface — likely via `opendal` + `oauth2` loopback/PKCE.
- Local↔remote sync (rclone-bisync-style; one-way first).
- Win11 main-menu context entry (needs a signed package — see GOTCHAS).

## Build & release

See [`native/README.md`](../native/README.md). TL;DR:
`export PATH="$USERPROFILE/.cargo/bin:/c/Strawberry/c/bin:$PATH"` then
`cargo build --release` in `native/`. Publish: bump `version` in
`native/Cargo.toml`, copy the exe into `release-native/update-feed/` (exe first,
then `version.txt`), rebuild the installer with `makensis`. Installed apps
self-update on next launch.
