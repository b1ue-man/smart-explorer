use super::prelude::*;
use super::*;

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
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Letzte Hintergrund-Sync-Läufe (neueste unten).")
                            .small()
                            .color(Color32::from_gray(140)),
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
        let jobs = self.sync_jobs.clone();
        let results = crate::syncjobs::load_results();
        egui::Window::new("⚙ Sync-Setups")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 440.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("＋ Neues Setup").clicked() {
                        new_blank = true;
                    }
                    ui.label(
                        RichText::new("Quelle ⇄ Ziel, Methode, Zeitplan — bleibt nach Neustart erhalten.")
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                });
                ui.separator();
                if jobs.is_empty() {
                    ui.add_space(8.0);
                    ui.colored_label(
                        Color32::from_gray(140),
                        "Noch keine Setups. „＋ Neues Setup“ anlegen oder im Split-View zwei Ordner per Rechtsklick verbinden.",
                    );
                    return;
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for j in &jobs {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(if j.name.is_empty() { "(ohne Name)" } else { &j.name }).strong());
                                if !j.enabled {
                                    ui.colored_label(Color32::from_gray(130), "⏸ deaktiviert");
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕").on_hover_text("Setup löschen").clicked() {
                                        del_id = Some(j.id.clone());
                                    }
                                    if ui.small_button("✎ Bearbeiten").clicked() {
                                        edit_id = Some(j.id.clone());
                                    }
                                    let enable_label = if j.enabled { "⏸ Aus" } else { "▶ Ein" };
                                    if ui.small_button(enable_label).on_hover_text("Zeitplan aktivieren/deaktivieren").clicked() {
                                        toggle_id = Some(j.id.clone());
                                    }
                                    if !self.bisync_running
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
                                    .color(Color32::from_gray(170)),
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
                            let last = if j.last_run == 0 {
                                "nie".to_string()
                            } else {
                                fmt_ms(j.last_run * 1000)
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{} · {} · {} · zuletzt: {}",
                                    j.direction.label(),
                                    j.conflict.label(),
                                    sched,
                                    last
                                ))
                                .small()
                                .color(Color32::from_gray(140)),
                            );
                            // Live status from the last recorded run.
                            if let Some(r) = results.get(&j.id) {
                                let color = match r.note.as_str() {
                                    "ok" => Color32::from_rgb(120, 200, 120),
                                    "Konflikte" => Color32::from_rgb(230, 200, 90),
                                    _ => Color32::from_rgb(230, 120, 120),
                                };
                                ui.label(
                                    RichText::new(format!(
                                        "● {} — {}→ {}← {}gelöscht · {}Konflikte · {}Fehler",
                                        r.note, r.a_to_b, r.b_to_a, r.deleted, r.conflicts, r.errors
                                    ))
                                    .small()
                                    .color(color),
                                );
                            }
                        });
                    }
                });
            });
        self.show_sync_jobs = open;
        if new_blank {
            self.job_editor = Some(JobEditor::blank(String::new(), String::new()));
        }
        if let Some(id) = edit_id {
            if let Some(j) = self.sync_jobs.iter().find(|j| j.id == id) {
                self.job_editor = Some(JobEditor::from_job(j));
            }
        }
        if let Some(id) = toggle_id {
            if let Some(mut j) = self.sync_jobs.iter().find(|j| j.id == id).cloned() {
                j.enabled = !j.enabled;
                match crate::syncjobs::upsert(&j) {
                    Ok(()) => self.reload_sync_jobs("Sync-Jobs neu laden"),
                    Err(error) => {
                        self.error_msg =
                            Some(format!("Sync-Job konnte nicht geändert werden: {error}"));
                    }
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
