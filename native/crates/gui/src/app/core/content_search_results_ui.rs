//! The results panel of „In Dateien suchen“, docked at the bottom like
//! Notepad++'s „Search results“: one virtualized row per header, file, hit
//! line and skipped entry (`ScrollArea::show_rows`); with „Lange Zeilen
//! umbrechen“ a long line takes several rows of the monospace grid.
use std::collections::HashSet;

use super::content_search_nav::WithinQuery;
use super::content_search_panel_menu::menu;
use super::content_search_results::{Row, RowKind, SearchResults};
use super::prelude::*;
use super::text_view_style::{append_marked, mono};

/// Rows a continuation of a wrapped line is indented by, in characters.
const CONTINUATION_INDENT: usize = 6;

/// What the panel asks the app for.
pub(in crate::app) enum PanelAction {
    /// Open the text view at a hit line (or at the file of a file row).
    Open(Row),
    /// „Pfade öffnen (Textansicht)“: (search id, full path) each.
    OpenPaths(Vec<(u64, String)>),
    CancelRuns,
    Clear,
    SearchWithin(u64, WithinQuery),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum MenuChoice {
    ExpandAll,
    CollapseAll,
    Copy,
    CopyLines,
    CopyPaths,
    OpenPaths,
    SelectAll,
    Clear,
    SearchWithin,
    ToggleWrap,
    ToggleClearEach,
}

/// The prompt of „In diesen Ergebnissen suchen“.
pub(in crate::app) struct WithinPrompt {
    pub(in crate::app) source: u64,
    pub(in crate::app) query: WithinQuery,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) focus: bool,
}

struct RowCache {
    generation: u64,
    chars_per_row: usize,
    rows: Vec<Row>,
    /// First visual row of each row; `total` visual rows in all.
    starts: Vec<usize>,
    total: usize,
    built: Instant,
}

#[derive(Default)]
pub(in crate::app) struct ResultsPanel {
    pub(in crate::app) visible: bool,
    pub(in crate::app) wrap: bool,
    pub(in crate::app) clear_each: bool,
    pub(in crate::app) selection: HashSet<Row>,
    pub(in crate::app) cursor: Option<Row>,
    anchor: Option<Row>,
    pub(in crate::app) scroll_to_cursor: bool,
    pub(in crate::app) within: Option<WithinPrompt>,
    cache: Option<RowCache>,
    pub(in crate::app) rect: Option<egui::Rect>,
}

/// Character ranges of the rows a display line of `chars` characters takes
/// with `per_row` characters per row (continuations indented).
pub(in crate::app) fn chunk_ranges(chars: usize, per_row: usize) -> Vec<(usize, usize)> {
    let first = per_row.max(1);
    if chars <= first {
        return vec![(0, chars)];
    }
    let next = per_row.saturating_sub(CONTINUATION_INDENT).max(1);
    let mut ranges = vec![(0, first)];
    let mut start = first;
    while start < chars {
        let end = (start + next).min(chars);
        ranges.push((start, end));
        start = end;
    }
    ranges
}

fn indent(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Header => "",
        RowKind::File(_) | RowKind::SkippedHeader => "  ",
        RowKind::Line(..) | RowKind::Skipped(_) => "      ",
    }
}

pub(in crate::app) fn foldable(kind: RowKind) -> bool {
    matches!(
        kind,
        RowKind::Header | RowKind::File(_) | RowKind::SkippedHeader
    )
}

/// (display text, byte offset where the line text starts, marks in line text).
fn display(results: &SearchResults, row: Row) -> (String, usize, Vec<(usize, usize)>) {
    let pad = indent(row.kind);
    if let Some(line) = results.line(row) {
        let prefix = if line.context {
            format!("{pad}Zeile {}  ", line.line)
        } else {
            format!("{pad}Zeile {}: ", line.line)
        };
        let marks = line
            .ranges
            .iter()
            .map(|(start, end)| (*start as usize, *end as usize))
            .collect();
        let suffix = if line.truncated { " …" } else { "" };
        return (
            format!("{prefix}{}{suffix}", line.text),
            prefix.len(),
            marks,
        );
    }
    let expanded = match row.kind {
        RowKind::Header => results.record(row.search).is_some_and(|r| r.expanded),
        RowKind::File(file) => results
            .record(row.search)
            .and_then(|r| r.files.get(file))
            .is_some_and(|f| f.expanded),
        RowKind::SkippedHeader => results.record(row.search).is_some_and(|r| r.skipped_open),
        _ => false,
    };
    let fold = if !foldable(row.kind) {
        ""
    } else if expanded {
        "▾ "
    } else {
        "▸ "
    };
    let text = format!("{pad}{fold}{}", results.row_text(row));
    let len = text.len();
    (text, len, Vec::new())
}

fn byte_at(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(index, _)| index)
}

