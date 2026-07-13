use crate::share::{
    DirectDecisionState, DirectRelayOutcome, DirectRequestDirection, DirectRequestId,
    DirectRetryState, ShareProfiles,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LifecycleFact {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub(super) struct RequestView {
    pub request_id: DirectRequestId,
    pub peer_device_id: String,
    pub peer_name: String,
    pub fingerprint: String,
    pub decision: DirectDecisionState,
    pub facts: Vec<LifecycleFact>,
    pub can_decide: bool,
    pub can_retry: bool,
    pub can_delete: bool,
}

#[derive(Clone, Debug)]
pub(super) struct AuthorizedDeviceView {
    pub device_id: String,
    pub device_name: String,
    pub fingerprint: String,
    pub authorization: String,
    pub connectivity: String,
    pub updated_at: String,
    pub accepted_request: Option<DirectRequestId>,
}

pub(super) fn request_views(
    profiles: &ShareProfiles,
    now: i64,
) -> (Vec<RequestView>, Vec<RequestView>) {
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    for entry in &profiles.direct_requests {
        let peer = match entry.direction {
            DirectRequestDirection::Incoming => &entry.record.request.requester,
            DirectRequestDirection::Outgoing => entry
                .decision
                .as_ref()
                .map(|decision| &decision.target)
                .or_else(|| {
                    entry
                        .request_receipt
                        .as_ref()
                        .map(|receipt| &receipt.target)
                })
                .unwrap_or(&entry.record.request.target),
        };
        let contact = entry.contact_id.as_deref().and_then(|contact_id| {
            profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.id == contact_id)
        });
        let mut facts = vec![
            LifecycleFact {
                label: "Erstellt".into(),
                value: timestamp(entry.record.request.created_at),
            },
            LifecycleFact {
                label: "Gueltig bis".into(),
                value: timestamp(entry.record.request.expires_at),
            },
        ];
        match entry.direction {
            DirectRequestDirection::Outgoing => outgoing_facts(entry, contact, &mut facts),
            DirectRequestDirection::Incoming => incoming_facts(entry, profiles, &mut facts),
        }
        append_failures(entry, &mut facts);
        let peer_name = contact
            .map(|contact| contact.display_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                if peer.device_name.trim().is_empty() {
                    format!(
                        "Geraet {}",
                        &peer.fingerprint[..peer.fingerprint.len().min(8)]
                    )
                } else {
                    peer.device_name.clone()
                }
            });
        let peer_device_id = contact
            .and_then(|contact| contact.remote_device_id.clone())
            .unwrap_or_else(|| peer.device_id.clone());
        let view = RequestView {
            request_id: entry.record.request.request_id.clone(),
            peer_device_id,
            peer_name,
            fingerprint: peer.fingerprint.clone(),
            decision: entry.record.decision.state,
            facts,
            can_decide: entry.direction == DirectRequestDirection::Incoming
                && entry.record.decision.state == DirectDecisionState::Pending
                && entry.record.request.expires_at >= now,
            can_retry: !entry.pending_outboxes(now).is_empty(),
            can_delete: entry.direction == DirectRequestDirection::Outgoing
                || entry.record.decision.state == DirectDecisionState::Pending
                || entry.removable_from_history(now),
        };
        match entry.direction {
            DirectRequestDirection::Incoming => incoming.push(view),
            DirectRequestDirection::Outgoing => outgoing.push(view),
        }
    }
    incoming.sort_by_key(|view| std::cmp::Reverse(created_at(profiles, &view.request_id)));
    outgoing.sort_by_key(|view| std::cmp::Reverse(created_at(profiles, &view.request_id)));
    (incoming, outgoing)
}

