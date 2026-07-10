use super::prelude::*;
use super::*;

impl App {
    /// The compare-result window: per-file differences, grouped by direction.
    pub(in crate::app) fn ui_preview(&mut self, ctx: &egui::Context) {
        let mut open = self.show_preview;
        // Set when the user clicks a row's "▶" to sync just that one file.
        let mut sync_one: Option<crate::bisync::Action> = None;
        egui::Window::new("🔍 Vergleich (Vorschau)")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([680.0, 460.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(&self.preview_title)
                        .small()
                        .color(Color32::from_gray(170)),
                );
                ui.separator();
                if self.preview_running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Vergleiche beide Seiten…");
                        if ui.button("⏹ Stop").clicked() {
                            if let Some(c) = &self.preview_cancel {
                                c.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    });
                    return;
                }
                let p = match &self.preview {
                    Some(p) => p,
                    None => {
                        ui.label("—");
                        return;
                    }
                };
                if let Some(e) = &p.error {
                    ui.colored_label(Color32::from_rgb(230, 120, 120), format!("Fehler: {}", e));
                    return;
                }
                let mut to_b = 0usize;
                let mut to_a = 0usize;
                let mut del = 0usize;
                for act in &p.actions {
                    match act {
                        crate::bisync::Action::CopyAtoB(_)
                        | crate::bisync::Action::KeepBothAtoB(_) => to_b += 1,
                        crate::bisync::Action::CopyBtoA(_)
                        | crate::bisync::Action::KeepBothBtoA(_) => to_a += 1,
                        crate::bisync::Action::DeleteA(_)
                        | crate::bisync::Action::DeleteB(_)
                        | crate::bisync::Action::FinalizeMoveAtoB(_)
                        | crate::bisync::Action::FinalizeMoveBtoA(_) => del += 1,
                    }
                }
                ui.label(format!(
                    "Quelle: {} Dateien · Ziel: {} Dateien",
                    p.a_files, p.b_files
                ));
                ui.label(
                    RichText::new(format!(
                        "{}→ zum Ziel · {}← zur Quelle · {} zu löschen · {} Konflikte",
                        to_b,
                        to_a,
                        del,
                        p.conflicts.len()
                    ))
                    .strong(),
                );
                if p.actions.is_empty() && p.conflicts.is_empty() {
                    ui.add_space(6.0);
                    ui.colored_label(
                        Color32::from_rgb(120, 200, 120),
                        "✓ Beide Seiten sind im Einklang — nichts zu tun.",
                    );
                    return;
                }
                ui.label(
                    RichText::new("▶ neben einer Zeile synchronisiert nur diese eine Datei.")
                        .small()
                        .color(Color32::from_gray(130)),
                );
                ui.separator();
                let busy = self.apply_one_rx.is_some();
                let conflict_rows = p.conflicts.len();
                let total_rows = conflict_rows.saturating_add(p.actions.len());
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, 24.0, total_rows, |ui, visible_rows| {
                        for row in visible_rows {
                            if row < conflict_rows {
                                let conflict = &p.conflicts[row];
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("⚠ Konflikt: {}", conflict.rel))
                                            .color(Color32::from_rgb(230, 200, 90)),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(conflict.rel.as_str());
                                continue;
                            }
                            let act = &p.actions[row - conflict_rows];
                            let (sym, color, rel) = match act {
                                crate::bisync::Action::CopyAtoB(r) => {
                                    ("→", Color32::from_rgb(120, 200, 120), r)
                                }
                                crate::bisync::Action::CopyBtoA(r) => {
                                    ("←", Color32::from_rgb(120, 200, 120), r)
                                }
                                crate::bisync::Action::DeleteB(r) => {
                                    ("🗑→", Color32::from_rgb(230, 150, 120), r)
                                }
                                crate::bisync::Action::DeleteA(r) => {
                                    ("🗑←", Color32::from_rgb(230, 150, 120), r)
                                }
                                crate::bisync::Action::FinalizeMoveAtoB(r) => {
                                    ("✓🗑→", Color32::from_rgb(230, 170, 100), r)
                                }
                                crate::bisync::Action::FinalizeMoveBtoA(r) => {
                                    ("✓🗑←", Color32::from_rgb(230, 170, 100), r)
                                }
                                crate::bisync::Action::KeepBothAtoB(r) => {
                                    ("⇄→", Color32::from_rgb(230, 200, 90), r)
                                }
                                crate::bisync::Action::KeepBothBtoA(r) => {
                                    ("⇄←", Color32::from_rgb(230, 200, 90), r)
                                }
                            };
                            ui.horizontal(|ui| {
                                if !busy
                                    && ui
                                        .small_button("▶")
                                        .on_hover_text("Nur diese Datei jetzt synchronisieren")
                                        .clicked()
                                {
                                    sync_one = Some(act.clone());
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("{}  {}", sym, rel)).color(color),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(rel.as_str());
                            });
                        }
                    });
            });
        self.show_preview = open;
        if let Some(act) = sync_one {
            if let Some(job_id) = self.preview_job_id.clone() {
                self.apply_one_action(job_id, act);
            }
        }
    }

    pub(in crate::app) fn drain_bisync(&mut self) {
        self.drain_conflict_resolution();
        let out = match self.bisync_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(out)) => out,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.bisync_rx = None;
                self.bisync_running = false;
                self.bisync_cancel = None;
                self.running_job = None;
                self.error_msg = Some("2-Wege-Sync wurde unerwartet abgebrochen".into());
                return;
            }
        };
        self.bisync_rx = None;
        self.bisync_running = false;
        self.bisync_cancel = None;
        // Stamp the saved job (if this run came from one) so its schedule and
        // "last run" reflect reality, then refresh the cached list.
        let mut persistence_errors = Vec::new();
        if let Some(id) = self.running_job.take() {
            if let Err(error) = crate::syncjobs::mark_run(&id) {
                persistence_errors.push(format!("Letzten Lauf speichern: {error}"));
            }
            let note = if out.errors.iter().any(|(k, _)| k == "abgebrochen") {
                "abgebrochen"
            } else if !out.errors.is_empty() {
                "Fehler"
            } else if !out.conflicts.is_empty() {
                "Konflikte"
            } else {
                "ok"
            };
            if let Err(error) = crate::syncjobs::record_result(
                &id,
                &crate::syncjobs::JobResult {
                    when: now_secs_i64(),
                    a_to_b: out.stats.a_to_b,
                    b_to_a: out.stats.b_to_a,
                    deleted: out.stats.deleted,
                    conflicts: out.conflicts.len() as u64,
                    errors: out.errors.len() as u64,
                    note: note.into(),
                },
            ) {
                persistence_errors.push(format!("Laufergebnis speichern: {error}"));
            }
            match crate::syncjobs::load() {
                Ok(jobs) => self.sync_jobs = jobs,
                Err(error) => {
                    persistence_errors.push(format!("Sync-Jobs neu laden: {error}"));
                }
            }
        }
        if let Some(ctx) = self.bisync_ctx.as_mut() {
            ctx.baseline = out.baseline;
        }
        self.conflict_bulk = None;
        self.conflict_baseline_dirty = false;
        self.bisync_conflicts = out.conflicts;
        let s = out.stats;
        let summary = format!(
            "⇄ Sync: {} →, {} ←, {} gelöscht, {} Konflikte ({} MB)",
            s.a_to_b,
            s.b_to_a,
            s.deleted,
            self.bisync_conflicts.len(),
            s.bytes / 1_048_576
        );
        if !out.errors.is_empty() {
            let persistence = if persistence_errors.is_empty() {
                String::new()
            } else {
                format!("; {}", persistence_errors.join("; "))
            };
            self.error_msg = Some(format!(
                "{summary}; {} Fehler{persistence}",
                out.errors.len()
            ));
        } else if !persistence_errors.is_empty() {
            self.error_msg = Some(persistence_errors.join("; "));
        } else {
            self.notice = Some((
                if self.bisync_conflicts.is_empty() {
                    summary
                } else {
                    format!("⚠ {summary} — Lösung erforderlich")
                },
                std::time::Instant::now(),
            ));
        }
        if !self.bisync_conflicts.is_empty() {
            self.show_bisync_conflicts = true;
        }
        // The current view may have changed on disk.
        if !self.root_path.is_empty() {
            self.rescan();
        }
    }
}
