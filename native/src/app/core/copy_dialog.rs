use super::prelude::*;
use super::*;
use crate::app::theme;

impl App {
    /// "Kopieren/Verschieben nach…": closes once the transfer started; its
    /// progress is in the transfer list, and several may run at once.
    pub(in crate::app) fn ui_copy_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let title = if self.copy_mode_pending == CopyMode::Copy {
            "Kopieren"
        } else {
            "Verschieben"
        };

        egui::Window::new(title)
            .default_size([600.0, 380.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .vscroll(true)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{} Einträge ausgewählt", self.selection.len()));
                ui.horizontal_wrapped(|ui| {
                    ui.label("Modus:");
                    ui.radio_value(&mut self.copy_mode_pending, CopyMode::Copy, "kopieren");
                    ui.radio_value(&mut self.copy_mode_pending, CopyMode::Move, "verschieben");
                });
                ui.colored_label(
                    theme::muted(ui),
                    "Es werden nur Dateien übernommen, die zum aktuellen Filter passen.",
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Ziel:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.copy_dest)
                            .desired_width((ui.available_width() - 100.0).max(160.0))
                            .hint_text("Zielordner…"),
                    );
                    if ui.button("Wählen…").clicked() {
                        let init = self.copy_dest.clone();
                        self.open_picker(PickerPurpose::CopyDest, &init);
                    }
                });
                let required_structure = self.recursive && self.filter_is_active();
                if required_structure {
                    self.copy_preserve = true;
                }
                ui.add_enabled(
                    !required_structure,
                    egui::Checkbox::new(
                        &mut self.copy_preserve,
                        "Ordnerstruktur erhalten (leere Ordner werden weggelassen)",
                    ),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Bei Konflikt:");
                    ui.radio_value(&mut self.copy_conflict, Conflict::Rename, "umbenennen");
                    ui.radio_value(
                        &mut self.copy_conflict,
                        Conflict::Overwrite,
                        "überschreiben",
                    );
                    ui.radio_value(&mut self.copy_conflict, Conflict::Skip, "überspringen");
                });
                ui.colored_label(
                    theme::muted(ui),
                    "Der Fortschritt steht danach unter „⇅ Übertragungen“.",
                );

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.copy_dest.trim().is_empty(),
                                egui::Button::new(RichText::new(title).strong()),
                            )
                            .clicked()
                        {
                            self.confirm_copy();
                        }
                        if ui.button("Schließen").clicked() {
                            close = true;
                        }
                    });
                });
            });

        if close {
            self.copy_open = false;
        }
    }
}
