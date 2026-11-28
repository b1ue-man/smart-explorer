//! The line area of a text view window. Rows are virtualized by line number
//! (`top` and the rows that fit), not by pixels, so files of any length
//! scroll exactly (a pixel offset in `f32` loses whole rows beyond ~16 M
//! pixels) and wrapped lines of unknown height work the same way. Line
//! numbers in the gutter select lines; the text is selectable labels.
use super::prelude::*;
use super::text_view_style::{mono, plain_job, rgba, styled_job};
use super::text_view_window::TextViewWindow;

const SCROLLBAR_WIDTH: f32 = 12.0;

fn digits(value: u64) -> usize {
    value.max(1).to_string().len()
}

/// Colours of the view: from the highlighter's theme, else from egui.
struct BodyColors {
    background: Color32,
    foreground: Color32,
    selection: Color32,
    line: Color32,
    gutter: Color32,
    mark: Color32,
}

impl TextViewWindow {
    fn body_colors(&self, ui: &egui::Ui) -> BodyColors {
        let mark = super::theme::warning(ui).gamma_multiply(0.45);
        match &self.highlighter {
            Some(highlighter) => {
                let theme = highlighter.theme_colors(self.theme());
                BodyColors {
                    background: rgba(theme.background),
                    foreground: rgba(theme.foreground),
                    selection: rgba(theme.selection),
                    line: rgba(theme.line_highlight),
                    gutter: rgba(theme.gutter_foreground),
                    mark,
                }
            }
            None => BodyColors {
                background: ui.visuals().extreme_bg_color,
                foreground: ui.visuals().text_color(),
                selection: ui.visuals().selection.bg_fill,
                line: ui.visuals().faint_bg_color,
                gutter: super::theme::muted(ui),
                mark,
            },
        }
    }

    fn selected(&self, line: u64) -> bool {
        self.selection
            .is_some_and(|(a, b)| line >= a.min(b) && line <= a.max(b))
    }

    fn marks_for(&self, line: u64) -> &[(usize, usize)] {
        if self.cursor == Some(line) {
            &self.marks
        } else {
            &[]
        }
    }

    fn click_number(&mut self, line: u64, shift: bool) {
        self.selection = match (self.selection, shift) {
            (Some((anchor, _)), true) => Some((anchor, line)),
            _ => Some((line, line)),
        };
    }

