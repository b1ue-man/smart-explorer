# SESSION STATE — historical handoff snapshot (not current)

> **Historical snapshot:** this file was written during the pre-0.5.24 remote
> file-operations push and is retained only to explain that old handoff. Do not
> use it as live repo state, release flow, branch guidance, or open-item truth.
> Current status lives in `docs/TODO.md`; current release flow lives in
> `docs/RELEASING.md`; current version truth lives in `native/Cargo.toml` plus
> `release-native/update-feed/version.txt`.

Snapshot written because of server issues. This file lets a fresh context resume
with zero gaps: the rules to follow, the exact current state, what's done/
in-progress/open, and the precise next actions up to committing/releasing.

---

## 0. ENVIRONMENT & REPO

- Repo: **`b1ue-man/smart-explorer`** (GitHub). App is a Rust + eframe/egui 0.29
  Windows file explorer. Built ON Linux, cross-compiled to Windows.
- **Dev branch: `claude/determined-keller-3r0i3m`** — develop + push here. Also
  push to `main` (the update feed is served from `main`) and force-push a
  `release/vX.Y.Z` branch to trigger the CI GitHub Release.
- GitHub ops ONLY via `mcp__github__*` tools (no `gh` CLI). Repo scope limited to
  `b1ue-man/smart-explorer`.
- Commit messages MUST end with the session URL footer:
  `https://claude.ai/code/session_01WeShsRxZkysniVf5o43PUz`
- NEVER put the model identifier in commits/artifacts. Do NOT create PRs unless
  explicitly asked.
- Push with retries/backoff: `for i in 1 2 3 4; do git push -u origin <branch> && break || sleep $((2**i)); done`.

## 1. HARD RULES (build/deps) — violating these breaks the Windows build

- **TLS = ring ONLY. NEVER aws-lc-rs** (needs NASM/CMake, breaks windows-gnu).
  After ANY dependency change run: `cargo tree --target x86_64-pc-windows-gnu | grep -ciE "aws-lc|openssl-sys"` → must be **0**. (`ring` itself is fine.)
- New deps must be **pure Rust** (no native libs). Already added safely:
  serde/serde_json/sha2/getrandom (cloud), snow/hkdf (share — snow's default
  resolver is pure Rust: x25519-dalek + chacha20poly1305 + aes-gcm + blake2/sha2).
- Cross-compile target: **`x86_64-pc-windows-gnu`** (mingw-w64 installed). Host
  build is for `cargo test`. rfd is split per-target (Win: common-controls-v6;
  else xdg-portal) — don't merge.
- egui/eframe **0.29** exactly. windows crate **0.58**, windows-sys **0.59**,
  winreg 0.52. Adding Win32 APIs = add the feature to `windows`/`windows-sys` in
  native/Cargo.toml.
- Disk/tmpfs can fill during repeated release builds. Reclaim with:
  `rm -rf native/target/debug native/target/*/release/incremental`.

## 2. RELEASE FLOW (exact, in order)

1. Bump `version` in `native/Cargo.toml`.
2. From repo root: `cd native && ./publish-feed.sh` — it: builds the app for
   win-gnu; copies to `release-native/update-feed/{smart_explorer.exe,version.txt}`,
   `release-native/Smart Explorer.exe`; builds the **NSIS installer**
   `release-native/Smart Explorer Setup X.Y.Z.exe`; and builds **`se-share-server`**
   for **linux + win-gnu** into `release-native/share-server/{se-share-server-linux,se-share-server.exe}`.
   NOTE: publish-feed runs from `native/`; `release-native/` is at repo root —
   when you `rm`/`sed` afterwards, use repo-root-relative paths (cd back to repo root).
3. `rm -f "release-native/Smart Explorer Setup <OLD>.exe"`.
4. Bump the installer link version in `README.md` (sed both `Smart Explorer Setup X.Y.Z.exe` and the `%20` URL-encoded form).
5. `git add -A && git commit` (feed + Cargo.toml/lock + README + docs).
6. Push: dev branch (`-u`), then `HEAD:main`, then force-push `release/vX.Y.Z`.
7. CI (`.github/workflows/build.yml`) publishes the GitHub Release on the
   `release/**` branch push. **CI guard:** committed `update-feed/version.txt`
   must equal `Cargo.toml` version, or it refuses.
