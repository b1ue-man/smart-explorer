use super::prelude::*;
use super::*;

impl App {
    /// Storage-analytics overlay: a dedicated low-memory size scan rendered as a
    /// nested (WizTree-style) squarified treemap. Defaults to the whole drive of
    /// the current folder; click a box to drill in, use the breadcrumb to go up.
    pub(in crate::app) fn ui_analytics(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering::Relaxed;
        self.poll_analytics_scan();
        // First open with nothing scanned yet → scan the current remote folder if
        // browsing a remote, otherwise the whole local drive.
        if self.analytics_tree.is_none() && self.analytics_scan.is_none() {
            if let Some(rs) = self.remote.as_ref() {
                let be = rs.backend.clone();
                let label = rs.label.clone();
                let root = self.root_path.clone();
                self.start_analytics_scan_remote(be, root, label);
            } else {
                let r = self.analytics_default_root();
                if !r.is_empty() {
                    self.start_analytics_scan(r);
                }
            }
        }
        if self.analytics_counts.is_none() {
            if let Some(node) = self.analytics_focus_node() {
                self.analytics_counts = Some(count_subtree(node));
            }
        }

        let drive = self.drive_usage(&self.analytics_root_path);
        let drives = self.drive_info.clone();
        let root_label = if self.analytics_root_path.is_empty() {
            "—".to_string()
        } else {
            self.analytics_root_path.clone()
        };
        // Current remote (for the "scan this remote folder" button) + the source
        // the current tree came from (for ⟳ to re-walk the same place).
        let remote_scan: Option<(crate::vfs::BackendHandle, String, String)> = self
            .remote
            .as_ref()
            .map(|rs| (rs.backend.clone(), self.root_path.clone(), rs.label.clone()));
        let cur_backend = self.analytics_backend.clone();
        let cur_root = self.analytics_root_path.clone();
        let focus_segs = self.analytics_focus.clone();
        let focus_path = self.analytics_focus_path();
        let focus_size = self.analytics_focus_node().map(|n| n.size).unwrap_or(0);
        let (n_files, n_dirs) = self.analytics_counts.unwrap_or((0, 0));
        let scan_info = self.analytics_scan.as_ref().map(|s| {
            (
                s.progress.files.load(Relaxed),
                s.progress.dirs.load(Relaxed),
                s.progress.bytes.load(Relaxed),
                s.root.clone(),
                s.started.elapsed().as_secs_f32(),
            )
        });

        let focus_node = self.analytics_focus_node();
        let cached_cells = &self.analytics_cells;
        let cached_rect = self.analytics_cells_rect;

        let mut open = true;
        let mut nav: Option<String> = None; // open folder in main explorer
        let mut reveal: Option<String> = None; // reveal file in main explorer
        let mut drill_path: Option<String> = None; // treemap click → drill into folder
        let mut set_focus: Option<usize> = None; // breadcrumb truncate
        let mut go_up = false;
        let mut rescan: Option<String> = None; // local path to (re)scan
        let mut rescan_remote: Option<(crate::vfs::BackendHandle, String, String)> = None;
        let mut pick_folder = false;
        let mut cancel = false;
        let mut recomputed: Option<(Vec<TmCell>, egui::Rect)> = None;

        {
            egui::Window::new("📊 Speicher-Analyse")
                .id(egui::Id::new("analyse_treemap_v2"))
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_size([880.0, 600.0])
                .min_width(440.0)
                .constrain(true)
                .show(ctx, |ui| {
                    // ── Row 1: scan targets ──
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Scannen:")
                                .small()
                                .color(Color32::from_gray(150)),
                        );
                        for (root, free, total) in &drives {
                            let dl: String = root.chars().take(2).collect();
                            let used = total.saturating_sub(*free);
                            let label = if *total > 0 {
                                format!("{} ({}/{})", dl, format_bytes(used), format_bytes(*total))
                            } else {
                                dl.clone()
                            };
                            if ui.button(label).clicked() {
                                rescan = Some(format!("{}/", dl));
                            }
                        }
                        if ui.button("📁 Ordner…").clicked() {
                            pick_folder = true;
                        }
                        if let Some((be, root, label)) = &remote_scan {
                            let txt = if label.is_empty() {
                                "📡 Remote-Ordner".to_string()
                            } else {
                                format!("📡 {}", label)
                            };
                            if ui
                                .button(txt)
                                .on_hover_text(format!("Aktuellen Remote-Ordner scannen: {}", root))
                                .clicked()
                            {
                                rescan_remote = Some((be.clone(), root.clone(), label.clone()));
                            }
                        }
                        if ui.button("⟳").on_hover_text("Neu scannen").clicked() {
                            // Re-walk whatever the current tree came from.
                            if let Some(be) = &cur_backend {
                                rescan_remote = Some((be.clone(), cur_root.clone(), String::new()));
                            } else {
                                rescan = Some(root_label.clone());
                            }
                        }
                    });

                    // ── Row 2: breadcrumb ──
                    ui.horizontal_wrapped(|ui| {
                        if !focus_segs.is_empty()
                            && ui.button("↑").on_hover_text("Eine Ebene höher").clicked()
                        {
                            go_up = true;
                        }
                        if ui.button(RichText::new(&root_label).strong()).clicked() {
                            set_focus = Some(0);
                        }
                        for (i, seg) in focus_segs.iter().enumerate() {
                            ui.label("›");
                            if ui.button(seg).clicked() {
                                set_focus = Some(i + 1);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("📂 Im Explorer öffnen").clicked() {
                                nav = Some(focus_path.clone());
                            }
                        });
                    });

                    if let Some((used, tot)) = drive {
                        let frac = used as f32 / tot as f32;
                        ui.add(
                            egui::ProgressBar::new(frac)
                                .desired_width(ui.available_width())
                                .text(format!(
                                    "Laufwerk: {} von {} belegt ({:.0}%)",
                                    format_bytes(used),
                                    format_bytes(tot),
                                    frac * 100.0
                                )),
                        );
                    }

                    if let Some((f, d, b, root, secs)) = &scan_info {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let rate = if *secs > 0.0 { *f as f32 / *secs } else { 0.0 };
                            ui.label(format!(
                                "Scanne {} … {} Dateien · {} Ordner · {}  ({:.0}/s)",
                                root,
                                f,
                                d,
                                format_bytes(*b),
                                rate
                            ));
                            if ui.button("Abbrechen").clicked() {
                                cancel = true;
                            }
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(150));
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format_bytes(focus_size)).strong());
                            ui.label(format!("· {} Dateien · {} Ordner", n_files, n_dirs));
                            ui.label(
                                RichText::new("· Klick = reinzoomen")
                                    .small()
                                    .color(Color32::from_gray(130)),
                            );
                        });
                    }
                    ui.separator();

                    // ── Nested treemap ──
                    let tm_w = ui.available_width();
                    let tm_h = ui.available_height().max(200.0);
                    let (tm_rect, tm_resp) =
                        ui.allocate_exact_size(egui::vec2(tm_w, tm_h), egui::Sense::click());

                    // (Re)lay out only on resize or drill — painting reuses cells.
                    let need = focus_node.is_some()
                        && (cached_cells.is_empty()
                            || (cached_rect.size() - tm_rect.size()).length() > 2.0);
                    let cells: &[TmCell] = if need {
                        let mut v = Vec::new();
                        if let Some(node) = focus_node {
                            nested_treemap(tm_rect, node, &focus_path, 0, None, &mut v);
                        }
                        recomputed = Some((v, tm_rect));
                        &recomputed.as_ref().unwrap().0
                    } else {
                        cached_cells
                    };

                    let painter = ui.painter_at(tm_rect);
                    painter.rect_filled(tm_rect, 0.0, Color32::from_gray(22));
                    for cell in cells {
                        if cell.container {
                            // Folder = darkened group hue + a lighter header strip.
                            let fill = cell.color.gamma_multiply(0.40);
                            painter.rect_filled(cell.rect, 2.0, fill);
                            painter.rect_stroke(
                                cell.rect,
                                2.0,
                                egui::Stroke::new(1.0, Color32::from_black_alpha(130)),
                            );
                            let hr = egui::Rect::from_min_max(
                                cell.rect.min,
                                egui::pos2(cell.rect.max.x, cell.rect.min.y + TM_HEADER),
                            );
                            painter.rect_filled(hr, 0.0, cell.color.gamma_multiply(0.7));
                            painter.with_clip_rect(hr.shrink(2.0)).text(
                                hr.min + egui::vec2(4.0, 1.0),
                                egui::Align2::LEFT_TOP,
                                format!("{}  {}", cell.name, format_bytes(cell.size)),
                                egui::FontId::proportional(11.0),
                                Color32::from_gray(235),
                            );
                        } else {
                            painter.rect_filled(cell.rect, 1.0, cell.color);
                            painter.rect_stroke(
                                cell.rect,
                                1.0,
                                egui::Stroke::new(0.5, Color32::from_black_alpha(70)),
                            );
                            if cell.rect.width() > 40.0 && cell.rect.height() > 15.0 {
                                let col = cell.color;
                                let lum = 0.299 * col.r() as f32
                                    + 0.587 * col.g() as f32
                                    + 0.114 * col.b() as f32;
                                let tc = if lum < 140.0 {
                                    Color32::from_gray(245)
                                } else {
                                    Color32::from_gray(20)
                                };
                                // Clip to the cell so long names don't bleed across.
                                painter.with_clip_rect(cell.rect.shrink(2.0)).text(
                                    cell.rect.left_top() + egui::vec2(3.0, 2.0),
                                    egui::Align2::LEFT_TOP,
                                    format!(
                                        "{}{}\n{}",
                                        if cell.is_dir { "📁 " } else { "" },
                                        cell.name,
                                        format_bytes(cell.size)
                                    ),
                                    egui::FontId::proportional(11.0),
                                    tc,
                                );
                            }
                        }
                    }

                    // Hover tooltip + click-to-drill: deepest cell under pointer.
                    let tm_resp = tm_resp.on_hover_ui(|ui| {
                        if let Some(pos) = ui.ctx().pointer_hover_pos() {
                            if let Some(cell) = cells.iter().rev().find(|c| c.rect.contains(pos)) {
                                let pct = if focus_size > 0 {
                                    cell.size as f64 / focus_size as f64 * 100.0
                                } else {
                                    0.0
                                };
                                // Don't wrap the tooltip into a narrow column.
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                ui.label(
                                    RichText::new(format!(
                                        "{}{}",
                                        if cell.is_dir { "📁 " } else { "" },
                                        cell.name
                                    ))
                                    .strong(),
                                );
                                ui.label(format!("{} · {:.1}%", format_bytes(cell.size), pct));
                            }
                        }
                    });
                    if tm_resp.clicked() {
                        if let Some(pos) = tm_resp.interact_pointer_pos() {
                            if let Some(cell) = cells.iter().rev().find(|c| c.rect.contains(pos)) {
                                if cell.is_dir {
                                    drill_path = Some(cell.path.clone());
                                } else {
                                    reveal = Some(cell.path.clone());
                                }
                            }
                        }
                    }
                });
        }

        // ── Apply deferred actions (self is free of the borrows here) ──
        if let Some((cells, rect)) = recomputed {
            self.analytics_cells = cells;
            self.analytics_cells_rect = rect;
        }
        if cancel {
            if let Some(s) = &self.analytics_scan {
                s.progress.cancel.store(true, Relaxed);
            }
            self.analytics_scan = None;
        }
        if let Some((be, root, label)) = rescan_remote {
            self.start_analytics_scan_remote(be, root, label);
        } else if pick_folder {
            let init = self.analytics_root_path.clone();
            self.open_picker(PickerPurpose::AnalyticsFolder, &init);
        } else if let Some(r) = rescan {
            self.start_analytics_scan(r);
        } else if let Some(p) = drill_path {
            self.analytics_focus = self.analytics_path_to_focus(&p);
            self.analytics_invalidate();
        } else if let Some(len) = set_focus {
            self.analytics_focus.truncate(len);
            self.analytics_invalidate();
        } else if go_up {
            self.analytics_focus.pop();
            self.analytics_invalidate();
        }
        if !open {
            if let Some(s) = &self.analytics_scan {
                s.progress.cancel.store(true, Relaxed);
            }
            self.show_analytics = false;
        }
        if let Some(p) = nav {
            self.start_scan(PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)));
        } else if let Some(p) = reveal {
            // Navigate the main explorer to the file's parent, then close.
            if let Some((parent, _)) = p.rsplit_once('/') {
                self.start_scan(PathBuf::from(
                    parent.replace('/', std::path::MAIN_SEPARATOR_STR),
                ));
            }
            self.show_analytics = false;
        }
    }
}
