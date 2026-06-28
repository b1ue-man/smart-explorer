use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_reclaim(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering::Relaxed;
        self.poll_reclaim_scan();
        if self.reclaim_report.is_none() && self.reclaim_scan.is_none() && self.remote.is_none() {
            let root = self.analytics_default_root();
            if !root.is_empty() {
                self.start_reclaim_scan(root);
            }
        }

        let drives = self.drive_info.clone();
        let is_remote = self.remote.is_some();
        let scan_info = self.reclaim_scan.as_ref().map(|s| {
            (
                s.progress.files.load(Relaxed),
                s.progress.dirs.load(Relaxed),
                s.progress.bytes.load(Relaxed),
                s.progress.hashed.load(Relaxed),
                s.root.clone(),
                s.started.elapsed().as_secs_f32(),
            )
        });
        let report = self.reclaim_report.clone();
        let mut selected = self.reclaim_selected.clone();
        let mut panel = self.analytics_panel;
        let mut open = true;
        let mut rescan: Option<String> = None;
        let mut pick_folder = false;
        let mut cancel = false;
        let mut reveal: Option<String> = None;
        let mut select_dupes = false;
        let mut clear_selection = false;
        let mut trash_selected = false;
        let mut large_gb = self.reclaim_large_min_gb;
        let mut stale_days = self.reclaim_stale_days;

        egui::Window::new("📊 Speicher-Analyse")
            .id(egui::Id::new("analyse_reclaim"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([920.0, 640.0])
            .min_width(500.0)
            .constrain(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut panel, AnalyticsPanel::Treemap, "Treemap");
                    ui.selectable_value(&mut panel, AnalyticsPanel::Reclaim, "Find & Reclaim");
                });
                ui.separator();

                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Scannen:")
                            .small()
                            .color(Color32::from_gray(150)),
                    );
                    for (root, _free, _total) in &drives {
                        let dl: String = root.chars().take(2).collect();
                        if ui.button(dl.clone()).clicked() {
                            rescan = Some(format!("{}/", dl));
                        }
                    }
                    if ui.button("Ordner...").clicked() {
                        pick_folder = true;
                    }
                    if ui.button("Neu scannen").clicked() {
                        if let Some(r) = report.as_ref().map(|r| r.root.clone()) {
                            rescan = Some(r);
                        }
                    }
                    ui.separator();
                    ui.label("Gross ab");
                    ui.add(
                        egui::DragValue::new(&mut large_gb)
                            .speed(0.25)
                            .range(0.01..=1024.0)
                            .suffix(" GB"),
                    );
                    ui.label("Alt ab");
                    ui.add(
                        egui::DragValue::new(&mut stale_days)
                            .speed(7.0)
                            .range(1..=3650)
                            .suffix(" Tage"),
                    );
                });

                if is_remote {
                    ui.colored_label(
                        Color32::from_rgb(255, 190, 90),
                        "Remote-Reclaim ist vorbereitet, Loeschen bleibt hier lokal.",
                    );
                }

                if let Some((files, dirs, bytes, hashed, root, secs)) = &scan_info {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let rate = if *secs > 0.0 {
                            *files as f32 / *secs
                        } else {
                            0.0
                        };
                        ui.label(format!(
                            "{} - {} Dateien - {} Ordner - {} - {} Hashes ({:.0}/s)",
                            root,
                            files,
                            dirs,
                            format_bytes(*bytes),
                            hashed,
                            rate
                        ));
                        if ui.button("Abbrechen").clicked() {
                            cancel = true;
                        }
                    });
                    ctx.request_repaint_after(std::time::Duration::from_millis(150));
                } else if let Some(r) = &report {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&r.root).strong());
                        ui.label(format!(
                            "- {} Dateien - {} Ordner - {} gescannt",
                            r.files,
                            r.dirs,
                            format_bytes(r.bytes)
                        ));
                        ui.label(format!(
                            "- {} moeglich",
                            format_bytes(r.reclaimable_bytes())
                        ));
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Duplikatkopien").clicked() {
                            select_dupes = true;
                        }
                        if ui.button("Cleanup").clicked() {
                            select_items(&mut selected, &r.cleanup);
                        }
                        if ui.button("Leere").clicked() {
                            select_items(&mut selected, &r.empty_files);
                            select_items(&mut selected, &r.empty_dirs);
                        }
                        if ui.button("Auswahl leeren").clicked() {
                            clear_selection = true;
                        }
                        let selected_bytes = selected_bytes(r, &selected);
                        if ui
                            .add_enabled(
                                !selected.is_empty(),
                                egui::Button::new(format!(
                                    "Papierkorb ({}, {})",
                                    selected.len(),
                                    format_bytes(selected_bytes)
                                )),
                            )
                            .clicked()
                        {
                            trash_selected = true;
                        }
                    });
                    if !r.errors.is_empty() {
                        ui.colored_label(
                            Color32::from_rgb(255, 160, 120),
                            format!("{} Pfade konnten nicht gelesen werden", r.errors.len()),
                        );
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("reclaim_results")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui_section(ui, "Duplikate", |ui| {
                                if r.duplicate_groups.is_empty() {
                                    ui_empty(ui);
                                }
                                for group in &r.duplicate_groups {
                                    ui.separator();
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(format_bytes(group.size)).strong());
                                        ui.label(format!(
                                            "- {} Kopien - {} frei",
                                            group.items.len(),
                                            format_bytes(group.reclaimable)
                                        ));
                                        ui.label(RichText::new(&group.md5[..8]).monospace());
                                    });
                                    for (idx, item) in group.items.iter().enumerate() {
                                        ui_item(ui, item, &mut selected, &mut reveal, idx == 0);
                                    }
                                }
                            });
                            ui_items(
                                ui,
                                "Grosse Dateien",
                                &r.large_files,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(
                                ui,
                                "Alte Dateien",
                                &r.stale_files,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(
                                ui,
                                "Leere Dateien",
                                &r.empty_files,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(
                                ui,
                                "Leere Ordner",
                                &r.empty_dirs,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(ui, "Cleanup-Ziele", &r.cleanup, &mut selected, &mut reveal);
                        });
                } else {
                    ui.colored_label(Color32::from_gray(140), "Kein lokaler Reclaim-Scan aktiv.");
                }
            });

        self.analytics_panel = panel;
        self.reclaim_large_min_gb = large_gb;
        self.reclaim_stale_days = stale_days;
        self.reclaim_selected = selected;
        if select_dupes {
            self.select_reclaim_duplicate_copies();
        }
        if clear_selection {
            self.reclaim_selected.clear();
        }
        if trash_selected {
            self.trash_reclaim_selected();
        }
        if cancel {
            self.cancel_reclaim_scan();
        }
        if pick_folder {
            let init = report
                .as_ref()
                .map(|r| r.root.clone())
                .unwrap_or_else(|| self.analytics_default_root());
            self.open_picker(PickerPurpose::ReclaimFolder, &init);
        } else if let Some(root) = rescan {
            self.start_reclaim_scan(root);
        }
        if let Some(path) = reveal {
            self.open_in_explorer(&path);
        }
        if !open {
            self.cancel_reclaim_scan();
            self.show_analytics = false;
        }
    }
}

