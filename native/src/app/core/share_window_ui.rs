use super::*;

impl App {
    pub(in crate::app) fn ui_share(&mut self, ctx: &egui::Context) {
        let mut open = self.show_share;
        let screen = ctx.screen_rect();
        let max_w = (screen.width() - 16.0)
            .max(240.0)
            .min(screen.width().max(1.0));
        let max_h = (screen.height() - 16.0)
            .max(240.0)
            .min(screen.height().max(1.0));
        egui::Window::new("Geräte & Freigaben")
            .open(&mut open)
            .resizable(true)
            .default_size([760.0_f32.min(max_w), 640.0_f32.min(max_h)])
            .max_width(max_w)
            .max_height(max_h)
            .constrain_to(screen.shrink(8.0))
            .show(ctx, |ui| {
                ui.set_max_width(max_w - 16.0);
                self.ui_share_top(ui);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for (i, label) in ["Geräte", "Räume", "Freigaben", "Diagnose", "Netzwerk"]
                        .iter()
                        .enumerate()
                    {
                        if ui.selectable_label(self.share_tab == i, *label).clicked() {
                            self.share_tab = i;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("share_page", self.share_tab))
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.share_tab {
                        0 => self.ui_share_direct(ui),
                        1 => self.ui_share_rooms(ui),
                        2 => self.ui_share_exports(ui),
                        4 => lan_ui::ui(self, ui),
                        _ => self.ui_share_diagnostics(ui),
                    });
            });
        self.show_share = open;
    }

    pub(super) fn ui_share_top(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Aktualisieren").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            ui.menu_button("Verbindung", |ui| {
                ui.label("Share-Server");
                share_value_field(ui, &self.share_server);
                if ui.button("Verbinden").clicked() { let _ = self.ensure_share(); ui.close_menu(); }
                if ui.button("Trennen").clicked() { let _ = self.share_cmd(crate::share::ShareCmd::Stop); ui.close_menu(); }
                if ui.button("Server & Gerätename einstellen…").clicked() {
                    self.open_settings(settings_ui::SettingsPage::Connections);
                    ui.close_menu();
                }
            });
        });
        ui.add(egui::Label::new(RichText::new(&self.share_status).color(theme::muted(ui))).wrap());
    }
}
