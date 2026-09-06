use super::prelude::*;
use super::*;
use crate::app::analytics_accessibility::treemap_accessible_list;

impl App {
    /// Storage-analytics overlay: a dedicated low-memory size scan rendered as a
    /// nested (WizTree-style) squarified treemap. Defaults to the whole drive of
    /// the current folder; click a box to drill in, use the breadcrumb to go up.
    pub(in crate::app) fn ui_analytics(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering::Relaxed;
        self.poll_analytics_scan();
        if self.analytics_panel == AnalyticsPanel::Reclaim {
            self.ui_reclaim(ctx);
            return;
        }
        if self.analytics_counts.is_none() {
            if let Some(node) = self.analytics_focus_node() {
                self.analytics_counts = Some(count_subtree(node));
            }
        }

        let source = self.analytics_source.clone();
        let root_path = source
            .as_ref()
            .map(StorageScanSource::root)
            .unwrap_or("")
            .to_string();
        let drive = self.drive_usage(&root_path);
        let drives = self.drive_info.clone();
        let root_label = source
            .as_ref()
            .map(StorageScanSource::display)
            .unwrap_or_else(|| "—".to_string());
        // Current remote (for the "scan this remote folder" button) + the source
        // the current tree came from (for ⟳ to re-walk the same place).
        let remote_scan: Option<(crate::vfs::BackendHandle, String, String)> = self
            .remote
            .as_ref()
            .map(|rs| (rs.backend.clone(), self.root_path.clone(), rs.label.clone()));
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
        let run_state = self.analytics_state;
        let issue_count = self.analytics_issues.len() as u64 + self.analytics_suppressed_issues;
        let first_issue = self.analytics_issues.first().cloned();

        let focus_node = self.analytics_focus_node();
        let cached_cells = &self.analytics_cells;
        let cached_rect = self.analytics_cells_rect;
        let mut panel = self.analytics_panel;

        let mut open = true;
        let mut nav: Option<String> = None; // open folder in main explorer
        let mut reveal: Option<String> = None; // reveal file in main explorer
        let mut drill_path: Option<String> = None; // treemap click → drill into folder
        let mut set_focus: Option<usize> = None; // breadcrumb truncate
        let mut go_up = false;
        let mut rescan_source: Option<StorageScanSource> = None;
        let mut pick_folder = false;
        let mut cancel = false;
        let mut request_access = false;
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
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut panel, AnalyticsPanel::Treemap, "Treemap");
                        ui.selectable_value(&mut panel, AnalyticsPanel::Reclaim, "Find & Reclaim");
                    });
                    ui.separator();
                    // ── Row 1: scan targets ──
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Scannen:")
                                .small()
                                .color(Color32::from_gray(150)),
                        );
                        for (root, free, total) in &drives {
                            let used = total.saturating_sub(*free);
                            let label = if *total > 0 {
                                format!(
                                    "{} ({}/{})",
                                    root,
                                    format_bytes(used),
                                    format_bytes(*total)
                                )
                            } else {
                                root.clone()
                            };
                            if ui.button(label).clicked() {
                                rescan_source = Some(StorageScanSource::local(root.clone()));
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
                                rescan_source = Some(StorageScanSource::remote(
                                    be.clone(),
                                    root.clone(),
                                    label.clone(),
                                ));
                            }
                        }
                        if ui
                            .add_enabled(source.is_some(), egui::Button::new("⟳"))
                            .on_hover_text("Dieselbe Quelle neu scannen")
                            .clicked()
                        {
                            rescan_source = source.clone();
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
                            if ui
                                .add_enabled(
                                    source.is_some() && focus_node.is_some(),
                                    egui::Button::new("📂 Im Explorer öffnen"),
                                )
                                .clicked()
                            {
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
                    match run_state {
                        StorageRunState::Idle => {
                            ui.colored_label(
                                Color32::from_gray(150),
                                "Waehlen Sie ein Laufwerk, einen Ordner oder die aktuelle Remote-Verbindung.",
                            );
                        }
                        StorageRunState::Canceled => {
                            ui.colored_label(
                                Color32::from_rgb(255, 190, 90),
                                "Scan abgebrochen. Ein neuer Scan startet nur nach Ihrer Auswahl.",
                            );
                        }
                        StorageRunState::Partial => {
                            ui.colored_label(
                                Color32::from_rgb(255, 190, 90),
                                format!("Teilresultat: {issue_count} Pfad(e) konnten nicht gelesen werden."),
                            );
                        }
                        StorageRunState::Failed => {
                            let detail = first_issue
                                .as_ref()
                                .map(|issue| format!("{}: {}", issue.path, issue.detail))
                                .unwrap_or_else(|| "Unbekannter Scan-Fehler".to_string());
                            ui.colored_label(
                                Color32::from_rgb(255, 120, 100),
                                format!("Scan fehlgeschlagen: {detail}"),
                            );
                        }
                        StorageRunState::Running | StorageRunState::Complete => {}
                    }
                    analytics_access::issues_ui(ui, &self.analytics_issues,
                        self.analytics_suppressed_issues, self.analytics_access.permission_denied);
                    request_access = analytics_access::access_ui(ui, &self.analytics_access);
                    ui.separator();

                    treemap_accessible_list(
                        ui,
                        focus_node,
                        &focus_path,
                        &mut drill_path,
                        &mut reveal,
                    );

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
                        &recomputed.insert((v, tm_rect)).0
                    } else {
                        cached_cells
                    };
                    tm_resp.widget_info(|| {
                        treemap_widget_info(
                            &root_label,
                            &focus_path,
                            focus_size,
                            cells.len(),
                            focus_node.is_some(),
                        )
                    });

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
                                egui::Stroke::new(1.0_f32, Color32::from_black_alpha(130)),
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
                                egui::Stroke::new(0.5_f32, Color32::from_black_alpha(70)),
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
        if request_access {
            self.request_analytics_access();
        }
        if let Some((cells, rect)) = recomputed {
            self.analytics_cells = cells;
            self.analytics_cells_rect = rect;
        }
        if cancel {
            self.cancel_analytics_scan();
        }
        if let Some(scan_source) = rescan_source {
            self.start_analytics_source(scan_source);
        } else if pick_folder {
            let init = root_path.clone();
            self.open_picker(PickerPurpose::AnalyticsFolder, &init);
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
            self.cancel_analytics_scan();
            self.cancel_reclaim_scan();
            self.show_analytics = false;
        }
        if panel != self.analytics_panel {
            match panel {
                AnalyticsPanel::Treemap => self.cancel_reclaim_scan(),
                AnalyticsPanel::Reclaim => self.cancel_analytics_scan(),
            }
        }
        self.analytics_panel = panel;
        if let (Some(p), Some(scan_source)) = (nav, source.as_ref()) {
            self.navigate_storage_source(scan_source, &p);
        } else if let (Some(p), Some(scan_source)) = (reveal, source.as_ref()) {
            // Navigate the main explorer to the file's parent, then close.
            if let Some((parent, _)) = p.rsplit_once('/') {
                let parent = if parent.is_empty() { "/" } else { parent };
                self.navigate_storage_source(scan_source, parent);
            }
            self.show_analytics = false;
        }
    }
}

