use super::ShareHost;

pub(crate) const MAX_PENDING_LEGACY_EVENTS: usize = 64;

pub(crate) struct LegacyPersistBatch {
    pub committed: Option<crate::share::ShareProfiles>,
    pub retry: Vec<(String, crate::share::PeerPresence)>,
    pub rejected: Vec<String>,
}

pub(crate) fn persist_all(events: Vec<(String, crate::share::PeerPresence)>) -> LegacyPersistBatch {
    let now = crate::share::core_now_secs();
    let mut batch = LegacyPersistBatch {
        committed: None,
        retry: Vec::new(),
        rejected: Vec::new(),
    };
    for (lookup_id, presence) in events {
        let result = crate::share::ShareIdentity::with_current_locked(
            super::default_device_name(),
            |identity| {
                if lookup_id != identity.direct_lookup_id {
                    return Err("legacy presence belongs to a stale local identity".into());
                }
                let mut verified = crate::share::ShareProfiles::default();
                verified.record_verified_legacy_direct_request(&lookup_id, &presence, now)?;
                verified.validate_legacy_evidence(identity)?;
                crate::share::ShareProfiles::mutate_persisted(
                    Some(super::default_home()),
                    |profiles| {
                        profiles.reconcile_legacy_identity(&identity.direct_lookup_id, now)?;
                        profiles.expire_legacy_direct_requests(now);
                        profiles
                            .record_verified_legacy_direct_request(&lookup_id, &presence, now)?;
                        profiles.validate_legacy_evidence(identity)
                    },
                )
            },
        );
        match result {
            Ok(committed) => batch.committed = Some(committed),
            Err(error) if permanent_event_error(&error) => batch.rejected.push(format!(
                "Legacy-Anfrage {} wurde dauerhaft abgewiesen: {error}",
                presence.device_id
            )),
            Err(error) => {
                if batch.retry.len() < MAX_PENDING_LEGACY_EVENTS
                    && !batch
                        .retry
                        .iter()
                        .any(|queued| same_event(queued, &lookup_id, &presence))
                {
                    batch.retry.push((lookup_id, presence));
                }
                batch.rejected.push(format!(
                    "Legacy-Anfrage konnte nicht gespeichert werden; Wiederholung vorgemerkt: {error}"
                ));
            }
        }
    }
    batch
}

pub(crate) fn enqueue(
    events: &mut Vec<(String, crate::share::PeerPresence)>,
    lookup_id: String,
    presence: crate::share::PeerPresence,
) -> Result<(), String> {
    if events
        .iter()
        .any(|queued| same_event(queued, &lookup_id, &presence))
    {
        return Ok(());
    }
    if events.len() >= MAX_PENDING_LEGACY_EVENTS {
        return Err(format!(
            "Legacy-Anfrage-Backlog ist voll (maximal {MAX_PENDING_LEGACY_EVENTS}); Event wurde verworfen"
        ));
    }
    events.push((lookup_id, presence));
    Ok(())
}

impl ShareHost {
    pub(crate) fn flush_legacy_answers(&self) {
        let (service, answers) = match self.state.lock() {
            Ok(state)
                if state.pending_profiles_base.is_none()
                    && state.pending_direct_events.is_empty()
                    && state.pending_legacy_events.is_empty() =>
            {
                (
                    state.service.clone(),
                    state
                        .profiles
                        .legacy_answers_due(crate::share::core_now_secs()),
                )
            }
            Ok(_) => return,
            Err(_) => return,
        };
        let Some(service) = service else {
            return;
        };
        for answer in answers {
            let result = service
                .cmd(crate::share::ShareCmd::AnswerLegacyDirectRequest {
                    selector: answer.selector.clone(),
                    decision_revision: answer.decision_revision,
                    lookup_id: answer.lookup_id,
                    requester_device_id: answer.requester_device_id,
                    accepted: answer.accepted,
                })
                .map(|_| ());
            let error = result.err();
            match crate::share::mark_legacy_answer_attempt(
                Some(super::default_home()),
                &answer.selector,
                answer.decision_revision,
                error.clone(),
            ) {
                Ok(committed) => {
                    if let Ok(mut state) = self.state.lock() {
                        state.profiles = committed;
                        if let Some(error) = error {
                            super::ui_events::push(
                                &mut state.ui_events,
                                crate::share::ShareEvent::Error(format!(
                                    "Legacy-Antwort blieb unbestaetigt und wird wiederholt: {error}"
                                )),
                            );
                        }
                    }
                }
                Err(persist_error) => {
                    if let Ok(mut state) = self.state.lock() {
                        super::ui_events::push(
                            &mut state.ui_events,
                            crate::share::ShareEvent::Error(format!(
                                "Legacy-Antwortstatus konnte nicht gespeichert werden: {persist_error}"
                            )),
                        );
                    }
                }
            }
        }
    }
}

fn same_event(
    queued: &(String, crate::share::PeerPresence),
    lookup_id: &str,
    presence: &crate::share::PeerPresence,
) -> bool {
    queued.0 == lookup_id
        && queued.1.device_id == presence.device_id
        && queued.1.public_key == presence.public_key
        && queued.1.node_id == presence.node_id
        && queued.1.nonce == presence.nonce
}

fn permanent_event_error(error: &str) -> bool {
    error.contains("legacy request inbox is full")
        || error.contains("legacy request selector conflict")
        || error.contains("invalid legacy")
        || error.contains("legacy presence")
        || error.contains("stale local identity")
        || error.contains("authentication no longer verifies")
}

#[cfg(test)]
mod tests {
    use super::{enqueue, MAX_PENDING_LEGACY_EVENTS};

    #[test]
    fn retry_backlog_is_deduplicated_and_bounded() {
        let mut events = Vec::new();
        let first = presence("same", "nonce");
        enqueue(&mut events, "lookup".into(), first.clone()).unwrap();
        enqueue(&mut events, "lookup".into(), first).unwrap();
        assert_eq!(events.len(), 1);
        for index in 1..MAX_PENDING_LEGACY_EVENTS {
            enqueue(
                &mut events,
                "lookup".into(),
                presence("same", &format!("nonce-{index}")),
            )
            .unwrap();
        }
        assert!(enqueue(
            &mut events,
            "lookup".into(),
            presence("overflow", "overflow")
        )
        .unwrap_err()
        .contains("Backlog ist voll"));
        assert_eq!(events.len(), MAX_PENDING_LEGACY_EVENTS);
    }

    fn presence(device_id: &str, nonce: &str) -> crate::share::PeerPresence {
        crate::share::PeerPresence {
            kind: "direct".into(),
            relation_id: "lookup".into(),
            device_id: device_id.into(),
            device_name: "Peer".into(),
            public_key: "public".into(),
            fingerprint: "fingerprint".into(),
            node_id: "node".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 1,
            nonce: nonce.into(),
            proof: "proof".into(),
        }
    }
}
