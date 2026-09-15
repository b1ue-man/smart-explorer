//! Bounded, selectable error report with actions outside the scrolling text.
use super::{theme, App};
use eframe::egui;

impl App {
    pub(in crate::app) fn ui_errors_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.show_errors_dialog;
        let mut close = false;
        let mut clear_app_log = false;
        let report = self.error_log_text();
        let scan_errors = self.progress.errors.max(self.failed_paths.len() as u64) as usize;
        let app_errors = self.app_errors.len()
            + usize::from(self.error_msg.as_ref().is_some_and(|current| {
                !self.app_errors.iter().any(|entry| entry.detail == *current)
            }));
        egui::Window::new(format!("Fehler-Protokoll ({})", scan_errors + app_errors))
            .id(egui::Id::new("error_log_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([700.0, 480.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Vollständige Fehler mit Version, Quelle und betroffenem Pfad.");
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(!report.is_empty(), egui::Button::new("Alles kopieren")).clicked() {
                        ctx.copy_text(report.clone());
                    }
                    if app_errors > 0 && ui.button("App-Protokoll leeren").clicked() {
                        clear_app_log = true;
                    }
                    if ui.button("Schließen").clicked() { close = true; }
                });
                ui.separator();
                if report.is_empty() {
                    ui.colored_label(theme::muted(ui), "Keine Fehler protokolliert.");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("error_report_text")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let mut text = report.as_str();
                            ui.add(egui::TextEdit::multiline(&mut text)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(ui.available_width())
                                .desired_rows(1)
                                .frame(false));
                        });
                }
            });
        if clear_app_log {
            self.app_errors.clear();
            self.error_msg = None;
            self.last_logged_error = None;
        }
        self.show_errors_dialog = open && !close;
    }
}
