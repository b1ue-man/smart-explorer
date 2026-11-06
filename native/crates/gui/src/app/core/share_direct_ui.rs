//! „Per Direkt-Code des anderen Geraets“: adding a direct contact from its
//! code, with the choice to open my exports for it as well.
use super::*;

impl App {
    /// „Per Direkt-Code des anderen Geraets“ on the page „Geraet hinzufuegen“.
    pub(super) fn ui_share_add_device(&mut self, ui: &mut egui::Ui) {
        // R1 (plan 2026-11-06): connecting sets up both directions unless
        // the user chooses „nur in eine Richtung“ (stored as that choice).
        let choice_id = egui::Id::new("share_direct_code_one_way");
        let mut one_way = ui.data_mut(|data| data.get_temp::<bool>(choice_id).unwrap_or_default());
        if ui
            .checkbox(
                &mut one_way,
                "Nur in eine Richtung (meine Freigaben fuer dieses Geraet nicht oeffnen)",
            )
            .changed()
        {
            ui.data_mut(|data| data.insert_temp(choice_id, one_way));
        }
        let share_back = !one_way;
        ui.label(RichText::new("Direkt-Code").small().color(theme::muted(ui)));
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_direct_code_input)
                    .hint_text("SE-D3-...")
                    .desired_width(share_input_width(ui, 360.0))
                    .clip_text(true),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.share_direct_name_input)
                    .hint_text("Name")
                    .desired_width(140.0),
            );
            if ui.button("Einfuegen").clicked() {
                self.notice = Some((
                    "Bitte mit Strg+V in das Code-Feld einfuegen".to_string(),
                    std::time::Instant::now(),
                ));
            }
            if ui.button("Hinzufuegen").clicked() {
                match crate::share::ShareProfiles::add_direct_from_code_persisted(
                    Some(dirs_home().to_string_lossy().replace('\\', "/")),
                    &self.share_direct_code_input,
                    &self.share_direct_name_input,
                ) {
                    Ok((profiles, id)) => {
                        self.share_profiles = profiles;
                        match crate::share::set_direct_share_back(
                            Some(dirs_home().to_string_lossy().replace('\\', "/")),
                            &id,
                            share_back,
                        ) {
                            Ok(change) => self.share_profiles = change.profiles,
                            Err(error) => {
                                self.error_msg = Some(error);
                                return;
                            }
                        }
                        self.share_direct_code_input.clear();
                        self.share_direct_name_input.clear();
                        ui.data_mut(|data| data.insert_temp(choice_id, false));
                        let _ = lifecycle_ui::queue_contact(self, &id);
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if ui.button("Leeren").clicked() {
                self.share_direct_code_input.clear();
                self.share_direct_name_input.clear();
                ui.data_mut(|data| data.insert_temp(choice_id, false));
            }
        });
    }
}
