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
        self.migrate_stored_share_server(ui);
        let allow_id = egui::Id::new("share_server_allow_plaintext");
        let mut allow_plaintext = ui
            .data_mut(|data| data.get_temp::<bool>(allow_id))
            .unwrap_or_else(|| stored_plaintext(&self.share_server));
        ui.label("Share-Server");
        ui.add(
            egui::TextEdit::singleline(&mut self.share_server_draft)
                .hint_text("wss://server[:port]/pfad oder server[:port] (verschlüsselt)")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Adresse deines eigenen Routing-Servers (se-share-server). Er vermittelt \
             nur die Verbindung — die Dateien gehen direkt zwischen den Geräten, \
             Ende-zu-Ende-verschlüsselt. Ohne Schema wird TLS (wss://) benutzt; einen \
             selbst signierten Server mit #sha256=<Fingerabdruck> anheften. Mehrere \
             Adressen mit Komma trennen.",
        );
        if ui
            .checkbox(&mut allow_plaintext, "Unverschlüsselt erlauben (unsicher)")
            .on_hover_text(
                "Nur für Server ohne TLS (tcp://, ws://). Server-Betreiber und jeder im \
                 Netz sehen dann Gerätenamen, Adressen und Beziehungs-IDs und können \
                 Meldungen fälschen; Dateien bleiben Ende-zu-Ende-verschlüsselt.",
            )
            .changed()
        {
            ui.data_mut(|data| data.insert_temp(allow_id, allow_plaintext));
        }
        if let Some(encrypted) = server_security_line(ui, &self.share_server_draft, allow_plaintext)
        {
            self.share_server_draft = encrypted;
            ui.data_mut(|data| data.insert_temp(allow_id, false));
        }
        ui.label("Gerätename");
        ui.add(
            egui::TextEdit::singleline(&mut self.share_device_draft)
                .hint_text("Gerätename")
                .desired_width(f32::INFINITY),
        );
        if ui.button("Verbindungseinstellungen speichern").clicked() {
            let server = match canonical_server_input(&self.share_server_draft, allow_plaintext) {
                Ok(server) => server,
                Err(error) => {
                    self.error_msg = Some(format!("Share-Server-Adresse: {error}"));
                    return;
                }
            };
            if let Some(identity) = self.share_identity.as_mut() {
                if let Err(error) = identity.set_device_name(self.share_device_draft.clone()) {
                    self.error_msg = Some(format!("Gerätename speichern: {error}"));
                    return;
                }
            }
            self.share_server_draft = server.clone();
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

    /// Rewrites a stored legacy address once per session (B21): a value
    /// without scheme becomes `tcp://host:port` with its plaintext permission.
    fn migrate_stored_share_server(&mut self, ui: &egui::Ui) {
        let tried = egui::Id::new("share_server_migration_tried");
        if ui
            .data_mut(|data| data.get_temp::<bool>(tried))
            .unwrap_or(false)
        {
            return;
        }
        ui.data_mut(|data| data.insert_temp(tried, true));
        match crate::share::migrate_server_file(&share_server_path()) {
            Ok(Some(canonical)) => {
                if self.share_server_draft.trim() == self.share_server.trim() {
                    self.share_server_draft = canonical.clone();
                }
                self.share_server = canonical;
            }
            Ok(None) => {}
            Err(error) => {
                self.error_msg = Some(format!("Share-Server-Adresse umschreiben: {error}"));
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

/// The address to store; empty input removes the server (LAN only).
fn canonical_server_input(draft: &str, allow_plaintext: bool) -> Result<String, String> {
    if draft.trim().is_empty() {
        return Ok(String::new());
    }
    crate::share::server_address::SignalServerConfig::parse_input(draft, allow_plaintext)
        .map(|config| config.canonical())
}

/// A stored plaintext entry carries the user's earlier permission.
fn stored_plaintext(stored: &str) -> bool {
    crate::share::server_address::SignalServerConfig::parse_stored(stored).is_ok_and(|config| {
        config
            .endpoints()
            .iter()
            .any(|endpoint| !endpoint.is_encrypted())
    })
}

/// Transport security of the entered address, or why it cannot be saved.
/// For a plaintext address it explains the encrypted setup and returns the
/// encrypted address when the user adopts it (saved with the usual button).
fn server_security_line(ui: &mut egui::Ui, draft: &str, allow_plaintext: bool) -> Option<String> {
    use crate::share::server_address::{ServerSecurity, SignalServerConfig};
    if draft.trim().is_empty() {
        ui.small("Kein Share-Server: Direktgeräte nur über das lokale Netz.");
        return None;
    }
    match SignalServerConfig::parse_input(draft, allow_plaintext) {
        Ok(config) if config.security() == ServerSecurity::Plaintext => {
            ui.colored_label(
                theme::warning(ui),
                format!(
                    "{} – Server und Netz sehen Gerätenamen und Adressen",
                    config.summary()
                ),
            );
            let encrypted = config.encrypted_alternative()?;
            ui.small(
                "Verschlüsselt verbinden: Der Server braucht ein TLS-Zertifikat für den \
                 eingetragenen Namen (se-share-server mit --tls-cert/--tls-key bzw. \
                 SE_SHARE_TLS_CERT/SE_SHARE_TLS_KEY). Ein selbst signiertes Zertifikat \
                 wird mit #sha256=<Fingerabdruck> angeheftet. Ein TLS-Fehler fällt nie \
                 auf Klartext zurück.",
            );
            if config.names_ip_address() {
                ui.colored_label(
                    theme::warning(ui),
                    "Die Adresse ist eine IP-Adresse: Trage den Servernamen ein, auf den das \
                     Zertifikat ausgestellt ist, oder hefte das Zertifikat mit #sha256= an.",
                );
            }
            ui.button(format!("Verschlüsselte Adresse übernehmen: {encrypted}"))
                .on_hover_text(
                    "Trägt die Adresse ein; danach „Verbindungseinstellungen speichern“.",
                )
                .clicked()
                .then_some(encrypted)
        }
        Ok(config) => {
            ui.colored_label(theme::success(ui), config.summary());
            None
        }
        Err(error) => {
            ui.colored_label(theme::danger(ui), error);
            None
        }
    }
}
