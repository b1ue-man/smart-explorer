use super::prelude::*;
use super::*;

impl App {
    pub(super) fn ui_settings_share(&mut self, ui: &mut egui::Ui) {
        // ─── Share-Server remote connections ─────────────────────────
        ui.add_space(12.0);
        ui.label(
            RichText::new("Geräteverbindung")
                .small()
                .color(theme::muted(ui)),
        );
        ui.label("Share-Server");
        ui.add(
            egui::TextEdit::singleline(&mut self.share_server_draft)
                .hint_text("Rendezvous-Server  host:port / wss://host/pfad")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Adresse deines eigenen Routing-Servers (se-share-server). Er vermittelt \
             nur die Verbindung — die Dateien gehen direkt zwischen den Geräten, \
             Ende-zu-Ende-verschlüsselt. Mehrere Fallbacks mit Komma trennen; \
             https:// wird als wss:// benutzt.",
        );
        ui.label("Gerätename");
        ui.add(
            egui::TextEdit::singleline(&mut self.share_device_draft)
                .hint_text("Gerätename")
                .desired_width(f32::INFINITY),
        );
        if ui.button("Verbindungseinstellungen speichern").clicked() {
            if let Some(identity) = self.share_identity.as_mut() {
                if let Err(error) = identity.set_device_name(self.share_device_draft.clone()) {
                    self.error_msg = Some(format!("Gerätename speichern: {error}"));
                    return;
                }
            }
            let server = self.share_server_draft.trim().to_string();
            match std::fs::write(share_server_path(), &server) {
                Ok(()) => {
                    self.share_server = server;
                    if let Some(svc) = self.share.take() {
                        if let Err(error) = svc.cmd(crate::share::ShareCmd::Stop) {
                            self.error_msg = Some(format!("Lokalen Share-Dienst stoppen: {error}"));
                            return;
                        }
                    }
                    match crate::daemon::refresh_share_worker_checked() {
                        Ok(true) => {
                            self.notice = Some((
                                "✓ Share-Server-Einstellungen gespeichert".to_string(),
                                std::time::Instant::now(),
                            ));
                        }
                        Ok(false) => {
                            self.error_msg = Some(
                                "Share-Server gespeichert, aber Worker ist nicht aktiv".into(),
                            );
                        }
                        Err(error) => {
                            self.error_msg = Some(format!(
                                "Share-Server gespeichert, aber Worker-Konfiguration fehlgeschlagen: {error}"
                            ));
                        }
                    }
                }
                Err(error) => {
                    self.error_msg = Some(format!("Share-Server-Einstellungen speichern: {error}"));
                }
            }
        }

    }

    pub(super) fn ui_settings_integration(&mut self, ui: &mut egui::Ui) {
        // ─── Shell integration (Windows) ───────────────────────────────
        if shell_integration_available() {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Dateimanager")
                    .small()
                    .color(theme::muted(ui)),
            );

            let resp = ui
                .checkbox(
                    &mut self.integration_ctx_menu,
                    "„In Smart Explorer öffnen“ im Rechtsklick",
                )
                .on_hover_text(
                    "Fügt einen Rechtsklick-Eintrag bei Ordnern, Laufwerken und im leeren Bereich hinzu. Jederzeit hier abschaltbar.",
                );
            if resp.changed() {
                let on = self.integration_ctx_menu;
                match set_context_menu_enabled(on) {
                    Ok(()) => {
                        self.notice = Some((
                            if on {
                                "✓ Rechtsklick-Eintrag hinzugefügt".to_string()
                            } else {
                                "✓ Rechtsklick-Eintrag entfernt".to_string()
                            },
                            std::time::Instant::now(),
                        ));
                    }
                    Err(e) => {
                        self.integration_ctx_menu = !on; // revert UI to real state
                        self.error_msg = Some(format!("Registry: {}", e));
                    }
                }
            }

            ui.colored_label(
                theme::muted(ui),
                "Hinweis: Der Eintrag liegt unter „Weitere Optionen anzeigen“ (Win11).",
            );
        }
    }
}
