# GUI clarity and light-mode redesign

## Goal and scope

Overhaul the complete first-party GUI's presentation: readable light and dark
themes, consistent spacing and controls, a quieter browsing workspace, and
progressive disclosure in the feature windows. Preserve file, connection, sync,
sharing, update and consent behavior. Implement the whole batch before running
one remote task suite and the existing terminal release transaction.

## Stage one: findings and approach

- egui/eframe 0.29.1 already provides separate light/dark styles and system-theme
  selection. Startup currently replaces only the active visuals with dark ones.
- UI text in the app modules repeatedly uses fixed gray, pale blue, orange and
  green colors. Those choices do not adapt to light backgrounds.
- `shell_toolbar.rs` reserves 660 points for commands in the same row as the
  path. `frame_layout.rs` adds a permanently visible filter heading above the
  already dense filter form. The file table always shows seven columns.
- Start-page sections all open, including duplicated index controls, connection
  setup promotions, sync results and a long mixed list of places. The sidebar
  repeats index instructions and maintenance controls.
- Settings are a single menu containing OAuth credentials, server settings,
  updates, rollback history, recovery and shell integration. `share.rs` mixes
  orchestration with several large UI pages and exceeds the source-size limit.
- Persisted UI state already has a small OS adapter. Extend its typed portable
  model without changing existing panel choices or touching provider protocols.

## Research pass one (2026-09-15)

- [WCAG text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html):
  use 4.5:1 for normal text, including secondary text and hints.