8. Verify CI green via `mcp__github__actions_list`/`actions_get` (the list output
   is huge → it gets saved to a file; parse with python by run id; look for
   `"conclusion":"success"`). The feed on `main` is what auto-updates users —
   independent of the Release CI.

## 3. ARCHITECTURE (what you must know to edit safely)

- **VFS** (`native/src/vfs.rs`): `pub trait Backend: Send+Sync` (BLOCKING).
  Methods: `scheme`, `root_display`, `list_dir(path)->Vec<VfsMeta>`, `stat`,
  `exists`(default via stat), `open_read->Box<dyn Read+Send>`,
  `open_write->Box<dyn Write+Send>`, `copy_file`(default read+write),
  `rename(src,dst)`, `remove_file`, `remove_dir`, `mkdir_all`, `parallelism`.
  `type BackendHandle = Arc<dyn Backend>`. Paths are **forward-slash** strings.
  `to_os` normalizes a bare drive letter `C:` → `C:/` (Windows drive-relative trap; fixed #24).
  `Scheme { Local, Sftp, Ftp, Webdav, GDrive }` (never matched exhaustively).
- **Backends**: `LocalBackend` (std::fs, also UNC/mapped drives), `sftp.rs`
  (russh 0.61 + russh-sftp, TOFU host keys), `ftp.rs` (suppaftp 6, FTP/FTPS),
  `webdav.rs` (ureq + roxmltree + base64), **`gdrive.rs`** (Drive v3 REST via
  ureq json; path→fileId cache; multipart upload; trash on delete; errors
  surfaced via `drive_err`/`resp_json`).
- **App** (`native/src/app.rs`, ~8000 lines): active tab's state lives in `App`
  fields; inactive tabs parked in `TabState` (swap_with_tab). `self.remote:
  Option<connect::RemoteState>` (`backend`+`label`) — set ⇒ remote view; nav
  walks the backend via `rscan` (async). `self.net_conn` for UNC shares (browsed
  locally). Background work = threads + crossbeam channels; **drain_* called each
  frame in `update()`**; `request_repaint_after` while work in flight.
- **connect.rs**: `RemoteState{backend,label}`, `ConnectForm`, `spawn_connect`,
  `do_connect` (per-protocol), `resolve_endpoint(endpoint)->(BackendHandle,root)`
  for `local`/`sftp://`/`ftp://`/`ftps://`/`webdav://`/`gdrive:///` (used by sync
  run + daemon), `open_gdrive`, `open_saved_at`, `remote_endpoint`/`gdrive_endpoint`.
- **cloud.rs** (#19): PKCE-loopback OAuth (Google), client-id config in
  `%APPDATA%/smart_explorer/cloud/gdrive.cfg`, refresh token in keyring; token
  exchange/refresh surface Google's error body (HTTP 400/403 reasons).
- **share.rs** (#21, client) + **`share-server/`** crate (standalone rendezvous,
  Linux+Windows): peer file sharing — signaling client, Noise NNpsk0 channel (PSK
  = HKDF of code), candidate dial, transfer to quarantine. UI = "📡 Teilen".
- **syncjobs.rs** + **bisync.rs** + **sync.rs** + **daemon.rs** (`--sync-daemon`)
  + **autostart.rs**: the sync system (#3/#4/#9/#12).

## 4. NUMBERED POINTS / GOALS — STATUS

Shipped & CI-green (auto-update live): #1 MIT/disclaimer, #2 per-tab remote,
#3 safe two-way sync, #4 background daemon, #5 per-tab/pane filter, #6a/#6b/#6c
drag in/between/out, #7 band-select fix, #8 nav-bar menus, #9 sync setups,
#10 breadcrumb, #11 sync cancel, #12 persist setups, #13 clear-search-on-nav,
#14 connections sidebar, #15 maximize, #16 maximize-flash, #17 in-app picker,
#18 trackpad scroll-tail, #19 Google Drive (OAuth + backend + setup guide),
#20.1 open remote files, #20.2 paste/drop into remote, #20.3 Ctrl+C remote→Explorer,
#21 peer sharing (server+client+view), #22 Drive pinned to sidebar.
Releases 0.5.10 → **0.5.23** all green.

**#24 (C: drive doesn't load in picker)** — FIXED in commit `4f8a050` +
`ef3677b` (vfs `to_os` + picker `ensure_dir_root`). Built into the 0.5.24 feed.

### ACTIVE GOAL (current /goal):
"**Full review + implementation of ALL locally-supported file operations for
remotes; implement remote file opening + routing for BOTH (a) temp-copy/watch
and (b) CfAPI, user-toggleable.**" Plus the user explicitly asked to **produce a
researched file-operation × backend × API matrix doc, then implement the gaps.**

## 5. CURRENT GIT STATE (exact)

- `origin/claude/determined-keller-3r0i3m` and `origin/main` are at **`85695ae`**
  (docs: REMOTE_EDIT decision). v0.5.23 is the live release.
- **Unpushed local commits** on the dev branch:
  - `4f8a050` — fix #24 (C: drive; app.rs picker + vfs to_os). **Complete, good.**
  - `ef3677b` — **WIP** (this snapshot): delete-on-remote + new-folder-on-remote
    via backend; Cargo.toml→0.5.24; release-native feed rebuilt **with the C: fix
    binary ONLY** (binary predates the delete/new-folder source edits → DO NOT
    cut a clean release from ef3677b without rebuilding).
- **Next push action**: `git push -u origin claude/determined-keller-3r0i3m`
  (branch only — main stays at 85695ae until a clean, complete release is cut).

## 6. WHAT'S DONE vs OPEN in the ACTIVE GOAL

DONE (in `ef3677b`, compiles host):
- **Delete on remotes** → `trash_selected` branches on `self.remote`: threads
  `backend.remove_dir`/`remove_file` per selected item, reuses `trash_rx`.
- **New folder on remotes** → `create_new_folder` branches on `self.remote`:
  threads unique-name (`backend.exists`) + `backend.mkdir_all`; result via new
  `mkdir_rx` field → `drain_mkdir` (rescans). Wired into update() + repaint cond.

OPEN (do next, in this order):
1. **Rename on remotes** — `confirm_rename` (app.rs ~3622, uses `std::fs::rename`
   at ~3643). Add `if let Some(rs)=&self.remote { backend.rename(old_fwd,new_fwd) }`
   (threaded; rescan after). Paths are forward-slash remote paths.
2. **Right-click context menu for remotes** — currently `row_rclick` (~6499) calls
   `show_shell_menu_for` (~4557) = Windows shell IContextMenu, **local paths only**
   ⇒ remotes have NO menu. Add an egui context menu when `self.remote.is_some()`:
   Öffnen, Herunterladen nach…, Umbenennen, Löschen, Neuer Ordner, Pfad kopieren,
   In Zwischenablage kopieren (download→CF_HDROP, already have for #20.3),
   Aktualisieren. Route each to the backend ops above.
3. **Drag-drop for remotes** — internal table drag (#6b) builds `drag_files` from
   `entry.path`; OLE drag-out (`dragout.rs`) is local-paths only. OS-drop INTO a
   remote folder already uploads (#20.2, `handle_os_drop`). Needed: (a) drag OUT
   of a remote (materialize via temp download then CF_HDROP/OLE); (b) drop/drag
   from another tab/pane INTO a remote pane → upload via backend. Document in matrix.
4. **FILE-OPS MATRIX doc** (`docs/FILE_OPS_MATRIX.md`) — researched: each op
   (UI action + Backend method) × {Local std::fs, SFTP, FTP/FTPS, WebDAV, GDrive}
   → concrete API/endpoint + status (works/not-wired/bug). Endpoints to cite:
   std::fs calls; SFTP protocol ops (open/read/write/mkdir/rename/remove/stat via
   russh-sftp); FTP commands (RETR/STOR/MKD/RNFR+RNTO/DELE/RMD/LIST); WebDAV
   methods (PROPFIND/GET/PUT/MKCOL/MOVE/DELETE/COPY); Drive v3 (files.list/get/
   create/update[multipart]/update?trashed/patch parents). Use WebSearch to ground.
5. **Toggleable remote file opening — temp-watch + CfAPI**:
   - **Temp-watch (re-add the reverted #23 code):** `open_temp_path(name)`,
     `file_mtime_ms`, `download_to_temp` already exist (0.5.19). Re-add the
     `RemoteEdit` struct + `remote_edits`/`edit_save_rx`/`last_edit_poll` fields,
     register an edit-watch in `open_file` (remote branch), set baseline in
     `drain_file_open`, add `poll_remote_edits` (1.5s debounce; upload via
     `upload_file` on mtime advance) + `drain_edit_saves`. (This was implemented
     then reverted; one edit — poll/drain methods — was REJECTED, so it's fully
     out of the tree now.)
   - **CfAPI (`native/src/cfsync.rs`, Windows-only):** `CfRegisterSyncRoot` under
     `%USERPROFILE%/Smart Explorer/<label>`; placeholders; `CF_CALLBACK FETCH_DATA`
     → hydrate via `backend.open_read` + `CfExecute(TRANSFER_DATA)`;
     `ReadDirectoryChangesW` → upload via `backend.open_write` on save. Needs
     `windows` feature `Win32_Storage_CloudFilters`. **Untestable here** (no
     Windows/cldflt.sys) — compile for win-gnu; gate opt-in.
   - **Toggle:** setting in Einstellungen "Remote-Dateien öffnen: Temp-Kopie ⟷
     CfAPI (Platzhalter)", persisted (e.g. `%APPDATA%/smart_explorer/remote_open_mode.txt`);
     `open_file` routes accordingly. Decision rationale: `docs/REMOTE_EDIT.md`.

## 7. PLAN/DECISION DOCS (already in repo — read before redoing work)

`docs/TODO.md` (live board), `docs/REMOTE_EDIT.md` (#23 strategy decision: CfAPI
chosen, temp-watch interim — now BOTH, toggleable), `docs/SHARE_PLAN.md` (#21),
`docs/SHARING_EVAL.md` (AirDrop ❌ / Quick Share / own pairing), `docs/CLOUD_OAUTH_PLAN.md`,
`docs/CLOUD_SETUP.md`, `docs/GOTCHAS.md` (aws-lc trap etc.), `docs/RELEASING.md`,
`docs/ROADMAP.md`, `docs/WIN11_CONTEXT_MENU.md`.

## 8. USER-REPORTED BUGS (live testing) — status

- Create folder on any remote — **FIXED (ef3677b)**, needs release.
- Delete on Google Drive — **FIXED (ef3677b)**, needs release.
- C: drive doesn't load in picker — **FIXED (4f8a050)**, in 0.5.24 feed.
- No right-click menu on remotes — **OPEN** (§6.2).
- No drag-drop on remotes — **OPEN** (§6.3).
- Drive connect works; OAuth 400 + Drive 403 now show the real reason
  (0.5.17/0.5.18). Trackpad scroll-tail fix (0.5.18). Sync now testable after C: fix.

## 9. ACTIONS UP TO COMMITTING (checklist to follow every change)

1. Edit code (route remote ops through `self.remote`'s `BackendHandle`; do remote
   I/O on a thread + channel + `drain_*` + repaint, never block the UI thread).
2. `cd native && cargo check --bin smart_explorer` (host) → 0 errors.
3. `cargo test --bin smart_explorer` → all pass (currently 64).
4. `cargo check --bin smart_explorer --target x86_64-pc-windows-gnu` → 0 errors.
5. If deps changed: `cargo tree --target x86_64-pc-windows-gnu | grep -ciE "aws-lc|openssl-sys"` → 0.
6. `git add <files>` and commit with a clear message + the session URL footer.
7. To RELEASE: follow §2 (bump version → publish-feed → rm old installer → bump
   README → commit release-native → push branch + main + release/vX.Y.Z → verify CI).
   Only release from a COMPLETE, compiling state (not a WIP commit).

## 10. IMMEDIATE NEXT STEP

Push `ef3677b` to the dev branch (preserve work). Then implement §6.1–§6.3
(rename, remote right-click menu, remote drag-drop), write §6.4 matrix, cut a
clean release (rebuild feed so the binary matches source), then §6.5 (temp-watch
+ CfAPI toggle).
