use super::prelude::*;
use super::*;

impl App {
    pub(super) fn ui_settings_updates(&mut self, ui: &mut egui::Ui) {
        // ─── Update ───────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(
            RichText::new("App-Updates")
                .small()
                .color(theme::muted(ui)),
        );
        ui.colored_label(
            theme::muted(ui),
            format!("Version {}", env!("CARGO_PKG_VERSION")),
        );
        if !self.show_update_dialog {
            if let Some(ReadyUpdate::Staged(bundle)) = self.update_ready.as_ref() {
                let version = bundle.version().to_string();
                if ui
                    .small_button(format!("Gestagtes Update v{version} anzeigen"))
                    .clicked()
                {
                    self.show_update_dialog = true;
                }
            }
        }
        if ui.button("Jetzt nach Updates suchen").clicked() { self.check_updates_manual(); }
        egui::CollapsingHeader::new("Updatequelle ändern")
            .id_salt("settings_update_source_v2").show(ui, |ui| {
        let update_payload = update_payload_name();
        ui.add(
            egui::TextEdit::singleline(&mut self.update_feed_draft)
                .hint_text("Feed-Ordner oder Git/HTTPS-URL…")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(format!(
            "Quelle mit version.txt und {update_payload}. Entweder ein Ordner \
             (lokal/Netzlaufwerk) ODER eine https-URL bzw. ein GitHub-Repo-Link \
             (z. B. https://github.com/b1ue-man/smart-explorer) — dann updatet \
             sich die App direkt aus dem Git. Beim Start wird automatisch geprüft."
        ));
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
        });

            });

        // Rollback to a previously-installed version + pause/resume auto-update.
        if let Some(pinned) = crate::updater::pinned_version() {
            ui.colored_label(
                theme::warning(ui),
                format!("⏸ Auto-Update pausiert (zurückgerollt auf v{})", pinned),
            );
            if ui.small_button("Auf neueste aktualisieren").clicked() {
                let (tx, rx) = unbounded();
                match crate::updater::update_to_latest_async(tx) {
                    Ok(()) => {
                        self.update_rx = Some(rx);
                        self.notice = Some((
                            "Suche neueste Version…".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(error) => {
                        self.update_rx = None;
                        self.error_msg =
                            Some(format!("Update konnte nicht gestartet werden: {error}"));
                    }
                }
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
                theme::success(ui),
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

        egui::CollapsingHeader::new("Frühere Version wiederherstellen")
            .id_salt("settings_rollback_v2").show(ui, |ui| {
        ui.label(
            RichText::new("Frühere Versionen (Releases)")
                .small()
                .color(theme::muted(ui)),
        );
        if self.remote_versions_rx.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label(
                    RichText::new("lade Release-Liste…")
                        .small()
                        .color(theme::muted(ui)),
                );
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        } else if let Some(list) = self.remote_versions.clone() {
            // Only OLDER versions are rollback targets; a newer one is offered as
            // an update by the banner above.
            let list: Vec<String> = list
                .into_iter()
                .filter(|v| v != current && !crate::updater::is_newer(v, current))
                .collect();
            if list.is_empty() {
                ui.colored_label(
                    theme::muted(ui),
                    "(keine — Feed ist kein GitHub-Repo, oder offline)",
                );
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
                ui.label(
                    RichText::new("lade Version…")
                        .small()
                        .color(theme::muted(ui)),
                );
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }

        // Locally-archived binaries (instant; also lets you go forward again
        // after a rollback, and works offline).
        let archived: Vec<(String, PathBuf)> = crate::updater::list_archived_versions()
            .into_iter()
            .filter(|(v, _)| v != current)
            .collect();
        if !archived.is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new("Lokal gesichert")
                    .small()
                    .color(theme::muted(ui)),
            );
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
                                .on_hover_text(
                                    "Auf diese (lokal gesicherte) Version zurückrollen (Neustart)",
                                )
                                .clicked()
                            {
                                revert_local = Some((ver.clone(), path.clone()));
                            }
                        });
                    }
                });
        }

            });

        if let Some(ver) = install_version {
            self.start_install_download(ver);
        }
        if let Some(ver) = dl_version {
            self.start_rollback_download(ver);
        }
        if let Some((ver, path)) = revert_local {
            match crate::updater::revert_to(&path, &ver) {
                Ok(executable) => {
                    self.update_ready = Some(ReadyUpdate::InstalledRollback {
                        version: ver,
                        executable,
                    });
                    self.show_update_dialog = true;
                }
                Err(e) => self.error_msg = Some(format!("Zurückrollen: {}", e)),
            }
        }

    }
}
