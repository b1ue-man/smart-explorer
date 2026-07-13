use super::share_lifecycle_view::{
    authorized_device_views, request_views, AuthorizedDeviceView, RequestView,
};
use super::*;

enum LifecycleAction {
    Decide {
        request_id: crate::share::DirectRequestId,
        fingerprint: String,
        decision: crate::share::DirectDecisionKind,
    },
    Retry {
        request_id: crate::share::DirectRequestId,
    },
    DeleteHistory {
        request_id: crate::share::DirectRequestId,
    },
    Revoke {
        request_id: crate::share::DirectRequestId,
        fingerprint: String,
    },
    RevokeLegacy {
        selector: String,
    },
    RevokeUnlinkedLegacyGrant {
        device_id: String,
    },
    SelectExports,
}

pub(super) fn ui_lifecycle(app: &mut App, ui: &mut egui::Ui) {
    let now = crate::share::core_now_secs();
    let (incoming, outgoing) = request_views(&app.share_profiles, now);
    let pending_incoming = incoming
        .iter()
        .filter(|request| request.can_decide)
        .collect::<Vec<_>>();
    let incoming_history = incoming
        .iter()
        .filter(|request| !request.can_decide)
        .collect::<Vec<_>>();
    let authorized = authorized_device_views(&app.share_profiles);
    let mut action = None;

    section_heading(ui, "EINGEHENDE ANFRAGEN");
    if pending_incoming.is_empty() {
        ui.label("Keine getrackten eingehenden Anfragen.");
    } else {
        for request in pending_incoming {
            request_card(ui, request, true, &mut action);
        }
    }

    if !incoming_history.is_empty() {
        egui::CollapsingHeader::new(format!(
            "ABGESCHLOSSENE EINGEHENDE ANFRAGEN ({})",
            incoming_history.len()
        ))
        .default_open(false)
        .show(ui, |ui| {
            for request in incoming_history {
                request_card(ui, request, true, &mut action);
            }
        });
    }

    ui.separator();
    section_heading(ui, "AUSGEHENDE ANFRAGEN");
    if outgoing.is_empty() {
        ui.label("Keine getrackten ausgehenden Anfragen.");
    } else {
        for request in &outgoing {
            request_card(ui, request, false, &mut action);
        }
    }

    ui.separator();
    section_heading(ui, "AUTORISIERTE GERAETE");
    if authorized.is_empty() {
        ui.label("Keine Geraete haben eine gespeicherte Autorisierung.");
    } else {
        for device in &authorized {
            authorized_card(ui, device, &mut action);
        }
    }

    ui.separator();
    crate::app::share_exec_ui::ui_exec_grants(app, ui);
    super::legacy_lifecycle_ui::ui(app, ui);

    if let Some(action) = action {
        perform_action(app, action);
    }
}

pub(super) fn queue_contact(app: &mut App, contact_id: &str) -> bool {
    let Some(identity) = app.share_identity.clone() else {
        app.error_msg = Some("Share-Identitaet nicht verfuegbar".into());
        return false;
    };
    let home = default_home();
    match crate::share::queue_direct_request_for_contact(Some(home), &identity, contact_id, None) {
        Ok(action) => {
            let request_id = action.entry.record.request.request_id;
            let state = if action.created {
                "queued — neue Anfrage dauerhaft gespeichert"
            } else {
                "queued — bestehende Anfrage mit gleicher ID erneut vorgemerkt"
            };
            refresh_after_action(
                app,
                format!("Anfrage {request_id}: {state}; Peer-Empfang offen"),
            )
        }
        Err(error) => {
            app.error_msg = Some(format!("Direkt-Anfrage nicht vorgemerkt: {error}"));
            false
        }
    }
}

