//! Uplink-sharing section of the LAN tab (Stage 2): the opt-in toggle with the
//! one-time platform setup, and the live sharing state with its reason.
use crate::app::theme;
use super::*;

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui, view: &crate::share::UplinkView) {
    ui.label(
        RichText::new("INTERNET TEILEN (NUR OHNE ROUTER)")
            .small()
            .color(theme::muted(ui)),
    );
    ui.label(format!("Plattform: {}", view.facility.label()));
    let mut enabled = view.enabled;
    if ui
        .checkbox(
            &mut enabled,
            "Internet automatisch an gekoppelte Geraete teilen, wenn sie ohne Router direkt verbunden sind",
        )
        .on_hover_text(
            "Einmalige Freigabe (Windows-UAC bzw. polkit); danach teilt dieses Geraet seinen Internetzugang automatisch, sobald ein gekoppeltes Geraet ohne eigenen Internetzugang direkt angeschlossen ist. Beendet sich, sobald das Geraet verschwindet.",
        )
        .changed()
    {
        app.set_lan_uplink_sharing_enabled(enabled);
    }
    let state_label = match view.state {
        crate::share::UplinkSharingState::Idle => "inaktiv",
        crate::share::UplinkSharingState::Starting => "wird eingerichtet",
        crate::share::UplinkSharingState::Sharing => "teilt Internet",
        crate::share::UplinkSharingState::Stopping => "wird beendet",
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Status: {state_label}"));
        if !view.reason.is_empty() {
            ui.label(
                RichText::new(&view.reason)
                    .small()
                    .color(theme::muted(ui)),
            );
        }
    });
    if let (Some(private_if), Some(public_if)) = (&view.private_if, &view.public_if) {
        ui.label(format!("{public_if}  →  {private_if}"));
    }
    if let Some(error) = &view.last_error {
        ui.colored_label(theme::danger(ui), error);
    }
    if view.repair_required || (view.enabled && !view.setup_done) || (!view.enabled && view.setup_done) {
        let label = if view.enabled { "Einrichtung reparieren" } else { "Installation entfernen / erneut versuchen" };
        if ui.button(label).clicked() { app.retry_lan_uplink_maintenance(); }
    }
    if view.state == crate::share::UplinkSharingState::Sharing
        && ui.button("Jetzt beenden").clicked()
    {
        app.stop_lan_uplink_sharing_now();
    }
    if let Some(notice) = &app.share_lan_notice {
        ui.label(RichText::new(notice).small());
    }
}

impl App {
    pub(in crate::app) fn set_lan_uplink_sharing_enabled(&mut self, enabled: bool) {
        match crate::share::LanSettings::update(|settings| {
            settings.uplink_sharing_enabled = enabled;
            if enabled {
                settings.uplink_stop_requested_at = None;
                settings.uplink_cleanup_pending = false;
                if !settings.uplink_setup_done {
                    settings.uplink_repair_requested_at = Some(crate::share::core_now_secs());
                }
            } else {
                settings.uplink_cleanup_pending = true;
            }
        }) {
            Ok(settings) => {
                self.share_lan_notice = Some(if !enabled {
                    "Internet-Teilen ausgeschaltet".into()
                } else if settings.uplink_setup_done {
                    "Internet-Teilen eingeschaltet".into()
                } else {
                    "Internet-Teilen eingeschaltet: die einmalige Einrichtung laeuft im Hintergrund (Windows-UAC bzw. polkit bestaetigen)".into()
                });
                let _ = self.configure_share_service();
                self.share_next_poll_at = Instant::now();
            }
            Err(error) => self.error_msg = Some(format!("LAN-Einstellung speichern: {error}")),
        }
    }

    fn retry_lan_uplink_maintenance(&mut self) {
        match crate::share::LanSettings::update(|settings| {
            let now = crate::share::core_now_secs();
            settings.uplink_repair_requested_at = Some(settings.uplink_repair_requested_at
                .map_or(now, |previous| now.max(previous.saturating_add(1))));
            if !settings.uplink_sharing_enabled { settings.uplink_cleanup_pending = true; }
        }) {
            Ok(_) => {
                self.share_lan_notice = Some("Reparatur/Entfernung angefordert: Freigabe bestaetigen".into());
                let _ = self.configure_share_service();
                self.share_next_poll_at = Instant::now();
            }
            Err(error) => self.error_msg = Some(format!("Uplink-Reparatur anfordern: {error}")),
        }
    }

    pub(in crate::app) fn stop_lan_uplink_sharing_now(&mut self) {
        match crate::share::LanSettings::update(|settings| {
            settings.uplink_stop_requested_at = Some(crate::share::core_now_secs());
        }) {
            Ok(_) => {
                self.share_lan_notice = Some("Internet-Teilen wird beendet".into());
                let _ = self.configure_share_service();
                self.share_next_poll_at = Instant::now();
            }
            Err(error) => self.error_msg = Some(format!("LAN-Einstellung speichern: {error}")),
        }
    }
}
