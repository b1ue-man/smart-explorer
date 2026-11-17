//! Persisted lifecycle actions for the Direct authorization UI.
use super::{default_home, refresh_after_action, App, LifecycleAction};

pub(super) fn perform_action(app: &mut App, action: LifecycleAction) {
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
        } => {
            // A request of a lookup id that a code renewal replaced cannot be
            // signed again (`require_incoming_request`); the grant is revoked
            // locally like one whose request history is gone.
            let current = app
                .share_identity
                .as_ref()
                .map(|identity| identity.direct_lookup_id.clone());
            let stale = app.share_profiles.direct_requests.iter().any(|entry| {
                entry.record.request.request_id == request_id && entry.local_lookup_id != current
            });
            let device_id = app
                .share_profiles
                .direct_grants
                .iter()
                .find(|grant| grant.fingerprint == fingerprint)
                .map(|grant| grant.device_id.clone());
            match device_id {
                Some(device_id) if stale => revoke_unlinked_grant(app, device_id),
                _ => decide(
                    app,
                    request_id,
                    fingerprint,
                    crate::share::DirectDecisionKind::Revoked,
                ),
            }
        }
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
            revoke_unlinked_grant(app, device_id)
        }
        LifecycleAction::RemoveDevice { device_id } => {
            let contact_id = app
                .share_profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.remote_device_id.as_deref() == Some(device_id.as_str()))
                .map(|contact| contact.id.clone());
            match contact_id {
                Some(contact_id) => {
                    app.remove_direct_peer_completely(&contact_id);
                }
                None => app.delete_direct_grant_entry(&device_id),
            }
        }
        LifecycleAction::AllowAgain { device_id } => {
            match crate::share::allow_direct_peer_again(Some(default_home()), &device_id) {
                Ok(_) => {
                    let _ = refresh_after_action(
                        app,
                        format!("Freigabe fuer {device_id} bestaetigt; Exec bleibt aus"),
                    );
                }
                Err(error) => app.error_msg = Some(format!("Direkt-Freigabe: {error}")),
            }
        }
        LifecycleAction::SelectExports => {
            app.share_export_scope = 0;
            app.share_export_target_id.clear();
            app.share_tab = 2;
        }
    }
}

/// Withdraws a grant without a request of the current lookup id: its
/// history was pruned, belongs to a replaced lookup id, or it is legacy.
fn revoke_unlinked_grant(app: &mut App, device_id: String) {
    let now = crate::share::core_now_secs();
    let result = crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |profiles| {
        let grant = profiles
            .direct_grants
            .iter()
            .find(|grant| grant.device_id == device_id)
            .ok_or_else(|| format!("Freigabe nicht gefunden: {device_id}"))?;
        let peer = crate::share::DirectPeerIdentity {
            device_id: grant.device_id.clone(),
            device_name: grant.device_name.clone(),
            public_key: grant.public_key.clone(),
            node_id: grant.node_id.clone(),
            fingerprint: grant.fingerprint.clone(),
        };
        profiles.withdraw_direct_key(&peer, now);
        profiles.mark_legacy_revoked_for_peer(&peer, now);
        Ok(())
    });
    match result {
        Ok(_) => {
            let _ = refresh_after_action(app, format!("Freigabe fuer {device_id} lokal gesperrt"));
        }
        Err(error) => app.error_msg = Some(format!("Freigabe: {error}")),
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
            app.error_msg = Some(crate::share::connection_rights_actions::decision_failure(
                &error,
                "Direkt-Entscheidung nicht gespeichert",
            ));
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