fn ui_section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| add(ui));
}

fn ui_items(
    ui: &mut egui::Ui,
    title: &str,
    items: &[crate::analytics::ReclaimItem],
    selected: &mut HashSet<String>,
    reveal: &mut Option<String>,
) {
    egui::CollapsingHeader::new(format!("{} ({})", title, items.len()))
        .default_open(false)
        .show(ui, |ui| {
            if items.is_empty() {
                ui_empty(ui);
            }
            for item in items {
                ui_item(ui, item, selected, reveal, false);
            }
        });
}

fn ui_item(
    ui: &mut egui::Ui,
    item: &crate::analytics::ReclaimItem,
    selected: &mut HashSet<String>,
    reveal: &mut Option<String>,
    first_duplicate: bool,
) {
    ui.horizontal(|ui| {
        let mut on = selected.contains(&item.path);
        if ui.checkbox(&mut on, "").changed() {
            if on {
                selected.insert(item.path.clone());
            } else {
                selected.remove(&item.path);
            }
        }
        ui.label(RichText::new(format_bytes(item.size)).monospace());
        if first_duplicate {
            ui.label(
                RichText::new("behalten")
                    .small()
                    .color(Color32::from_gray(140)),
            );
        }
        let date = if item.mtime_ms > 0 {
            format_date(item.mtime_ms)
        } else {
            "-".to_string()
        };
        ui.label(RichText::new(date).small().color(Color32::from_gray(150)));
        ui.add(egui::Label::new(&item.name).truncate())
            .on_hover_text(&item.path);
        if !item.reason.is_empty() {
            ui.label(
                RichText::new(&item.reason)
                    .small()
                    .color(Color32::from_gray(150)),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Anzeigen").clicked() {
                *reveal = Some(item.path.clone());
            }
        });
    });
}

fn ui_empty(ui: &mut egui::Ui) {
    ui.colored_label(Color32::from_gray(120), "(keine)");
}

fn select_items(selected: &mut HashSet<String>, items: &[crate::analytics::ReclaimItem]) {
    for item in items {
        selected.insert(item.path.clone());
    }
}

fn selected_bytes(report: &crate::analytics::ReclaimReport, selected: &HashSet<String>) -> u64 {
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for item in report
        .large_files
        .iter()
        .chain(report.stale_files.iter())
        .chain(report.empty_files.iter())
        .chain(report.empty_dirs.iter())
        .chain(report.cleanup.iter())
        .chain(report.duplicate_groups.iter().flat_map(|g| g.items.iter()))
    {
        if selected.contains(&item.path) && seen.insert(item.path.as_str()) {
            total = total.saturating_add(item.size);
        }
    }
    total
}
