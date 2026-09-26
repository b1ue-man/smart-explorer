//! Keeps one Share snapshot response inside the IPC line budget.
use serde::Serialize;

use super::{IpcResponse, ShareWorkerSnapshot, MAX_IPC_LINE};

const MAX_EVENT_BYTES: usize = 384 * 1024;
const MAX_LEGACY_REQUEST_BYTES: usize = 256 * 1024;
const MAX_CANDIDATE_BYTES: usize = 128 * 1024;
const MAX_STATUS_TEXT_BYTES: usize = 16 * 1024;

pub(in crate::daemon) fn bound_snapshot_for_ipc(
    mut snapshot: ShareWorkerSnapshot,
) -> ShareWorkerSnapshot {
    let dropped_events = retain_newest_with_budget(&mut snapshot.events, MAX_EVENT_BYTES);
    let dropped_legacy = retain_newest_with_budget(
        &mut snapshot.pending_direct_requests,
        MAX_LEGACY_REQUEST_BYTES,
    );
    let dropped_candidates =
        retain_newest_with_budget(&mut snapshot.candidates, MAX_CANDIDATE_BYTES);
    if let Some(error) = &mut snapshot.last_error {
        truncate_utf8(error, MAX_STATUS_TEXT_BYTES);
    }
    truncate_utf8(&mut snapshot.relay_url, MAX_STATUS_TEXT_BYTES);

    if dropped_events + dropped_legacy + dropped_candidates > 0 {
        snapshot.events.push(crate::share::ShareEvent::Error(format!(
            "Share status truncated transient backlog: events={dropped_events}, legacy_requests={dropped_legacy}, candidates={dropped_candidates}; durable request/profile state is complete"
        )));
    }

    while encoded_response_len(&snapshot) > MAX_IPC_LINE {
        if !snapshot.events.is_empty() {
            snapshot.events.remove(0);
            continue;
        }
        if snapshot.candidates.pop().is_some() {
            continue;
        }
        if snapshot.pending_direct_requests.pop().is_some() {
            continue;
        }
        break;
    }
    snapshot
}

fn retain_newest_with_budget<T: Serialize>(values: &mut Vec<T>, budget: usize) -> usize {
    let original_len = values.len();
    let mut used = 2usize;
    let mut kept = Vec::new();
    for value in std::mem::take(values).into_iter().rev() {
        let bytes = serde_json::to_vec(&value)
            .map(|encoded| encoded.len().saturating_add(1))
            .unwrap_or(budget.saturating_add(1));
        if used.saturating_add(bytes) <= budget {
            used += bytes;
            kept.push(value);
        }
    }
    kept.reverse();
    *values = kept;
    original_len.saturating_sub(values.len())
}

pub(super) fn encoded_response_len(snapshot: &ShareWorkerSnapshot) -> usize {
    serde_json::to_vec(&IpcResponse::ShareEvents {
        snapshot: Box::new(snapshot.clone()),
    })
    .map(|encoded| encoded.len().saturating_add(1))
    .unwrap_or(usize::MAX)
}

fn truncate_utf8(value: &mut String, max: usize) {
    if value.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}
