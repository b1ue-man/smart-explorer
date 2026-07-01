# Smart Explorer — TODO / status board

Live status of every request. Legend: ✅ done · 🚧 working (shipped but needs a
real-world test) · ⬜ open. New items get appended here. History in ROADMAP.md.

## Still open (what's actually left)

Everything in the sections below the dividers is **shipped**. This is the
remaining work, roughly by value:

| # | Item | State | Notes |
|---|---|---|---|
| A2 | **Find & reclaim** — local scanner for duplicates (size-group → MD5), large/stale files, empty files/folders, and cleanup targets (node_modules/.git/caches/logs/target) | 🚧 | local UI + recycle-bin action shipped in 0.5.106; remaining: remote/free-hash backends + richer review journal |
| A1 | Analytics **charts + export** — click a category/type bar → filter, size + age histograms, CSV export | ⬜ | builds on A0/A0b |
| A3 | Analytics **scale** — all-drives dashboard, snapshots / growth-over-time (persist aggregation + diff) | ⬜ | |
| 23 | **Faster remote browsing** — listing cache ✅ 0.5.59; prefetch dropped; 0.5.110 adds recursive parallel remote listing for high-latency backends, persistent Drive path→id/mime hints, and lazy `stat` reuse from fresh listings. | 🚧 | code shipped; remaining: live Drive latency validation on a real account |
| 24 | **SSH agent real-server smoke test** — implementation is complete for Linux x86_64/aarch64 and locally covered by socket + real bundled-musl child-process tests; only a live SSH-server exercise remains. | 🚧 | code shipped through 0.5.73; needs a real SSH box only |
| 24d | **Agent as the FAST PATH** ✅ **DONE** — the agent handles *everything* now (read/write transfers, server-local copy/move/rm/mkdir, bulk folder transfer, search, sync+MD5 hashing); SFTP is the per-op fallback. Shipped P0 proto v2 (0.5.69) → P1 read (0.5.69) → P2 write + P3 server-local (0.5.70) → P4 bulk folders (0.5.71) → P5 search (0.5.72) → P6 sync/hashing (0.5.73). See [`docs/SSH_AGENT_PLAN.md`](SSH_AGENT_PLAN.md). | ✅ | one musl binary covers all phases |
| 21 | **Peer file sharing** — a real two-machine NAT/handshake test (code shipped 0.5.23) | 🚧 | |
| A4 | **NTFS MFT** instant local drive scan | ⬜ | deferred by you; plan in SSH_AGENT-style doc note |
| 20.4 | Drag remote files **OUT** via OLE deferred-contents | later | Ctrl+C already copies out |
| 19.3 | **Dropbox / OneDrive** backends | later | after Drive proves out on a real account |
| 21b | Peer-agent over the **P2P share** transport | later | #24 is the practical SSH variant |
| Q2 | **Quick Share** transfer (UKEY2 + OfflineFrames) | later | needs real-device iteration |

**Per-user setup input, not an implementation blocker:** Google Drive auth needs
a Google OAuth *Client ID* (Desktop type) from the user/publisher — see
[`docs/CLOUD_OAUTH_PLAN.md`](CLOUD_OAUTH_PLAN.md).

---

## Shipped this cycle (0.5.x)

