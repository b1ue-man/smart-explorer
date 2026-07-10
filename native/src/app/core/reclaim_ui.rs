use super::prelude::*;
use super::*;
use crate::app::reclaim_results_ui::{
    result_count_label, select_items, selected_bytes, ui_empty, ui_item, ui_items, ui_section,
};

impl App {
    pub(in crate::app) fn ui_reclaim(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering::Relaxed;
        self.poll_reclaim_scan();
        let drives = self.drive_info.clone();
        let source = self.reclaim_source.clone();
        let current_remote = self.remote.as_ref().map(|remote| {
            StorageScanSource::remote(
                remote.backend.clone(),
                self.root_path.clone(),
                remote.label.clone(),
            )
        });
        let scan_info = self.reclaim_scan.as_ref().map(|s| {
            (
                s.progress.files.load(Relaxed),
                s.progress.dirs.load(Relaxed),
                s.progress.bytes.load(Relaxed),
                s.progress.fingerprinted.load(Relaxed),
                s.progress.hashed.load(Relaxed),
                s.root.clone(),
                s.started.elapsed().as_secs_f32(),
            )
        });
        let report = self.reclaim_report.as_ref();
        let mut selected = self.reclaim_selected.clone();
        let mut panel = self.analytics_panel;
        let mut open = true;
        let mut rescan: Option<StorageScanSource> = None;
        let mut pick_folder = false;
        let mut cancel = false;
        let mut reveal: Option<String> = None;
        let mut select_dupes = false;
        let mut clear_selection = false;
        let mut trash_selected = false;
        let mut large_gb = self.reclaim_large_min_gb;
        let mut stale_days = self.reclaim_stale_days;
        let report_is_remote = report.is_some_and(|r| r.is_remote);
        let is_remote =
            report_is_remote || source.as_ref().is_some_and(StorageScanSource::is_remote);
        let run_state = self.reclaim_state;
        let issue_count = self.reclaim_issues.len() as u64 + self.reclaim_suppressed_issues;
        let first_issue = self.reclaim_issues.first().cloned();
        let issue_text = if issue_count > 0 {
            let mut lines = self.reclaim_issues.clone();
            if self.reclaim_suppressed_issues > 0 {
                lines.push(format!(
                    "… {} weitere Probleme unterdrückt",
                    self.reclaim_suppressed_issues
                ));
            }
            lines.join("\n")
        } else {
            String::new()
        };

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
                        if ui.button(root.as_str()).clicked() {
                            rescan = Some(StorageScanSource::local(root.clone()));
                        }
                    }
                    if ui.button("Ordner...").clicked() {
                        pick_folder = true;
                    }
                    if let Some(remote_source) = &current_remote {
                        if ui.button("Aktueller Remote-Ordner").clicked() {
                            rescan = Some(remote_source.clone());
                        }
                    }
                    if ui
                        .add_enabled(source.is_some(), egui::Button::new("Neu scannen"))
                        .clicked()
                    {
                        rescan = source.clone();
                    }
                    ui.separator();
                    ui.label("Groß ab");
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

                if let Some(report) = report {
                    let large_bytes =
                        (large_gb.max(0.01) * 1024.0 * 1024.0 * 1024.0) as u64;
                    if report.large_min_bytes != large_bytes || report.stale_days != stale_days {
                        ui.colored_label(
                            Color32::from_rgb(255, 190, 90),
                            "Einstellungen geändert — für passende Ergebnisse neu scannen.",
                        );
                    }
                }

                if is_remote {
                    ui.colored_label(
                        Color32::from_rgb(255, 190, 90),
                        "Remote-Reclaim ist schreibgeschützt: Hash-Ergebnisse dienen nur der Prüfung.",
                    );
                }

                if issue_count > 0 {
                    egui::CollapsingHeader::new(format!("Probleme ({issue_count})"))
                        .default_open(run_state == StorageRunState::Failed)
                        .show(ui, |ui| {
                            let mut text = issue_text.clone();
                            let rows = text.lines().count().clamp(2, 10);
                            ui.add(
                                egui::TextEdit::multiline(&mut text)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(rows),
                            );
                            if ui.button("Probleme kopieren").clicked() {
                                ctx.copy_text(issue_text.clone());
                            }
                        });
                }

