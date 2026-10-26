//! Connection/trust and ordinary screen actions of a selected contact.
use super::*;

pub(super) fn connection_section(app: &mut App, ui: &mut egui::Ui, contact: &crate::share::DirectContact) {
    pages_ui::heading(ui, "VERBINDUNG");
    ui.label(format!(
        "Status: {} · Zugriff: {}",
        contact.status.label(),
        contact.access_state.label()
    ));
    if let Some(error) = &contact.last_error {
        ui.colored_label(theme::danger(ui), error);
    }
    let mut form = crate::app::settings_form::Form::new(
        ui,
        "contact_form",
        &[
            "Fingerabdruck",
            "Automatisch verbinden",
            "Automatisch öffnen",
        ],
    );
    form.row(ui, "Fingerabdruck", |ui| {
        ui.horizontal_wrapped(|ui| {
            share_value_field(ui, &contact.expected_fingerprint);
            if ui.button("Kopieren").clicked() {
                ui.ctx().copy_text(contact.expected_fingerprint.clone());
            }
        });
    });
    let mut open = false;
    let mut request = false;
    let mut diagnose = false;
    ui.horizontal_wrapped(|ui| {
        open = ui.button("Oeffnen").clicked();
        request = ui
            .add_enabled(
                contact.access_state != crate::share::DirectAccessState::Accepted,
                egui::Button::new("Zugriff anfragen"),
            )
            .clicked();
        diagnose = ui.button("Diagnose").clicked();
    });
    let mut auto_connect = contact.auto_connect;
    let mut auto_open = contact.auto_open;
    let connect_changed = form.row_labelled(ui, "Automatisch verbinden", |ui, label| {
        ui.checkbox(&mut auto_connect, "Aktiv")
            .labelled_by(label)
            .changed()
    });
    let open_changed = form.row_labelled(ui, "Automatisch öffnen", |ui, label| {
        ui.checkbox(&mut auto_open, "Sobald erreichbar")
            .labelled_by(label)
            .changed()
    });
    if connect_changed || open_changed {
        persist(app, "Verbindungseinstellung", |profiles| {
            profiles.set_contact_options(
                &contact.id,
                connect_changed.then_some(auto_connect),
                open_changed.then_some(auto_open),
            )
        });
    }
    if ui
        .button("Vertrauen zuruecksetzen")
        .on_hover_text(
            "Vergisst die gepinnte Identitaet dieses Geraets und entzieht seine Autorisierung (Exec aus); der Kontakt bleibt und muss neu bestaetigt werden.",
        )
        .clicked()
    {
        let now = crate::share::core_now_secs();
        persist(app, "Vertrauen", |profiles| {
            profiles.reset_contact_trust(&contact.id, now)
        });
    }
    if diagnose {
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
    if request {
        let _ = super::super::lifecycle_ui::queue_contact(app, &contact.id);
    }
    if open {
        app.open_share_target(crate::share::PeerOpenTarget::Direct {
            contact_id: contact.id.clone(),
        });
    }
}

/// „Bildschirm öffnen“ and „Bildschirm steuern“ (SCR-M7): the viewer window.
pub(super) fn screen_section(app: &mut App, ui: &mut egui::Ui, contact: &crate::share::DirectContact) {
    pages_ui::heading(ui, "BILDSCHIRM");
    let accepted = contact.access_state == crate::share::DirectAccessState::Accepted;
    let device_id = contact.remote_device_id.as_deref();
    let mut launch = None;
    ui.horizontal_wrapped(|ui| {
        for button in crate::app::screen_launch::device_buttons(device_id, accepted) {
            if crate::app::screen_launch::show_button(ui, &button) {
                launch = Some(button.control);
            }
        }
    });
    if let Some(control) = launch {
        let mut target = crate::app::screen_target::ScreenTarget::direct(&contact.id, device_id);
        target.control = control;
        if let Err(error) = crate::app::screen_launch::open_view(&target) {
            app.error_msg = Some(error);
        }
    }
}