pub(super) fn authorized_device_views(profiles: &ShareProfiles) -> Vec<AuthorizedDeviceView> {
    let mut views = Vec::new();
    for grant in &profiles.direct_grants {
        let accepted_request = profiles
            .direct_requests
            .iter()
            .filter(|entry| {
                entry.direction == DirectRequestDirection::Incoming
                    && entry.record.request.requester.device_id == grant.device_id
                    && entry.record.decision.state == DirectDecisionState::Accepted
            })
            .max_by_key(|entry| entry.record.decision.changed_at)
            .map(|entry| entry.record.request.request_id.clone());
        let authorization = match grant.state {
            crate::share::DirectGrantState::Accepted => "active — Zugriff erlaubt",
            crate::share::DirectGrantState::Ignored => "inactive — Zugriff gesperrt",
        }
        .to_string();
        let connectivity = profiles
            .direct_contacts
            .iter()
            .find(|contact| contact.remote_device_id.as_deref() == Some(&grant.device_id))
            .map(|contact| contact.status.label())
            .unwrap_or_else(|| "unknown — keine aktive Sitzung gemeldet".into());
        views.push(AuthorizedDeviceView {
            device_id: grant.device_id.clone(),
            device_name: grant.device_name.clone(),
            fingerprint: grant.fingerprint.clone(),
            authorization,
            connectivity,
            updated_at: timestamp(grant.updated_at),
            accepted_request,
        });
    }
    views.sort_by(|left, right| left.device_name.cmp(&right.device_name));
    views
}

fn outgoing_facts(
    entry: &crate::share::DirectRequestEntry,
    contact: Option<&crate::share::DirectContact>,
    facts: &mut Vec<LifecycleFact>,
) {
    facts.push(LifecycleFact {
        label: "Lokaler Versand Anfrage".into(),
        value: transport_state(&entry.retries.request),
    });
    facts.push(LifecycleFact {
        label: "Peer-Empfang Anfrage".into(),
        value: if entry.request_receipt.is_some() {
            "received — signierte Empfangsbestaetigung".into()
        } else if entry.decision.is_some() {
            "received — durch signierte Entscheidung belegt".into()
        } else {
            "unconfirmed — keine Peer-Bestaetigung".into()
        },
    });
    facts.push(LifecycleFact {
        label: "Entscheidung vom Peer".into(),
        value: decision_label(entry.record.decision.state).into(),
    });
    if entry.decision_receipt.is_some() {
        facts.push(LifecycleFact {
            label: "Bestaetigung an Peer".into(),
            value: transport_state(&entry.retries.decision_receipt),
        });
    }
    facts.push(LifecycleFact {
        label: "Autorisierung".into(),
        value: contact
            .map(|contact| match contact.access_state {
                crate::share::DirectAccessState::Accepted => {
                    "active — vom Peer freigegeben".to_string()
                }
                crate::share::DirectAccessState::Pending => {
                    "pending — Entscheidung offen".to_string()
                }
                crate::share::DirectAccessState::Ignored => "inactive — abgelehnt".to_string(),
                crate::share::DirectAccessState::IdentityConflict => {
                    "inactive — Identitaetskonflikt".to_string()
                }
            })
            .unwrap_or_else(|| "unknown — Kontakt fehlt".into()),
    });
    facts.push(LifecycleFact {
        label: "Verbindung".into(),
        value: contact
            .map(|contact| contact.status.label())
            .unwrap_or_else(|| "unknown — Kontakt fehlt".into()),
    });
    append_retry("Anfrage-Retry", &entry.retries.request, facts);
    if entry.decision_receipt.is_some() {
        append_retry(
            "Entscheidungsbestaetigung-Retry",
            &entry.retries.decision_receipt,
            facts,
        );
    }
}

