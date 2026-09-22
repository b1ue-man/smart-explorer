use crate::app::theme;
use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_menu_connect(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("VERBINDEN")
                    .small()
                    .color(theme::muted(ui)),
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
            ui.colored_label(theme::accent(ui), format!("● {}", rs.label));
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
                    .color(theme::muted(ui)),
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
                theme::muted(ui),
                "Gespeicherte Verbindungen: in der Sidebar links.",
            );
        }
        if let Some(acc) = to_remove {
            self.remove_saved_connection_completely(&acc);
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
                ui.colored_label(theme::muted(ui), format!("({n})"));
            }
        });
        // Quick-create from the current location.
        if !self.root_path.is_empty()
            && ui
                .small_button("＋ Setup aus aktuellem Ordner…")
                .on_hover_text("Neues Sync-Setup mit dem aktuellen Ordner als Quelle anlegen")
                .clicked()
        {
            self.begin_pane_sync_setup(self.active_tab, None);
        }

        if ui.button("Hintergrund-Sync einstellen…").clicked() {
            self.open_settings(settings_ui::SettingsPage::Integration);
            ui.close_menu();
        }

    }
}
