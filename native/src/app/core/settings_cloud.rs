use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_menu_cloud(&mut self, ui: &mut egui::Ui) {
        use crate::cloud::Provider;
        let p = Provider::GDrive;
        ui.add_space(12.0);
        ui.label(
            RichText::new("CLOUD (GOOGLE DRIVE)")
                .small()
                .color(theme::muted(ui)),
        );
        let auth = crate::cloud::auth_status(p);
        ui.label(RichText::new(&auth.message).small());
        if let Some(error) = &auth.provider_error {
            ui.label(
                RichText::new(format!(
                    "OAuth: {}{}",
                    error.code,
                    error
                        .http_status
                        .map(|status| format!(" (HTTP {status})"))
                        .unwrap_or_default()
                ))
                .small(),
            );
        }
        if let Some(changed) = auth.client_changed_at {
            ui.label(
                RichText::new(format!(
                    "Client-ID zuletzt geändert: {}",
                    cloud_time(changed)
                ))
                .small()
                .color(theme::muted(ui)),
            );
        }
        if let Some(login) = auth.login_at {
            ui.label(
                RichText::new(format!("Angemeldet seit: {}", cloud_time(login)))
                    .small()
                    .color(theme::muted(ui)),
            );
        }
        if let Some(refresh) = auth.last_refresh_at {
            ui.label(
                RichText::new(format!(
                    "Letzte erfolgreiche Erneuerung: {}",
                    cloud_time(refresh)
                ))
                .small()
                .color(theme::muted(ui)),
            );
        }
        if auth.signed_in {
            ui.horizontal(|ui| {
                ui.colored_label(theme::accent(ui), "● Anmeldung gespeichert");
                if ui
                    .small_button("☁ Drive öffnen")
                    .on_hover_text("Google Drive durchsuchen")
                    .clicked()
                {
                    self.open_gdrive_browse();
                }
                if ui.small_button("Trennen").clicked() {
                    match crate::cloud::disconnect(p) {
                        Ok(()) => {
                            self.notice = Some((
                                "Google Drive getrennt".to_string(),
                                std::time::Instant::now(),
                            ));
                        }
                        Err(error) => {
                            self.error_msg = Some(format!("Google Drive trennen: {error}"));
                        }
                    }
                }
            });
        }
        let mut form = crate::app::settings_form::Form::new(
            ui,
            "gdrive_credentials",
            &["Client-ID", "Client-Secret"],
        );
        form.row_labelled(ui, "Client-ID", |ui, field_label| {
            ui.add(
                egui::TextEdit::singleline(&mut self.cloud_client_id_draft)
                    .hint_text("OAuth Client-ID (…apps.googleusercontent.com)")
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(field_label)
            .on_hover_text(
                "Aus DEINEM eigenen Google-Cloud-Projekt (Desktop-OAuth-Client). \
             Diese App ist kein Dienst — siehe Anleitung unten / docs/CLOUD_SETUP.md.",
            );
        });
        form.row_labelled(ui, "Client-Secret", |ui, field_label| {
            ui.add(
                egui::TextEdit::singleline(&mut self.cloud_secret_draft)
                    .hint_text("Client-Secret (von Google, falls vergeben)")
                    .password(true)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(field_label);
        });
        ui.horizontal(|ui| {
            if self.cloud_authing {
                ui.spinner();
                ui.label("Browser-Anmeldung läuft…");
            } else if ui
                .small_button(
                    if matches!(
                        auth.state,
                        crate::cloud::AuthState::ReauthorizeRequired
                            | crate::cloud::AuthState::ClientChanged
                    ) {
                        "Anmeldung erneuern"
                    } else {
                        "Mit Google verbinden"
                    },
                )
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
                    match crate::cloud::save_config(p, &cfg) {
                        Err(error) => {
                            self.error_msg =
                                Some(format!("Cloud-Konfiguration speichern: {error}"));
                        }
                        Ok(()) => {
                            let (tx, rx) = unbounded();
                            let spawn = std::thread::Builder::new()
                                .name("cloud-auth".into())
                                .spawn(move || {
                                    let _ = tx.send(crate::cloud::authorize(p).map(|_| ()));
                                });
                            match spawn {
                                Ok(_) => {
                                    self.cloud_auth_rx = Some(rx);
                                    self.cloud_authing = true;
                                    self.notice = Some((
                                        "Browser zur Google-Anmeldung geöffnet…".to_string(),
                                        std::time::Instant::now(),
                                    ));
                                }
                                Err(error) => {
                                    self.cloud_auth_rx = None;
                                    self.cloud_authing = false;
                                    self.error_msg = Some(format!(
                                        "Cloud-Anmeldung konnte nicht gestartet werden: {error}"
                                    ));
                                }
                            }
                        }
                    }
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
                    ui.label(RichText::new(line).small().color(theme::muted(ui)));
                }
                ui.hyperlink_to("→ Google Cloud Console öffnen", "https://console.cloud.google.com");
                ui.label(
                    RichText::new(
                        "Hinweis: Bei einem externen Google-Projekt im „Testing“-Status kann die Drive-Anmeldung nach sieben Tagen ablaufen. \
                         Der Veröffentlichungsstatus deines Projekts ist nicht bestätigt; sieben Tage allein beweisen die Ursache nicht. \
                         Prüfe den Status in der Google Cloud Console. Bei invalid_grant hier erneut anmelden. Details: docs/CLOUD_SETUP.md.",
                    )
                    .small()
                    .color(theme::muted(ui)),
                );
            });
        ui.separator();
    }
}

fn cloud_time(at: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(at, 0)
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Zeitpunkt nicht darstellbar".into())
}