fn incoming_facts(
    entry: &crate::share::DirectRequestEntry,
    profiles: &ShareProfiles,
    facts: &mut Vec<LifecycleFact>,
) {
    facts.push(LifecycleFact {
        label: "Lokaler Empfang Anfrage".into(),
        value: "received — dauerhaft gespeichert".into(),
    });
    if entry.request_receipt.is_some() {
        facts.push(LifecycleFact {
            label: "Empfangsbestaetigung an Peer".into(),
            value: transport_state(&entry.retries.request_receipt),
        });
    }
    facts.push(LifecycleFact {
        label: "Lokale Entscheidung".into(),
        value: decision_label(entry.record.decision.state).into(),
    });
    if entry.decision.is_some() {
        facts.push(LifecycleFact {
            label: "Lokaler Versand Entscheidung".into(),
            value: transport_state(&entry.retries.decision),
        });
        facts.push(LifecycleFact {
            label: "Peer-Empfang Entscheidung".into(),
            value: if entry.decision_receipt.is_some() {
                "received — signierte Bestaetigung".into()
            } else {
                "unconfirmed — keine Peer-Bestaetigung".into()
            },
        });
    }
    let grant = profiles
        .grant_for(&entry.record.request.requester.device_id)
        .map(|grant| match grant.state {
            crate::share::DirectGrantState::Accepted => "active — Zugriff erlaubt",
            crate::share::DirectGrantState::Ignored => "inactive — Zugriff gesperrt",
        })
        .unwrap_or("inactive — keine Freigabe");
    facts.push(LifecycleFact {
        label: "Autorisierung".into(),
        value: grant.into(),
    });
    facts.push(LifecycleFact {
        label: "Verbindung".into(),
        value: "unknown — keine aktive Sitzung gemeldet".into(),
    });
    if entry.request_receipt.is_some() {
        append_retry(
            "Empfangsbestaetigung-Retry",
            &entry.retries.request_receipt,
            facts,
        );
    }
    if entry.decision.is_some() {
        append_retry("Entscheidungs-Retry", &entry.retries.decision, facts);
    }
}

fn append_retry(label: &str, retry: &DirectRetryState, facts: &mut Vec<LifecycleFact>) {
    let mut value = format!("Versuche: {}", retry.attempt_count);
    if let Some(last) = retry.last_attempt_at {
        value.push_str(&format!("; zuletzt {}", timestamp(last)));
    }
    if let Some(outcome) = retry.relay_outcome {
        let outcome = match outcome {
            DirectRelayOutcome::Forwarded => "forwarded",
            DirectRelayOutcome::TargetOffline => "target_offline",
        };
        value.push_str(&format!("; Relay: {outcome}"));
        if let Some(changed) = retry.relay_changed_at {
            value.push_str(&format!(" ({})", timestamp(changed)));
        }
    }
    if let Some(error) = &retry.last_error {
        value.push_str(&format!("; Fehler {}: {}", error.code, error.message));
    }
    facts.push(LifecycleFact {
        label: label.into(),
        value,
    });
}

fn append_failures(entry: &crate::share::DirectRequestEntry, facts: &mut Vec<LifecycleFact>) {
    if let Some(failure) = &entry.record.delivery.failure {
        facts.push(LifecycleFact {
            label: "Anfragefehler".into(),
            value: format!(
                "{}: {} ({})",
                failure.code,
                failure.message,
                timestamp(entry.record.delivery.changed_at)
            ),
        });
    }
    if let Some(failure) = &entry.record.decision.failure {
        facts.push(LifecycleFact {
            label: "Entscheidungsfehler".into(),
            value: format!(
                "{}: {} ({})",
                failure.code,
                failure.message,
                timestamp(entry.record.decision.changed_at)
            ),
        });
    }
}

fn transport_state(retry: &DirectRetryState) -> String {
    match retry.relay_outcome {
        Some(DirectRelayOutcome::Forwarded) => {
            "relay_forwarded — Peer-Empfang nicht bestaetigt".into()
        }
        Some(DirectRelayOutcome::TargetOffline) => {
            "sent — Relay meldet target_offline; Retry aktiv".into()
        }
        None if retry.attempt_count == 0 => "queued — lokal dauerhaft vorgemerkt".into(),
        None => "sent — an Relay gesendet; Peer-Empfang offen".into(),
    }
}

fn decision_label(state: DirectDecisionState) -> &'static str {
    match state {
        DirectDecisionState::Pending => "pending",
        DirectDecisionState::Accepted => "accepted",
        DirectDecisionState::Rejected => "rejected",
        DirectDecisionState::Revoked => "revoked",
        DirectDecisionState::Failed => "failed",
        DirectDecisionState::Expired => "expired",
    }
}

fn timestamp(value: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(value, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| format!("{value} (ungueltig)"))
}

fn created_at(profiles: &ShareProfiles, request_id: &DirectRequestId) -> i64 {
    profiles
        .direct_request(request_id)
        .map(|entry| entry.record.request.created_at)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "share_lifecycle_view_tests.rs"]
mod tests;
