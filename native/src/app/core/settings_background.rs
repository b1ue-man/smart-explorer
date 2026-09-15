use super::prelude::*;
use super::*;

impl App {
    pub(super) fn ui_settings_background(&mut self, ui: &mut egui::Ui) {
        // ─── Background sync (runs setups on their schedule, app closed) ──
        ui.separator();
        ui.label(
            RichText::new("HINTERGRUND")
                .small()
                .color(theme::muted(ui)),
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
                theme::accent(ui),
                format!("● Dienst aktiv (vor {age}s)"),
            );
        } else if bg {
            ui.colored_label(
                theme::muted(ui),
                "Dienst startet beim nächsten Anmelden.",
            );
        } else if crate::daemon::is_running() {
            ui.colored_label(
                theme::muted(ui),
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
                    ui.colored_label(theme::danger(ui), "nicht lesbar")
                        .on_hover_text(format!("Zeitsteuerung ist sicher gesperrt: {error}"));
                }
            }
        });

        // Pause / resume.
        ui.horizontal(|ui| {
            match crate::daemon::pause_remaining() {
                Ok(Some(r)) if r == i64::MAX => {
                    ui.colored_label(theme::warning(ui), "⏸ pausiert (dauerhaft)");
                }
                Ok(Some(r)) => {
                    ui.colored_label(
                        theme::warning(ui),
                        format!("⏸ pausiert (noch {} min)", (r / 60).max(1)),
                    );
                }
                Ok(None) => {
                    ui.colored_label(theme::muted(ui), "Pause:");
                }
                Err(error) => {
                    ui.colored_label(theme::danger(ui), "⏸ Status nicht lesbar")
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
                    theme::danger(ui),
                    "Automatische Pause nicht lesbar · Hintergrund-Sync gesperrt",
                )
                .on_hover_text(error.to_string());
            }
        }

        ui.label(
            RichText::new("Hintergrund-Auslöser: Echtzeit & USB-Anschluss brauchen lokale Pfade.")
                .small()
                .color(theme::muted(ui)),
        );
    }

    fn report_daemon_control(&mut self, action: &str, result: std::io::Result<()>) {
        if let Err(error) = result {
            self.error_msg = Some(format!("{action} fehlgeschlagen: {error}"));
        }
    }
}
