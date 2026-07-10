use super::prelude::*;
use super::table_accessibility::TableRowSemantics;
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
                    let row_semantics = TableRowSemantics::new(
                        e.name.as_ref(),
                        e.path.as_ref(),
                        e.is_dir,
                        selected,
                    );

                    let mut handle_resp =
                        |resp: egui::Response, ui: &egui::Ui, column: &str, value: &str| {
                            row_semantics.annotate_cell(&resp, column, value);
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
                        handle_resp(resp, ui, "Name", e.name.as_ref());
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
                        handle_resp(resp, ui, "Pfad", &rel);
                    });

                    // ─── Size ──────────────────────────────────────────
                    row.col(|ui| {
                        let txt = if e.is_dir {
                            String::new()
                        } else {
                            format_bytes(e.size)
                        };
                        let resp = handle_cell(ui, &txt, true);
                        handle_resp(resp, ui, "Größe", &txt);
                    });

                    // ─── Dates ─────────────────────────────────────────
                    row.col(|ui| {
                        let value = format_date(e.mtime_ms);
                        let resp = handle_cell(ui, &value, false);
                        handle_resp(resp, ui, "Geändert", &value);
                    });
                    row.col(|ui| {
                        let value = format_date(e.btime_ms);
                        let resp = handle_cell(ui, &value, false);
                        handle_resp(resp, ui, "Erstellt", &value);
                    });

                    // ─── Ext ───────────────────────────────────────────
                    row.col(|ui| {
                        let resp = handle_cell(ui, e.ext.as_ref(), false);
                        handle_resp(resp, ui, "Typ", e.ext.as_ref());
                    });

                    // ─── Depth ─────────────────────────────────────────
                    row.col(|ui| {
                        let value = e.depth.to_string();
                        let resp = handle_cell(ui, &value, true);
                        handle_resp(resp, ui, "Tiefe", &value);
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

        let row_hit = row_click.is_some() || row_dblclick.is_some() || row_rclick.is_some();
        self.update_table_background_interaction(
            ui,
            &visible_rows,
            row_h,
            total_rows,
            row_hit,
            row_rclick.is_some(),
        );

        // Queue icon extraction for any type seen this frame but not cached.
        for key in needed_icons {
            self.icon_cache.request(key);
        }
    }
}
