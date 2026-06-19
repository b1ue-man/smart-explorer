use super::prelude::*;
use super::*;

impl App {
    /// Cloud (Google Drive) connect (#19): configure your OWN Google OAuth
    /// client ID and run the authorize flow. Smart Explorer is not a hosted
    /// service — each user supplies their own client (see docs/CLOUD_SETUP.md).
    pub(in crate::app) fn ui_menu_cloud(&mut self, ui: &mut egui::Ui) {
        use crate::cloud::Provider;
        let p = Provider::GDrive;
        ui.add_space(12.0);
        ui.label(RichText::new("CLOUD (GOOGLE DRIVE)").small().color(Color32::from_gray(140)));
        if crate::cloud::is_connected(p) {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(120, 200, 255), "● Verbunden");
                if ui.small_button("☁ Drive öffnen").on_hover_text("Google Drive durchsuchen").clicked() {
                    self.open_gdrive_browse();
                }
                if ui.small_button("Trennen").clicked() {
                    crate::cloud::disconnect(p);
                    self.notice = Some(("Google Drive getrennt".to_string(), std::time::Instant::now()));
                }
            });
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.cloud_client_id_draft)
                .hint_text("OAuth Client-ID (…apps.googleusercontent.com)")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Aus DEINEM eigenen Google-Cloud-Projekt (Desktop-OAuth-Client). \
             Diese App ist kein Dienst — siehe Anleitung unten / docs/CLOUD_SETUP.md.",
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.cloud_secret_draft)
                .hint_text("Client-Secret (von Google, falls vergeben)")
                .password(true)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if self.cloud_authing {
                ui.spinner();
                ui.label("Browser-Anmeldung läuft…");
            } else if ui
                .small_button("Mit Google verbinden")
                .on_hover_text("Speichert die Client-ID und öffnet den Browser zur Anmeldung")
                .clicked()
            {
                let cfg = crate::cloud::ClientConfig {
                    client_id: self.cloud_client_id_draft.trim().to_string(),
                    client_secret: self.cloud_secret_draft.trim().to_string(),
                };
                if cfg.client_id.is_empty() {
                    self.error_msg = Some("Bitte zuerst die Client-ID eintragen.".to_string());
                } else {
                    let _ = crate::cloud::save_config(p, &cfg);
                    let (tx, rx) = unbounded();
                    self.cloud_auth_rx = Some(rx);
                    self.cloud_authing = true;
                    std::thread::Builder::new()
                        .name("cloud-auth".into())
                        .spawn(move || {
                            let _ = tx.send(crate::cloud::authorize(p).map(|_| ()));
                        })
                        .ok();
                    self.notice = Some((
                        "Browser zur Google-Anmeldung geöffnet…".to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }
        });
        // Inline setup guide — the user runs their own Google project; this app
        // hosts nothing. Full version: docs/CLOUD_SETUP.md.
        egui::CollapsingHeader::new("ℹ Einrichtung (eigenes Google-Projekt)")
            .id_salt("cloud_setup_help")
            .show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "Smart Explorer ist kein Cloud-Dienst — du nutzt dein eigenes \
                         Google-Konto. Einmalig (~5 min):",
                    )
                    .small(),
                );
                for line in [
                    "1. Google Cloud Console → Projekt anlegen.",
                    "2. APIs & Dienste → Bibliothek → „Google Drive API“ aktivieren.",
                    "3. OAuth-Zustimmungsbildschirm → Extern; dich als Testnutzer hinzufügen.",
                    "4. Anmeldedaten → OAuth-Client-ID → Typ „Desktop-App“ (keine Redirect-URI nötig).",
                    "5. Client-ID (+ ggf. Secret) oben einfügen → „Mit Google verbinden“.",
                ] {
                    ui.label(RichText::new(line).small().color(Color32::from_gray(180)));
                }
                ui.hyperlink_to("→ Google Cloud Console öffnen", "https://console.cloud.google.com");
                ui.label(
                    RichText::new(
                        "Hinweis: Im „Testing“-Modus laufen die Tokens nach ~7 Tagen ab — \
                         dann einfach erneut verbinden. Details: docs/CLOUD_SETUP.md.",
                    )
                    .small()
                    .color(Color32::from_gray(140)),
                );
            });
        ui.separator();
    }

    pub(in crate::app) fn ui_menu_settings(&mut self, ui: &mut egui::Ui) {
        self.ui_menu_cloud(ui);

        // ─── Teilen (peer file sharing) ───────────────────────────────
        ui.add_space(12.0);
        ui.label(RichText::new("TEILEN (P2P)").small().color(Color32::from_gray(140)));
        ui.add(
            egui::TextEdit::singleline(&mut self.share_server_draft)
                .hint_text("Rendezvous-Server  host:port")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Adresse deines eigenen Routing-Servers (se-share-server). Er vermittelt \
             nur die Verbindung — die Dateien gehen direkt zwischen den Geräten, \
             Ende-zu-Ende-verschlüsselt.",
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.share_device_draft)
                .hint_text("Gerätename")
                .desired_width(f32::INFINITY),
        );
        if ui.small_button("Speichern").clicked() {
            self.share_server = self.share_server_draft.trim().to_string();
            let _ = std::fs::write(share_server_path(), &self.share_server);
            // Restart the service so the new server/name take effect.
            self.share = None;
            self.notice = Some(("✓ Teilen-Einstellungen gespeichert".to_string(), std::time::Instant::now()));
        }

        // ─── Update ───────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(RichText::new("UPDATE").small().color(Color32::from_gray(140)));
        ui.colored_label(
            Color32::from_gray(140),
            format!("Version {}", env!("CARGO_PKG_VERSION")),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.update_feed_draft)
                .hint_text("Feed-Ordner oder Git/HTTPS-URL…")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Quelle mit version.txt und smart_explorer.exe. Entweder ein Ordner \
             (lokal/Netzlaufwerk) ODER eine https-URL bzw. ein GitHub-Repo-Link \
             (z. B. https://github.com/b1ue-man/smart-explorer) — dann updatet \
             sich die App direkt aus dem Git. Beim Start wird automatisch geprüft.",
        );
        ui.horizontal(|ui| {
            if ui.small_button("Speichern").clicked() {
                match crate::updater::set_update_source(&self.update_feed_draft) {
                    Ok(_) => {
                        self.notice = Some((
                            "✓ Update-Feed gespeichert".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(e) => self.error_msg = Some(format!("Feed speichern: {}", e)),
                }
            }
            if ui.small_button("Jetzt prüfen").clicked() {
                self.check_updates_manual();
            }
        });

        // Rollback to a previously-installed version + pause/resume auto-update.
        if let Some(pinned) = crate::updater::pinned_version() {
            ui.colored_label(
                Color32::from_rgb(255, 190, 90),
                format!("⏸ Auto-Update pausiert (zurückgerollt auf v{})", pinned),
            );
            if ui.small_button("Auf neueste aktualisieren").clicked() {
                let (tx, rx) = unbounded();
                self.update_rx = Some(rx);
                crate::updater::update_to_latest_async(tx);
                self.notice =
                    Some(("Suche neueste Version…".to_string(), std::time::Instant::now()));
            }
        }
        // Rollback section. Primary source = the actual RELEASES on the GitHub
        // feed (so you see every previous version, not just what you happened to
        // archive locally); locally-archived binaries are the offline fallback.
        ui.add_space(2.0);
        self.fetch_remote_versions(); // one-time, cached
        let current = env!("CARGO_PKG_VERSION");
        let downloading = self.rollback_rx.is_some();
        let mut dl_version: Option<String> = None; // older release → download+rollback
        let mut install_version: Option<String> = None; // newer release → download+install
        let mut revert_local: Option<(String, PathBuf)> = None;

        // A newer release than the running version → offer it as an update right
        // here (auto-discovered, so no "Jetzt prüfen" needed, and independent of
        // the main-branch feed version).
        if let Some(newest) = self.update_release_available.clone() {
            ui.colored_label(
                Color32::from_rgb(120, 220, 130),
                format!("⬆ Update verfügbar: v{newest}"),
            );
            if ui
                .add_enabled(!downloading, egui::Button::new("⬆ Installieren"))
                .on_hover_text("Diese neuere Version laden und installieren (Neustart)")
                .clicked()
            {
                install_version = Some(newest);
            }
            ui.add_space(4.0);
        }

        ui.label(RichText::new("Frühere Versionen (Releases)").small().color(Color32::from_gray(140)));
        if self.remote_versions_rx.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label(RichText::new("lade Release-Liste…").small().color(Color32::from_gray(120)));
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        } else if let Some(list) = self.remote_versions.clone() {
            // Only OLDER versions are rollback targets; a newer one is offered as
            // an update by the banner above.
            let list: Vec<String> = list
                .into_iter()
                .filter(|v| v != current && !crate::updater::is_newer(v, current))
                .collect();
            if list.is_empty() {
                ui.colored_label(Color32::from_gray(110), "(keine — Feed ist kein GitHub-Repo, oder offline)");
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("rollback_remote")
                    .max_height(160.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for ver in &list {
                            ui.horizontal(|ui| {
                                ui.label(format!("v{}", ver));
                                if ui
                                    .add_enabled(!downloading, egui::Button::new("↩ Zurück").small())
                                    .on_hover_text("Diese veröffentlichte Version laden und zurückrollen (Neustart)")
                                    .clicked()
                                {
                                    dl_version = Some(ver.clone());
                                }
                            });
                        }
                    });
            }
        }
        if downloading {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label(RichText::new("lade Version…").small().color(Color32::from_gray(120)));
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }

        // Locally-archived binaries (instant; also lets you go forward again
        // after a rollback, and works offline).
        let archived: Vec<(String, PathBuf)> = crate::updater::list_archived_versions()
            .into_iter()
            .filter(|(v, _)| v != current)
            .collect();
        if !archived.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("Lokal gesichert").small().color(Color32::from_gray(140)));
            egui::ScrollArea::vertical()
                .id_salt("rollback_local")
                .max_height(140.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (ver, path) in &archived {
                        ui.horizontal(|ui| {
                            ui.label(format!("v{}", ver));
                            if ui
                                .small_button("↩ Zurück")
                                .on_hover_text("Auf diese (lokal gesicherte) Version zurückrollen (Neustart)")
                                .clicked()
                            {
                                revert_local = Some((ver.clone(), path.clone()));
                            }
                        });
                    }
                });
        }

        if let Some(ver) = install_version {
            self.start_install_download(ver);
        }
        if let Some(ver) = dl_version {
            self.start_rollback_download(ver);
        }
        if let Some((ver, path)) = revert_local {
            match crate::updater::revert_to(&path, &ver) {
                Ok(exe) => self.update_ready = Some((ver, exe)),
                Err(e) => self.error_msg = Some(format!("Zurückrollen: {}", e)),
            }
        }

        // ─── Shell integration (Windows) ───────────────────────────────
        #[cfg(windows)]
        {
            ui.add_space(12.0);
            ui.label(RichText::new("INTEGRATION").small().color(Color32::from_gray(140)));

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
                match crate::shell_register::set_context_menu(on) {
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
                Color32::from_gray(110),
                "Hinweis: Der Eintrag liegt unter „Weitere Optionen anzeigen“ (Win11).",
            );
        }
    }


}