fn request_card(
    ui: &mut egui::Ui,
    request: &RequestView,
    incoming: bool,
    action: &mut Option<LifecycleAction>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} — {}",
                    request.peer_name,
                    request.decision.code()
                ))
                .strong(),
            );
            if !request.peer_device_id.is_empty() {
                ui.label(format!("Geraet-ID: {}", request.peer_device_id));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Request-ID:");
            super::helpers::share_value_field(ui, request.request_id.as_str());
            if ui.button("ID kopieren").clicked() {
                ui.ctx().copy_text(request.request_id.to_string());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Fingerprint:");
            super::helpers::share_value_field(ui, &request.fingerprint);
            if ui.button("Fingerprint kopieren").clicked() {
                ui.ctx().copy_text(request.fingerprint.clone());
            }
        });
        egui::Grid::new(("share_lifecycle_facts", request.request_id.as_str()))
            .num_columns(2)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                for fact in &request.facts {
                    ui.label(format!("{}:", fact.label));
                    ui.add(egui::Label::new(&fact.value).wrap());
                    ui.end_row();
                }
            });

        if incoming && request.can_decide {
            if request.identity_conflict {
                ui.colored_label(
                    Color32::from_rgb(255, 120, 120),
                    "Identitaetskonflikt: Akzeptieren ist gesperrt. Die Konfliktzeile nennt die bestehende Freigabe oder Anfrage, die zuerst aufgeloest werden muss; alternativ diese Anfrage ablehnen oder loeschen.",
                );
            }
            let confirmation_id = egui::Id::new((
                "share_direct_fingerprint_confirm",
                request.request_id.to_string(),
            ));
            let mut confirmed =
                ui.data_mut(|data| data.get_temp::<bool>(confirmation_id).unwrap_or_default());
            if request.can_accept {
                if ui
                    .checkbox(&mut confirmed, "Angezeigten Fingerprint geprueft")
                    .changed()
                {
                    ui.data_mut(|data| data.insert_temp(confirmation_id, confirmed));
                }
            } else {
                confirmed = false;
                ui.data_mut(|data| data.insert_temp(confirmation_id, false));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        request.can_accept && confirmed,
                        egui::Button::new("Akzeptieren"),
                    )
                    .clicked()
                {
                    *action = Some(LifecycleAction::Decide {
                        request_id: request.request_id.clone(),
                        fingerprint: request.fingerprint.clone(),
                        decision: crate::share::DirectDecisionKind::Accepted,
                    });
                }
                if ui.button("Ablehnen").clicked() {
                    *action = Some(LifecycleAction::Decide {
                        request_id: request.request_id.clone(),
                        fingerprint: request.fingerprint.clone(),
                        decision: crate::share::DirectDecisionKind::Rejected,
                    });
                }
                if ui.button("Freigaben waehlen").clicked() {
                    *action = Some(LifecycleAction::SelectExports);
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if !incoming
                && request.can_retry
                && ui
                    .button("Mit gleicher Request-ID erneut versuchen")
                    .clicked()
            {
                *action = Some(LifecycleAction::Retry {
                    request_id: request.request_id.clone(),
                });
            }
            let delete_label = if request.decision == crate::share::DirectDecisionState::Pending {
                "Anfrage lokal loeschen"
            } else {
                "Aus Verlauf loeschen"
            };
            if ui
                .add_enabled(request.can_delete, egui::Button::new(delete_label))
                .on_hover_text(
                    "Pending-Anfragen koennen lokal geloescht werden. Eine angenommene Anfrage muss zuerst unter Autorisierte Geraete widerrufen und vom Peer bestaetigt werden.",
                )
                .clicked()
            {
                *action = Some(LifecycleAction::DeleteHistory {
                    request_id: request.request_id.clone(),
                });
            }
        });
    });
    ui.add_space(4.0);
}
fn authorized_card(
    ui: &mut egui::Ui,
    device: &AuthorizedDeviceView,
    action: &mut Option<LifecycleAction>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(&device.device_name).strong());
        ui.label(format!("Geraet-ID: {}", device.device_id));
        ui.horizontal_wrapped(|ui| {
            ui.label("Fingerprint:");
            super::helpers::share_value_field(ui, &device.fingerprint);
        });
        ui.label(format!("Autorisierung: {}", device.authorization));
        ui.label(format!("Verbindung: {}", device.connectivity));
        ui.label(format!("Aktualisiert: {}", device.updated_at));
        if let Some(request_id) = &device.accepted_request {
            ui.label(format!("Autorisierungs-Request: {request_id}"));
            if device.authorization.starts_with("active")
                && ui.button("Autorisierung widerrufen").clicked()
            {
                *action = Some(LifecycleAction::Revoke {
                    request_id: request_id.clone(),
                    fingerprint: device.fingerprint.clone(),
                });
            }
        } else if let Some(selector) = &device.accepted_legacy_request {
            ui.colored_label(
                Color32::from_rgb(255, 185, 120),
                "Legacy-Freigabe; ein Widerruf ist nur lokal und ungetrackt.",
            );
            if ui.button("Legacy-Freigabe lokal sperren").clicked() {
                *action = Some(LifecycleAction::RevokeLegacy {
                    selector: selector.clone(),
                });
            }
        } else if device.authorization.starts_with("active") {
            ui.label("Legacy-Freigabe ohne gespeicherten Request-Verlauf.");
            if ui
                .button("Unverknuepfte Legacy-Freigabe lokal sperren")
                .clicked()
            {
                *action = Some(LifecycleAction::RevokeUnlinkedLegacyGrant {
                    device_id: device.device_id.clone(),
                });
            }
        }
    });
    ui.add_space(4.0);
}

