use super::*;

impl App {
    pub(in crate::app) fn ui_share(&mut self, ctx: &egui::Context) {
        let mut open = self.show_share;
        let screen = ctx.screen_rect();
        let limit = theme::window_content_limit(ctx);
        let (max_w, max_h) = (limit.x, limit.y);
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
                let requests = pages_ui::open_request_count(&self.share_profiles);
                ui.horizontal_wrapped(|ui| {
                    for tab in pages_ui::TAB_ORDER {
                        if tab == pages_ui::TAB_TRASH && !crate::host_trash::available() {
                            continue;
                        }
                        let label = match (tab, requests) {
                            (pages_ui::TAB_REQUESTS, count) if count > 0 => {
                                format!("{} ({count})", pages_ui::tab_label(tab))
                            }
                            _ => pages_ui::tab_label(tab).to_string(),
                        };
                        if ui.selectable_label(self.share_tab == tab, label).clicked() {
                            if self.share_tab == tab {
                                // A click on the open tab returns to its overview.
                                pages_ui::back(ui);
                            }
                            self.share_tab = tab;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("share_page", self.share_tab))
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.share_tab {
                        pages_ui::TAB_DEVICES => device_page_ui::ui(self, ui),
                        pages_ui::TAB_ROOMS => room_page_ui::ui(self, ui),
                        pages_ui::TAB_EXPORTS => self.ui_share_exports(ui),
                        pages_ui::TAB_NETWORK => {
                            lan_ui::ui(self, ui);
                            ui.separator();
                            egui::CollapsingHeader::new(format!(
                                "Quick Share (LAN) — {} gefunden",
                                self.qs_devices.len()
                            ))
                            .id_salt("quickshare_devices")
                            .show(ui, |ui| self.ui_quickshare_devices(ui));
                        }
                        pages_ui::TAB_TRASH => self.ui_share_host_trash(ui),
                        pages_ui::TAB_REQUESTS => {
                            self.ui_share_request_policy(ui);
                            ui.separator();
                            lifecycle_ui::ui_requests(self, ui);
                        }
                        pages_ui::TAB_EXEC => exec_page(self, ui),
                        pages_ui::TAB_THIS_DEVICE => this_device_ui::ui(self, ui),
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
                if ui.button("Verbinden").clicked() {
                    let _ = self.ensure_share();
                    ui.close_menu();
                }
                if ui.button("Trennen").clicked() {
                    let _ = self.share_cmd(crate::share::ShareCmd::Stop);
                    ui.close_menu();
                }
                if ui.button("Server & Gerätename einstellen…").clicked() {
                    self.open_settings(settings_ui::SettingsPage::Connections);
                    ui.close_menu();
                }
            });
        });
        ui.add(egui::Label::new(RichText::new(&self.share_status).color(theme::muted(ui))).wrap());
    }
}

/// Page „Befehle“: the overview of every Exec target, or one target's card.
fn exec_page(app: &mut App, ui: &mut egui::Ui) {
    if let pages_ui::SharePage::ExecTarget(key) = pages_ui::current(ui, pages_ui::TAB_EXEC) {
        if pages_ui::back_row(ui, "Befehle eines Geraets") {
            pages_ui::back(ui);
            return;
        }
        if !crate::app::share_exec_ui::ui_exec_target(app, ui, &key) {
            ui.label("Dieses Geraet ist nicht mehr freigegeben oder hat eine neue Identitaet.");
        }
        return;
    }
    if let Some(key) = crate::app::share_exec_ui::ui_exec_overview(app, ui) {
        pages_ui::open(
            app,
            ui,
            pages_ui::TAB_EXEC,
            pages_ui::SharePage::ExecTarget(key),
        );
    }
}
