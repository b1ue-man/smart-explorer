use crate::app::theme;
use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_copy_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let running = self.copy_handle.is_some() || self.copy_rx.is_some();
        let displayed_mode = if running {
            self.copy_active_mode.unwrap_or(self.copy_mode_pending)
        } else {
            self.copy_mode_pending
        };
        let title = if displayed_mode == CopyMode::Copy {
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
                ui.add_enabled_ui(!running, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Modus:");
                        ui.radio_value(&mut self.copy_mode_pending, CopyMode::Copy, "kopieren");
                        ui.radio_value(
                            &mut self.copy_mode_pending,
                            CopyMode::Move,
                            "verschieben",
                        );
                    });
                });
                ui.colored_label(
                    theme::muted(ui),
                    "Es werden nur Dateien übernommen, die zum aktuellen Filter passen.",
                );
                ui.add_space(6.0);
                ui.add_enabled_ui(!running, |ui| {
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
                    ui.add_enabled(!required_structure, egui::Checkbox::new(
                        &mut self.copy_preserve,
                        "Ordnerstruktur erhalten (leere Ordner werden weggelassen)",
                    ));
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Bei Konflikt:");
                        ui.radio_value(&mut self.copy_conflict, Conflict::Rename, "umbenennen");
                        ui.radio_value(
                            &mut self.copy_conflict,
                            Conflict::Overwrite,
                            "überschreiben",
                        );
                        ui.radio_value(
                            &mut self.copy_conflict,
                            Conflict::Skip,
                            "überspringen",
                        );
                    });
                });

                if let Some(p) = self.copy_progress.as_ref().filter(|_| running) {
                    let fraction = if p.bytes_total > 0 {
                        p.bytes_done as f32 / p.bytes_total as f32
                    } else if p.files_total > 0 {
                        p.files_done as f32 / p.files_total as f32
                    } else {
                        0.0
                    };
                    ui.add(egui::ProgressBar::new(fraction).show_percentage());
                    ui.label(format!(
                        "{}/{} Dateien · {} / {} · {:.1}s{}",
                        p.files_done,
                        p.files_total,
                        format_bytes(p.bytes_done),
                        format_bytes(p.bytes_total),
                        p.elapsed_ms as f64 / 1000.0,
                        if p.errors > 0 {
                            format!(" · {} Fehler", p.errors)
                        } else {
                            String::new()
                        },
                    ));
                }

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.copy_dest.is_empty() && !running,
                                egui::Button::new(RichText::new(if displayed_mode == CopyMode::Copy { "Kopieren" } else { "Verschieben" }).strong()),
                            )
                            .clicked()
                        {
                            self.confirm_copy();
                        }
                        let cancel_label = if running {
                            "Vorgang abbrechen"
                        } else {
                            "Schließen"
                        };
                        if ui.button(cancel_label).clicked() {
                            if running {
                                self.cancel_copy_job();
                            } else {
                                close = true;
                            }
                        }
                    });
                });
            });

        if close {
            self.copy_open = false;
        }
    }
}
