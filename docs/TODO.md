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

## Open

| # | Item | Prio | Notes |
|---|---|---|---|
| 12 | Sync setups **persist across restart** | med | converge into the sync-jobs store (#9/#4) |
| 9 | **Rich sync-setup menu**: source / target / method / method-settings; own quick-setup button; **split-view right-click to sync the two open folders** | — | the configurable "sync job" model |
| 4 | **Background sync service/worker** (runs when app closed) + sync settings (interval, hidden-folder handling, ignore paths, …). Worker = same exe via `--sync-daemon` scheduled task, so self-update covers it. | — | builds on #3 engine + #9 jobs |
| 6 | **Native Windows drag-and-drop**: into the app (egui dropped_files), between tabs, and **out to Explorer** (OLE `DoDragDrop` + `IDataObject`/`IDropSource` — the hard part) | — | drop-in/between-tabs first; drag-out via COM |

## Design notes

- **Update ↔ background worker:** the worker is the *same* `smart_explorer.exe`
  run with a flag (scheduled task). Self-update swaps the one exe → the worker
  updates automatically; no separate update path.
- Sync jobs (source, target, direction, conflict mode, retention, schedule,
  ignores) live in one persisted store in appdata, shared by the UI, the
  right-click "sync these folders", and the background worker.