    /// Draws the lines into the rest of `ui`.
    pub(in crate::app) fn body(&mut self, ui: &mut egui::Ui) {
        let font = mono(ui);
        let row_height = ui.fonts(|fonts| fonts.row_height(&font)).max(1.0);
        let glyph = ui.fonts(|fonts| fonts.glyph_width(&font, '0')).max(1.0);
        let colors = self.body_colors(ui);
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, colors.background);
        // The wheel scrolls lines; taken before the horizontal scroll area
        // (which would turn it sideways) sees it.
        if ui.rect_contains_pointer(rect) {
            let delta = ui.ctx().input_mut(|input| {
                let delta = input.smooth_scroll_delta.y;
                input.smooth_scroll_delta.y = 0.0;
                delta
            });
            if delta != 0.0 {
                self.scroll_rest -= delta / row_height;
                let whole = self.scroll_rest.trunc();
                self.scroll_rest -= whole;
                self.scroll_by(whole as i64);
            }
        }
        let fit = ((rect.height() / row_height).floor() as usize).max(1);
        if !self.wrap {
            self.visible_rows = fit;
        }
        self.scroll_to(self.top);
        self.ensure_lines(self.top, fit + 1);
        let known = self.known_lines();
        let gutter = (digits(known) as f32) * glyph + 14.0;
        let text_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.right() - SCROLLBAR_WIDTH, rect.bottom()),
        );
        if self.wrap {
            self.wrapped_rows(ui, text_rect, gutter, row_height, &font, &colors);
        } else {
            self.plain_rows(ui, text_rect, gutter, fit, row_height, &font, &colors);
        }
        self.scrollbar(ui, rect, known);
    }

    fn number(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        line: u64,
        font: &egui::FontId,
        colors: &BodyColors,
    ) {
        ui.painter().text(
            egui::pos2(rect.right() - 6.0, rect.top()),
            egui::Align2::RIGHT_TOP,
            (line + 1).to_string(),
            font.clone(),
            colors.gutter,
        );
        let response = ui.interact(
            rect,
            egui::Id::new(("text_view_number", self.id, line)),
            egui::Sense::click(),
        );
        if response.clicked() {
            let shift = ui.input(|input| input.modifiers.shift);
            self.click_number(line, shift);
        }
    }

    fn row_background(&self, line: u64, colors: &BodyColors) -> Option<Color32> {
        if self.selected(line) {
            Some(colors.selection)
        } else if self.cursor == Some(line) {
            Some(colors.line)
        } else {
            None
        }
    }

    fn line_job(
        &self,
        line: u64,
        font: &egui::FontId,
        colors: &BodyColors,
    ) -> egui::text::LayoutJob {
        match self.line(line) {
            Some(styled) => styled_job(
                styled,
                font,
                colors.foreground,
                self.marks_for(line),
                colors.mark,
            ),
            None => plain_job(" ", font, colors.gutter, &[], colors.mark),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn plain_rows(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        gutter: f32,
        fit: usize,
        row_height: f32,
        font: &egui::FontId,
        colors: &BodyColors,
    ) {
        let known = self.known_lines();
        let rows: Vec<u64> = (self.top..known).take(fit).collect();
        for (index, line) in rows.iter().enumerate() {
            let top = rect.top() + index as f32 * row_height;
            let row = egui::Rect::from_min_size(
                egui::pos2(rect.left(), top),
                egui::vec2(rect.width(), row_height),
            );
            if let Some(fill) = self.row_background(*line, colors) {
                ui.painter().rect_filled(row, 0.0, fill);
            }
            let number = egui::Rect::from_min_size(row.min, egui::vec2(gutter, row_height));
            self.number(ui, number, *line, font, colors);
        }
        let text_rect =
            egui::Rect::from_min_max(egui::pos2(rect.left() + gutter, rect.top()), rect.max);
        let jobs: Vec<egui::text::LayoutJob> = rows
            .iter()
            .map(|line| self.line_job(*line, font, colors))
            .collect();
        let mut clicked_text = false;
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(text_rect), |ui| {
            ui.set_clip_rect(text_rect.intersect(ui.clip_rect()));
            egui::ScrollArea::horizontal()
                .id_salt(("text_view_h", self.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for job in jobs {
                        let response = ui.add(egui::Label::new(job).extend().selectable(true));
                        clicked_text |= response.clicked() || response.drag_started();
                    }
                });
        });
        if clicked_text {
            self.selection = None;
        }
    }

    fn wrapped_rows(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        gutter: f32,
        row_height: f32,
        font: &egui::FontId,
        colors: &BodyColors,
    ) {
        let known = self.known_lines();
        let mut shown = 0usize;
        let mut clicked_text = false;
        let mut numbers = Vec::new();
        let top = self.top;
        let jobs: Vec<(u64, egui::text::LayoutJob, Option<Color32>)> = (top..known)
            .take((rect.height() / row_height) as usize + 1)
            .map(|line| {
                (
                    line,
                    self.line_job(line, font, colors),
                    self.row_background(line, colors),
                )
            })
            .collect();
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing.y = 0.0;
            for (line, job, fill) in jobs {
                if ui.cursor().top() >= rect.bottom() {
                    break;
                }
                let background = ui.painter().add(egui::Shape::Noop);
                let row = ui.horizontal_top(|ui| {
                    let (number, _) = ui
                        .allocate_exact_size(egui::vec2(gutter, row_height), egui::Sense::hover());
                    let response = ui.add(egui::Label::new(job).wrap().selectable(true));
                    clicked_text |= response.clicked() || response.drag_started();
                    number
                });
                let row_rect = row.response.rect;
                if let Some(fill) = fill {
                    let full = egui::Rect::from_min_max(
                        row_rect.min,
                        egui::pos2(rect.right(), row_rect.bottom()),
                    );
                    ui.painter()
                        .set(background, egui::Shape::rect_filled(full, 0.0, fill));
                }
                numbers.push((row.inner, line));
                shown += 1;
            }
        });
        for (number, line) in numbers {
            self.number(ui, number, line, font, colors);
        }
        self.visible_rows = shown.max(1);
        if clicked_text {
            self.selection = None;
        }
    }

    fn scrollbar(&mut self, ui: &mut egui::Ui, rect: egui::Rect, known: u64) {
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.right() - SCROLLBAR_WIDTH, rect.top()),
            rect.max,
        );
        let max_top = self.max_top();
        let colors = super::theme::palette(ui);
        ui.painter().rect_filled(bar, 0.0, colors.subtle);
        let visible = self.visible_rows.max(1) as f32;
        let total = known.max(1) as f32;
        let height = bar.height();
        let thumb = (height * (visible / total).min(1.0)).max(18.0).min(height);
        let travel = (height - thumb).max(0.0);
        let fraction = if max_top == 0 {
            0.0
        } else {
            self.top as f32 / max_top as f32
        };
        let thumb_rect = egui::Rect::from_min_size(
            egui::pos2(bar.left() + 2.0, bar.top() + travel * fraction),
            egui::vec2(SCROLLBAR_WIDTH - 4.0, thumb),
        );
        ui.painter()
            .rect_filled(thumb_rect, 3.0, colors.control_border);
        let response = ui.interact(
            bar,
            egui::Id::new(("text_view_scroll", self.id)),
            egui::Sense::click_and_drag(),
        );
        if response.dragged() || response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let at = ((pointer.y - bar.top() - thumb / 2.0) / travel.max(1.0)).clamp(0.0, 1.0);
                self.scroll_to((at as f64 * max_top as f64).round() as u64);
            }
        }
    }
}
