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
| 19.1 | **Cloud OAuth foundation** — `cloud.rs`: PKCE loopback flow, client-ID config, token storage (refresh token in keyring), Google-Drive endpoints; Settings → "CLOUD (GOOGLE DRIVE)" to paste the client ID + "Mit Google verbinden". 5 unit tests (incl. RFC 7636 PKCE vector). | ✅ slice 1 | 0.5.15 |
| 19.2 | **Google Drive `Backend`** (`gdrive.rs`): full `vfs::Backend` over Drive v3 REST — list/stat/read **and** write/mkdir/rename(move)/trash, path→id cache, token auto-refresh, paginated listing, multipart upload. Wired: "☁ Drive öffnen" (browse), Drive as a place in the picker, `gdrive:///path` sync endpoints resolved in GUI + daemon. So Drive can be browsed AND two-way-synced. | ✅ slice 2 | 0.5.16 |
| 19.4 | **Self-setup instructions** — the app is not a hosted service: each user creates their own Google OAuth client. In-app collapsible guide + console link in Settings, full walkthrough in [`docs/CLOUD_SETUP.md`](CLOUD_SETUP.md), README note. Covers the Desktop-app loopback (no redirect URI) and the Testing-mode 7-day-token caveat. | ✅ | 0.5.17 |

## Open / upcoming

| # | Item | Prio | Notes |
|---|---|---|---|
| 20.1 | **Open remote files directly** — double-click / Enter on a file on any remote (SFTP/FTP/WebDAV/Drive) downloads it to a temp copy off-thread and launches it in its associated app. | ✅ | 0.5.19 |
| 20.2 | **Ctrl+V / drag-drop into a remote folder** — paste OS-clipboard files (or drop them) into the current remote folder; uploaded recursively via the backend (flush-on-write so Drive uploads correctly). | ✅ | 0.5.20 |
| 20.3 | **Ctrl+C a remote file → paste in Explorer** — put a CF_HDROP on the clipboard pointing at a temp download (eager) or a virtual `CFSTR_FILECONTENTS` data object (deferred). | next | virtual data object is the COM-heavy part |
| 20.4 | Drag remote files in/out (extend #6 drag to remote, materialize via temp). | later | builds on 20.2/20.3 |
| 19.3 | Generalize to **Dropbox / OneDrive** (same `cloud.rs` OAuth, new `Backend` impls). | later | after Drive proves out on a real account |

**One input still needed from you:** a Google OAuth *Client ID* (Desktop type)
from your own Google Cloud project — see [`docs/CLOUD_OAUTH_PLAN.md`](CLOUD_OAUTH_PLAN.md).
A desktop app can't ship a usable shared secret or pass Drive verification, so
each publisher uses their own client. With it, slice 1 authorizes end-to-end.

## Notes

- **In-app picker (#17):** `PickerState` in app.rs drives a modal that lists Home
  + drives + saved connections; local nav is instant `std::fs`, remote connects
  async then lists via `Backend::list_dir`. "Choose" returns a local path or a
  `proto://user@host:port/path` endpoint. `connect::resolve_endpoint` re-opens
  the matching saved connection (by protocol+user+host+port) using the keyring
  secret — so remote jobs run both interactively (off-thread) and in the daemon.
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
