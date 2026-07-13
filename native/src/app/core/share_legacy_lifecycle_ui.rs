use super::*;

enum LegacyAction {
    Decide {
        selector: String,
        fingerprint: String,
        accepted: bool,
    },
    Retry(String),
    Revoke(String),
    Delete(String),
}

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui) {
    let now = crate::share::core_now_secs();
    let pending = app
        .share_profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| entry.is_pending(now))
        .cloned()
        .collect::<Vec<_>>();
    let history = app
        .share_profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| !entry.is_pending(now))
        .cloned()
        .collect::<Vec<_>>();
    ui.separator();
    heading(ui, "LEGACY-ANFRAGEN — LOKAL PERSISTIERT");
    ui.colored_label(
        Color32::from_rgb(255, 185, 120),
        "Legacy-Anfragen sind HMAC-authentifiziert und lokal gespeichert. Der alte Peer bestaetigt Empfang oder Entscheidung nicht; Versand bleibt immer unbestaetigt.",
    );
    let mut action = None;
    if pending.is_empty() {
        ui.label("Keine offenen Legacy-Anfragen.");
    } else {
        for entry in &pending {
            card(app, ui, entry, true, &mut action);
        }
    }
    if !history.is_empty() {
        egui::CollapsingHeader::new(format!("LEGACY-VERLAUF ({})", history.len()))
            .default_open(false)
            .show(ui, |ui| {
                for entry in &history {
                    card(app, ui, entry, false, &mut action);
                }
            });
    }
    if let Some(action) = action {
        perform(app, action);
    }
}

fn card(
    app: &App,
    ui: &mut egui::Ui,
    entry: &crate::share::LegacyDirectRequestEntry,
    pending: bool,
    action: &mut Option<LegacyAction>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(&entry.peer.device_name).strong());
        ui.horizontal_wrapped(|ui| {
            ui.label("Lokaler Selector:");
            super::helpers::share_value_field(ui, &entry.selector);
        });
        ui.label(format!("Geraet-ID: {}", entry.peer.device_id));
        ui.horizontal_wrapped(|ui| {
            ui.label("Fingerprint:");
            super::helpers::share_value_field(ui, &entry.peer.fingerprint);
        });
        ui.label(
            "Empfang: received — HMAC-geprueft und lokal persistiert; Peer-Receipt: unsupported",
        );
        ui.label(format!("Entscheidung: {}", entry.decision.code()));
        ui.label(format!(
            "Entscheidungsversand: {} ueber {}; Peer-Receipt: unsupported; Versuche: {}",
            entry.decision_delivery.state.code(),
            entry.decision_delivery_channel(),
            entry.decision_delivery.attempt_count
        ));
        if let Some(error) = &entry.decision_delivery.last_error {
            ui.colored_label(Color32::LIGHT_RED, format!("Letzter Versandfehler: {error}"));
        }
        let active = entry.authorization_active(&app.share_profiles);
        ui.label(format!(
            "Autorisierung: {}",
            if active { "active" } else { "inactive" }
        ));
        if entry.identity_conflict {
            let active_blocker = app.share_profiles.direct_grants.iter().find(|grant| {
                grant.state == crate::share::DirectGrantState::Accepted
                    && grant.device_id == entry.peer.device_id
                    && (grant.public_key != entry.peer.public_key
                        || grant.node_id != entry.peer.node_id
                        || grant.fingerprint != entry.peer.fingerprint)
            });
            let resolution = active_blocker.map_or_else(
                || "Eine der konfligierenden Anfragen ablehnen oder lokal loeschen.".to_string(),
                |grant| format!(
                    "Bestehende Autorisierung {} (Fingerprint {}) unter Autorisierte Geraete widerrufen, um diese Anfrage zu behalten; alternativ diese Anfrage ablehnen oder loeschen.",
                    grant.device_name, grant.fingerprint
                ),
            );
            ui.colored_label(
                Color32::LIGHT_RED,
                format!("Identitaetskonflikt: dieselbe Geraet-ID ist mit einem anderen Schluessel gespeichert; Freigabe ist gesperrt. {resolution}"),
            );
        }
        if pending {
            let confirmation_id = egui::Id::new(("legacy_confirm", entry.selector.clone()));
            let mut confirmed =
                ui.data_mut(|data| data.get_temp::<bool>(confirmation_id).unwrap_or_default());
            if entry.identity_conflict {
                confirmed = false;
                ui.data_mut(|data| data.insert_temp(confirmation_id, false));
            } else if ui
                .checkbox(&mut confirmed, "Angezeigten Fingerprint geprueft")
                .changed()
            {
                ui.data_mut(|data| data.insert_temp(confirmation_id, confirmed));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        confirmed && !entry.identity_conflict,
                        egui::Button::new("Legacy freigeben"),
                    )
                    .clicked()
                {
                    *action = Some(LegacyAction::Decide {
                        selector: entry.selector.clone(),
                        fingerprint: entry.peer.fingerprint.clone(),
                        accepted: true,
                    });
                }
                if ui.button("Legacy ablehnen").clicked() {
                    *action = Some(LegacyAction::Decide {
                        selector: entry.selector.clone(),
                        fingerprint: entry.peer.fingerprint.clone(),
                        accepted: false,
                    });
                }
                if ui.button("Pending lokal loeschen").clicked() {
                    *action = Some(LegacyAction::Delete(entry.selector.clone()));
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                if matches!(
                    entry.decision_delivery.state,
                    crate::share::LegacyDirectDeliveryState::AttemptedUntracked
                        | crate::share::LegacyDirectDeliveryState::FailedUntracked
                ) && ui.button("Unbestaetigte Antwort erneut versuchen").clicked()
                {
                    *action = Some(LegacyAction::Retry(entry.selector.clone()));
                }
                if active && ui.button("Legacy-Freigabe widerrufen").clicked() {
                    *action = Some(LegacyAction::Revoke(entry.selector.clone()));
                }
                if !active && ui.button("Aus Verlauf loeschen").clicked() {
                    *action = Some(LegacyAction::Delete(entry.selector.clone()));
                }
            });
        }
    });
    ui.add_space(4.0);
}

