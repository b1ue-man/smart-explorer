use super::*;

impl App {
    pub(in crate::app) fn ui_share(&mut self, ctx: &egui::Context) {
        // Existing sidebar, event and --open-page callers keep setting the
        // legacy tab. The settings window consumes that exact route next frame.
        self.settings.open = true;
        self.settings.page = crate::app::settings_navigation::for_share_tab(self.share_tab);
        self.show_share = false;
        ctx.request_repaint();
    }

    pub(in crate::app) fn open_share_request_count(&self) -> usize {
        pages_ui::open_request_count(&self.share_profiles)
    }

    pub(in crate::app) fn ui_share_settings_content(&mut self, ui: &mut egui::Ui) {
        use crate::app::settings_ui::SettingsPage;
        let page = crate::app::settings_navigation::canonical(self.settings.page);
        let valid = crate::app::settings_navigation::for_share_tab(self.share_tab) == page;
        if !valid {
            if let Some(tab) = crate::app::settings_navigation::default_share_tab(page) {
                self.share_tab = tab;
            }
        }
        self.ui_share_top(ui);
        let tabs: &[usize] = match page {
            SettingsPage::ThisDevice => &[pages_ui::TAB_THIS_DEVICE, pages_ui::TAB_EXPORTS],
            SettingsPage::Devices => &[
                pages_ui::TAB_DEVICES,
                pages_ui::TAB_REQUESTS,
                pages_ui::TAB_EXEC,
                pages_ui::TAB_SCREEN,
            ],
            SettingsPage::Rooms => &[pages_ui::TAB_ROOMS],
            SettingsPage::Connections => &[pages_ui::TAB_NETWORK],
            SettingsPage::Diagnostics => &[pages_ui::TAB_DIAGNOSTICS, pages_ui::TAB_TRASH],
            _ => &[],
        };
        let requests = pages_ui::open_request_count(&self.share_profiles);
        ui.horizontal_wrapped(|ui| {
            for &tab in tabs {
                if tab == pages_ui::TAB_TRASH && !crate::host_trash::available() {
                    continue;
                }
                let label = if tab == pages_ui::TAB_REQUESTS && requests > 0 {
                    format!("Anfragen ({requests})")
                } else {
                    pages_ui::tab_label(tab).to_string()
                };
                if ui.selectable_label(self.share_tab == tab, label).clicked() {
                    if self.share_tab == tab {
                        pages_ui::back(ui);
                    }
                    self.share_tab = tab;
                }
            }
        });
        ui.separator();
        match self.share_tab {
            pages_ui::TAB_DEVICES => device_page_ui::ui(self, ui),
            pages_ui::TAB_ROOMS => room_page_ui::ui(self, ui),
            pages_ui::TAB_EXPORTS => self.ui_share_exports(ui),
            pages_ui::TAB_NETWORK => {
                lan_ui::ui(self, ui);
                egui::CollapsingHeader::new(format!(
                    "Quick Share (LAN) · {} gefunden",
                    self.qs_devices.len()
                ))
                .id_salt("quickshare_devices")
                .show(ui, |ui| self.ui_quickshare_devices(ui));
            }
            pages_ui::TAB_TRASH => self.ui_share_host_trash(ui),
            pages_ui::TAB_REQUESTS => lifecycle_ui::ui_requests(self, ui),
            pages_ui::TAB_EXEC => exec_page(self, ui),
            pages_ui::TAB_SCREEN => crate::app::share_screen_tab_ui::ui(ui),
            pages_ui::TAB_THIS_DEVICE => {
                this_device_ui::ui(self, ui);
                self.ui_settings_share(ui);
                self.ui_share_request_policy(ui);
                crate::app::share_exec_ui::ui_host_accounts(ui);
            }
            _ => self.ui_share_diagnostics(ui),
        }
        // Navigation actions inside a legacy page still pick their original
        // tab, including diagnostics and pending requests.
        if crate::app::settings_navigation::for_share_tab(self.share_tab) != page {
            self.settings.page = crate::app::settings_navigation::for_share_tab(self.share_tab);
        }
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

/// Page „Befehle“: the overview of every Exec target, or one target's card
/// „Zugriff eines Geräts“ (the section „ZUGRIFF“ of the device page).
fn exec_page(app: &mut App, ui: &mut egui::Ui) {
    if let pages_ui::SharePage::ExecTarget(key) = pages_ui::current(ui, pages_ui::TAB_EXEC) {
        if pages_ui::back_row(ui, "Zugriff eines Geräts") {
            pages_ui::back(ui);
            return;
        }
        if let Some(view) = crate::share::resolve_exec_target(&app.share_profiles, &key) {
            let (device_id, public_key) = match &view.target {
                crate::share::ExecGrantTarget::Direct {
                    device_id,
                    public_key,
                    ..
                }
                | crate::share::ExecGrantTarget::RoomMember {
                    device_id,
                    public_key,
                    ..
                } => (device_id.clone(), public_key.clone()),
            };
            pages_ui::open(
                app,
                ui,
                pages_ui::TAB_DEVICES,
                pages_ui::SharePage::Device {
                    device_id,
                    public_key,
                },
            );
        } else {
            ui.label("Dieses Gerät ist nicht mehr freigegeben oder hat eine neue Identität.");
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
