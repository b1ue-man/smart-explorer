use super::*;

impl App {
    pub(super) fn ui_share_diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Server testen").clicked() {
                let _ = self.ensure_share();
            }
            if ui.button("Presence neu senden").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Direct Watches neu abonnieren").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Raeume neu beitreten").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Alle Peers pruefen").clicked() {
                self.append_share_diag("Peer-Pruefung ueber Oeffnen/Diagnose pro Geraet");
            }
            if ui.button("Aktiven Peer pruefen").clicked() {
                self.append_share_diag("Aktiver Peer: Root-Probe laeuft beim Oeffnen");
            }
            if ui.button("Log kopieren").clicked() {
                ui.ctx().copy_text(self.share_diag_log.clone());
            }
            if ui.button("Security-Details anzeigen").clicked() {
                if let Some(identity) = &self.share_identity {
                    self.append_share_diag(format!(
                        "device_id={}\nnode_id={}\nfingerprint={}\niroh=aktiv wenn verbunden\nrelay={}\nkandidaten={:?}\n",
                        identity.device_id,
                        identity.node_id,
                        identity.fingerprint,
                        self.share_worker_relay_url,
                        self.share_worker_candidates
                    ));
                } else {
                    self.append_share_diag(
                        self.share_identity_error
                            .clone()
                            .unwrap_or_else(|| "Share-Identitaet nicht verfuegbar".into()),
                    );
                }
            }
        });
        ui.separator();
        ui.label(format!(
            "Listener: {}",
            if self.share_worker_running {
                "aktiv"
            } else {
                "inaktiv"
            }
        ));
        if !self.share_worker_relay_url.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Iroh-Relay:");
                share_value_field(ui, &self.share_worker_relay_url);
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Signaling:");
            ui.add(egui::Label::new(self.share_status.clone()).wrap());
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("SmartExplorer-Fingerprint:");
            let fingerprint = self
                .share_identity
                .as_ref()
                .map(|identity| identity.fingerprint.as_str())
                .unwrap_or("nicht verfuegbar");
            share_value_field(ui, fingerprint);
        });
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(self.share_diag_log.as_str())
                            .monospace()
                            .color(theme::muted(ui)),
                    )
                    .wrap(),
                );
            });
    }
}
