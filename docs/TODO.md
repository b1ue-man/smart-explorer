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

## Open

| # | Item | Prio | Notes |
|---|---|---|---|
| 4 | **Background sync service/worker** (runs when app closed) + remaining sync settings wiring. Worker = same exe via `--sync-daemon` scheduled task, so self-update covers it. Job model (interval/hidden/ignore) + `due()` already in `syncjobs.rs`. | — | builds on #3 engine + #9 jobs (both done) |
| 6 | **Native Windows drag-and-drop**: into the app (egui dropped_files), between tabs, and **out to Explorer** (OLE `DoDragDrop` + `IDataObject`/`IDropSource` — the hard part) | — | drop-in/between-tabs first; drag-out via COM |

## Notes for #4 (next up)

- `syncjobs::SyncJob` already carries `interval_min`, `include_hidden`, `ignore`
  and a `due(now)` test-covered scheduler check; `run_job()` in app.rs runs one
  by id. The daemon is a headless loop: `--sync-daemon` → load jobs → for each
  `due()` job run a bisync (no UI) → `mark_run` → sleep → repeat.
- Autostart: register the scheduled task / `Run` key pointing at the installed
  exe with `--sync-daemon`. Self-update already swaps that exe.

## Design notes

- **Update ↔ background worker:** the worker is the *same* `smart_explorer.exe`
  run with a flag (scheduled task). Self-update swaps the one exe → the worker
  updates automatically; no separate update path.
- Sync jobs (source, target, direction, conflict mode, retention, schedule,
  ignores) live in one persisted store in appdata, shared by the UI, the
  right-click "sync these folders", and the background worker.
