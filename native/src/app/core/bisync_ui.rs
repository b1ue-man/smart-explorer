use super::prelude::*;
use super::*;
use crate::app::theme;

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
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(&self.preview_title)
                        .small()
                        .color(theme::muted(ui)),
                );
                ui.separator();
                if self.preview_running || self.apply_one_rx.is_some() {
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
                    ui.colored_label(theme::danger(ui), format!("Fehler: {}", e));
                    return;
                }
                if let Some(block) = &p.blocked {
                    ui.colored_label(theme::warning(ui), block.message());
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
                if let Some(summary) = p.omissions.summary() {
                    ui.colored_label(theme::warning(ui), format!("⚠ {summary}"));
                    ui.collapsing("Ausgelassene Verknüpfungen (bis zu 100 Pfade)", |ui| {
                        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                            for path in p.omissions.reported_paths().take(100) {
                                ui.label(path);
                            }
                        });
                    });
                }
                if p.duplicate_removals > 0 {
                    ui.label(format!("{} doppelte Dateien werden beim Sync nach Sicherung entfernt; die gemeinsame Version bleibt erhalten.", p.duplicate_removals));
                }
                if p.actions.is_empty() && p.conflicts.is_empty() {
                    ui.add_space(6.0);
                    ui.colored_label(
                        theme::success(ui),
                        if p.omissions.is_empty() {
                            "✓ Beide Seiten sind im Einklang — nichts zu tun."
                        } else {
                            "Die berücksichtigten Dateien sind im Einklang."
                        },
                    );
                    return;
                }
                ui.label(
                    RichText::new("▶ neben einer Zeile synchronisiert nur diese eine Datei.")
                        .small()
                        .color(theme::muted(ui)),
                );
                ui.separator();
                let busy = self.apply_one_rx.is_some() || p.blocked.is_some();
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
                                            .color(theme::warning(ui)),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(conflict.rel.as_str());
                                continue;
                            }
                            let act = &p.actions[row - conflict_rows];
                            let (sym, color, rel) = match act {
                                crate::bisync::Action::CopyAtoB(r) => {
                                    ("→", theme::success(ui), r)
                                }
                                crate::bisync::Action::CopyBtoA(r) => {
                                    ("←", theme::success(ui), r)
                                }
                                crate::bisync::Action::DeleteB(r) => {
                                    ("🗑→", theme::warning(ui), r)
                                }
                                crate::bisync::Action::DeleteA(r) => {
                                    ("🗑←", theme::warning(ui), r)
                                }
                                crate::bisync::Action::FinalizeMoveAtoB(r) => {
                                    ("✓🗑→", theme::warning(ui), r)
                                }
                                crate::bisync::Action::FinalizeMoveBtoA(r) => {
                                    ("✓🗑←", theme::warning(ui), r)
                                }
                                crate::bisync::Action::KeepBothAtoB(r) => {
                                    ("⇄→", theme::warning(ui), r)
                                }
                                crate::bisync::Action::KeepBothBtoA(r) => {
                                    ("⇄←", theme::warning(ui), r)
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
                self.desktop_run = None;
                self.error_msg =
                    Some("Sync endete ohne Ergebnis; gespeicherten Laufzustand prüfen.".into());
                return;
            }
        };
        let cancelled = out.canceled
            || self
                .bisync_cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Acquire));
        self.bisync_rx = None;
        self.bisync_running = false;
        self.bisync_cancel = None;
        let mut persistence_errors = Vec::new();
        let mut pending = Vec::new();
        if let Some(run) = self.desktop_run.take() {
            let mut data = run.mailbox.lock().unwrap_or_else(|e| e.into_inner());
            self.bisync_ctx = data.context.take();
            persistence_errors = std::mem::take(&mut data.persistence_errors);
            pending = std::mem::take(&mut data.pending);
        }
        if self.running_job.take().is_some() {
            self.reload_sync_jobs("Sync-Setups neu laden");
        }
        if let Some(context) = self.bisync_ctx.as_mut() {
            context.state = out.state.clone();
            context.baseline = out.baseline;
        }
        self.conflict_bulk = None;
        self.conflict_baseline_dirty = false;
        self.bisync_conflicts = out.conflicts;
        for conflict in pending {
            if !self
                .bisync_conflicts
                .iter()
                .any(|current| current.rel == conflict.rel)
            {
                self.bisync_conflicts.push(conflict);
            }
        }
        let s = out.stats;
        let mut summary = if let Some(block) = &out.blocked {
            block.message()
        } else if out.busy {
            "Sync ist bereits in einem anderen Fenster oder Dienst aktiv.".into()
        } else {
            format!(
                "Sync: {} →, {} ←, {} gelöscht, {} Konflikte ({} MB)",
                s.a_to_b,
                s.b_to_a,
                s.deleted,
                self.bisync_conflicts.len(),
                s.bytes / 1_048_576
            )
        };
        if cancelled {
            summary = format!("Abgebrochen; {summary}");
        }
        if let Some(stop) = &out.stopped {
            summary.push_str(&format!("; {}", stop.message()));
        }
        if !out.deferred.is_empty() {
            summary.push_str(&format!(
                "; {} Dateien wegen neuer Änderungen zurückgestellt",
                out.deferred.len()
            ));
        }
        if let Some(omitted) = out.omissions.summary() {
            summary.push_str(&format!("; {omitted}"));
        }
        let errors = s.errors.max(out.errors.len() as u64);
        if errors > 0 || !persistence_errors.is_empty() {
            let example = out
                .errors
                .first()
                .map(|(path, detail)| format!("; {path}: {detail}"))
                .unwrap_or_default();
            let persistence = if persistence_errors.is_empty() {
                String::new()
            } else {
                format!("; {}", persistence_errors.join("; "))
            };
            self.error_msg = Some(format!("{summary}; {errors} Fehler{example}{persistence}"));
        } else {
            if !self.bisync_conflicts.is_empty() {
                summary.push_str(" — Lösung erforderlich");
            }
            self.notice = Some((summary, std::time::Instant::now()));
        }
        if !self.bisync_conflicts.is_empty() {
            self.show_bisync_conflicts = true;
        }
        if !self.root_path.is_empty() {
            self.rescan();
        }
    }
}
