//! Page „Dieses Gerät“: Direkt-Code and fingerprint, device name, whether the
//! Direkt-Code is reachable, a new code, and the connection to the Share
//! server (formerly the collapsible „Dieses Gerät & Direkt-Code“ of the
//! device tab).
use super::*;

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui) {
    identity(app, ui);
    crate::app::share_login_ui::host_indicator(ui);
    super::pages_ui::heading(ui, "GERAETENAME");
    let mut form = crate::app::settings_form::Form::new(
        ui,
        "this_device_form",
        &["Gerätename", "Share-Server"],
    );
    form.row(ui, "Gerätename", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.share_device_draft)
                    .hint_text("Geraetename")
                    .desired_width(share_input_width(ui, 260.0)),
            );
            if ui.button("Name aendern").clicked() {
                rename(app);
            }
        });
    });
    super::pages_ui::heading(ui, "DIREKT-CODE ERREICHBAR");
    ui.label("Offline ist der Direkt-Code fuer neue Kontakte nicht erreichbar; gekoppelte Geraete und Raeume bleiben verbunden. Gilt bis zum naechsten Start des Share-Workers.");
    ui.horizontal_wrapped(|ui| {
        if ui.button("Online schalten").clicked() {
            let _ = app.ensure_share();
            let _ = app.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: true });
        }
        if ui.button("Offline schalten").clicked() {
            let _ = app.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: false });
        }
    });
    super::pages_ui::heading(ui, "NEUER DIREKT-CODE");
    if ui.button("Code neu generieren").clicked() {
        app.share_regenerate_direct_confirm = true;
    }
    if app.share_regenerate_direct_confirm {
        ui.colored_label(
            theme::warning(ui),
            "Ein neuer Code pausiert bestehende Direkt-Freigaben bis zur erneuten Bestaetigung. Exec wird abgeschaltet.",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Wirklich neu generieren").clicked() {
                super::identity_rotation::rotate(app);
            }
            if ui.button("Abbrechen").clicked() {
                app.share_regenerate_direct_confirm = false;
            }
        });
    }
    super::pages_ui::heading(ui, "SHARE-SERVER");
    form.row(ui, "Share-Server", |ui| {
        let server = if app.share_server.trim().is_empty() {
            "keiner (nur lokales Netz)".to_string()
        } else {
            app.share_server.clone()
        };
        share_value_field(ui, &server);
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Verbinden").clicked() {
            let _ = app.ensure_share();
        }
        if ui.button("Trennen").clicked() {
            let _ = app.share_cmd(crate::share::ShareCmd::Stop);
        }
        if ui.button("Server & Geraetename einstellen…").clicked() {
            app.open_settings(settings_ui::SettingsPage::Connections);
        }
    });
}

fn identity(app: &mut App, ui: &mut egui::Ui) {
    super::pages_ui::heading(ui, "DIREKT-CODE");
    let local_identity = app
        .share_identity
        .as_ref()
        .map(|identity| (identity.direct_code(), identity.fingerprint.clone()));
    match &local_identity {
        Some((direct_code, fingerprint)) => {
            let mut identity_form = crate::app::settings_form::Form::new(
                ui,
                "this_device_identity",
                &["Direkt-Code", "Fingerabdruck"],
            );
            identity_form.row(ui, "Direkt-Code", |ui| {
                ui.horizontal_wrapped(|ui| {
                    share_value_field(ui, direct_code);
                    if ui.button("Code kopieren").clicked() {
                        ui.ctx().copy_text(direct_code.clone());
                    }
                });
            });
            identity_form.row(ui, "Fingerabdruck", |ui| {
                ui.horizontal_wrapped(|ui| {
                    share_value_field(ui, fingerprint);
                    if ui.button("Fingerprint kopieren").clicked() {
                        ui.ctx().copy_text(fingerprint.clone());
                    }
                });
            });
        }
        None => {
            ui.colored_label(
                theme::danger(ui),
                app.share_identity_error
                    .as_deref()
                    .unwrap_or("Share-Identitaet nicht verfuegbar"),
            );
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "Freigegeben: {}",
            export_summary(&app.share_profiles.default_direct_exports)
        ));
        if ui.button("Freigaben fuer diesen Code").clicked() {
            app.share_export_scope = 0;
            app.share_export_target_id.clear();
            app.share_tab = super::pages_ui::TAB_EXPORTS;
        }
    });
}

fn rename(app: &mut App) {
    match app.share_identity.as_mut() {
        Some(identity) => match identity.set_device_name(app.share_device_draft.clone()) {
            Ok(()) => {
                let _ = app.configure_share_service();
            }
            Err(error) => {
                app.error_msg = Some(format!("Share-Geraetename nicht gespeichert: {error}"));
            }
        },
        None => {
            app.error_msg = Some("Share-Identitaet nicht verfuegbar".into());
        }
    }
}
