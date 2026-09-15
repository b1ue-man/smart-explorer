//! "LAN" tab of the Share view: local-network presence of paired devices,
//! link classification, and (Stage 2) automatic uplink sharing.
use crate::app::theme;
use super::*;

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui) {
    let status = app.share_lan_status.clone();
    ui.label(
        RichText::new("GEKOPPELTE GERAETE IM LOKALEN NETZ")
            .small()
            .color(theme::muted(ui)),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Erkennung (mDNS): {}", status.presence.label()));
        if let Some(id) = &status.announced_id {
            ui.label(
                RichText::new(format!("eigene Kennung {id}"))
                    .small()
                    .color(theme::muted(ui)),
            );
        }
    });
    let mut presence_enabled = matches!(
        status.presence,
        crate::share::LanFacility::Available
            | crate::share::LanFacility::Starting
            | crate::share::LanFacility::Unavailable(_)
    );
    if ui
        .checkbox(&mut presence_enabled, "Gekoppelte Geraete im lokalen Netz automatisch finden")
        .on_hover_text(
            "Kuendigt die eigene Iroh-Adresse per mDNS an und findet gekoppelte Geraete ohne Share-Server, z. B. ueber ein direktes Kabel. Fremde Geraete bleiben ausgeschlossen.",
        )
        .changed()
    {
        app.set_lan_presence_enabled(presence_enabled);
    }
    if status.peers.is_empty() {
        ui.label("Kein gekoppeltes Geraet im lokalen Netz sichtbar.");
    } else {
        for peer in &status.peers {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&peer.display_name).strong());
                    ui.label(match peer.uplink {
                        Some(true) => "hat eigenen Internetzugang",
                        Some(false) => "ohne Internetzugang",
                        None => "Internetzugang unbekannt",
                    });
                });
                ui.label(format!("Adressen: {}", peer.candidates.join(", ")));
                if ui.button("Oeffnen").clicked() {
                    app.open_share_target(crate::share::PeerOpenTarget::Direct {
                        contact_id: peer.contact_id.clone(),
                    });
                }
            });
        }
    }
    if status.unknown_devices > 0 {
        ui.label(
            RichText::new(format!(
                "{} weitere Smart-Explorer-Geraete sichtbar, aber nicht gekoppelt",
                status.unknown_devices
            ))
            .small()
            .color(theme::muted(ui)),
        );
    }

    ui.separator();
    ui.label(
        RichText::new("NETZWERKVERBINDUNGEN")
            .small()
            .color(theme::muted(ui)),
    );
    if let Some(error) = &status.links_error {
        ui.colored_label(
            theme::danger(ui),
            format!("Schnittstellen nicht lesbar: {error}"),
        );
    }
    if status.links.is_empty() && status.links_error.is_none() {
        ui.label("Keine aktiven Netzwerkverbindungen gemeldet.");
    }
    for link in &status.links {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&link.name).strong());
            ui.label(&link.class);
            if link.peer_present {
                ui.colored_label(theme::accent(ui), "gekoppeltes Geraet hier");
            }
            ui.label(
                RichText::new(link.addrs.join(", "))
                    .small()
                    .color(theme::muted(ui)),
            );
        });
    }

    ui.separator();
    super::lan_uplink_ui::ui(app, ui, &status.uplink);
}

impl App {
    pub(in crate::app) fn set_lan_presence_enabled(&mut self, enabled: bool) {
        match crate::share::LanSettings::update(|settings| settings.presence_enabled = enabled) {
            Ok(_) => {
                self.share_lan_notice = Some(if enabled {
                    "LAN-Erkennung eingeschaltet".into()
                } else {
                    "LAN-Erkennung ausgeschaltet".into()
                });
                let _ = self.configure_share_service();
                self.share_next_poll_at = Instant::now();
            }
            Err(error) => self.error_msg = Some(format!("LAN-Einstellung speichern: {error}")),
        }
    }
}