fn treemap_widget_info(
    root_label: &str,
    focus_path: &str,
    focus_size: u64,
    cell_count: usize,
    has_data: bool,
) -> egui::WidgetInfo {
    let location = if focus_path.is_empty() {
        root_label
    } else {
        focus_path
    };
    let label = if has_data {
        format!(
            "Treemap für {location}. {cell_count} sichtbare Elemente, insgesamt {}. Ordner oder Datei anklicken, um sie zu öffnen",
            format_bytes(focus_size)
        )
    } else {
        "Treemap. Noch keine Scan-Daten vorhanden".to_string()
    };
    let mut info = egui::WidgetInfo::labeled(egui::WidgetType::Button, has_data, label);
    info.value = has_data.then_some(focus_size as f64);
    info
}

#[cfg(test)]
mod accessibility_tests {
    use super::*;

    #[test]
    fn treemap_semantics_include_location_count_and_size_state() {
        let info = treemap_widget_info("C:/", "C:/Users", 1024, 7, true);
        let label = info.label.as_deref().unwrap_or_default();
        assert!(label.contains("C:/Users"));
        assert!(label.contains("7 sichtbare Elemente"));
        assert_eq!(info.typ, egui::WidgetType::Button);
        assert_eq!(info.value, Some(1024.0));
        assert!(info.enabled);
    }

    #[test]
    fn empty_treemap_is_disabled_and_named() {
        let info = treemap_widget_info("—", "", 0, 0, false);
        assert!(!info.enabled);
        assert!(info.label.is_some_and(|label| !label.is_empty()));
    }
}
