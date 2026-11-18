use super::analytics_controls_ui::AnalyticsControls;
use super::prelude::*;
use super::*;
use crate::app::analytics_accessibility::treemap_accessible_list;
use crate::app::analytics_measure::MeasureView;
use crate::app::analytics_overview_ui;
use crate::app::analytics_view::{is_system_files, AnalyticsTab, TreemapCache, SYSTEM_FILES_HINT};
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
                            .min_scrolled_height(0.0)
                            .max_height(super::dialog_geometry::remaining_height(ui, 2.0))
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
        self.ensure_analytics_overview();
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
        let measure = self.analytics_view.measure;
        let focus_weight = self.analytics_focus_node().map_or(0, |n| measure.weight(n));
        let focus_node = self.analytics_focus_node();
        let cached = &self.analytics_view.treemap;
        let tab = self.analytics_view.tab;
        let overview = self.analytics_view.overview.as_ref();
        let mut tab_choice = None;
        let mut controls = AnalyticsControls::new(self.analytics_panel);

        let mut open = true;
        let mut reveal: Option<String> = None; // reveal file in main explorer
        let mut drill_path: Option<String> = None; // treemap click → drill into folder
        let mut request_access = false;
        let mut recomputed: Option<TreemapCache> = None;

        {
            super::dialog_geometry::window(ctx, "analytics_ui.0", "Speicher-Analyse")
                .id(egui::Id::new("analyse_treemap_v2"))
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .vscroll(true)
                .default_size([880.0, 600.0])
                .max_size(theme::window_content_limit(ctx))
                .constrain_to(ctx.screen_rect().shrink(16.0))
                .min_width(440.0_f32.min(theme::window_content_limit(ctx).x))
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
                    tab_choice = analytics_overview_ui::tabs_ui(ui, tab);
                    if tab != AnalyticsTab::Treemap {
                        let (base, reveal) = (&focus_path, &mut reveal);
                        analytics_overview_ui::overview_ui(
                            ui, tab, overview, measure, base, reveal,
                        );
                        return;
                    }

                    treemap_accessible_list(
                        ui,
                        focus_node,
                        &focus_path,
                        measure,
                        &mut drill_path,
                        &mut reveal,
                    );

                    // ── Nested treemap ──
                    let tm_w = ui.available_width();
                    let tm_h = super::dialog_geometry::remaining_height(ui, 0.0);
                    let (tm_rect, tm_resp) =
                        ui.allocate_exact_size(egui::vec2(tm_w, tm_h), egui::Sense::click());

                    // (Re)lay out only on resize, drill or measure change;
                    // painting reuses the cells and their mesh.
                    let need = treemap_needs_layout(
                        focus_node.is_some(),
                        cached.cells.is_empty(),
                        cached.rect,
                        tm_rect,
                    );
                    let cache: &TreemapCache = if need {
                        let mut cells = Vec::new();
                        if let Some(node) = focus_node {
                            let base = &focus_path;
                            nested_treemap(tm_rect, node, base, 0, None, measure, &mut cells);
                        }
                        recomputed.insert(TreemapCache {
                            cells,
                            rect: tm_rect,
                            ..TreemapCache::default()
                        })
                    } else {
                        cached
                    };
                    let cells = cache.cells.as_slice();
                    tm_resp.widget_info(|| {
                        treemap_widget_info(
                            &root_label,
                            &focus_path,
                            focus_node.map(|node| (node, measure)),
                            cells.len(),
                        )
                    });

                    analytics_paint::paint(ui, tm_rect, cache);

                    // Hover tooltip + click-to-drill: deepest cell under pointer.
                    let tm_resp = tm_resp.on_hover_ui(|ui| {
                        if let Some(pos) = ui.ctx().pointer_hover_pos() {
                            if let Some(cell) = cells.iter().rev().find(|c| c.rect.contains(pos)) {
                                let pct = if focus_weight > 0 {
                                    cell.weight as f64 / focus_weight as f64 * 100.0
                                } else {
                                    0.0
                                };
                                // Don't wrap the tooltip into a narrow column.
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                ui.label(
                                    RichText::new(format!(
                                        "{}{}",
                                        if cell.is_dir { "Ordner: " } else { "" },
                                        cell.name
                                    ))
                                    .strong(),
                                );
                                ui.label(format!(
                                    "{} · {:.1}% ({})",
                                    measure.both_text(cell.size, cell.allocated),
                                    pct,
                                    measure.effective().label()
                                ));
                                if is_system_files(&cell.name, cell.is_dir) {
                                    ui.label(SYSTEM_FILES_HINT);
                                }
                            }
                        }
                    });
                    if tm_resp.clicked() {
                        if let Some(pos) = tm_resp.interact_pointer_pos() {
                            if let Some(cell) = cells.iter().rev().find(|c| c.rect.contains(pos)) {
                                if cell.is_dir {
                                    drill_path = Some(cell.path.clone());
                                } else if !is_system_files(&cell.name, false) {
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
        if let Some(cache) = recomputed {
            self.analytics_view.treemap = cache;
        }
        if let Some(tab) = tab_choice {
            self.analytics_view.tab = tab;
        }
        if controls.cancel {
            self.cancel_analytics_scan();
        }
        if let Some(chosen) = controls.measure {
            self.set_analytics_measure(chosen);
        }
        if let Some(scan_source) = controls.rescan_source {
            if controls.accept_drive_identity && !scan_source.is_remote() {
                self.accept_drive_consumer(
                    super::drive_snapshot::DriveConsumer::Analytics,
                    scan_source.root(),
                );
            }
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

/// `focus`: the shown node and the measure its cells follow.
fn treemap_widget_info(
    root_label: &str,
    focus_path: &str,
    focus: Option<(&crate::analytics::SizeNode, MeasureView)>,
    cell_count: usize,
) -> egui::WidgetInfo {
    let location = if focus_path.is_empty() {
        root_label
    } else {
        focus_path
    };
    let label = match focus {
        Some((node, measure)) => format!(
            "Treemap für {location}. {cell_count} sichtbare Elemente, insgesamt {}. Kacheln nach {}. Ordner oder Datei anklicken, um sie zu öffnen",
            measure.both_text(node.size, node.allocated),
            measure.effective().label()
        ),
        None => "Treemap. Noch keine Scan-Daten vorhanden".to_string(),
    };
    let mut info = egui::WidgetInfo::labeled(egui::WidgetType::Button, focus.is_some(), label);
    info.value = focus.map(|(node, measure)| measure.weight(node) as f64);
    info
}

#[cfg(test)]
mod accessibility_tests {
    use super::*;
    use crate::app::analytics_measure::SizeMeasure;

    #[test]
    fn treemap_semantics_include_location_count_and_size_state() {
        let node = crate::analytics::SizeNode {
            name: "Users".into(),
            size: 1024,
            allocated: 512,
            is_dir: true,
            children: Vec::new(),
        };
        let measure = MeasureView {
            chosen: SizeMeasure::Allocated,
            allocated_known: true,
        };
        let info = treemap_widget_info("C:/", "C:/Users", Some((&node, measure)), 7);
        let label = info.label.as_deref().unwrap_or_default();
        assert!(label.contains("C:/Users"));
        assert!(label.contains("7 sichtbare Elemente"));
        assert!(label.contains("Kacheln nach Belegt"));
        assert_eq!(info.typ, egui::WidgetType::Button);
        assert_eq!(info.value, Some(512.0));
        assert!(info.enabled);
    }

    #[test]
    fn empty_treemap_is_disabled_and_named() {
        let info = treemap_widget_info("—", "", None, 0);
        assert!(!info.enabled);
        assert!(info.label.is_some_and(|label| !label.is_empty()));
    }
}
