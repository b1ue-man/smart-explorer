//! Scan selection, navigation and measured status of the analytics overlay.
use super::prelude::*;
use super::*;
use crate::app::theme;

/// Actions remain deferred until the treemap releases its read borrows.
pub(in crate::app) struct AnalyticsControls {
    pub(in crate::app) panel: AnalyticsPanel,
    pub(in crate::app) nav: Option<String>,
    pub(in crate::app) set_focus: Option<usize>,
    pub(in crate::app) go_up: bool,
    pub(in crate::app) rescan_source: Option<StorageScanSource>,
    pub(in crate::app) pick_folder: bool,
    pub(in crate::app) cancel: bool,
}

impl AnalyticsControls {
    pub(in crate::app) fn new(panel: AnalyticsPanel) -> Self {
        Self {
            panel,
            nav: None,
            set_focus: None,
            go_up: false,
            rescan_source: None,
            pick_folder: false,
            cancel: false,
        }
    }
}

impl App {
    pub(in crate::app) fn ui_analytics_controls(
        &self,
        ui: &mut egui::Ui,
        controls: &mut AnalyticsControls,
    ) {
        let source = self.analytics_source.clone();
        let drive = source.as_ref().and_then(|source| self.drive_usage(source));
        let drives = &self.drive_info;
        let root_label = source
            .as_ref()
            .map(StorageScanSource::display)
            .unwrap_or_else(|| "—".to_string());
        // Retain the current remote's full endpoint and backend identity.
        let remote_scan = self.remote.as_ref().map(|rs| {
            StorageScanSource::remote_at(
                rs.backend.clone(),
                self.root_path.clone(),
                rs.label.clone(),
                rs.endpoint_prefix.clone(),
                rs.account.clone(),
            )
        });
        let focus_segs = &self.analytics_focus;
        let focus_path = self.analytics_focus_path();
        let focus_size = self.analytics_focus_node().map(|n| n.size).unwrap_or(0);
        let (n_files, n_dirs) = self.analytics_counts.unwrap_or((0, 0));
        let scan_info = self.analytics_scan.as_ref().map(|s| {
            (
                s.progress.snapshot(),
                s.progress.remote_report_age(),
                s.root.clone(),
                s.started.elapsed().as_secs_f32(),
                s.progress.cancel.load(std::sync::atomic::Ordering::Relaxed),
            )
        });
        let run_state = self.analytics_state;
        let issue_count = self.analytics_issues.len() as u64 + self.analytics_suppressed_issues;
        let first_issue = self.analytics_issues.first().cloned();
        let focus_node = self.analytics_focus_node();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut controls.panel, AnalyticsPanel::Treemap, "Treemap");
            ui.selectable_value(&mut controls.panel, AnalyticsPanel::Reclaim, "Find & Reclaim");
        });
        ui.separator();
        // ── Row 1: scan targets ──
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Scannen:")
                    .small()
                    .color(theme::muted(ui)),
            );
            for (root, free, total) in drives {
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
                    controls.rescan_source = Some(StorageScanSource::local(root.clone()));
                }
            }
            if ui.button("📁 Ordner…").clicked() {
                controls.pick_folder = true;
            }
            if let Some(remote @ StorageScanSource::Remote { root, label, .. }) = &remote_scan {
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
                    controls.rescan_source = Some(remote.clone());
                }
            }
            if ui
                .add_enabled(source.is_some(), egui::Button::new("⟳"))
                .on_hover_text("Dieselbe Quelle neu scannen")
                .clicked()
            {
                controls.rescan_source = source.clone();
            }
        });

        // ── Row 2: breadcrumb ──
        ui.horizontal_wrapped(|ui| {
            if !focus_segs.is_empty()
                && ui.button("↑").on_hover_text("Eine Ebene höher").clicked()
            {
                controls.go_up = true;
            }
            if ui.button(RichText::new(&root_label).strong()).clicked() {
                controls.set_focus = Some(0);
            }
            for (i, seg) in focus_segs.iter().enumerate() {
                ui.label("›");
                if ui.button(seg).clicked() {
                    controls.set_focus = Some(i + 1);
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
                    controls.nav = Some(focus_path.clone());
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

        if let Some((state, remote_age, root, secs, cancel_pending)) = &scan_info {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                let dirs = if state.directories_unreported {
                    "nicht gemeldet".to_string()
                } else { state.dirs.to_string() };
                ui.label(format!(
                    "{} · {} Dateien · {} Ordner · {} · {:.1} s",
                    state.phase.label(), state.files, dirs, format_bytes(state.bytes), secs,
                ));
                if *cancel_pending {
                    ui.label("Abbruch angefordert · wartet auf Worker-Ende");
                } else if ui.button("Abbrechen").clicked() { controls.cancel = true; }
            });
            ui.label(if state.current.is_empty() { root } else { &state.current });
            if state.transfer_total > 0 {
                ui.add(egui::ProgressBar::new(state.transferred as f32 / state.transfer_total as f32)
                    .text(format!("Ergebnis: {} von {} empfangen",
                        format_bytes(state.transferred), format_bytes(state.transfer_total))));
            }
            if state.unchanged_ms >= 2000 {
                ui.colored_label(theme::warning(ui), format!(
                    "Seit {:.1} s keine neue Arbeit bestätigt; letzter Zustand: {}",
                    state.unchanged_ms as f64 / 1000.0, state.phase.label(),
                ));
            }
            if let Some(age) = remote_age {
                ui.label(format!("Letzte Fortschrittsmeldung vor {:.1} s", age.as_secs_f64()));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format_bytes(focus_size)).strong());
                if focus_segs.is_empty() {
                    if let Some((totals, seconds)) = &self.analytics_totals {
                        let dirs = if totals.directories_unreported {
                            "nicht gemeldet".into()
                        } else { totals.dirs.to_string() };
                        ui.label(format!("· {} erfasste Dateien · {} Ordner · {:.1} s",
                            totals.files, dirs, seconds));
                        if !matches!(run_state, StorageRunState::Complete | StorageRunState::Partial) {
                            ui.label(format!("· zuletzt {} erfasst", format_bytes(totals.bytes)));
                        }
                    }
                } else {
                    ui.label(format!("· {} Datei-Einträge · {} Ordner-Einträge dargestellt", n_files, n_dirs));
                }
                ui.label(
                    RichText::new("· Klick = reinzoomen")
                        .small()
                        .color(theme::muted(ui)),
                );
            });
        }
        match run_state {
            StorageRunState::Idle => {
                ui.colored_label(
                    theme::muted(ui),
                    "Waehlen Sie ein Laufwerk, einen Ordner oder die aktuelle Remote-Verbindung.",
                );
            }
            StorageRunState::Canceled => {
                ui.colored_label(
                    theme::warning(ui),
                    "Scan abgebrochen. Ein neuer Scan startet nur nach Ihrer Auswahl.",
                );
            }
            StorageRunState::Partial => {
                ui.colored_label(
                    theme::warning(ui),
                    format!("Teilresultat: {issue_count} Pfad(e) konnten nicht gelesen werden."),
                );
            }
            StorageRunState::Failed => {
                let detail = first_issue
                    .as_ref()
                    .map(|issue| format!("{}: {}", issue.path, issue.detail))
                    .unwrap_or_else(|| "Unbekannter Scan-Fehler".to_string());
                ui.colored_label(
                    theme::danger(ui),
                    format!("Scan fehlgeschlagen: {detail}"),
                );
            }
            StorageRunState::Running | StorageRunState::Complete => {}
        }
    }
}
