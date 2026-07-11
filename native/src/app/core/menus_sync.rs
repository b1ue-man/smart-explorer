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
            if (self.remote.is_some() || self.net_conn.is_some())
                && ui
                    .small_button("⏏")
                    .on_hover_text("Verbindung trennen")
                    .clicked()
            {
                self.clear_disconnected_source_view();
                self.notice = Some(("Verbindung getrennt".to_string(), std::time::Instant::now()));
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
            match crate::creds::remove_connection(&acc) {
                Ok(()) => {
                    self.saved_connections = crate::creds::load_connections();
                    self.notice = Some((
                        "Gespeicherte Verbindung entfernt".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.error_msg = Some(format!(
                        "Gespeicherte Verbindung konnte nicht entfernt werden: {error}"
                    ));
                }
            }
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
                    Ok(_) => match crate::daemon::request_daemon_replacement() {
                        Ok(()) => {
                            self.notice = Some((
                                "✓ Hintergrund-Sync aktiviert".to_string(),
                                std::time::Instant::now(),
                            ));
                        }
                        Err(error) => {
                            let rollback = crate::autostart::disable()
                                .err()
                                .map(|rollback| format!("; Autostart-Rücknahme: {rollback}"))
                                .unwrap_or_default();
                            self.error_msg = Some(format!(
                                    "Hintergrund-Sync bleibt aus: Dienst konnte nicht sicher gestartet werden: {error}{rollback}"
                                ));
                        }
                    },
                    Err(e) => self.error_msg = Some(format!("Autostart: {}", e)),
                }
            } else {
                match crate::autostart::disable() {
                    Ok(()) => {
                        self.notice = Some((
                            "Hintergrund-Sync deaktiviert".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(error) => {
                        self.error_msg = Some(format!("Autostart: {error}"));
                    }
                }
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
        if bg && crate::daemon::is_running() {
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
        } else if crate::daemon::is_running() {
            ui.colored_label(
                Color32::from_gray(150),
                "Hintergrund-Sync aus · Share-Sitzungsdienst aktiv.",
            );
        }
        // Check cadence (how often the daemon evaluates schedules / reacts).
        ui.horizontal(|ui| {
            ui.label("Prüfintervall").on_hover_text(
                "Wie oft der Dienst nach fälligen Aufträgen, Änderungen (Echtzeit) und \
                 angeschlossenen Geräten sieht. Kürzer = reaktiver, mehr CPU.",
            );
            match crate::daemon::cadence_secs() {
                Ok(mut cadence) => {
                    if ui
                        .add(
                            egui::DragValue::new(&mut cadence)
                                .range(2..=3600)
                                .suffix(" s"),
                        )
                        .changed()
                    {
                        self.report_daemon_control(
                            "Prüfintervall speichern",
                            crate::daemon::set_cadence_secs(cadence),
                        );
                    }
                }
                Err(error) => {
                    ui.colored_label(Color32::from_rgb(230, 120, 100), "nicht lesbar")
                        .on_hover_text(format!("Zeitsteuerung ist sicher gesperrt: {error}"));
                }
            }
        });

        // Pause / resume.
        ui.horizontal(|ui| {
            match crate::daemon::pause_remaining() {
                Ok(Some(r)) if r == i64::MAX => {
                    ui.colored_label(Color32::from_rgb(230, 180, 90), "⏸ pausiert (dauerhaft)");
                }
                Ok(Some(r)) => {
                    ui.colored_label(
                        Color32::from_rgb(230, 180, 90),
                        format!("⏸ pausiert (noch {} min)", (r / 60).max(1)),
                    );
                }
                Ok(None) => {
                    ui.colored_label(Color32::from_gray(140), "Pause:");
                }
                Err(error) => {
                    ui.colored_label(Color32::from_rgb(230, 120, 100), "⏸ Status nicht lesbar")
                        .on_hover_text(format!("Zeitsteuerung ist sicher gesperrt: {error}"));
                }
            }
            if ui.small_button("2 h").clicked() {
                self.report_daemon_control(
                    "Pause speichern",
                    crate::daemon::pause_for_secs(2 * 3600),
                );
            }
            if ui.small_button("8 h").clicked() {
                self.report_daemon_control(
                    "Pause speichern",
                    crate::daemon::pause_for_secs(8 * 3600),
                );
            }
            if ui.small_button("24 h").clicked() {
                self.report_daemon_control(
                    "Pause speichern",
                    crate::daemon::pause_for_secs(24 * 3600),
                );
            }
            if ui
                .small_button("∞")
                .on_hover_text("Dauerhaft pausieren")
                .clicked()
            {
                self.report_daemon_control("Pause speichern", crate::daemon::pause_indefinite());
            }
            if ui.small_button("▶ Fortsetzen").clicked() {
                self.report_daemon_control("Pause aufheben", crate::daemon::resume());
            }
        });

        // Auto-pause conditions.
        match crate::daemon::autopause_flags() {
            Ok((mut battery, mut metered)) => {
                ui.horizontal(|ui| {
                    let battery_changed = ui
                        .checkbox(&mut battery, "Im Energiesparmodus pausieren")
                        .on_hover_text("Synchronisierung anhalten, solange der Windows-Energiesparmodus aktiv ist")
                        .changed();
                    let metered_changed = ui
                        .checkbox(&mut metered, "Bei getakteter Verbindung")
                        .on_hover_text("Synchronisierung anhalten, solange eine getaktete Netzwerkverbindung erkannt wird (Windows)")
                        .changed();
                    if battery_changed || metered_changed {
                        self.report_daemon_control(
                            "Automatische Pause speichern",
                            crate::daemon::set_autopause_flags(battery, metered),
                        );
                    }
                });
            }
            Err(error) => {
                ui.colored_label(
                    Color32::from_rgb(230, 120, 100),
                    "Automatische Pause nicht lesbar · Hintergrund-Sync gesperrt",
                )
                .on_hover_text(error.to_string());
            }
        }

        ui.label(
            RichText::new("Hintergrund-Auslöser: Echtzeit & USB-Anschluss brauchen lokale Pfade.")
                .small()
                .color(Color32::from_gray(120)),
        );
    }

    fn report_daemon_control(&mut self, action: &str, result: std::io::Result<()>) {
        if let Err(error) = result {
            self.error_msg = Some(format!("{action} fehlgeschlagen: {error}"));
        }
    }
}
