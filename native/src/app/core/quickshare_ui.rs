use super::prelude::*;
use super::*;

impl App {
    /// Keep Android Quick Share discovery scoped to the visible Share window.
    /// A failed start is retained until the user retries, avoiding an error
    /// storm from the per-frame drain loop.
    pub(in crate::app) fn drain_quickshare(&mut self) {
        if !self.show_share {
            self.quickshare = None;
            self.qs_devices.clear();
            self.quickshare_error = None;
            return;
        }
        if self.quickshare.is_none() && self.quickshare_error.is_none() {
            let name = if self.share_device_draft.trim().is_empty() {
                default_device_name()
            } else {
                self.share_device_draft.trim().to_string()
            };
            match crate::quickshare::QuickShare::start(&name) {
                Ok(discovery) => self.quickshare = Some(discovery),
                Err(error) => {
                    let detail = format!("Quick Share (LAN): {error}");
                    self.quickshare_error = Some(detail.clone());
                    self.push_app_error("Quick Share", detail);
                }
            }
        }
        if let Some(discovery) = &self.quickshare {
            for devices in discovery.events.try_iter() {
                self.qs_devices = devices;
            }
        }
    }

    pub(in crate::app) fn ui_quickshare_devices(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = self.quickshare_error.clone() {
            ui.colored_label(Color32::from_rgb(255, 150, 120), error);
            if ui.button("LAN-Suche erneut versuchen").clicked() {
                self.quickshare_error = None;
            }
        } else if self.quickshare.is_none() {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label("LAN-Suche wird gestartet …");
        } else if self.qs_devices.is_empty() {
            ui.colored_label(
                Color32::from_gray(140),
                "Suche … Auf Android Quick Share fuer andere Geraete sichtbar machen.",
            );
        }
        for device in &self.qs_devices {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{}  {}", device.name, device.addr));
                ui.add_enabled(false, egui::Button::new("Dateien senden"))
                    .on_disabled_hover_text(
                        "Quick-Share-Dateiuebertragung benoetigt noch UKEY2/Protobuf; die LAN-Erkennung ist aktiv.",
                    );
            });
        }
        ui.label(
            RichText::new(
                "Die LAN-Erkennung ist funktionsfaehig. Dateiuebertragung zu Android Quick Share ist noch nicht implementiert; fuer Geraete mit Smart Explorer oben Direkt oder Raum verwenden.",
            )
            .small()
            .color(Color32::from_gray(120)),
        );
    }
}
