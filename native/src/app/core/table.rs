use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_table(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let prefix = self.root_prefix();
        let total_rows = self.view.len();
        let row_h = 22.0;

        let mut row_click: Option<(usize, bool, bool)> = None; // (idx, ctrl, shift)
        let mut row_dblclick: Option<usize> = None;
        let mut row_rclick: Option<usize> = None;
        let mut sort_clicked: Option<SortKey> = None;
        // Entry index of a row whose drag just started this frame (file drag to
        // another tab/pane or out to Explorer). Resolved after the table.
        let mut drag_start: Option<usize> = None;
        // (row index, name-cell rect) of rendered rows — used for rubber-band
        // geometry below.
        let mut visible_rows: Vec<(usize, egui::Rect)> = Vec::new();
        // Icon keys seen this frame that aren't cached yet (requested after the
        // table, since we can't mutably borrow self.icon_cache inside the body).
        let mut needed_icons: Vec<String> = Vec::new();

        let header_def: &[(SortKey, &str)] = &[
            (SortKey::Name, "Name"),
            (SortKey::Path, "Pfad"),
            (SortKey::Size, "Größe"),
            (SortKey::Mtime, "Geändert"),
            (SortKey::Btime, "Erstellt"),
            (SortKey::Ext, "Typ"),
            (SortKey::Depth, "Tiefe"),
        ];

        let mut builder = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(
                Column::initial(240.0)
                    .at_least(120.0)
                    .resizable(true)
                    .clip(true),
            ) // name
            .column(
                Column::initial(360.0)
                    .at_least(120.0)
                    .resizable(true)
                    .clip(true),
            ) // path
            .column(Column::initial(90.0).at_least(60.0).resizable(true)) // size
            .column(Column::initial(130.0).at_least(80.0).resizable(true)) // mtime
            .column(Column::initial(130.0).at_least(80.0).resizable(true)) // btime
            .column(Column::initial(60.0).at_least(40.0).resizable(true)) // ext
            .column(Column::remainder().at_least(40.0)); // depth

        if let Some(r) = self.pending_scroll_row.take() {
            builder = builder.scroll_to_row(r, Some(egui::Align::Center));
        }

        builder
            .header(22.0, |mut header| {
                for (key, label) in header_def {
                    header.col(|ui| {
                        let arrow = if self.sort_key == *key {
                            if self.sort_dir == SortDir::Asc {
                                " ▲"
                            } else {
                                " ▼"
                            }
                        } else {
                            ""
                        };
                        let txt = RichText::new(format!("{}{}", label, arrow)).strong();
                        if ui.selectable_label(self.sort_key == *key, txt).clicked() {
                            sort_clicked = Some(*key);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, total_rows, |mut row| {
                    let row_index = row.index();
                    let (entry_idx, display_depth) = self.view[row_index];
                    let e = &self.entries[entry_idx];
                    let selected = self.selection.contains(&e.key());
                    row.set_selected(selected);

                    let mut handle_resp = |resp: egui::Response, ui: &egui::Ui| {
                        if resp.clicked() {
                            let m = ui.input(|i| {
                                (i.modifiers.ctrl || i.modifiers.command, i.modifiers.shift)
                            });
                            row_click = Some((entry_idx, m.0, m.1));
                        }
                        if resp.double_clicked() {
                            row_dblclick = Some(entry_idx);
                        }
                        if resp.secondary_clicked() {
                            row_rclick = Some(entry_idx);
                        }
                        // Dragging a row begins a file drag (resolved after the
                        // table). The rubber-band bails when a drag is active, so
                        // these don't fight.
                        if resp.drag_started() {
                            drag_start = Some(entry_idx);
                        }
                    };

                    let handle_cell = |ui: &mut egui::Ui, content: &str, right_align: bool| {
                        let cell_w = ui.available_width();
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(cell_w, row_h),
                            egui::Sense::click_and_drag(),
                        );
                        let color = if selected {
                            ui.visuals().selection.stroke.color
                        } else {
                            ui.visuals().text_color()
                        };
                        paint_cell_text(ui, rect, content, right_align, color, 0.0);
                        resp
                    };

                    // ─── Name (with indent + native icon) ──────────────
                    row.col(|ui| {
                        let cell_w = ui.available_width();
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(cell_w, row_h),
                            egui::Sense::click_and_drag(),
                        );
                        visible_rows.push((row_index, rect));
                        let indent = display_depth.min(32) as f32 * 14.0;
                        let color = if selected {
                            ui.visuals().selection.stroke.color
                        } else {
                            ui.visuals().text_color()
                        };
                        // 16px icon slot at the left of the cell (after indent);
                        // the name always sits at indent+20 so it never shifts
                        // when the real icon replaces the emoji placeholder.
                        let icon_center =
                            egui::pos2(rect.left() + 4.0 + indent + 8.0, rect.center().y);
                        let key = crate::icons::icon_key(e.is_dir, e.ext.as_ref());
                        if let Some(tex) = self.icon_cache.get(&key) {
                            let icon_rect =
                                egui::Rect::from_center_size(icon_center, egui::vec2(16.0, 16.0));
                            egui::Image::from_texture(egui::load::SizedTexture::new(
                                tex.id(),
                                egui::vec2(16.0, 16.0),
                            ))
                            .paint_at(ui, icon_rect);
                        } else {
                            needed_icons.push(key);
                            let emoji = if e.is_dir { "📁" } else { "📄" };
                            ui.painter().text(
                                icon_center,
                                egui::Align2::CENTER_CENTER,
                                emoji,
                                egui::TextStyle::Body.resolve(ui.style()),
                                color,
                            );
                        }
                        paint_cell_text(ui, rect, e.name.as_ref(), false, color, indent + 20.0);
                        handle_resp(resp, ui);
                    });

                    // ─── Path (relative) ───────────────────────────────
                    row.col(|ui| {
                        let rel = if e.path.starts_with(&prefix) {
                            let r = e
                                .path
                                .as_ref()
                                .trim_start_matches(prefix.as_str())
                                .trim_start_matches('/');
                            if r.is_empty() {
                                "/".to_string()
                            } else {
                                r.to_string()
                            }
                        } else {
                            e.path.to_string()
                        };
                        let resp = handle_cell(ui, &rel, false);
                        handle_resp(resp, ui);
                    });

                    // ─── Size ──────────────────────────────────────────
                    row.col(|ui| {
                        let txt = if e.is_dir {
                            String::new()
                        } else {
                            format_bytes(e.size)
                        };
                        let resp = handle_cell(ui, &txt, true);
                        handle_resp(resp, ui);
                    });

                    // ─── Dates ─────────────────────────────────────────
                    row.col(|ui| {
                        let resp = handle_cell(ui, &format_date(e.mtime_ms), false);
                        handle_resp(resp, ui);
                    });
                    row.col(|ui| {
                        let resp = handle_cell(ui, &format_date(e.btime_ms), false);
                        handle_resp(resp, ui);
                    });

                    // ─── Ext ───────────────────────────────────────────
                    row.col(|ui| {
                        let resp = handle_cell(ui, e.ext.as_ref(), false);
                        handle_resp(resp, ui);
                    });

                    // ─── Depth ─────────────────────────────────────────
                    row.col(|ui| {
                        let resp = handle_cell(ui, &format!("{}", e.depth), true);
                        handle_resp(resp, ui);
                    });
                });
            });

        // A row drag started → capture the files (the whole selection if the
        // dragged row is part of it, otherwise just that row). Local files only
        // (remote items would need a download to drop elsewhere).
        if let Some(idx) = drag_start {
            if !self.drag_active {
                let dragged = self.entries[idx].key();
                let dragged_path = self.entries[idx].path.clone();
                let mut files: Vec<String> = if self.selection.contains(&dragged) {
                    self.selection
                        .iter()
                        .map(|k| sel_key_path(k).to_string())
                        .collect()
                } else {
                    vec![dragged_path.to_string()]
                };
                // From a local view we only carry local paths; from a remote view
                // the paths are remote and `drag_src` is the source backend.
                if self.remote.is_none() {
                    files.retain(|p| is_local_style(p));
                }
                if !files.is_empty() {
                    let has_dir = if self.selection.contains(&dragged) {
                        self.entries
                            .iter()
                            .any(|e| e.is_dir && self.selection.contains(&e.key()))
                    } else {
                        self.entries[idx].is_dir
                    };
                    self.drag_files = files;
                    self.drag_active = true;
                    self.drag_src = self.remote.as_ref().map(|rs| rs.backend.clone());
                    self.drag_filter = (has_dir && self.filter_is_active())
                        .then(|| (self.filter.clone(), self.root_prefix()));
                    self.drag_source_tab = self.current_render_tab;
                    self.drag_out_started = false;
                }
            }
        }

        if let Some(k) = sort_clicked {
            if self.sort_key == k {
                self.sort_dir = if self.sort_dir == SortDir::Asc {
                    SortDir::Desc
                } else {
                    SortDir::Asc
                };
            } else {
                self.sort_key = k;
                self.sort_dir = SortDir::Asc;
            }
            self.recompute_view();
        }

        if let Some((idx, ctrl, shift)) = row_click {
            let path = self.entries[idx].path.clone();
            let key = self.entries[idx].key();
            if shift && !ctrl {
                // Explorer semantics: Shift+Click replaces the selection with
                // the anchor→clicked range.
                if let Some(anchor) = self.last_anchor.clone() {
                    let a = self
                        .view
                        .iter()
                        .position(|&(i, _)| self.entries[i].key() == anchor);
                    let b = self
                        .view
                        .iter()
                        .position(|&(i, _)| self.entries[i].key() == key);
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                        self.selection.clear();
                        for i in lo..=hi {
                            self.selection.insert(self.entries[self.view[i].0].key());
                        }
                    } else {
                        self.selection.insert(key.clone());
                    }
                } else {
                    self.selection.insert(key.clone());
                    self.last_anchor = Some(key.clone());
                }
            } else if shift && ctrl {
                // Ctrl+Shift+Click: add range to existing selection
                if let Some(anchor) = self.last_anchor.clone() {
                    let a = self
                        .view
                        .iter()
                        .position(|&(i, _)| self.entries[i].key() == anchor);
                    let b = self
                        .view
                        .iter()
                        .position(|&(i, _)| self.entries[i].key() == key);
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                        for i in lo..=hi {
                            self.selection.insert(self.entries[self.view[i].0].key());
                        }
                    }
                }
            } else if ctrl {
                if !self.selection.insert(key.clone()) {
                    self.selection.remove(&key);
                }
                self.last_anchor = Some(key.clone());
            } else {
                self.selection.clear();
                self.selection.insert(key.clone());
                self.last_anchor = Some(key.clone());
            }
            self.cursor = Some(path);
        }

        if let Some(idx) = row_dblclick {
            self.activate_entry(idx);
        }

        if let Some(idx) = row_rclick {
            let key = self.entries[idx].key();
            if !self.selection.contains(&key) {
                self.selection.clear();
                self.selection.insert(key.clone());
                self.last_anchor = Some(key.clone());
            }
            // Remotes have no Windows shell menu (those paths aren't local) — show
            // our own egui context menu instead.
            if self.remote.is_some() {
                let pos = ui
                    .ctx()
                    .input(|i| i.pointer.interact_pos())
                    .unwrap_or_else(|| ui.min_rect().center());
                self.remote_ctx = Some((pos, idx));
            } else {
                let path = self.entries[idx].path.to_string();
                let ctx = ui.ctx().clone();
                self.show_shell_menu_for(&path, &ctx);
            }
        }

        // ─── Rubber-band selection + empty-space interactions ─────────────
        let table_rect = ui.min_rect();
        let body_viewport = egui::Rect::from_min_max(
            egui::pos2(table_rect.left(), table_rect.top() + 24.0),
            table_rect.max,
        );

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

        // A row was interacted with this frame? Then the pointer is over a row,
        // not empty space — the rubber-band / empty-space-clear logic must not
        // touch the selection that the row handlers just set.
        let row_hit = row_click.is_some() || row_dblclick.is_some() || row_rclick.is_some();

        if primary_pressed && !anything_dragged && !self.band_suppressed {
            if let Some(p) = ptr_pos {
                if body_viewport.contains(p) {
                    // Store the press in SCREEN coordinates so the drag-distance
                    // test is stable even if the table's base-Y shifts a pixel
                    // when layout settles (which previously could both spuriously
                    // start a band and mis-clear the bottom row's selection).
                    self.band_press = Some((p.x, p.y));
                    self.band_base = if ctrl_now {
                        self.selection.clone()
                    } else {
                        HashSet::new()
                    };
                }
            }
        }

        if let Some((press_x, press_y)) = self.band_press.filter(|_| !self.band_suppressed) {
            if anything_dragged {
                // A column-resize (or other) drag claimed the pointer.
                self.band_press = None;
                self.band_active = false;
            } else if primary_down {
                if let (Some(p), Some(by)) = (ptr_pos, base_y) {
                    if self.band_active
                        || (p.y - press_y).abs() > 4.0
                        || (p.x - press_x).abs() > 4.0
                    {
                        self.band_active = true;
                        let (lo_y, hi_y) = if press_y < p.y {
                            (press_y, p.y)
                        } else {
                            (p.y, press_y)
                        };
                        // Map both screen endpoints to rows via the current base-Y.
                        let lo_off = lo_y - by;
                        let hi_off = hi_y - by;
                        let mut sel = self.band_base.clone();
                        if total_rows > 0 && hi_off >= 0.0 {
                            let lo_row = (lo_off / row_h).floor().max(0.0) as usize;
                            let hi_row =
                                ((hi_off / row_h).floor() as isize).min(total_rows as isize - 1);
                            if hi_row >= 0 && lo_row < total_rows {
                                for r in lo_row..=(hi_row as usize) {
                                    sel.insert(self.entries[self.view[r].0].path.clone());
                                }
                            }
                        }
                        self.selection = sel;

                        // Draw the band (screen coords, clamped to the viewport)
                        let y0 = lo_y.max(body_viewport.top());
                        let y1 = hi_y.min(body_viewport.bottom());
                        let x0 = press_x.min(p.x).max(body_viewport.left());
                        let x1 = press_x.max(p.x).min(body_viewport.right());
                        if y1 > y0 && x1 > x0 {
                            let rect =
                                egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1));
                            let painter = ui.painter();
                            painter.rect_filled(
                                rect,
                                0.0,
                                Color32::from_rgba_unmultiplied(90, 140, 255, 36),
                            );
                            painter.rect_stroke(
                                rect,
                                0.0,
                                egui::Stroke::new(1.0, Color32::from_rgb(90, 140, 255)),
                            );
                        }

                        // Auto-scroll when the pointer leaves the viewport
                        if p.y > body_viewport.bottom() - 4.0 {
                            let bottom_row = (((body_viewport.bottom() - by) / row_h) as usize + 2)
                                .min(total_rows.saturating_sub(1));
                            self.pending_scroll_row = Some(bottom_row);
                        } else if p.y < body_viewport.top() + 4.0 {
                            let top_row = (((body_viewport.top() - by) / row_h).max(0.0) as usize)
                                .saturating_sub(2);
                            self.pending_scroll_row = Some(top_row);
                        }
                        ui.ctx().request_repaint();
                    }
                }
            }
            if primary_released {
                // Click (no drag) on empty space below the rows clears the
                // selection, like Explorer — but ONLY if the click didn't land
                // on a row (otherwise we'd wipe the just-made selection).
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

        // Right-click on empty space → folder background menu
        if secondary_clicked && row_rclick.is_none() {
            if let Some(p) = ptr_pos {
                let on_empty = match base_y {
                    Some(by) => p.y > by + total_rows as f32 * row_h,
                    None => true,
                };
                if body_viewport.contains(p) && on_empty {
                    self.show_background_menu();
                }
            }
        }

        // Queue icon extraction for any type seen this frame but not cached.
        for key in needed_icons {
            self.icon_cache.request(key);
        }
    }
}
