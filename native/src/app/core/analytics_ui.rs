use super::analytics_controls_ui::AnalyticsControls;
use super::prelude::*;
use super::*;
use crate::app::analytics_accessibility::treemap_accessible_list;
use crate::app::theme;

fn host_figures_ui(
    ui: &mut egui::Ui,
    tree: &crate::analytics::SizeNode,
    focus: &[String],
    figures: &crate::analytics::PlatformFigures,
    complete: bool,
) {
    use crate::analytics::{node_view, Approximations, NodeKind};
    let approx = Approximations::compute(tree, &figures.place(), figures.totals(), complete);
    let Some(view) = node_view(tree, focus, &approx, 0) else {
        return;
    };
    for row in view.children {
        if row.kind == NodeKind::Apps {
            ui.collapsing(
                format!(
                    "{}: {} · Angaben der Gegenstelle",
                    row.name,
                    format_bytes(row.size)
                ),
                |ui| {
                    if let Some(apps) = node_view(tree, &[row.name], &approx, 256) {
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .show(ui, |ui| {
                                for app in apps.children {
                                    let response = ui.label(format!(
                                        "{}: {}",
                                        app.name,
                                        format_bytes(app.size)
                                    ));
                                    if let Some(usage) = app.app {
                                        response.on_hover_text(format!(
                                            "{}\nApp: {} · Daten: {} · davon Cache: {}",
                                            usage.package,
                                            format_bytes(usage.app_bytes),
                                            format_bytes(usage.data_bytes),
                                            format_bytes(usage.cache_bytes)
                                        ));
                                    }
                                }
                            });
                    }
                },
            );
        } else if matches!(row.kind, NodeKind::Protected | NodeKind::Rest) {
            ui.label(format!(
                "{}: {} · Angaben der Gegenstelle",
                row.name,
                format_bytes(row.size)
            ));
        }
    }
}

impl App {
    /// Storage-analytics overlay: a dedicated low-memory size scan rendered as a
    /// nested (WizTree-style) squarified treemap. Defaults to the whole drive of
    /// the current folder; click a box to drill in, use the breadcrumb to go up.
    pub(in crate::app) fn ui_analytics(&mut self, ctx: &egui::Context) {
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
        let root_label = source
            .as_ref()
            .map(StorageScanSource::display)
            .unwrap_or_else(|| "—".to_string());
        let focus_segs = self.analytics_focus.clone();
        let focus_path = self.analytics_focus_path();
        let focus_size = self.analytics_focus_node().map(|n| n.size).unwrap_or(0);
        let focus_node = self.analytics_focus_node();
        let cached_cells = &self.analytics_cells;
        let cached_rect = self.analytics_cells_rect;
        let mut controls = AnalyticsControls::new(self.analytics_panel);

        let mut open = true;
        let mut reveal: Option<String> = None; // reveal file in main explorer
        let mut drill_path: Option<String> = None; // treemap click → drill into folder
        let mut request_access = false;
        let mut recomputed: Option<(Vec<TmCell>, egui::Rect)> = None;

        {
            egui::Window::new("📊 Speicher-Analyse")
                .id(egui::Id::new("analyse_treemap_v2"))
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_size([880.0, 600.0])
                .max_size(theme::window_content_limit(ctx))
                .constrain_to(ctx.screen_rect().shrink(16.0))
                .min_width(440.0)
                .constrain(true)
                .show(ctx, |ui| {
                    self.ui_analytics_controls(ui, &mut controls);
                    analytics_access::issues_ui(
                        ui,
                        &self.analytics_issues,
                        self.analytics_suppressed_issues,
                        self.analytics_access.permission_denied,
                        &self.analytics_notes,
                    );
                    if let Some(StorageScanSource::Remote {
                        host_platform: Some(figures),
                        ..
                    }) = &source
                    {
                        if let Some(tree) = self.analytics_tree.as_ref() {
                            host_figures_ui(
                                ui,
                                tree,
                                &focus_segs,
                                figures,
                                self.analytics_state == StorageRunState::Complete,
                            );
                        }
                    }
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
                    let need = treemap_needs_layout(
                        focus_node.is_some(),
                        cached_cells.is_empty(),
                        cached_rect,
                        tm_rect,
                    );
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

                    analytics_paint::paint(ui, tm_rect, cells);

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
        if controls.cancel {
            self.cancel_analytics_scan();
        }
        if let Some(scan_source) = controls.rescan_source {
            self.start_analytics_source(scan_source);
        } else if controls.pick_folder {
            let init = root_path.clone();
            self.open_picker(PickerPurpose::AnalyticsFolder, &init);
        } else if let Some(p) = drill_path {
            self.analytics_focus = self.analytics_path_to_focus(&p);
            self.analytics_invalidate();
        } else if let Some(len) = controls.set_focus {
            self.analytics_focus.truncate(len);
            self.analytics_invalidate();
        } else if controls.go_up {
            self.analytics_focus.pop();
            self.analytics_invalidate();
        }
        if !open {
            self.cancel_analytics_scan();
            self.cancel_reclaim_scan();
            self.show_analytics = false;
        }
        if controls.panel != self.analytics_panel {
            match controls.panel {
                AnalyticsPanel::Treemap => self.cancel_reclaim_scan(),
                AnalyticsPanel::Reclaim => self.cancel_analytics_scan(),
            }
        }
        self.analytics_panel = controls.panel;
        if let (Some(p), Some(scan_source)) = (controls.nav, source.as_ref()) {
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