- [WCAG non-text contrast](https://www.w3.org/WAI/WCAG21/Understanding/non-text-contrast.html):
  use 3:1 for necessary control boundaries and visible focus indicators.
- [Windows settings guidance](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings):
  group related preferences, expose advanced options on demand, apply appearance
  changes immediately, and offer light, dark and system modes.
- [Windows command bars](https://learn.microsoft.com/en-us/windows/apps/design/controls/command-bar):
  group frequent actions and keep secondary commands in a labelled overflow.
- [egui 0.29.1 source](https://github.com/emilk/egui/tree/0.29.1/crates/egui/src):
  retain this dependency version; use its style, layout, accessibility and input
  primitives rather than adopting a new toolkit or native dependency.

## Stage two: final milestone plan

| Milestone | Files/modules and dependencies | Expected result / acceptance signal |
| --- | --- | --- |
| M1: coherent appearance | New `app/core/theme.rs`, `ui_preferences.rs`; `app/mod.rs`, `lib.rs`, `state.rs`, `init.rs`, `prefs_tabs.rs`, OS preference adapter, analysis-window startup; migrate semantic text/status colors across app UI modules | Both themes keep normal, secondary, status, hint and selected text readable; keyboard focus remains distinct. Appearance and density survive restart; old preference files keep their panel choices. |
| M2: browsing workspace | `shell_toolbar.rs`, new command-bar module, `frame_layout.rs`, `filterbar.rs`, `central_tabs.rs`, `table.rs`, `table_interaction.rs`; depends on M1 | Path navigation owns its row. Search stays directly reachable. Extended filters are collapsible with visible active state and reset. Default table omits redundant columns; an explicit option restores them. Single and split views remain usable at the 900×600 minimum and 1400×900, with scroll/selection/keyboard behavior intact. |
| M3: navigation and start page | `sidebar.rs`, `sidebar_locations.rs`, `landing.rs`, `landing_tiles.rs`; depends on M1/M2 | Places and primary actions are immediately scannable. Administrative information is collapsed or moved to settings; lists have sensible previews and a route to every remaining item. Narrow cards do not overflow, and warnings/meters do not overlap. |
| M4: settings and feature windows | New settings window/state and extracted cloud/update settings; `menus_settings.rs`, `menus_sync.rs`, `frame_keyboard.rs`; extract Share presentation from `share.rs` into focused files; connection/copy/sync/analysis/picker windows | Settings have labelled categories and bounded scrolling; mode/density switch immediately. Share shows relevant device/room content first and retains diagnosis, permissions and destructive-action consent. Connection, copy and other dialogs share typography, spacing and wrapping, without hiding essential risks. |
| M5: documentation and integration | README, release documentation, this plan, root graph; after M1–M4 create `native/test-gui-design-task.sh`, supporting GUI task cases and `.github/workflows/gui-design-task.yml` | One suite maps all milestone expectations to theme math, preference migration, actual egui layout/input and directly affected UI flows. Remote visual artifacts cover representative views in both themes. Only the affected host library development target may be built incrementally. |

## Research pass two: resolved integration gaps

- Inspected the installed 0.29.1 sources: `set_style_of`, `set_theme`,
  `ThemePreference`, `Window` constraints and `WidgetVisuals` exist in the pinned
  version. Configure both styles once, then change the preference; do not reset
  fonts or styles each frame. System mode follows egui's supplied OS facts.
- Keep preference parsing and defaults in portable core types, filesystem I/O
  in the existing OS adapter. Missing/new keys receive conservative defaults.
- Use semantic palette functions for UI text. Keep the treemap's categorical
  colors and its explicit luminance-dependent text as chart data, and preserve
  the intentional dark drag/consent overlays and accelerator badges.
- Split the oversized Share module by behavior before adding UI behavior. For
  other existing oversized UI files, any color-only substitutions are a narrow
  mechanical exception: no additional responsibility or control flow there.
- Existing APIs support headless egui frames, real pointer/keyboard events,
  accessibility output and painted geometry. The task suite will isolate app
  data and disable constructor background workers; rendering must not perform
  actual transfers, updates, discovery, consent or destructive actions.
- Ordinary pushes whose final message ends in `[task candidate]` bypass the
  generic build jobs. Batch the milestone commits into that candidate push,
  dispatch only the GUI task workflow, then use the existing `build.yml`
  `complete_release_source_sha` mode. No local compiler, test or release command.
- All suite/build jobs have at least 30 minutes; the existing complete release
  has six hours and a 330-minute top-level wrapper invocation. Check the same
  release no more often than every 30 minutes. Publish through the stable wrapper.

## Verification status

Implementation and the combined task entrypoint are complete. Each candidate's
behavioral results, binary/source binding and real egui paint/PNG artifacts are
recorded in the [GUI / Drive task workflow](https://github.com/b1ue-man/smart-explorer/actions/workflows/gui-design-task.yml).
This plan does not declare a released version; publication evidence belongs to
the matching repository release.

Remote review also corrected missing bundled-font fallbacks, reserved space for
window title/frame geometry, and kept reset/date controls together at 900×600.
The same task entrypoint now checks real visible window rectangles and the
additional table header, and exports the detailed-column and drag-hover views.

## Added batch scope: Drive names (2026-09-15)

The user added the exact root-scan failure for the title containing `I/O` before
the combined candidate was pushed. This is part of the same suite and release.

### Stage one: diagnosis and approach

`rscan/os/shared/walk_state.rs::validate_listing` emits the reported error when
`vfs::validate_child_name` sees `/`, `\\`, NUL, `.` or `..`.
`gdrive/core/backend.rs::list_dir` only disambiguates duplicate names; unique
Drive titles pass through unchanged. `metadata.rs::resolve` splits those names
on `/`. The reported `I/O` therefore fails at this adapter boundary, before a
download or authentication request could explain this particular error.

Introduce one reversible Drive-title/path-segment codec before disambiguation.
Keep exact IDs, decode only at the provider boundary, and retain the generic
walk guard. Include uncached resolution, cache reload, stat, upload, folder
creation, rename and staged promotion in the affected boundary.

### Research pass one

- [Drive files resource](https://developers.google.com/workspace/drive/api/reference/rest/v3/files):
  `id`, `name` and `parents` are separate fields; names need not be unique.
- [Drive search](https://developers.google.com/workspace/drive/api/guides/search-files):
  query names must escape both apostrophes and backslashes, independently of URL encoding.
- [Drive list](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list):
  follow `nextPageToken`; a partial page is not proof of uniqueness or absence.
- [Drive update](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/update):
  mutations address `fileId`; supplying a `name` changes the actual title.
- [rclone's Drive implementation guidance](https://rclone.org/drive/#restricted-filename-characters)
  confirms the comparable adapter problem: `/`, `.` and `..` are valid Drive names.
- [Windows filenames](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file):
  separators, reserved characters/device names and trailing dots/spaces need a
  portable representation for downloads. This mapping is an app convention,
  not a Google requirement or a rename of the cloud object.

### Stage two: final added milestone

| Milestone | Files/modules and dependencies | Expected result / acceptance signal |
| --- | --- | --- |
| M6: safe Drive namespace | New `gdrive/core/names.rs`, extracted resolution; listing/disambiguation/cache, folder creation, transfer, copy writer and promotion boundaries | The exact reported title lists and opens; recursive scans accept it as one child. Reversible percent escapes distinguish literal escape strings, dot names, separators and duplicate markers. Duplicate IDs sharing a short prefix remain addressable. All metadata queries use original titles, all content-only replacements preserve them, and rename/create decode destination segments exactly once. Cache reload and paginated lookup preserve the same mapping. |

### Research pass two: resolved gaps

- Encode reserved path characters as uppercase percent escapes (for example
  `I%2FO`); escape literal percent signs and marker-shaped literal titles too.
  Do not replace `/` with `_`, which would merge different names.
- Disambiguate encoded titles, checking ID-prefix uniqueness against every
  sibling. Never decode before splitting the virtual path into segments.
- Invalidate the older path-cache format and version the provider's sync-state
  identity so old path spellings cannot act as deletion evidence.
- Extract path resolution from metadata before changing it. Follow every
  same-name query page and reject repeated tokens/incomplete results.
- Existing-file media uploads must send empty metadata instead of renaming a
  Drive object to its displayed escape/duplicate alias. Other metadata methods
  receive decoded original names explicitly.
- M6 joins the existing GUI task selector and remote entrypoint. Protocol
  fixtures exercise real HTTP requests against a loopback service on CI;
  no live account files are mutated. Local execution remains prohibited.
