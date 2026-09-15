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

Planning complete. Implementation and remote evaluation pending; this document
does not claim a shipped version or completed visual verification.
