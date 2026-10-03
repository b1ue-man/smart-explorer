use super::*;

impl App {
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
        let previous = self.share_profiles.clone();
        let mut changed = false;
        for grant in &mut self.share_profiles.direct_grants {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} · {}", grant.device_name, grant.device_id));
                // Withdrawing a stored write flag remains possible while the
                // authorization is inactive. Enabling never readmits a peer.
                let editable =
                    grant.write || grant.state == crate::share::DirectGrantState::Accepted;
                changed |= ui
                    .add_enabled(
                        editable,
                        egui::Checkbox::new(&mut grant.write, "Darf schreiben"),
                    )
                    .changed();
                if grant.state != crate::share::DirectGrantState::Accepted {
                    ui.label("Freigabe inaktiv; zuerst ausdruecklich wieder erlauben.");
                }
            });
        }
        if self.share_profiles.direct_grants.is_empty() {
            ui.label("Noch keine Geraete fuer meine Freigaben zugelassen.");
        }
        if changed {
            let _ = self.commit_share_profiles(previous);
        }
    }
}
