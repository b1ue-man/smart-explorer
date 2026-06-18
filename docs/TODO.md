# Smart Explorer — TODO / status board

Live status of every request. Legend: ✅ done · 🚧 working · ⬜ open.
New items get appended here as they come in. Roadmap history is in ROADMAP.md.

## Shipped this cycle (0.5.x)

| # | Item | State | Version |
|---|---|---|---|
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

## In progress

| # | Item | State | Notes |
|---|---|---|---|
| 24 | **Remote file-op parity** + bug fixes — C: drive loads in picker; delete/new-folder/rename via backend; right-click menu for remotes. | ✅ | 0.5.24 |
| 25 | **Remote edit + save-back, toggleable** — open a remote file, edit, save → uploaded back. Two modes (Einstellungen → REMOTE-DATEIEN ÖFFNEN): **Temp-Kopie** (ephemeral) and **CfAPI/Platzhalter** (persistent per-connection sync folder mirroring the remote). Native on-demand CfAPI placeholders = documented next Windows-tested layer. File-ops matrix: [`docs/FILE_OPS_MATRIX.md`](FILE_OPS_MATRIX.md). | ✅ | 0.5.25 |

| 26 | **Remote drag-drop** — drag rows between tabs/panes: local→local copy, **local→remote upload**, **remote→local download**, and **drag remote files OUT to Explorer** (materialize via temp + OLE). Remote→remote deferred. | ✅ | 0.5.26 |

