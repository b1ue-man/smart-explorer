use super::*;

impl App {
    /// „Neue Direktanfragen“: always ask, or accept requests carrying this
    /// device's Direct code automatically (`share_direct_policy.json`, read
    /// at each decision; CLI `se share request policy`, Android
    /// `share.setPolicy`).
    pub(super) fn ui_share_request_policy(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("NEUE DIREKTANFRAGEN")
                .small()
                .color(theme::muted(ui)),
        );
        let loaded = crate::share::DirectRequestPolicy::load();
        if let Err(error) = &loaded {
            ui.colored_label(
                theme::danger(ui),
                format!("Richtlinie nicht lesbar, es gilt „immer fragen“: {error}"),
            );
        }
        let current = loaded.unwrap_or_default();
        let mut chosen = current;
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(
                &mut chosen,
                crate::share::DirectRequestPolicy::Ask,
                "Immer fragen",
            );
            ui.radio_value(
                &mut chosen,
                crate::share::DirectRequestPolicy::AutoAccept,
                "Anfragen mit meinem Direkt-Code automatisch annehmen (unsicherer)",
            );
        });
        if chosen == crate::share::DirectRequestPolicy::AutoAccept {
            ui.label("Rechte der Geräte, die automatisch angenommen werden:");
            crate::app::share_rights_choice_ui::code_row(self, ui);
            ui.colored_label(
                theme::warning(ui),
                crate::app::share_rights_model::auto_accept_text(
                    self.share_profiles.code_rights.as_ref(),
                ),
            );
        }
        if chosen != current {
            match chosen.save() {
                Ok(()) => {
                    self.notice =
                        Some(("Anfrageregel gespeichert".into(), std::time::Instant::now()));
                }
                Err(error) => self.error_msg = Some(error),
            }
        }
    }

    pub(super) fn ui_share_write_rights(&mut self, ui: &mut egui::Ui) {
        ui.label("Schreibrecht je Geraet fuer meine Freigaben");
        match crate::share::DirectRequestPolicy::load() {
            Ok(crate::share::DirectRequestPolicy::Ask) => {
                ui.label("Neue Direktanfragen: immer fragen.");
            }
            Ok(crate::share::DirectRequestPolicy::AutoAccept) => {
                ui.label(
                    "Neue Direktanfragen: automatisch annehmen (bestehende Geraeteeinstellung).",
                );
            }
            Err(error) => {
                ui.label(format!(
                    "Neue Direktanfragen: immer fragen; Richtlinie nicht lesbar: {error}"
                ));
            }
        }
        let mut device = None;
        for grant in &self.share_profiles.direct_grants {
            ui.horizontal_wrapped(|ui| {
                ui.label(&grant.device_name);
                ui.label(if grant.write {
                    "Schreibrecht erlaubt"
                } else {
                    "Nur lesen"
                });
                if ui.button("Geräteeinstellungen …").clicked() {
                    device = Some(grant.device_id.clone());
                }
            });
        }
        if self.share_profiles.direct_grants.is_empty() {
            ui.label("Noch keine Geräte für meine Freigaben zugelassen.");
        }
        if let Some(device) = device {
            pages_ui::open(
                self,
                ui,
                pages_ui::TAB_DEVICES,
                pages_ui::SharePage::Grant(device),
            );
        }
    }
}