                if let Some((files, dirs, bytes, fingerprinted, hashed, root, secs)) = &scan_info
                {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let rate = if *secs > 0.0 {
                            *files as f32 / *secs
                        } else {
                            0.0
                        };
                        ui.label(format!(
                            "Scanne {} - {} Dateien - {} Ordner - {} - {} Fingerprints - {} Hashes ({:.0}/s)",
                            root,
                            files,
                            dirs,
                            format_bytes(*bytes),
                            fingerprinted,
                            hashed,
                            rate
                        ));
                        if ui.button("Abbrechen").clicked() {
                            cancel = true;
                        }
                    });
                    ctx.request_repaint_after(std::time::Duration::from_millis(150));
                } else if let Some(r) = report {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&r.root).strong());
                        ui.label(format!(
                            "- {} Dateien - {} Ordner - {} gescannt",
                            r.files,
                            r.dirs,
                            format_bytes(r.bytes)
                        ));
                        ui.label(format!(
                            "- {} möglich",
                            format_bytes(r.reclaimable_bytes())
                        ));
                    });
                    if r.has_truncated_results() {
                        ui.colored_label(
                            Color32::from_rgb(255, 190, 90),
                            "Ergebnislisten sind begrenzt; Gesamtzahlen stehen in den Überschriften.",
                        );
                    }
                    if r.duplicate_candidates_truncated() {
                        ui.colored_label(
                            Color32::from_rgb(255, 190, 90),
                            format!(
                                "Duplikatprüfung: {} der {} größten geeigneten Kandidaten zurückbehalten.",
                                r.duplicate_candidates_retained, r.duplicate_candidates
                            ),
                        );
                    }
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
                                !selected.is_empty() && !r.is_remote,
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
                    if !r.errors.is_empty() || r.suppressed_errors > 0 {
                        ui.colored_label(
                            Color32::from_rgb(255, 160, 120),
                            format!(
                                "{} Pfade konnten nicht gelesen werden",
                                r.errors.len() as u64 + r.suppressed_errors
                            ),
                        );
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("reclaim_results")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let duplicate_title = result_count_label(
                                "Duplikate",
                                r.duplicate_groups.len(),
                                r.result_counts.duplicate_groups,
                            );
                            ui_section(ui, &duplicate_title, |ui| {
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
                                        let hash_short =
                                            &group.hash.hex[..group.hash.hex.len().min(8)];
                                        ui.label(
                                            RichText::new(format!(
                                                "{} {} · {}",
                                                group.hash.algorithm.label(),
                                                hash_short,
                                                group.evidence.label()
                                            ))
                                            .monospace(),
                                        );
                                    });
                                    for (idx, item) in group.items.iter().enumerate() {
                                        ui_item(ui, item, &mut selected, &mut reveal, idx == 0);
                                    }
                                }
                            });
                            ui_items(
                                ui,
                                "Große Dateien",
                                &r.large_files,
                                r.result_counts.large_files,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(
                                ui,
                                "Alte Dateien",
                                &r.stale_files,
                                r.result_counts.stale_files,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(
                                ui,
                                "Leere Dateien",
                                &r.empty_files,
                                r.result_counts.empty_files,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(
                                ui,
                                "Leere Ordner",
                                &r.empty_dirs,
                                r.result_counts.empty_dirs,
                                &mut selected,
                                &mut reveal,
                            );
                            ui_items(
                                ui,
                                "Bereinigungsziele",
                                &r.cleanup,
                                r.result_counts.cleanup,
                                &mut selected,
                                &mut reveal,
                            );
                        });
                } else {
                    match run_state {
                        StorageRunState::Idle => {
                            ui.colored_label(
                                Color32::from_gray(150),
                                "Wählen Sie eine Quelle. Es startet kein Scan automatisch.",
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
                            ui.colored_label(
                                Color32::from_rgb(255, 120, 100),
                                format!(
                                    "Scan fehlgeschlagen: {}",
                                    first_issue
                                        .clone()
                                        .unwrap_or_else(|| "Unbekannter Fehler".to_string())
                                ),
                            );
                        }
                        StorageRunState::Running | StorageRunState::Complete => {}
                    }
                }
            });

        if panel != self.analytics_panel {
            match panel {
                AnalyticsPanel::Treemap => self.cancel_reclaim_scan(),
                AnalyticsPanel::Reclaim => self.cancel_analytics_scan(),
            }
        }
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
            let init = source
                .as_ref()
                .map(StorageScanSource::root)
                .map(str::to_string)
                .unwrap_or_else(|| self.analytics_default_root());
            self.open_picker(PickerPurpose::ReclaimFolder, &init);
        } else if let Some(scan_source) = rescan {
            self.start_reclaim_source(scan_source);
        }
        if let (Some(path), Some(scan_source)) = (reveal, source.as_ref()) {
            let parent = path
                .rsplit_once('/')
                .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                .unwrap_or(scan_source.root());
            self.navigate_storage_source(scan_source, parent);
            self.show_analytics = false;
        }
        if !open {
            self.cancel_reclaim_scan();
            self.cancel_analytics_scan();
            self.show_analytics = false;
        }
    }
}
