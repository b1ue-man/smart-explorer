use super::prelude::*;
use super::*;
use crate::app::theme;

impl App {
    /// Saved-setups manager: list jobs with run / edit / delete / enable, plus
    /// "new". This is the rich overview the user asked for (source → target,
    /// method, schedule). Persists one checked file per setup on every change.
    /// Read-only viewer for the background daemon's run log (Group J).
    pub(in crate::app) fn ui_daemon_log(&mut self, ctx: &egui::Context) {
        let mut open = self.show_daemon_log;
        egui::Window::new("📜 Sync-Protokoll")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 380.0])
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Letzte Hintergrund-Sync-Läufe (neueste unten).")
                            .small()
                            .color(theme::muted(ui)),
                    );
                });
                ui.separator();
                let log = crate::daemon::read_log_tail(300);
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut log.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .desired_rows(18),
                        );
                    });
            });
        self.show_daemon_log = open;
    }

    pub(in crate::app) fn ui_sync_jobs(&mut self, ctx: &egui::Context) {
        let mut open = self.show_sync_jobs;
        let mut run_id: Option<String> = None;
        let mut compare_id: Option<String> = None;
        let mut edit_id: Option<String> = None;
        let mut del_id: Option<String> = None;
        let mut toggle_id: Option<String> = None;
        let mut new_blank = false;
        let mut versions_id = None;
        let mut confirmation = None;
        let jobs = self.sync_jobs.clone();
        let orphaned: std::collections::HashSet<String> = jobs
            .iter()
            .filter(|job| self.sync_job_is_orphaned(job))
            .map(|job| job.id.clone())
            .collect();
        let states = super::sync_job_state_ui::states(ctx, &jobs);
        egui::Window::new("⚙ Sync-Setups")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 440.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("＋ Neues Setup").clicked() {
                        new_blank = true;
                    }
                    ui.label(
                        RichText::new("Quelle ⇄ Ziel, Methode, Zeitplan — bleibt nach Neustart erhalten.")
                            .small()
                            .color(theme::muted(ui)),
                    );
                });
                ui.separator();
                if jobs.is_empty() {
                    ui.add_space(8.0);
                    ui.colored_label(
                        theme::muted(ui),
                        "Noch keine Setups. „＋ Neues Setup“ anlegen oder im Split-View zwei Ordner per Rechtsklick verbinden.",
                    );
                    return;
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for j in &jobs {
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(if j.name.is_empty() { "(ohne Name)" } else { &j.name }).strong());
                                if !j.enabled {
                                    ui.colored_label(theme::muted(ui), "⏸ deaktiviert");
                                }
                                if orphaned.contains(&j.id) {
                                    ui.colored_label(theme::warning(ui), "⚠ verwaist")
                                        .on_hover_text("Die Verbindung dieses Setups wurde entfernt. Das Setup bleibt erhalten, kann aber erst nach einer neuen Verbindung wieder laufen.");
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add_enabled(!self.desktop_job_busy(&j.id), egui::Button::new("×").small()).on_hover_text("Setup löschen").clicked() {
                                        del_id = Some(j.id.clone());
                                    }
                                    if ui.small_button("✎ Bearbeiten").clicked() {
                                        edit_id = Some(j.id.clone());
                                    }
                                    let enable_label = if j.enabled { "⏸ Aus" } else { "▶ Ein" };
                                    if ui.small_button(enable_label).on_hover_text("Zeitplan aktivieren/deaktivieren").clicked() {
                                        toggle_id = Some(j.id.clone());
                                    }
                                    if ui.small_button("Versionen").clicked() { versions_id = Some(j.id.clone()); }
                                    if !self.bisync_running
                                        && states.get(&j.id).is_some_and(|state| state.blocked.is_none() && state.load_error.is_none() && state.running_now(now_secs_i64()).is_none())
                                        && ui.button("▶ Jetzt").on_hover_text("Diesen Sync jetzt ausführen").clicked()
                                    {
                                        run_id = Some(j.id.clone());
                                    }
                                    if !self.preview_running
                                        && ui.small_button("🔍 Vergleichen").on_hover_text("Beide Seiten vergleichen, ohne etwas zu ändern (zeigt, was synchronisiert würde)").clicked()
                                    {
                                        compare_id = Some(j.id.clone());
                                    }
                                });
                            });
                            ui.label(
                                RichText::new(format!("{}  →  {}", j.source, j.target))
                                    .small()
                                    .color(theme::muted(ui)),
                            );
                            let sched = match j.trigger {
                                crate::syncjobs::Trigger::Manual => "manuell".to_string(),
                                crate::syncjobs::Trigger::Interval => {
                                    format!("alle {} min", j.interval_min)
                                }
                                crate::syncjobs::Trigger::Calendar => {
                                    let t = min_to_hm(j.cal_time_min);
                                    if j.cal_monthday != 0 {
                                        format!("monatl. am {}. um {}", j.cal_monthday, t)
                                    } else if j.cal_weekdays == 0 {
                                        format!("täglich {}", t)
                                    } else {
                                        const D: [&str; 7] =
                                            ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
                                        let days: Vec<&str> = (0..7)
                                            .filter(|i| (j.cal_weekdays >> i) & 1 == 1)
                                            .map(|i| D[i])
                                            .collect();
                                        format!("{} {}", days.join(","), t)
                                    }
                                }
                                crate::syncjobs::Trigger::RealTime => {
                                    format!("Echtzeit (+{}s)", j.rt_debounce_secs)
                                }
                                crate::syncjobs::Trigger::OnStartup => "beim Start".to_string(),
                                crate::syncjobs::Trigger::OnConnect => {
                                    if j.connect_match.is_empty() {
                                        "bei USB/Gerät".to_string()
                                    } else {
                                        format!("bei Gerät „{}“", j.connect_match)
                                    }
                                }
                            };
                            ui.label(RichText::new(format!("{} · {} · {}", j.direction.label(), j.conflict.label(), sched)).small().color(theme::muted(ui)));
                            let state = states.get(&j.id);
                            super::sync_job_state_ui::render(ui, state);
                            if let Some(block) = state.and_then(|state| state.blocked.as_ref()) {
                                if ui.add_enabled(!self.bisync_running && block.kind != crate::syncjobs::BlockKind::Other,
                                    egui::Button::new("Sicherheitsstopp prüfen…")).clicked() {
                                    confirmation = Some(super::sync_job_state_ui::BlockReview { id:j.id.clone(), kind:block.kind.clone(), detail:block.detail.clone(), source:j.source.clone(), target:j.target.clone() });
                                }
                            }

                        });
                    }
                });
            });
        if let Some(review) = super::sync_job_state_ui::confirmation(ctx, confirmation) {
            self.start_saved_desktop_run(
                &review.id,
                Some(super::sync_run_state::JobConfirmation {
                    kind: review.kind,
                    source: review.source,
                    target: review.target,
                }),
            );
        }
        if let Some(id) = versions_id {
            self.open_sync_versions(&id);
        }
        self.ui_sync_versions(ctx);
        self.show_sync_jobs = open || self.sync_versions.is_some();
        if new_blank {
            self.job_editor = Some(JobEditor::blank(String::new(), String::new()));
        }
        if let Some(id) = edit_id {
            if let Some(j) = self.sync_jobs.iter().find(|j| j.id == id) {
                self.job_editor = Some(JobEditor::from_job(j));
            }
        }
        if let Some(id) = toggle_id {
            let changed = (|| -> Result<(), String> {
                let mut job = crate::syncjobs::load()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|job| job.id == id)
                    .ok_or_else(|| "Setup wurde inzwischen entfernt.".to_string())?;
                job.enabled = !job.enabled;
                crate::syncjobs::upsert(&job).map_err(|e| e.to_string())
            })();
            match changed {
                Ok(()) => self.reload_sync_jobs("Sync-Setups neu laden"),
                Err(error) => {
                    self.error_msg = Some(format!("Setup konnte nicht geändert werden: {error}"))
                }
            }
        }
        if let Some(id) = del_id {
            match crate::syncjobs::remove(&id) {
                Ok(()) => self.reload_sync_jobs("Sync-Jobs nach dem Löschen neu laden"),
                Err(error) => {
                    self.error_msg =
                        Some(format!("Sync-Job konnte nicht gelöscht werden: {error}"));
                }
            }
        }
        if let Some(id) = run_id {
            self.run_job(&id);
        }
        if let Some(id) = compare_id {
            if let Some(j) = self.sync_jobs.iter().find(|j| j.id == id).cloned() {
                self.launch_preview(&j);
            }
        }
    }

    pub(in crate::app) fn reload_sync_jobs(&mut self, context: &str) {
        match crate::syncjobs::load() {
            Ok(jobs) => self.sync_jobs = jobs,
            Err(error) => {
                self.error_msg = Some(format!("{context}: {error}"));
            }
        }
    }
}