| 27 | **Native CfAPI** — CfRegisterSyncRoot (folder = OS-managed sync root) + CfConvertToPlaceholder/CfSetInSyncState (mark hydrated files in-sync), best-effort, in CfAPI mode. | ✅ (eager) | 0.5.27; on-demand FETCH_DATA hydration still TODO (#30) |
| 29 | **"Neu" dropdown** — New button is now a menu: Ordner + editable files (.txt/.md/.csv/.json/.html/.rs), created locally (opened to edit) or via the backend on remotes. | ✅ | 0.5.27 |

| 28 | **Remote→remote drag** — cross-backend copy by streaming each file through a temp (download from source backend → upload to target). | ✅ | 0.5.29 |
| 31 | **Fix:** CfAPI mode broke file-open ("invalid name request", os -2145452027) — registering a sync root without a connected provider made Windows' cloud filter reject file creation. Removed the `CfRegisterSyncRoot`/placeholder calls; the mode is now a plain **persistent sync folder** (relabeled in Settings, "Platzhalter" wording dropped). | ✅ | 0.5.29 |

| 30 | **Native CfAPI on-demand provider** — `cfprovider.rs` on the `cloud-filter` crate (real sync-engine wrapper, windows 0.58): CfApi mode mounts the connection as a Cloud-Files sync root; dirs populate on demand (fetch_placeholders→list_dir), files hydrate on open (fetch_data→open_read), placeholder blob = remote path; save-back via the edit-watch. API follows the crate's behavior test verbatim. | ✅ | 0.5.30 — Windows-only, needs a real Windows run |
| Q1 | **Quick Share LAN discovery** — browse/advertise the `_FC9F5ED42C8A._tcp` mDNS service; nearby Android/Windows Quick Share devices show in 📡 Teilen. | ✅ | 0.5.28 (quickshare.rs) |
| Q2 | **Quick Share transfer** — Nearby Connections UKEY2 + protobuf OfflineFrames (+ BLE wake). Needs real-device iteration; own paired share already covers transfer. | later | docs/QUICKSHARE.md; AirDrop infeasible on Windows |
| 19.1 | **Cloud OAuth foundation** — `cloud.rs`: PKCE loopback flow, client-ID config, token storage (refresh token in keyring), Google-Drive endpoints; Settings → "CLOUD (GOOGLE DRIVE)" to paste the client ID + "Mit Google verbinden". 5 unit tests (incl. RFC 7636 PKCE vector). | ✅ slice 1 | 0.5.15 |
| 19.2 | **Google Drive `Backend`** (`gdrive.rs`): full `vfs::Backend` over Drive v3 REST — list/stat/read **and** write/mkdir/rename(move)/trash, path→id cache, token auto-refresh, paginated listing, multipart upload. Wired: "☁ Drive öffnen" (browse), Drive as a place in the picker, `gdrive:///path` sync endpoints resolved in GUI + daemon. So Drive can be browsed AND two-way-synced. | ✅ slice 2 | 0.5.16 |
| 19.4 | **Self-setup instructions** — the app is not a hosted service: each user creates their own Google OAuth client. In-app collapsible guide + console link in Settings, full walkthrough in [`docs/CLOUD_SETUP.md`](CLOUD_SETUP.md), README note. Covers the Desktop-app loopback (no redirect URI) and the Testing-mode 7-day-token caveat. | ✅ | 0.5.17 |

## Open / upcoming

| # | Item | Prio | Notes |
|---|---|---|---|
| 20.1 | **Open remote files directly** — double-click / Enter on a file on any remote (SFTP/FTP/WebDAV/Drive) downloads it to a temp copy off-thread and launches it in its associated app. | ✅ | 0.5.19 |
| 20.2 | **Ctrl+V / drag-drop into a remote folder** — paste OS-clipboard files (or drop them) into the current remote folder; uploaded recursively via the backend (flush-on-write so Drive uploads correctly). | ✅ | 0.5.20 |
| 20.3 | **Ctrl+C a remote file → paste in Explorer** — downloads the selected remote files to temp and puts those local paths on the clipboard as CF_HDROP (eager). | ✅ | 0.5.21 |
| 20.4 | Drag remote files **out** to Explorer via OLE (deferred `CFSTR_FILECONTENTS`), and drag remote→tab. Ctrl+C already covers copy-out; this is the drag gesture. | later | COM-heavy; eager temp-copy on drag-start could be a simpler first cut |
| 19.3 | Generalize to **Dropbox / OneDrive** (same `cloud.rs` OAuth, new `Backend` impls). | later | after Drive proves out on a real account |
| 22 | **Connected Google Drive pinned to the sidebar** — stays under VERBINDUNGEN whenever Drive is connected, even with no tab open (click to browse, × to disconnect). | ✅ | 0.5.22 |
| 21 | **Peer file sharing** — plan in [`docs/SHARE_PLAN.md`](SHARE_PLAN.md); eval in [`docs/SHARING_EVAL.md`](SHARING_EVAL.md). Server routes discovery only; bytes go **direct P2P, E2E-encrypted** (Noise NNpsk0 keyed by the code). **Shipped 0.5.23:** standalone `se-share-server` (Linux+Windows, in the release); client `share.rs` (signaling, candidate dial, Noise channel, transfer to quarantine); **Geräte/Räume view** with direct pair-by-code + rooms (share to all). | 🚧 | 0.5.23 — needs a real two-machine test (NAT/handshake) |
| 21b | **Peer-agent Backend** (far Smart Explorer runs scans/filters/search, streams results) + AirDrop ❌ (Windows can't do AWDL) / optional Quick Share interop. | later | builds on the share transport |
| 24 | **SSH Remote-Agent** — deploy a headless `se-agent` over the existing SSH connection so listing / storage-analysis / search run *locally on the server* and only results stream back (kills the per-dir round-trip latency). Opt-in, SSH-only, plain-SFTP fallback. Full plan + status in [`docs/SSH_AGENT_PLAN.md`](SSH_AGENT_PLAN.md). The SSH-deploy variant of 21b. | 🚧 | **phases 1–3 done** (proto+`se-agent` bin+`AgentBackend`+analytics `walk_tree` integration, SSH transport+deploy logic; compile-verified host+win, tested w/o SSH). Remaining: **4** connect UX, **5** musl agent binaries+hashes (blocked: no musl toolchain here), real-server test |
| 23 | **Faster remote browsing** — transfer sync's efficiency ideas to interactive browsing. Connections already reused. **(1) directory-listing cache** ✅ 0.5.59: `vfs::CachingBackend` wraps the interactive remote backend (wired at `drain_connect` + `drain_picker_connect`, local backends pass through) — 20 s TTL, mutating ops invalidate the dir + parent, F5/`rescan` clears it; sync's `resolve_endpoint` stays uncached. Still open: **(2) prefetch immediate sub-folders** after a listing, **(3) parallel listing for deep paths** on `parallelism()>1` backends (Drive), **(4)** persist Drive path→ID cache, **(5)** lazy-stat reuse from the last listing. | 🚧 | (1) done; prefetch next |

**One input still needed from you:** a Google OAuth *Client ID* (Desktop type)
from your own Google Cloud project — see [`docs/CLOUD_OAUTH_PLAN.md`](CLOUD_OAUTH_PLAN.md).
A desktop app can't ship a usable shared secret or pass Drive verification, so
each publisher uses their own client. With it, slice 1 authorizes end-to-end.

## Storage analytics — roadmap (OPEN)

WizTree-style "where is my space". First overlay shipped in **0.5.54** (toolbar
📊 / `›Analyse`): squarified treemap of the current folder's children, largest
folders/files, by-category + by-type bars, drive used/total gauge, ↑-up, click
folder → drill (navigate), click file → reveal. Built on the existing scan/view.

Also recently shipped (keyboard line): cursorless filter-nav (0.5.51), omnibox
combo-field (0.5.52), Alt accelerator overlay (0.5.53).

### ⚠ Must-fix before going further — dedicated analytics scanner
The current overlay reuses the **main scanner**, which loads full per-file
metadata (mtime/btime/flags/ext/id …) → **kills RAM** on big trees and is slow.
Analytics needs its **own lightweight recursive scan**:
- minimal per-node data (path + size + is_dir only), much lower memory, faster;
- **depth-capped aggregation**: beyond a depth threshold (≈5), DON'T keep every
  individual file path — roll their bytes up into the depth-N folder node, so a
  huge tree stays bounded in memory. (WizTree *appears* to keep all paths to the
  bottom — unconfirmed — but sizes are aggregated from ~depth 5.)
- deeper drill into a capped folder can scan that subtree on demand.

### Phases (dependency-ordered)
| # | Item | State | Notes |
|---|---|---|---|
| A0 | **Dedicated analytics scanner** (`analytics.rs`): compact size tree — each node stores only its own name + size + is_dir + children (full paths reconstructed on descent), parallel walk, size-only metadata. Far lower RAM + faster than the rich main scanner. Paths kept to full depth (no cap — confirmed WizTree keeps them too). | ✅ | 0.5.55 |
| A0b | **Consolidate drill + recursive** into *one scan, in-memory drill*: own scan tree decoupled from the explorer's `root_path`; drill = move `analytics_focus` within the tree (no re-scan, nested) with a clickable breadcrumb + ↑; the `recursive`-prompt is gone; "📂 Im Explorer öffnen" / file-reveal is the only thing that moves the main view. **0.5.56:** rendered as a *nested* WizTree-style treemap (folders = dark containers with header, files coloured by type), defaults to the **whole drive** (drive buttons + folder picker), side button-list panel removed. | ✅ | 0.5.55–56 |
| A1 | Interaction + charts: click category/type bar → filter; size + age histograms; CSV export | ⬜ | builds on A0/A0b |
| A2 | **Find & reclaim**: duplicate finder (size-group → hash via sync MD5, 1-click delete) + large-stale + empty files/folders + cleanup targets (node_modules/.git/caches/logs) | ⬜ | highest user value |
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

## Notes

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