fn perform(app: &mut App, action: LegacyAction) {
    let result = match action {
        LegacyAction::Decide {
            selector,
            fingerprint,
            accepted,
        } => app
            .share_identity
            .as_ref()
            .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())
            .and_then(|identity| {
                crate::share::decide_legacy_direct_request(
                    Some(default_home()),
                    identity,
                    &selector,
                    &fingerprint,
                    accepted,
                )
                .map(|_| format!("Legacy-Entscheidung fuer {selector} gespeichert"))
            }),
        LegacyAction::Retry(selector) => {
            crate::share::retry_legacy_direct_answer(Some(default_home()), &selector)
                .map(|_| format!("Legacy-Antwort fuer {selector} erneut vorgemerkt"))
        }
        LegacyAction::Revoke(selector) => {
            crate::share::revoke_legacy_direct_request(Some(default_home()), &selector)
                .map(|_| format!("Legacy-Freigabe fuer {selector} widerrufen"))
        }
        LegacyAction::Delete(selector) => {
            crate::share::delete_legacy_direct_request(Some(default_home()), &selector)
                .map(|_| format!("Legacy-Anfrage {selector} geloescht"))
        }
    };
    match result {
        Ok(notice) => {
            if let Err(error) = super::profile_cache::reload(app) {
                app.error_msg = Some(format!(
                    "Legacy-Anfrage gespeichert, aber GUI-Stand konnte nicht geladen werden: {error}"
                ));
                return;
            }
            let _ = app.configure_share_service();
            app.notice = Some((notice, Instant::now()));
            app.share_next_poll_at = Instant::now();
        }
        Err(error) => app.error_msg = Some(format!("Legacy-Anfrage: {error}")),
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(Color32::from_gray(140)));
}

fn default_home() -> String {
    dirs_home().to_string_lossy().replace('\\', "/")
}