fn perform_action(app: &mut App, action: LifecycleAction) {
    match action {
        LifecycleAction::Decide {
            request_id,
            fingerprint,
            decision,
        } => decide(app, request_id, fingerprint, decision),
        LifecycleAction::Retry { request_id } => retry(app, request_id),
        LifecycleAction::DeleteHistory { request_id } => delete_history(app, request_id),
        LifecycleAction::Revoke {
            request_id,
            fingerprint,
        } => decide(
            app,
            request_id,
            fingerprint,
            crate::share::DirectDecisionKind::Revoked,
        ),
        LifecycleAction::RevokeLegacy { selector } => {
            match crate::share::revoke_legacy_direct_request(Some(default_home()), &selector) {
                Ok(_) => {
                    let _ = refresh_after_action(
                        app,
                        format!("Legacy-Freigabe fuer {selector} lokal widerrufen"),
                    );
                }
                Err(error) => app.error_msg = Some(format!("Legacy-Freigabe: {error}")),
            }
        }
        LifecycleAction::RevokeUnlinkedLegacyGrant { device_id } => {
            let now = crate::share::core_now_secs();
            let result =
                crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |profiles| {
                    let grant = profiles
                        .direct_grants
                        .iter_mut()
                        .find(|grant| grant.device_id == device_id)
                        .ok_or_else(|| format!("Legacy-Freigabe nicht gefunden: {device_id}"))?;
                    grant.state = crate::share::DirectGrantState::Ignored;
                    grant.updated_at = now;
                    grant.exec.disable_without_decision(now);
                    Ok(())
                });
            match result {
                Ok(_) => {
                    let _ = refresh_after_action(
                        app,
                        format!("Unverknuepfte Legacy-Freigabe fuer {device_id} gesperrt"),
                    );
                }
                Err(error) => app.error_msg = Some(format!("Legacy-Freigabe: {error}")),
            }
        }
        LifecycleAction::SelectExports => {
            app.share_export_scope = 0;
            app.share_export_target_id.clear();
            app.share_tab = 2;
        }
    }
}

fn decide(
    app: &mut App,
    request_id: crate::share::DirectRequestId,
    fingerprint: String,
    decision: crate::share::DirectDecisionKind,
) {
    let Some(identity) = app.share_identity.clone() else {
        app.error_msg = Some("Share-Identitaet nicht verfuegbar".into());
        return;
    };
    match crate::share::decide_direct_request(
        Some(default_home()),
        &identity,
        &request_id,
        &fingerprint,
        decision,
        None,
    ) {
        Ok(_) => {
            let label = match decision {
                crate::share::DirectDecisionKind::Accepted => "accepted",
                crate::share::DirectDecisionKind::Rejected => "rejected",
                crate::share::DirectDecisionKind::Revoked => "revoked",
            };
            let _ = refresh_after_action(
                app,
                format!(
                    "Anfrage {request_id}: Entscheidung {label} gespeichert; Peer-Empfang offen"
                ),
            );
        }
        Err(error) => {
            app.error_msg = Some(format!("Direkt-Entscheidung nicht gespeichert: {error}"));
        }
    }
}

fn retry(app: &mut App, request_id: crate::share::DirectRequestId) {
    match crate::share::retry_direct_request_now(Some(default_home()), &request_id) {
        Ok(_) => {
            let _ = refresh_after_action(
                app,
                format!("Anfrage {request_id}: gleiche ID erneut queued; Peer-Empfang offen"),
            );
        }
        Err(error) => {
            app.error_msg = Some(format!("Direkt-Anfrage nicht erneut vorgemerkt: {error}"));
        }
    }
}

fn delete_history(app: &mut App, request_id: crate::share::DirectRequestId) {
    match crate::share::delete_direct_request_history(Some(default_home()), &request_id) {
        Ok(()) => {
            let _ = refresh_after_action(app, format!("Anfrage {request_id} geloescht"));
        }
        Err(error) => {
            app.error_msg = Some(format!("Anfrage nicht geloescht: {error}"));
        }
    }
}

fn refresh_after_action(app: &mut App, notice: String) -> bool {
    if let Err(error) = super::profile_cache::reload(app) {
        app.error_msg = Some(format!(
            "Share-Aenderung gespeichert, aber GUI-Stand konnte nicht geladen werden: {error}"
        ));
        return false;
    }
    let _ = app.configure_share_service();
    app.notice = Some((notice, Instant::now()));
    app.share_next_poll_at = Instant::now();
    true
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(Color32::from_gray(140)));
}

fn default_home() -> String {
    dirs_home().to_string_lossy().replace('\\', "/")
}
