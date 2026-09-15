# Desktop GUI correction — 2026-09-15

## Explicit batch goal

Restore the compact desktop character of the pre-redesign source (`0be9eaa`),
then tidy alignment, crowding and light-mode readability. The user accepted
that earlier direction and explicitly rejected a web-like appearance. Keep the
released Drive name/path fixes and include the already committed empty-page
and diagnostic corrections (`edcc3a5`, `de9d183`) in this batch's single remote
acceptance and terminal release. The user's later root-only scan excerpt still
does not identify its live cause; do not claim it does.

## Stage one: evidence and approach

Compared the root graph, current source and actual remote egui captures with
`0be9eaa`. The redesign added a fourth top band, increased controls from the
egui defaults to 30 px, margins from 6 to 16 px, headings to 22 px, blue-gray
fills and conspicuous outlines throughout. Landing entries grew from 58/72 px
to 76/96 px. These changes explain the departure from the previous design.

Use the previous compact proportions and neutral grays. Consolidate navigation
and commands again, use restrained ordinary menus and desktop list selection,
keep readable text colors, and remove large landing cards. Preserve functional
navigation, selection, keyboard, scrolling and window-boundary fixes.

## First research pass

Primary sources checked 2026-09-15:

- [Microsoft desktop toolbars](https://learn.microsoft.com/en-us/windows/win32/uxguide/cmd-toolbars): compact related commands, familiar icon shortcuts and overflow only when space requires it. This is explicitly historical desktop guidance, chosen for the requested established desktop character.
- [Microsoft desktop layout](https://learn.microsoft.com/en-us/windows/win32/uxguide/vis-layout): consistent control sizes, alignment and effective workspace use.
- [W3C text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html): use the 4.5:1 normal-text ratio as a measurable color criterion, without claiming full WCAG conformance.
- Pinned egui 0.29.1 `style.rs`, `menu.rs`, `widgets/button.rs`: preserve native renderer and default compact geometry; menu bars make inactive buttons flat. No browser runtime or styling dependency is needed.
- Drive protocol research and pending acceptance are recorded in [the follow-up plan](2026-09-15-drive-scan-followup.md).

## Stage two: final milestones

| Milestone | Files / boundary | Expected result in the single remote suite |
| --- | --- | --- |
| D1: compact desktop shell | `app/core/theme.rs`, `ui_preferences.rs`, `frame_layout.rs`, `shell_toolbar.rs`, `shell_commands.rs`, `filterbar.rs`, `table.rs` | Neutral light/dark surfaces, compact controls, one combined navigation/command toolbar, collapsible search/filter area, usable 900×600 and 1400×900 workspaces. Selection, filtering, absolute breadcrumbs and split view work. |
| D2: familiar locations and dialogs | `sidebar_locations.rs`, `sidebar.rs`, `landing.rs`, `landing_tiles.rs`, `settings_ui.rs` | Familiar sections and direct actions remain reachable; locations use compact unboxed entries; no enlarged cards or pill navigation. Long names truncate with full hover details; capacity labels do not overlap. Settings and feature dialogs stay within the viewport and retain shortcuts/persistence. |
| D3: pending Drive/diagnostic integration | Existing `gdrive/core/gui_task_tests.rs`, HTTP fixtures, `app/core/gui_design_task_*`; new narrowly scoped fixtures if needed | Valid omitted/null/empty list pages succeed through actual listing/resolution; malformed/incomplete pages still fail. Complete path/cause/version text is visible and copied unchanged even for long reports. Clearing app errors preserves scan details. Existing name and mutation fixtures remain covered. |
| D4: candidate and delivery | Existing `native/test-gui-design-task.sh`, `gui-design-task.yml`, canonical docs and root graph | One source-bound remote suite evaluates D1–D3, with actual egui screenshots reviewed before the established complete-release workflow runs once. No local native/test/build execution. |

## Second research pass / resolved gaps

- egui weak text derives from `noninteractive.weak_bg_fill`; retain the explicit
  muted target while removing the global blue backgrounds and excessive margins.
- The old toolbar's fixed 660 px reservation can overflow small windows. Size
  the path from the actual remaining width and reserve measured command widths;
  use a compact overflow menu for secondary commands at narrow sizes.
- Keep shared `window_content_limit`: egui's resize limit excludes title/frame
  space, so reverting it would reintroduce the observed clipped dialogs.
- Keep the bundled Hack glyph fallback, semantic custom-paint colors, optional
  secondary table columns, Drive path codec and safe mutation identity handling.
- The collapsed search must open on Ctrl+F, and active filters remain indicated
  while collapsed. Long reports need clipboard assertions on the click frame,
  since egui platform output is transient.
- Inspection of egui 0.29.1 `menu.rs` showed that menus consume Escape during
  layout, after `frame_keyboard.rs`. The latter now defers file shortcuts to
  open popup/menu areas, preserving the selection when Escape closes a menu.

## Status

D1 is implemented in `d134c8c`; D2 in `b7aa220`. The existing combined entrypoint
now maps D1–D4, including the pending Drive corrections and real scan-to-clipboard
flow. Static Rust parsing and module size checks completed without local native
execution. Acceptance images/logs and subsequent publication are recorded by the
[existing workflows](https://github.com/b1ue-man/smart-explorer/actions). Live
open work remains on `docs/TODO.md`; this plan records implementation evidence.

### Remote review correction

The [first correction run](https://github.com/b1ue-man/smart-explorer/actions/runs/35008277267)
produced real light/dark images and exercised the restored shell and report.
Visual review found centered sidebar labels and an unsupported pencil glyph.
Pinned egui `Layout::left_to_right` defaults its main alignment to Center;
explicit Min alignment corrects the location rows. Static font character tables
confirm that the bundled fonts contain `✏` and `×`, whereas `✎` and `✕` are
absent. The same close-glyph correction covers tabs, date filters and saved
sync setups. New stroke widths also explicitly use `f32`, resolving the
compiler warning from this candidate.

The Drive mutation-absence fixture initially asserted at `open_write`, which
only creates a private spool. The actual upload boundary is `flush`, as defined
by `DriveWriter::commit`; the fixture now writes and flushes before requiring
the invalid listing error and confirming that no mutation request was sent.
The same remote entrypoint verifies these corrections together.
