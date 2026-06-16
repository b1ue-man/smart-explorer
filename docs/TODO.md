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

## Open / upcoming

| # | Item | Prio | Notes |
|---|---|---|---|
| 19 | **Cloud integrations** (Google Drive first) via **OAuth** — new `Backend` impls + PKCE OAuth flow, tokens in keyring. **Designed:** see [`docs/CLOUD_OAUTH_PLAN.md`](CLOUD_OAUTH_PLAN.md). **Blocked on one input:** a Google OAuth *Client ID* you create (a desktop app can't ship a usable shared secret / pass Drive verification — each publisher uses their own project). Code can be built dormant; the live flow needs that client ID + a real Windows test. | next | plan written; awaiting your Google OAuth client ID to wire/verify |

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
