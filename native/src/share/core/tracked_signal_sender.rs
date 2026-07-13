use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use super::backend::ShareIrohNode;
use super::core::{eio, now_secs};
use super::direct_ledger::DirectEnvelopeKind;
use super::direct_lifecycle::DirectFailure;
use super::direct_protocol::SignedDirectRequest;
use super::direct_signal_event::DirectSignalEvent;
use super::profiles::ShareProfiles;
use super::signal_connection::{send_line, SignalConnection};
use super::signal_presence::build_presence;
use super::tracked_signal_outbox::pending_envelopes;
use super::types::{ShareAuthState, ShareEvent};

pub(super) type AttemptCounters = HashMap<String, u32>;

pub(super) fn send_pending_tracked(
    signal: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    events: &crossbeam_channel::Sender<ShareEvent>,
    counters: &mut AttemptCounters,
) -> io::Result<usize> {
    send_pending_tracked_with(signal, auth, events, counters, |state, request| {
        legacy_presence(state, request, iroh)
    })
}

pub(super) fn send_pending_tracked_with<F>(
    signal: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    events: &crossbeam_channel::Sender<ShareEvent>,
    counters: &mut AttemptCounters,
    mut build_legacy_presence: F,
) -> io::Result<usize>
where
    F: FnMut(&ShareAuthState, &SignedDirectRequest) -> Result<super::types::PeerPresence, String>,
{
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let entries = state.direct_requests.clone();
    let now = now_secs();
    let mut sent = 0;
    for pending in pending_envelopes(&entries, now) {
        let envelope = pending.envelope;
        let key = attempt_key(envelope.request_id().as_str(), envelope.kind());
        let previous = counters
            .get(&key)
            .copied()
            .unwrap_or_default()
            .max(pending.persisted_attempt_count);
        let attempt_count = previous
            .checked_add(1)
            .ok_or_else(|| eio("Direct-Retry-Zaehler ist erschoepft"))?;
        counters.insert(key, attempt_count);

        let legacy_presence = match &envelope {
            super::tracked_signal_outbox::TrackedOutboxEnvelope::Request(request) => {
                match build_legacy_presence(&state, request) {
                    Ok(presence) => Some(presence),
                    Err(error) => {
                        let _ = events.send(ShareEvent::Error(format!(
                            "Legacy-Fallback fuer Direct-Anfrage {} nicht verfuegbar; signierte Anfrage wird trotzdem gesendet: {error}",
                            request.request_id
                        )));
                        None
                    }
                }
            }
            _ => None,
        };
        let result = send_line(signal, &envelope.wire_message(legacy_presence));
        let failure = result.as_ref().err().map(|error| DirectFailure {
            code: "signal_send_failed".into(),
            message: error.to_string(),
        });
        let _ = events.send(ShareEvent::DirectSignal(
            DirectSignalEvent::EnvelopeAttempted {
                request_id: envelope.request_id().clone(),
                envelope: envelope.kind(),
                attempt_count,
                at: now,
                failure,
            },
        ));
        result?;
        sent += 1;
    }
    Ok(sent)
}

fn legacy_presence(
    state: &ShareAuthState,
    request: &SignedDirectRequest,
    iroh: &ShareIrohNode,
) -> Result<super::types::PeerPresence, String> {
    let entry = state
        .direct_requests
        .iter()
        .find(|entry| entry.record.request.request_id == request.request_id)
        .ok_or("Direct-Outbox-Eintrag fehlt")?;
    let contact_id = entry
        .contact_id
        .as_deref()
        .ok_or("Direktkontaktbezug fehlt")?;
    let contact = state
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .ok_or("Direktkontakt fehlt")?;
    let secret = ShareProfiles::direct_secret_checked(contact)?.ok_or("Direkt-Secret fehlt")?;
    build_presence("direct", &request.lookup_id, &state.identity, &secret, iroh)
        .map_err(|error| error.to_string())
}

fn attempt_key(request_id: &str, kind: DirectEnvelopeKind) -> String {
    let suffix = match kind {
        DirectEnvelopeKind::Request => "request",
        DirectEnvelopeKind::RequestReceipt => "request_receipt",
        DirectEnvelopeKind::Decision => "decision",
        DirectEnvelopeKind::DecisionReceipt => "decision_receipt",
    };
    format!("{request_id}:{suffix}")
}
