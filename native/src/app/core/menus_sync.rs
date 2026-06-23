use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_menu_connect(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("VERBINDEN")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            if self.remote.is_some() || self.net_conn.is_some() {
                if ui
                    .small_button("⏏")
                    .on_hover_text("Verbindung trennen")
                    .clicked()
                {
                    self.remote = None;
                    self.net_conn = None;
                    self.notice =
                        Some(("Verbindung getrennt".to_string(), std::time::Instant::now()));
                }
            }
        });
        if let Some(rs) = &self.remote {
            ui.colored_label(Color32::from_rgb(120, 200, 255), format!("● {}", rs.label));
        }
        if ui
            .small_button("＋ Neue Verbindung")
            .on_hover_text("SFTP / FTP / FTPS / Netzlaufwerk")
            .clicked()
        {
            self.connect_form = crate::connect::ConnectForm::default();
            self.show_connect = true;
        }
        if ui
            .small_button("Share-Server verbinden")
            .on_hover_text(
                "Direkt oder per Raum ein anderes Smart-Explorer-Geraet als Remote oeffnen",
            )
            .clicked()
        {
            self.show_share = true;
        }
        // Established connections live on the sidebar (most recent first). Only
        // the overflow — older ones beyond the sidebar cap — appears here, so
        // the menu stays uncluttered but no saved connection is ever hidden.
        let mut to_remove: Option<String> = None;
        let mut to_connect: Option<crate::creds::SavedConnection> = None;
        let conns: Vec<crate::creds::SavedConnection> =
            self.saved_connections.iter().rev().cloned().collect();
        if conns.len() > SIDEBAR_CONN_CAP {
            ui.add_space(4.0);
            ui.label(
                RichText::new("WEITERE (ältere)")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            for c in conns.iter().skip(SIDEBAR_CONN_CAP) {
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new(format!("🖧 {}", c.display())).small())
                                .frame(false),
                        )
                        .on_hover_text(c.to_target())
                        .clicked()
                    {
                        to_connect = Some(c.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Entfernen").clicked() {
                            to_remove = Some(c.account());
                        }
                    });
                });
            }
        } else if !conns.is_empty() {
            ui.colored_label(
                Color32::from_gray(120),
                "Gespeicherte Verbindungen: in der Sidebar links.",
            );
        }
        if let Some(acc) = to_remove {
            let _ = crate::creds::remove_connection(&acc);
            self.saved_connections = crate::creds::load_connections();
        }
        if let Some(c) = to_connect {
            self.connect_saved(&c);
        }
    }

    pub(in crate::app) fn ui_menu_sync(&mut self, ui: &mut egui::Ui) {
        // One-way mirror of the current location to a local folder (backup).
        if !self.root_path.is_empty() {
            if self.sync_running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Spiegelung läuft…");
                    if ui.button("⏹ Stop").clicked() {
                        if let Some(c) = &self.sync_cancel {
                            c.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            } else if self.bisync_running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("2-Wege-Sync läuft…");
                    if ui.button("⏹ Stop").clicked() {
                        if let Some(c) = &self.bisync_cancel {
                            c.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            } else {
                if ui
                    .small_button("⇅ Spiegeln nach…")
                    .on_hover_text("Aktuellen Ordner (lokal oder remote) EINSEITIG in einen lokalen Zielordner spiegeln (Backup)")
                    .clicked()
                {
                    self.open_picker(PickerPurpose::MirrorDest, "");
                }
                if ui
                    .small_button("⇄ 2-Wege-Sync…")
                    .on_hover_text("Sicher in BEIDE Richtungen abgleichen: nur tatsächlich geänderte Dateien werden übertragen, beidseitige Änderungen werden als Konflikt gemeldet (nichts wird stillschweigend überschrieben), Änderungen sind reversibel.")
                    .clicked()
                {
                    self.open_picker(PickerPurpose::BisyncDest, "");
                }
            }
        }
        // ─── Saved sync setups (persist across restarts) ──────────────────
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .small_button("⚙ Sync-Setups…")
                .on_hover_text("Gespeicherte Sync-Aufträge verwalten (Quelle, Ziel, Methode, Zeitplan) — bleiben nach Neustart erhalten")
                .clicked()
            {
                self.show_sync_jobs = true;
            }
            let n = self.sync_jobs.len();
            if n > 0 {
                ui.colored_label(Color32::from_gray(140), format!("({n})"));
            }
        });
        // Quick-create from the current location.
        if !self.root_path.is_empty()
            && ui
                .small_button("＋ Setup aus aktuellem Ordner…")
                .on_hover_text("Neues Sync-Setup mit dem aktuellen Ordner als Quelle anlegen")
                .clicked()
        {
            let src = if is_local_style(&self.root_path) {
                self.root_path.clone()
            } else {
                String::new()
            };
            self.job_editor = Some(JobEditor::blank(src, String::new()));
            self.show_sync_jobs = true;
        }

        // ─── Background sync (runs setups on their schedule, app closed) ──
        ui.separator();
        ui.label(
            RichText::new("HINTERGRUND")
                .small()
                .color(Color32::from_gray(140)),
        );
        let mut bg = crate::autostart::is_enabled();
        if ui
            .checkbox(&mut bg, "Beim Anmelden im Hintergrund synchronisieren")
            .on_hover_text(
                "Startet einen unsichtbaren Dienst (dieselbe App via Autostart), der \
                 gespeicherte Setups mit Zeitplan automatisch ausführt — auch wenn das \
                 Fenster geschlossen ist. Updates erfassen den Dienst automatisch.",
            )
            .changed()
        {
            if bg {
                match crate::autostart::enable() {
                    Ok(_) => {
                        crate::daemon::clear_stop();
                        crate::autostart::spawn_daemon_now();
                        self.notice = Some((
                            "✓ Hintergrund-Sync aktiviert".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(e) => self.error_msg = Some(format!("Autostart: {}", e)),
                }
            } else {
                let _ = crate::autostart::disable();
                crate::daemon::request_stop();
                self.notice = Some((
                    "Hintergrund-Sync deaktiviert".to_string(),
                    std::time::Instant::now(),
                ));
            }
        }
        ui.horizontal(|ui| {
            if ui
                .small_button("📜 Protokoll")
                .on_hover_text("Protokoll der Hintergrund-Sync-Läufe anzeigen")
                .clicked()
            {
                self.show_daemon_log = true;
            }
        });
        if crate::daemon::is_running() {
            let age = crate::daemon::last_heartbeat_age().unwrap_or(0);
            ui.colored_label(
                Color32::from_rgb(120, 200, 255),
                format!("● Dienst aktiv (vor {age}s)"),
            );
        } else if bg {
            ui.colored_label(
                Color32::from_gray(150),
                "Dienst startet beim nächsten Anmelden.",
            );
        }
        // Check cadence (how often the daemon evaluates schedules / reacts).
        ui.horizontal(|ui| {
            ui.label("Prüfintervall").on_hover_text(
                "Wie oft der Dienst nach fälligen Aufträgen, Änderungen (Echtzeit) und \
                 angeschlossenen Geräten sieht. Kürzer = reaktiver, mehr CPU.",
            );
            let mut cad = crate::daemon::cadence_secs();
            if ui
                .add(egui::DragValue::new(&mut cad).range(2..=3600).suffix(" s"))
                .changed()
            {
                crate::daemon::set_cadence_secs(cad);
            }
        });

        // Pause / resume.
        ui.horizontal(|ui| {
            match crate::daemon::pause_remaining() {
                Some(r) if r == i64::MAX => {
                    ui.colored_label(Color32::from_rgb(230, 180, 90), "⏸ pausiert (dauerhaft)");
                }
                Some(r) => {
                    ui.colored_label(
                        Color32::from_rgb(230, 180, 90),
                        format!("⏸ pausiert (noch {} min)", (r / 60).max(1)),
                    );
                }
                None => {
                    ui.colored_label(Color32::from_gray(140), "Pause:");
                }
            }
            if ui.small_button("2 h").clicked() {
                crate::daemon::pause_for_secs(2 * 3600);
            }
            if ui.small_button("8 h").clicked() {
                crate::daemon::pause_for_secs(8 * 3600);
            }
            if ui.small_button("24 h").clicked() {
                crate::daemon::pause_for_secs(24 * 3600);
            }
            if ui
                .small_button("∞")
                .on_hover_text("Dauerhaft pausieren")
                .clicked()
            {
                crate::daemon::pause_indefinite();
            }
            if ui.small_button("▶ Fortsetzen").clicked() {
                crate::daemon::resume();
            }
        });

        // Auto-pause conditions.
        let (mut bat, mut met) = crate::daemon::autopause_flags();
        ui.horizontal(|ui| {
            let c1 = ui
                .checkbox(&mut bat, "Im Energiesparmodus pausieren")
                .on_hover_text("Synchronisierung anhalten, solange der Windows-Energiesparmodus aktiv ist")
                .changed();
            let c2 = ui
                .checkbox(&mut met, "Bei getakteter Verbindung")
                .on_hover_text("Synchronisierung anhalten, solange eine getaktete Netzwerkverbindung erkannt wird (Windows)")
                .changed();
            if c1 || c2 {
                crate::daemon::set_autopause_flags(bat, met);
            }
        });

        ui.label(
            RichText::new("Hintergrund-Auslöser: Echtzeit & USB-Anschluss brauchen lokale Pfade.")
                .small()
                .color(Color32::from_gray(120)),
        );
    }

    /// Saved-setups manager: list jobs with run / edit / delete / enable, plus
    /// "new". This is the rich overview the user asked for (source → target,
    /// method, schedule). Persists to sync/jobs.tsv on every change.
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
            if let Some(j) = self.sync_jobs.iter_mut().find(|j| j.id == id) {
                j.enabled = !j.enabled;
                let job = j.clone();
                let _ = crate::syncjobs::upsert(&job);
                self.sync_jobs = crate::syncjobs::load();
            }
        }
        if let Some(id) = del_id {
            let _ = crate::syncjobs::remove(&id);
            self.sync_jobs = crate::syncjobs::load();
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
}