| # | Item | State | Version |
|---|---|---|---|
| 25 | **Sort folders+files together**: `📁↑` toggle (persisted) — off = files and folders ranked purely by the active column (e.g. by date), instead of folders pinned first | ✅ | 0.5.64 |
| 26 | **ZIP support**: open a `.zip` in-app and browse it like a folder (read-only `zipfs::ZipBackend` via the remote path; ⏏ closes it), and "📦 Hier entpacken" (shell menu) to extract to a sibling folder. Pure-Rust deflate, no new C deps | ✅ | 0.5.64 |
| 27 | **Version rollback from the GitHub releases** — the rollback list now reads the repo's `release/v*` branches (every released version, not just what's archived locally), downloads the chosen one's binary + swaps it; both lists scroll (capped). Local archives kept as offline fallback. | ✅ | 0.5.66 |
| 28 | **Agent fully wired into analytics** — `CachingBackend` now forwards `supports_walk_tree`/`walk_tree`, so the treemap/storage-analysis uses the agent's one-shot server-side walk (was falling back to per-dir listing through the cache wrapper). | ✅ | 0.5.66 |
| 29 | **Find & Reclaim (local)** — the storage-analysis window now has a `Find & Reclaim` mode: local large/stale/empty/cleanup scans, size-grouped MD5 duplicate groups, quick selection of duplicate copies/empty/cleanup targets, Explorer reveal, and recycle-bin deletion with confirmation. | ✅ | 0.5.106 |
| 30 | **Incremental managed mirror sync (V1)** — one-way Mirror jobs bootstrap once, then keep a SQLite sync index and use Google Drive Changes/local dirty walks to touch only changed paths. Unsafe states (cursor loss, root drift, target drift, two-way/non-Drive remotes) fall back to the existing full safe sync. | ✅ | 0.5.109 |
| 31 | **Faster remote browsing (Drive-first)** — recursive remote scans list breadth levels in parallel when the backend advertises safe width; Drive persists non-secret path→id/mime hints with first-use validation; `CachingBackend::stat` can reuse a fresh unique parent listing instead of doing another network round-trip. | 🚧 | 0.5.110 — needs live Drive latency validation |
| 1 | Legal disclaimer → standard **MIT License** (installer accept-page + first-run notice + README) | ✅ | 0.5.4 / 0.5.6 |
| 2 | Remote connection **per-tab**; remote/share tabs show a name | ✅ | 0.5.5 |
| 5 | **Per-tab + per-split-pane** filter/search | ✅ | 0.5.6 |
| 3 | Safe **two-way sync** (baseline diff, conflict window, reversible, strict default) | ✅ | 0.5.7 |
| 7 | Bug: rubber-band (box) select bled across both split panes | ✅ | 0.5.8 |
| 8 | **UI reorg**: nav-bar dropdown menus (Verbindung / Sync / Einstellungen); sidebar → recent + quick-access; copy/paste off the toolbar | ✅ | 0.5.9 |
| 10 | Bug: breadcrumb click → "Wurzel kann nicht gelesen werden" (dropped leading `/` // → relative path) | ✅ | 0.5.9 |
| 11 | **Stop/cancel a running sync** (mirror + two-way) | ✅ | 0.5.9 |
| 13 | Clear the name-search field when opening a folder (keep other filters) | ✅ | 0.5.9 |
| 9 | **Rich sync-setup menu** (source/target/method/settings); jobs manager + add/edit dialog; quick-setup; **split-view right-click → sync the two open folders / save as setup** | ✅ | 0.5.10 |
| 12 | Sync setups **persist across restart** (`%APPDATA%/smart_explorer/sync/jobs.tsv`) | ✅ | 0.5.10 |
| 14 | **Established connections pinned to the sidebar**, freshest first; overflow (>10) into the Verbindung menu | ✅ | 0.5.10 |
| 4 | **Background sync daemon** (`--sync-daemon`, logon autostart, runs due setups with app closed) + heartbeat/status + on/off toggle | ✅ | 0.5.11 |
| 15 | Window opened partly off-screen at near-full size → **open maximized** by default | ✅ | 0.5.11 |
| 6a | Drag-and-drop **into the app**: drop OS files (Explorer/desktop) onto a folder view → copy (Shift = move); full-window drop hint | ✅ | 0.5.12 |
| 6b | Drag files **between tabs/panes**: drag rows onto a tab header or the other split pane → copy (Shift = move); cursor chip + drop-target highlight. Band-select stays intact (it bails while a drag is active). | ✅ | 0.5.13 |
| 6c | Drag files **out to Explorer** (Windows): OLE `DoDragDrop` + minimal CF_HDROP `IDataObject`/`IDropSource` (`dragout.rs`); kicks in when an internal drag leaves the window. Isolated + best-effort (failure aborts the out-drag, never crashes). | ✅ | 0.5.13 |
| 16 | Maximize regression: builder-`maximized` showed a white default-size window then jumped → "flashbang". Now opens at a sane size and maximizes on the first painted frame. | ✅ | 0.5.13 |
| 17 | **In-app folder picker** for sync setups: browse local drives **and saved remote connections** through the same `Backend` and pick a folder — no more typing a remote location. Remote jobs re-open the saved connection (GUI off-thread + background daemon via keyring creds). | ✅ | 0.5.14 |
| 18 | Trackpad **inertia scroll stuttered**; now repaints while egui animates the smooth-scroll so it glides to a smooth stop. | ✅ | 0.5.14 |

## Shipped — remote files, cloud (Drive), CfAPI history, sharing, Quick Share

(Most rows below are historical shipped slices; **Q2** Quick-Share transfer is
still `later`, and CfAPI rows are explicitly superseded/historical. Note: item
numbers 24/25/26 here are the *older* remote-file series — the newer 24/25/26 in
the lists above are the SSH agent / sort / ZIP items. Legacy numbering kept.
CfAPI entries document superseded experiments/research; the active remote-open
path is temp-watch.)

| # | Item | State | Notes |
|---|---|---|---|
| 24 | **Remote file-op parity** + bug fixes — C: drive loads in picker; delete/new-folder/rename via backend; right-click menu for remotes. | ✅ | 0.5.24 |
| 25 | **Remote edit + save-back** — open a remote file, edit, save → uploaded back. Current code path downloads a temp copy, launches the associated app, watches mtime, and re-uploads via `Backend::open_write` on save. The older CfAPI/persistent-placeholder direction is documented as historical research, not the active implementation. File-ops matrix: [`docs/FILE_OPS_MATRIX.md`](FILE_OPS_MATRIX.md). | ✅ | 0.5.25+ current path = temp-watch |

| 26 | **Remote drag-drop** — drag rows between tabs/panes: local→local copy, **local→remote upload**, **remote→local download**, and **drag remote files OUT to Explorer** (materialize via temp + OLE). Remote→remote deferred. | ✅ | 0.5.26 |

| 27 | **CfAPI registration experiment** — earlier builds tried Cloud Files registration/placeholders; this is superseded and not the current remote-open path. | superseded | removed from the active source path by the later remote-open fixes |
| 29 | **"Neu" dropdown** — New button is now a menu: Ordner + editable files (.txt/.md/.csv/.json/.html/.rs), created locally (opened to edit) or via the backend on remotes. | ✅ | 0.5.27 |

| 28 | **Remote→remote drag** — cross-backend copy by streaming each file through a temp (download from source backend → upload to target). | ✅ | 0.5.29 |
| 31 | **Fix:** Cloud Files registration broke file-open ("invalid name request", os -2145452027). The active code no longer registers a sync root; remote open/save-back uses a plain temp file watched by Smart Explorer. | ✅ | 0.5.29+ |

| 30 | **Native CfAPI on-demand provider research/prototype** — `docs/CFAPI_REVIEW.md` records the review of the old provider approach and why it is risky. There is no active `cfprovider.rs`/`cfsync.rs` provider in current `native/src`; revive only as a new feature after the documented safety fixes. | historical | not an active shipped code path |
| Q1 | **Quick Share LAN discovery** — browse/advertise the `_FC9F5ED42C8A._tcp` mDNS service; nearby Android/Windows Quick Share devices show in 📡 Teilen. | ✅ | 0.5.28 (quickshare.rs) |
| Q2 | **Quick Share transfer** — Nearby Connections UKEY2 + protobuf OfflineFrames (+ BLE wake). Needs real-device iteration; own paired share already covers transfer. | later | docs/QUICKSHARE.md |
| 19.1 | **Cloud OAuth foundation** — `cloud.rs`: PKCE loopback flow, client-ID config, token storage (refresh token in keyring), Google-Drive endpoints; Settings → "CLOUD (GOOGLE DRIVE)" to paste the client ID + "Mit Google verbinden". 5 unit tests (incl. RFC 7636 PKCE vector). | ✅ slice 1 | 0.5.15 |
| 19.2 | **Google Drive `Backend`** (`gdrive.rs`): full `vfs::Backend` over Drive v3 REST — list/stat/read **and** write/mkdir/rename(move)/trash, path→id cache, token auto-refresh, paginated listing, multipart upload. Wired: "☁ Drive öffnen" (browse), Drive as a place in the picker, `gdrive:///path` sync endpoints resolved in GUI + daemon. So Drive can be browsed AND two-way-synced. | ✅ slice 2 | 0.5.16 |
| 19.4 | **Self-setup instructions** — the app is not a hosted service: each user creates their own Google OAuth client. In-app collapsible guide + console link in Settings, full walkthrough in [`docs/CLOUD_SETUP.md`](CLOUD_SETUP.md), README note. Covers the Desktop-app loopback (no redirect URI) and the Testing-mode 7-day-token caveat. | ✅ | 0.5.17 |

## Remote / cloud / sharing follow-ups (mostly shipped; open ones tracked at the top)

| # | Item | Prio | Notes |
|---|---|---|---|
| 20.1 | **Open remote files directly** — double-click / Enter on a file on any remote (SFTP/FTP/WebDAV/Drive) downloads it to a temp copy off-thread and launches it in its associated app. | ✅ | 0.5.19 |
| 20.2 | **Ctrl+V / drag-drop into a remote folder** — paste OS-clipboard files (or drop them) into the current remote folder; uploaded recursively via the backend (flush-on-write so Drive uploads correctly). | ✅ | 0.5.20 |
| 20.3 | **Ctrl+C a remote file → paste in Explorer** — downloads the selected remote files to temp and puts those local paths on the clipboard as CF_HDROP (eager). | ✅ | 0.5.21 |
| 20.4 | Drag remote files **out** to Explorer via OLE (deferred `CFSTR_FILECONTENTS`), and drag remote→tab. Ctrl+C already covers copy-out; this is the drag gesture. | later | COM-heavy; eager temp-copy on drag-start could be a simpler first cut |
| 19.3 | Generalize to **Dropbox / OneDrive** (same `cloud.rs` OAuth, new `Backend` impls). | later | after Drive proves out on a real account |
| 22 | **Connected Google Drive pinned to the sidebar** — stays under VERBINDUNGEN whenever Drive is connected, even with no tab open (click to browse, × to disconnect). | ✅ | 0.5.22 |
| 21 | **Peer file sharing** — plan in [`docs/SHARE_PLAN.md`](SHARE_PLAN.md); eval in [`docs/SHARING_EVAL.md`](SHARING_EVAL.md). Server routes discovery only; bytes go **direct P2P, E2E-encrypted** (Noise NNpsk0 keyed by the code). **Shipped 0.5.23:** standalone `se-share-server` (Linux+Windows, in the release); client `share.rs` (signaling, candidate dial, Noise channel, transfer to quarantine); **Geräte/Räume view** with direct pair-by-code + rooms (share to all). | 🚧 | 0.5.23 — needs a real two-machine test (NAT/handshake) |
| 21b | **Peer-agent Backend** (far Smart Explorer runs scans/filters/search, streams results) + native AirDrop/AWDL ❌ (Windows can't implement AWDL) / optional Quick Share interop. | later | builds on the share transport |
| 24 | **SSH Remote-Agent** — deploy a headless `se-agent` over the existing SSH connection so listing / storage-analysis / search / transfers / sync run *locally on the server* and only results stream back. Opt-in, SSH-only, plain-SFTP fallback. Full plan + status in [`docs/SSH_AGENT_PLAN.md`](SSH_AGENT_PLAN.md). The SSH-deploy variant of 21b. | ✅ | **fully implemented** for Linux x86_64/aarch64: proto v2 (multiplexed/streaming) + `AgentBackend` mux, read/write/copy/rename/remove/mkdir/get-tree/put-tree/search/walk-hashed, analytics `walk_tree` w/ live progress, SSH transport+deploy, opt-in + live activate + remove-agent, static-musl binaries bundled. Build-verified host+win; tested via socket + real-binary child-process. Remaining: **real-server smoke test** only |
| 23 | **Faster remote browsing** — transfer sync's efficiency ideas to interactive browsing. Connections already reused. **(1) directory-listing cache** ✅ 0.5.59: `vfs::CachingBackend` wraps the interactive remote backend (wired at `drain_connect` + `drain_picker_connect`, local backends pass through) — 20 s TTL, mutating ops invalidate the dir + parent, F5/`rescan` clears it; sync's `resolve_endpoint` stays uncached. **Prefetch was dropped** because the SSH agent makes `list_dir` a fast server-side op and the cache makes revisits instant. **0.5.110:** recursive remote scans parallelize breadth-level listing for high-latency backends, Drive persists path→ID/mime hints with first-use validation, and `stat` reuses unique fresh parent listings. | 🚧 | code shipped; needs live Drive latency validation |

**Per-user Drive setup:** each user/publisher supplies a Google OAuth *Client ID*
(Desktop type) from their own Google Cloud project — see
[`docs/CLOUD_OAUTH_PLAN.md`](CLOUD_OAUTH_PLAN.md). A desktop app can't ship a
usable shared secret or pass Drive verification as a generic public service, so
each publisher uses their own client.

## Storage analytics — roadmap

WizTree-style "where is my space" (toolbar 📊 / `›Analyse`). **Current state**
(0.5.56–62): a dedicated low-memory parallel scanner (`analytics.rs`), a *nested*
squarified treemap with instant in-memory drill + breadcrumb, group-coloured
cells, whole-drive default + drive/folder picker, and **remote** analysis over
any VFS backend (parallel for Drive/WebDAV). Open work below: A1/A2/A3/A4.

Also shipped (keyboard line): cursorless filter-nav (0.5.51), omnibox
combo-field (0.5.52), Alt accelerator overlay (0.5.53).

### ✅ Resolved — dedicated analytics scanner (was the must-fix)
The overlay no longer reuses the heavy main scanner. `analytics.rs` does its own
compact, parallel, size-only walk (A0, 0.5.55); the nested treemap with in-memory
drill replaced the first version (A0b, 0.5.56); remote + parallel-remote walks
followed (A5, 0.5.61–62). Full paths ARE kept (no depth cap) — confirmed
acceptable on memory because each node stores only its own name. Open analytics
work is A1/A2/A3/A4 (see "Still open" at the top).

### Phases (dependency-ordered)
| # | Item | State | Notes |
|---|---|---|---|
| A0 | **Dedicated analytics scanner** (`analytics.rs`): compact size tree — each node stores only its own name + size + is_dir + children (full paths reconstructed on descent), parallel walk, size-only metadata. Far lower RAM + faster than the rich main scanner. Paths kept to full depth (no cap — confirmed WizTree keeps them too). | ✅ | 0.5.55 |
| A0b | **Consolidate drill + recursive** into *one scan, in-memory drill*: own scan tree decoupled from the explorer's `root_path`; drill = move `analytics_focus` within the tree (no re-scan, nested) with a clickable breadcrumb + ↑; the `recursive`-prompt is gone; "📂 Im Explorer öffnen" / file-reveal is the only thing that moves the main view. **0.5.56:** rendered as a *nested* WizTree-style treemap (folders = dark containers with header, files coloured by type), defaults to the **whole drive** (drive buttons + folder picker), side button-list panel removed. | ✅ | 0.5.55–56 |
| A1 | Interaction + charts: click category/type bar → filter; size + age histograms; CSV export | ⬜ | builds on A0/A0b |
| A2 | **Find & reclaim**: local mode shipped 0.5.106 (large/stale/empty/cleanup + size-grouped MD5 duplicate groups + recycle-bin action). Remote/cloud follow-up: reuse Drive/WebDAV free hashes and agent `walk_hashed`, then add a richer review/undo journal. | 🚧 | highest user value; local path usable now |
| A3 | Scale: "scan whole drive" with live-filling treemap; all-drives dashboard; snapshots / growth-over-time (persist aggregation, diff) | ⬜ | needs stable aggregation format from A0 |
| A5 | **Remote storage analysis** ✅ 0.5.61: `analytics::scan_backend` walks any VFS backend (SFTP/FTP/WebDAV/Drive) into the same `SizeNode` tree; the overlay auto-scans the current remote folder when browsing a remote, with a "📡 <conn>" scan button. Reuses the cached backend (cap-bounded). Drill/treemap/reveal all work on the remote path. **0.5.62:** parallel level-by-level walk for `parallelism()>1` backends (WebDAV 2, Drive 16; bounded 2–16) — concurrent `list_dir` per tree depth, the dominant latency lever for HTTP; SFTP/FTP stay serial. | ✅ | 0.5.61–62 |
| A4 | **NTFS MFT direct read** (Windows-only), user-selectable opt-in fast scan — *planned, deferred by user (decide later)*. Plan: main app stays unprivileged; the MFT scan relaunches our own exe **elevated via UAC** (`--mft-scan=C:`), reads the raw `\\.\C:` MFT (pure-Rust `ntfs` crate → cross-compiles on windows-gnu, no C deps), serialises the compact `SizeNode` tree to a temp file, parent reads it. **Needs Admin**; **NTFS-local only** → the rayon walker stays the universal fallback (cloud/network/exFAT/unelevated). Also: NTFS compression/sparse awareness, cluster slack. ⚠ untestable in the Linux build env — must be verified on a real NTFS box. | ⬜ | large, risky; user wants it but deferred |

## Notes

- **In-app picker (#17):** `PickerState` in app.rs drives a modal that lists Home
  + drives + saved connections; local nav is instant `std::fs`, remote connects
  async then lists via `Backend::list_dir`. "Choose" returns a local path or a
  `proto://user@host:port/path` endpoint. `connect::resolve_endpoint` re-opens
  the matching saved connection (by protocol+user+host+port) using the keyring
  secret — so remote jobs run both interactively (off-thread) and in the daemon.
  **0.5.58:** generalised to a `PickerPurpose` enum used by *all* folder dialogs
  (open/scan, analytics target, mirror/bisync dest, copy dest, remote
  download-to) — the OS folder dialog (rfd `pick_folder`) is gone; `local_only`
  purposes hide the remote connections. Only the SSH *key-file* pick (a file,
  not a folder) still uses the native dialog.
- **#6c drag-out** is Win32 COM (`dragout.rs`) compiled for Windows but not
  runtime-exercised here; wrapped so any COM failure silently aborts the drag.
- **Background daemon (#4):** `daemon.rs` is a headless loop started by a per-user
  `HKCU\…\Run` entry (`autostart.rs`) → `--sync-daemon`. Loads jobs, runs every
  `due()` one via the same `bisync::run` (local↔local; remote needs re-auth so
  it's skipped), `mark_run`, writes `sync/daemon.heartbeat` for the GUI status,
  honours a `sync/daemon.stop` sentinel. Single-instance via heartbeat freshness.
  Self-update swaps the one exe, so the daemon updates on next logon.
- **Drag-and-drop (#6, next):** drop INTO the app via egui `RawInput.dropped_files`
  (cross-platform, easy); between tabs internally; drag OUT to Explorer needs OLE
  `DoDragDrop` + `IDataObject`/`IDropSource` (Win32, the hard part).

## Design notes

- **Update ↔ background worker:** the worker is the *same* `smart_explorer.exe`
  run with a flag (scheduled task). Self-update swaps the one exe → the worker
  updates automatically; no separate update path.
- Sync jobs (source, target, direction, conflict mode, retention, schedule,
  ignores) live in one persisted store in appdata, shared by the UI, the
  right-click "sync these folders", and the background worker.
