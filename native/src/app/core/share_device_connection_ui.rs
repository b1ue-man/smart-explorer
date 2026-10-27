//! Head of a contact's device page (status and the main actions: files,
//! screen, access request) and its card „Verbindung“ (automatic connect and
//! open, identity, route, diagnosis, trust reset).
use super::*;
use crate::app::device_page_layout as layout;

/// Status line and the actions used most: open files, view or control the
/// screen (SCR-M7) and ask for access while it is not granted.
pub(super) fn header(app: &mut App, ui: &mut egui::Ui, contact: &crate::share::DirectContact) {
    let now = crate::share::core_now_secs();
    ui.horizontal_wrapped(|ui| {
        layout::status_chip(
            ui,
            &contact.status.label(),
            crate::share::device_view::online(contact, now),
        );
        ui.label(RichText::new(contact.access_state.label()).color(theme::muted(ui)));
    });
    if let Some(error) = &contact.last_error {
        ui.colored_label(theme::danger(ui), error);
    }
    let accepted = contact.access_state == crate::share::DirectAccessState::Accepted;
    let device_id = contact.remote_device_id.as_deref();
    let (mut open, mut request, mut launch) = (false, false, None);
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        open = ui.button("Dateien öffnen").clicked();
        for button in crate::app::screen_launch::device_buttons(device_id, accepted) {
            if crate::app::screen_launch::show_button(ui, &button) {
                launch = Some(button.control);
            }
        }
        if !accepted {
            request = ui.button("Zugriff anfragen").clicked();
        }
    });
    if let Some(control) = launch {
        let mut target = crate::app::screen_target::ScreenTarget::direct(&contact.id, device_id);
        target.control = control;
        if let Err(error) = crate::app::screen_launch::open_view(&target) {
            app.error_msg = Some(error);
        }
    }
    if request {
        let _ = super::super::lifecycle_ui::queue_contact(app, &contact.id);
    }
    if open {
        app.open_share_target(crate::share::PeerOpenTarget::Direct {
            contact_id: contact.id.clone(),
        });
    }
}

/// Card „Verbindung“ of a contact.
pub(super) fn connection_card(
    app: &mut App,
    ui: &mut egui::Ui,
    contact: &crate::share::DirectContact,
) {
    layout::card(ui, "Verbindung", None, |ui| {
        let mut auto_connect = contact.auto_connect;
        let mut auto_open = contact.auto_open;
        let connect_changed = ui
            .checkbox(&mut auto_connect, "Automatisch verbinden")
            .changed();
        let open_changed = ui
            .checkbox(
                &mut auto_open,
                "Automatisch öffnen, sobald es erreichbar ist",
            )
            .changed();
        if connect_changed || open_changed {
            persist(app, "Verbindungseinstellung", |profiles| {
                profiles.set_contact_options(
                    &contact.id,
                    connect_changed.then_some(auto_connect),
                    open_changed.then_some(auto_open),
                )
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Fingerabdruck").color(theme::muted(ui)));
            let _ = share_value_field(ui, &contact.expected_fingerprint);
            if ui.button("Kopieren").clicked() {
                ui.ctx().copy_text(contact.expected_fingerprint.clone());
            }
        });
        if let (Some(device), Some(key)) = (&contact.remote_device_id, &contact.remote_public_key) {
            layout::tool(
                ui,
                ("route", device),
                "Verbindungsweg (direkt oder Relay)",
                |ui| {
                    crate::app::share_routes_ui::device(ui, app.share_routes.as_ref(), device, key);
                },
            );
        }
        let (mut diagnose, mut reset) = (false, false);
        ui.horizontal_wrapped(|ui| {
            diagnose = ui.button("Diagnose").clicked();
            reset = ui
                .button("Vertrauen zurücksetzen")
                .on_hover_text(
                    "Vergisst die gepinnte Identität dieses Geräts und entzieht seine Autorisierung (Befehle aus); der Kontakt bleibt und muss neu bestätigt werden.",
                )
                .clicked();
        });
        if reset {
            let now = crate::share::core_now_secs();
            persist(app, "Vertrauen", |profiles| {
                profiles.reset_contact_trust(&contact.id, now)
            });
        }
        if diagnose {
            diagnose_contact(app, contact);
        }
    });
}

fn diagnose_contact(app: &mut App, contact: &crate::share::DirectContact) {
    let presence = contact
        .presence
        .as_ref()
        .map(|p| {
            format!(
                "node={}, relay={}, candidates={:?}, expires_at={}",
                p.node_id, p.relay_url, p.candidates, p.expires_at
            )
        })
        .unwrap_or_else(|| "keine Presence".to_string());
    app.append_share_diag(format!(
        "Direct {}: lookup={}, fp={}, status={}, {}\n",
        contact.display_name,
        contact.lookup_id,
        contact.expected_fingerprint,
        contact.status.label(),
        presence
    ));
    app.share_tab = pages_ui::TAB_DIAGNOSTICS;
}