impl ResultsPanel {
    fn refresh(&mut self, results: &SearchResults, chars_per_row: usize, running: bool) {
        if let Some(cache) = &self.cache {
            let same =
                cache.generation == results.generation && cache.chars_per_row == chars_per_row;
            // While a search streams in, a wrapped tree is recounted at most
            // five times a second.
            let throttled = running && self.wrap && cache.built.elapsed().as_millis() < 200;
            if same || (throttled && cache.chars_per_row == chars_per_row) {
                return;
            }
        }
        let rows = results.rows();
        let mut starts = Vec::with_capacity(rows.len());
        let mut total = 0;
        for row in &rows {
            starts.push(total);
            total += if self.wrap && matches!(row.kind, RowKind::Line(..)) {
                let chars = display(results, *row).0.chars().count();
                chunk_ranges(chars, chars_per_row).len()
            } else {
                1
            };
        }
        self.cursor = self.cursor.filter(|cursor| rows.contains(cursor));
        self.cache = Some(RowCache {
            generation: results.generation,
            chars_per_row,
            rows,
            starts,
            total,
            built: Instant::now(),
        });
    }

    /// The visible rows (after the last refresh), top to bottom.
    pub(in crate::app) fn rows(&self) -> &[Row] {
        self.cache
            .as_ref()
            .map(|cache| cache.rows.as_slice())
            .unwrap_or(&[])
    }

    /// Selected rows in display order, or the cursor row without a selection.
    pub(in crate::app) fn chosen(&self) -> Vec<Row> {
        let selected: Vec<Row> = self
            .rows()
            .iter()
            .copied()
            .filter(|row| self.selection.contains(row))
            .collect();
        if selected.is_empty() {
            self.cursor.into_iter().collect()
        } else {
            selected
        }
    }

    pub(in crate::app) fn select(&mut self, row: Row, ctrl: bool, shift: bool) {
        if shift {
            let rows = self.rows();
            let anchor = self.anchor.or(self.cursor).unwrap_or(row);
            let a = rows.iter().position(|r| *r == anchor);
            let b = rows.iter().position(|r| *r == row);
            if let (Some(a), Some(b)) = (a, b) {
                let range: Vec<Row> = rows[a.min(b)..=a.max(b)].to_vec();
                self.selection = range.into_iter().collect();
            }
        } else if ctrl {
            if !self.selection.remove(&row) {
                self.selection.insert(row);
            }
            self.anchor = Some(row);
        } else {
            self.selection.clear();
            self.selection.insert(row);
            self.anchor = Some(row);
        }
        self.cursor = Some(row);
    }

