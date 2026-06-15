# Smart Explorer — Roadmap & Status

Native Windows file explorer (Rust + eframe/egui), GNU toolchain. Current
release: **0.5.0**. Distribution: per-user NSIS installer + self-update from a
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
3. ✅ **FTP/FTPS backend** (`ftp.rs`) — 0.4.2. `suppaftp 6.3` (blocking, rustls/
   ring, verified no aws-lc, cross-compiles windows-gnu). One `RustlsFtpStream`
   covers `ftp://` (plain, anonymous default) and `ftps://` (explicit AUTH TLS);
   single control connection behind a `Mutex` (`parallelism()=1`). Listings via
   suppaftp's `list::File` parser (unix/dos/mlsx); whole-file buffered I/O
   (`retr_as_buffer` / `put_file` on flush+drop). Standalone module — edits are
   `mod ftp;` + one `backend_for` arm. 6 host unit tests (URL parse + ring TLS
   config build). Browsing wires up in the connect-UI step (5).
4. ✅ **Network drives** (`net.rs`) — 0.4.3. `\\server\share` UNC + mapped drives
   already browse through `LocalBackend` (std::fs); added authenticated
   connect-by-address via `WNetAddConnection2W` (mpr.dll), released on drop
   (`NetConnection`). Windows-only FFI is cfg-gated with a non-Windows stub, so
   the path helpers (`is_unc`, `share_root`) are host-tested (4 tests) and the
   FFI is cross-compiled. Discovery deliberately omitted (unreliable on Win11 —
   GOTCHAS). Standalone module (`mod net;`); wires into the connect UI (5).
5. **Connect UI** (protocol/host/port/user/auth) + credential storage
   (`keyring` → Windows Credential Manager). Built in three isolated releases:
   - ✅ **5a (0.4.4) `rscan.rs`** — backend-driven walk for remote roots; streams
     the same `ScanMessage`s as the local scanner over the same channel, via
     `vfs::Backend::list_dir`. Hot local scanner untouched. 2 host tests walk a
     real tree through `LocalBackend`.
   - ✅ **5b (0.4.5) `creds.rs`** — secrets in the OS keyring (Windows Credential
     Manager via `keyring windows-native`; in-memory off-Windows), connection
     metadata (protocol/host/port/user/auth/root/label, no secret) in a TSV file
     in appdata. `SavedConnection` + `to_target()` URL/UNC builder. 6 host tests.
   - ✅ **5c (0.4.6) Connect dialog + app wiring** — `connect.rs` builds the right
     backend off the UI thread from a `ConnectForm` (or a saved connection +
     keyring secret). app.rs gained a sidebar **VERBINDEN** section (new/​saved
     connections, disconnect) and a Connect dialog. Navigation routing is a
     single central edit in `start_scan_navigated`: remote sessions walk via
     `rscan`, local/UNC paths keep the std::fs scanner — decided by path style
     (`is_local_style`), so no per-handler edits. Shares authenticate via
     `net::NetConnection` (kept alive) and browse the UNC locally. This is the
     first build where the remote stack is reachable (not LTO-stripped).
     **Remote browsing is live; remote write-ops (copy/delete/rename of remote
     entries) are a follow-up — they still go through std::fs.** 4 connect tests.

The remote layer (roadmap points 1–5) is COMPLETE.

## Later (not planned in detail) — in progress

- ✅ **Cloud backends — WebDAV (0.5.0)** `webdav.rs`: full `vfs::Backend` over the
  verified ring-rustls `ureq` (no opendal/reqwest → avoids the aws-lc/native-tls
  trap on windows-gnu). PROPFIND (Depth 1) listings parsed with `roxmltree`;
  GET/PUT/DELETE/MKCOL/MOVE/COPY. HTTP Basic auth, added as a Connect-dialog
  protocol. Covers Nextcloud/ownCloud/any WebDAV. 3 host tests (multistatus
  parse, path encode/decode, HTTP-date). **S3 and OAuth providers (Google Drive
  / OneDrive / Dropbox) slot onto the same interface the same way** — a new
  module + a Connect protocol; OAuth ones additionally need a registered app
  client id + a loopback/PKCE consent flow (no per-provider app credentials are
  bundled, so they're scaffolding, not shipped).
- Local↔remote sync (rclone-bisync-style; one-way first).
- Win11 main-menu context entry (needs a signed package — see GOTCHAS).

## Build & release

See [`native/README.md`](../native/README.md). TL;DR:
`export PATH="$USERPROFILE/.cargo/bin:/c/Strawberry/c/bin:$PATH"` then
`cargo build --release` in `native/`. Publish: bump `version` in
`native/Cargo.toml`, copy the exe into `release-native/update-feed/` (exe first,
then `version.txt`), rebuild the installer with `makensis`. Installed apps
self-update on next launch.
