use super::prelude::*;
use super::ui_preferences::ColorMode;
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SettingsPage {
    #[default]
    Appearance,
    Connections,
    Updates,
    Storage,
    Integration,
}

#[derive(Default)]
pub(super) struct SettingsState {
    pub(super) open: bool,
    pub(super) page: SettingsPage,
}

impl App {
    pub(super) fn open_settings(&mut self, page: SettingsPage) {
        self.settings.open = true;
        self.settings.page = page;
    }

    pub(in crate::app) fn ui_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings.open;
        let bounds = ctx.screen_rect().shrink(16.0);
        egui::Window::new("Einstellungen")
            .open(&mut open).collapsible(false)
            .default_size([640.0, 400.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(bounds)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (page, label) in [
                        (SettingsPage::Appearance, "Darstellung"),
                        (SettingsPage::Connections, "Verbindungen"),
                        (SettingsPage::Updates, "Updates"),
                        (SettingsPage::Storage, "Suche & Daten"),
                        (SettingsPage::Integration, "Integration"),
                    ] {
                        ui.selectable_value(&mut self.settings.page, page, label);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt(("settings_page", self.settings.page as u8))
                    .auto_shrink([false, false]).show(ui, |ui| {
                        match self.settings.page {
                            SettingsPage::Appearance => self.ui_settings_appearance(ui),
                            SettingsPage::Connections => {
                                self.ui_settings_share(ui);
                                egui::CollapsingHeader::new("Google Drive").id_salt("settings_cloud_v2")
                                    .show(ui, |ui| self.ui_menu_cloud(ui));
                            }
                            SettingsPage::Updates => self.ui_settings_updates(ui),
                            SettingsPage::Storage => {
                                self.ui_settings_search(ui);
                                ui.separator();
                                self.ui_temp_recovery(ui);
                            }
                            SettingsPage::Integration => {
                                self.ui_settings_integration(ui);
                                self.ui_settings_background(ui);
                            }
                        }
                    });
            });
        self.settings.open = open;
    }

    fn ui_settings_appearance(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Farbschema");
        let previous = self.appearance;
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.appearance.mode, ColorMode::Light, "Hell");
            ui.radio_value(&mut self.appearance.mode, ColorMode::Dark, "Dunkel");
            ui.radio_value(&mut self.appearance.mode, ColorMode::System, "Wie im System");
        });
        ui.label(RichText::new("Wird sofort auf alle Ansichten angewendet.").color(theme::muted(ui)));
        theme::section(ui, "Dateiliste");
        ui.checkbox(&mut self.appearance.compact, "Kompakte Zeilen");
        ui.checkbox(&mut self.appearance.detailed_columns, "Pfad, Erstelldatum und Tiefe zusätzlich anzeigen");
        ui.label(RichText::new("Bei einer Suche in Unterordnern erscheint der Pfad automatisch.").color(theme::muted(ui)));
        if self.appearance != previous {
            ui.ctx().set_theme(self.appearance.mode.preference());
            self.save_ui_state();
        }
    }

    fn ui_settings_search(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Ordnersuche");
        ui.label("Der Index findet Ordner über die Suchleiste. Mit / beginnend suchen.");
        ui.horizontal_wrapped(|ui| {
            if self.index_building {
                ui.spinner();
                ui.label(format!("{} Ordner erfasst", self.index_progress));
                if ui.button("Abbrechen").clicked() { self.cancel_index_build(); }
            } else {
                ui.label(format!("{} Ordner im Index", self.folder_index.len()));
                if ui.button(if self.folder_index.is_empty() { "Index erstellen" } else { "Index aktualisieren" }).clicked() {
                    self.start_index_build();
                }
            }
        });
    }
}
