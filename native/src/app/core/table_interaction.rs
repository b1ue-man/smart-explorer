use super::prelude::*;
use super::*;

impl App {
    pub(super) fn update_table_background_interaction(
        &mut self,
        ui: &mut egui::Ui,
        visible_rows: &[(usize, egui::Rect)],
        row_h: f32,
        total_rows: usize,
        row_hit: bool,
        row_right_clicked: bool,
    ) {
        let table_rect = ui.min_rect();
        let body_viewport = egui::Rect::from_min_max(
            egui::pos2(table_rect.left(), table_rect.top() + 24.0),
            table_rect.max,
        )
        .intersect(ui.clip_rect());

        let (primary_pressed, primary_down, primary_released, ptr_pos, ctrl_now, secondary_clicked) =
            ui.input(|i| {
                (
                    i.pointer.primary_pressed(),
                    i.pointer.primary_down(),
                    i.pointer.primary_released(),
                    i.pointer.latest_pos(),
                    i.modifiers.ctrl || i.modifiers.command,
                    i.pointer.secondary_clicked(),
                )
            });

        // base_y maps content row i to screen y: row_top(i) = base_y + i*row_h
        let base_y = visible_rows
            .first()
            .map(|&(idx, rect)| rect.top() - idx as f32 * row_h);
        let anything_dragged = ui.ctx().dragged_id().is_some();

        if primary_pressed && !anything_dragged && !self.band_suppressed {
            if let Some(p) = ptr_pos.filter(|p| body_viewport.contains(*p)) {
                // Store the press in screen coordinates so drag distance remains
                // stable while the virtualized table settles its layout.
                self.band_press = Some((p.x, p.y));
                self.band_base = if ctrl_now {
                    self.selection.clone()
                } else {
                    HashSet::new()
                };
            }
        }

        if let Some((press_x, press_y)) = self.band_press.filter(|_| !self.band_suppressed) {
            if anything_dragged {
                self.band_press = None;
                self.band_active = false;
            } else if primary_down {
                if let (Some(p), Some(by)) = (ptr_pos, base_y) {
                    if self.band_active
                        || (p.y - press_y).abs() > 4.0
                        || (p.x - press_x).abs() > 4.0
                    {
                        self.band_active = true;
                        self.update_table_band_selection(
                            ui,
                            body_viewport,
                            by,
                            row_h,
                            total_rows,
                            press_x,
                            press_y,
                            p,
                        );
                    }
                }
            }
            if primary_released {
                // A click below the rows clears selection, unless a row handler
                // already claimed the same pointer event.
                if !self.band_active && !row_hit {
                    if let (Some(p), Some(by)) = (ptr_pos, base_y) {
                        let last_bottom = by + total_rows as f32 * row_h;
                        if p.y > last_bottom + 2.0 && body_viewport.contains(p) {
                            self.selection.clear();
                            self.cursor = None;
                        }
                    }
                }
                self.band_press = None;
                self.band_active = false;
            }
        }

        if secondary_clicked && !row_right_clicked {
            if let Some(p) = ptr_pos {
                let on_empty = base_y.is_none_or(|by| p.y > by + total_rows as f32 * row_h);
                if body_viewport.contains(p) && on_empty {
                    self.show_background_menu();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update_table_band_selection(
        &mut self,
        ui: &egui::Ui,
        body_viewport: egui::Rect,
        base_y: f32,
        row_h: f32,
        total_rows: usize,
        press_x: f32,
        press_y: f32,
        pointer: egui::Pos2,
    ) {
        let (lo_y, hi_y) = if press_y < pointer.y {
            (press_y, pointer.y)
        } else {
            (pointer.y, press_y)
        };
        let lo_off = lo_y - base_y;
        let hi_off = hi_y - base_y;
        let mut selection = self.band_base.clone();
        if total_rows > 0 && hi_off >= 0.0 {
            let lo_row = (lo_off / row_h).floor().max(0.0) as usize;
            let hi_row = ((hi_off / row_h).floor() as isize).min(total_rows as isize - 1);
            if hi_row >= 0 && lo_row < total_rows {
                for row in lo_row..=(hi_row as usize) {
                    selection.insert(self.entries[self.view[row].0].path.clone());
                }
            }
        }
        self.selection = selection;

        let y0 = lo_y.max(body_viewport.top());
        let y1 = hi_y.min(body_viewport.bottom());
        let x0 = press_x.min(pointer.x).max(body_viewport.left());
        let x1 = press_x.max(pointer.x).min(body_viewport.right());
        if y1 > y0 && x1 > x0 {
            let rect = egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1));
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(90, 140, 255, 36));
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0_f32, Color32::from_rgb(90, 140, 255)),
            );
        }

        if pointer.y > body_viewport.bottom() - 4.0 {
            let bottom_row = (((body_viewport.bottom() - base_y) / row_h) as usize + 2)
                .min(total_rows.saturating_sub(1));
            self.pending_scroll_row = Some(bottom_row);
        } else if pointer.y < body_viewport.top() + 4.0 {
            let top_row =
                (((body_viewport.top() - base_y) / row_h).max(0.0) as usize).saturating_sub(2);
            self.pending_scroll_row = Some(top_row);
        }
        ui.ctx().request_repaint();
    }
}
