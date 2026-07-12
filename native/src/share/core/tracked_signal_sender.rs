use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use super::core::{eio, now_secs};
use super::direct_ledger::DirectEnvelopeKind;
use super::direct_lifecycle::DirectFailure;
use super::direct_signal_event::DirectSignalEvent;
use super::signal_connection::{send_line, SignalConnection};
use super::tracked_signal_outbox::pending_envelopes;
use super::types::{ShareAuthState, ShareEvent};

pub(super) type AttemptCounters = HashMap<String, u32>;

pub(super) fn send_pending_tracked(
    signal: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    events: &crossbeam_channel::Sender<ShareEvent>,
    counters: &mut AttemptCounters,
) -> io::Result<usize> {
    let entries = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .direct_requests
        .clone();
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

        let result = send_line(signal, &envelope.wire_message());
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

fn attempt_key(request_id: &str, kind: DirectEnvelopeKind) -> String {
    let suffix = match kind {
        DirectEnvelopeKind::Request => "request",
        DirectEnvelopeKind::RequestReceipt => "request_receipt",
        DirectEnvelopeKind::Decision => "decision",
        DirectEnvelopeKind::DecisionReceipt => "decision_receipt",
    };
    format!("{request_id}:{suffix}")
}
