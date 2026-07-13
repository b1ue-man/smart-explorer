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
    RevokeLegacyGrant {
        device_id: String,
    },
    LegacyDecision {
        presence: Box<crate::share::PeerPresence>,
        accepted: bool,
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
    if !app.share_direct_requests.is_empty() {
        ui.separator();
        legacy_requests(app, ui, &mut action);
    }

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
            let confirmation_id = egui::Id::new((
                "share_direct_fingerprint_confirm",
                request.request_id.to_string(),
            ));
            let mut confirmed =
                ui.data_mut(|data| data.get_temp::<bool>(confirmation_id).unwrap_or_default());
            if ui
                .checkbox(&mut confirmed, "Angezeigten Fingerprint geprueft")
                .changed()
            {
                ui.data_mut(|data| data.insert_temp(confirmation_id, confirmed));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(confirmed, egui::Button::new("Akzeptieren"))
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
        } else if device.authorization.starts_with("active") {
            ui.colored_label(
                Color32::from_rgb(255, 185, 120),
                "Legacy-Freigabe ohne getrackte Request-ID; der Peer kann nicht benachrichtigt werden.",
            );
            if ui.button("Legacy-Freigabe lokal sperren").clicked() {
                *action = Some(LifecycleAction::RevokeLegacyGrant {
                    device_id: device.device_id.clone(),
                });
            }
        }
    });
    ui.add_space(4.0);
}

fn legacy_requests(app: &App, ui: &mut egui::Ui, action: &mut Option<LifecycleAction>) {
    section_heading(ui, "LEGACY-ANFRAGEN — UNBESTAETIGT");
    ui.colored_label(
        Color32::from_rgb(255, 185, 120),
        "Legacy-Fallback ohne stabile Request-ID: Versand, Peer-Empfang und Entscheidung bleiben unbestaetigt.",
    );
    for presence in app.share_direct_requests.clone() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new(&presence.device_name).strong());
            ui.horizontal_wrapped(|ui| {
                ui.label("Fingerprint:");
                super::helpers::share_value_field(ui, &presence.fingerprint);
            });
            let confirmation_id = egui::Id::new((
                "share_legacy_fingerprint_confirm",
                presence.device_id.clone(),
                presence.nonce.clone(),
            ));
            let mut confirmed =
                ui.data_mut(|data| data.get_temp::<bool>(confirmation_id).unwrap_or_default());
            if ui
                .checkbox(&mut confirmed, "Angezeigten Fingerprint geprueft")
                .changed()
            {
                ui.data_mut(|data| data.insert_temp(confirmation_id, confirmed));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(confirmed, egui::Button::new("Legacy freigeben"))
                    .clicked()
                {
                    *action = Some(LifecycleAction::LegacyDecision {
                        presence: Box::new(presence.clone()),
                        accepted: true,
                    });
                }
                if ui.button("Legacy loeschen / ablehnen").clicked() {
                    *action = Some(LifecycleAction::LegacyDecision {
                        presence: Box::new(presence.clone()),
                        accepted: false,
                    });
                }
            });
        });
    }
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
        LifecycleAction::RevokeLegacyGrant { device_id } => revoke_legacy_grant(app, &device_id),
        LifecycleAction::LegacyDecision { presence, accepted } => {
            legacy_decide(app, *presence, accepted)
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

fn legacy_decide(app: &mut App, presence: crate::share::PeerPresence, accepted: bool) {
    let Some(identity) = app.share_identity.clone() else {
        app.error_msg = Some("Share-Identitaet nicht verfuegbar".into());
        return;
    };
    let state = if accepted {
        crate::share::DirectGrantState::Accepted
    } else {
        crate::share::DirectGrantState::Ignored
    };
    let persisted =
        crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |profiles| {
            profiles.set_direct_grant(&presence, state.clone());
            Ok(())
        });
    if let Err(error) = persisted {
        app.error_msg = Some(format!("Legacy-Freigabe nicht gespeichert: {error}"));
        return;
    }
    if app.share_cmd(crate::share::ShareCmd::AnswerDirectRequest {
        lookup_id: identity.direct_lookup_id,
        presence: presence.clone(),
        accepted,
    }) {
        app.share_direct_requests
            .retain(|request| request.device_id != presence.device_id);
        app.notice = Some((
            format!(
                "Legacy-Entscheidung fuer {} gesendet; Peer-Empfang unbestaetigt",
                presence.device_name
            ),
            Instant::now(),
        ));
        app.share_next_poll_at = Instant::now();
    }
}

fn revoke_legacy_grant(app: &mut App, device_id: &str) {
    let now = crate::share::core_now_secs();
    let result = crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |profiles| {
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
                format!(
                    "Legacy-Freigabe fuer {device_id} lokal gesperrt; Peer-Benachrichtigung nicht moeglich"
                ),
            );
        }
        Err(error) => {
            app.error_msg = Some(format!("Legacy-Freigabe nicht gesperrt: {error}"));
        }
    }
}

fn refresh_after_action(app: &mut App, notice: String) -> bool {
    if !app.configure_share_service() {
        return false;
    }
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