    pub(in crate::app) fn show(
        &mut self,
        ctx: &egui::Context,
        results: &mut SearchResults,
        running: bool,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        let max = (ctx.screen_rect().height() * 0.8).max(120.0);
        let panel = egui::TopBottomPanel::bottom("content_search_results")
            .resizable(true)
            .default_height(220.0)
            .height_range(80.0..=max)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Suchergebnisse");
                    if running && ui.button("Abbrechen").clicked() {
                        actions.push(PanelAction::CancelRuns);
                    }
                    if ui.button("Leeren").clicked() {
                        actions.push(PanelAction::Clear);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").on_hover_text("Ausblenden").clicked() {
                            self.visible = false;
                        }
                    });
                });
                ui.separator();
                ui.scope(|ui| self.body(ui, results, running, &mut actions));
            });
        self.rect = Some(panel.response.rect);
        self.show_within(ctx, &mut actions);
        actions
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        results: &mut SearchResults,
        running: bool,
        actions: &mut Vec<PanelAction>,
    ) {
        let font = mono(ui);
        let row_height = ui.fonts(|fonts| fonts.row_height(&font));
        let glyph = ui.fonts(|fonts| fonts.glyph_width(&font, '0')).max(1.0);
        let chars_per_row = if self.wrap {
            ((ui.available_width() - 16.0) / glyph).floor().max(16.0) as usize
        } else {
            usize::MAX
        };
        self.refresh(results, chars_per_row, running);
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(cache) = self.cache.take() else {
            return;
        };
        let mut area = if self.wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        }
        .id_salt("content_search_rows")
        .auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_to_cursor) {
            if let Some(index) = self
                .cursor
                .and_then(|c| cache.rows.iter().position(|r| *r == c))
            {
                let visual = cache.starts[index];
                area = area.vertical_scroll_offset((visual.saturating_sub(3)) as f32 * row_height);
            }
        }
        let mut choice = None;
        let mut clicked: Option<(Row, bool, bool, bool)> = None;
        area.show_rows(ui, row_height, cache.total, |ui, range| {
            for visual in range {
                let index = cache
                    .starts
                    .partition_point(|start| *start <= visual)
                    .saturating_sub(1);
                let Some(row) = cache.rows.get(index).copied() else {
                    continue;
                };
                let chunk = visual - cache.starts[index];
                let response =
                    self.paint_row(ui, results, row, chunk, chars_per_row, &font, row_height);
                if response.clicked() || response.secondary_clicked() || response.double_clicked() {
                    let pointer = response.interact_pointer_pos().unwrap_or_default();
                    let fold_end = response.rect.left()
                        + (indent(row.kind).chars().count() + 2) as f32 * glyph;
                    let on_fold = chunk == 0 && foldable(row.kind) && pointer.x <= fold_end;
                    clicked = Some((
                        row,
                        response.double_clicked(),
                        response.secondary_clicked(),
                        on_fold,
                    ));
                }
                let (wrap, clear_each) = (self.wrap, self.clear_each);
                response.context_menu(|ui| menu(ui, wrap, clear_each, &mut choice));
            }
        });
        // The empty space below the rows has the same menu.
        let rest = ui.allocate_response(
            ui.available_size().max(egui::vec2(1.0, 1.0)),
            egui::Sense::click(),
        );
        let (wrap, clear_each) = (self.wrap, self.clear_each);
        rest.context_menu(|ui| menu(ui, wrap, clear_each, &mut choice));
        self.cache = Some(cache);
        if let Some((row, double, secondary, on_fold)) = clicked {
            let modifiers = ui.input(|input| input.modifiers);
            if secondary {
                if !self.selection.contains(&row) {
                    self.select(row, false, false);
                }
            } else if on_fold && !double {
                results.toggle(row);
                self.cursor = Some(row);
            } else if double {
                if matches!(row.kind, RowKind::Line(..)) {
                    actions.push(PanelAction::Open(row));
                } else if foldable(row.kind) {
                    results.toggle(row);
                }
            } else {
                self.select(row, modifiers.command, modifiers.shift);
            }
        }
        if let Some(choice) = choice {
            self.apply_choice(ui.ctx(), results, choice, actions);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_row(
        &self,
        ui: &mut egui::Ui,
        results: &SearchResults,
        row: Row,
        chunk: usize,
        chars_per_row: usize,
        font: &egui::FontId,
        row_height: f32,
    ) -> egui::Response {
        let colors = super::theme::palette(ui);
        let (text, text_start, marks) = display(results, row);
        let chars = text.chars().count();
        let (from, to) = if self.wrap && matches!(row.kind, RowKind::Line(..)) {
            chunk_ranges(chars, chars_per_row)
                .get(chunk)
                .copied()
                .unwrap_or((chars, chars))
        } else {
            (0, chars)
        };
        let (start, end) = (byte_at(&text, from), byte_at(&text, to));
        let context = results.line(row).is_some_and(|line| line.context);
        let base = match row.kind {
            RowKind::Header => colors.text,
            RowKind::File(_) => colors.accent,
            RowKind::SkippedHeader => colors.warning,
            RowKind::Skipped(_) => colors.muted,
            RowKind::Line(..) if context => colors.muted,
            RowKind::Line(..) => colors.text,
        };
        let mut job = egui::text::LayoutJob::default();
        if chunk > 0 {
            job.append(
                &" ".repeat(CONTINUATION_INDENT),
                0.0,
                egui::TextFormat::simple(font.clone(), base),
            );
        }
        let mark = colors.warning.gamma_multiply(0.35);
        if start < text_start {
            let prefix_end = end.min(text_start);
            let color = if matches!(row.kind, RowKind::Line(..)) {
                colors.muted
            } else {
                base
            };
            job.append(
                &text[start..prefix_end],
                0.0,
                egui::TextFormat::simple(font.clone(), color),
            );
        }
        if end > text_start {
            let piece_start = start.max(text_start);
            let format = egui::TextFormat::simple(font.clone(), base);
            append_marked(
                &mut job,
                &text[piece_start..end],
                piece_start - text_start,
                &format,
                &marks,
                mark,
            );
        }
        let galley = ui.fonts(|fonts| fonts.layout_job(job));
        let width = ui.available_width().max(galley.size().x + 8.0);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::click());
        if self.selection.contains(&row) {
            ui.painter().rect_filled(rect, 0.0, colors.selection);
        }
        if self.cursor == Some(row) {
            ui.painter()
                .rect_stroke(rect.shrink(0.5), 0.0, egui::Stroke::new(1.0, colors.accent));
        }
        ui.painter()
            .galley(rect.left_top() + egui::vec2(4.0, 0.0), galley, base);
        let truncated = results.line(row).is_some_and(|line| line.truncated);
        if truncated {
            response.on_hover_text("Zeile gekürzt (wie geliefert)")
        } else {
            response
        }
    }

    /// Drops the row cache (the next frame lays the rows out again).
    pub(in crate::app) fn invalidate(&mut self) {
        self.cache = None;
    }
}
