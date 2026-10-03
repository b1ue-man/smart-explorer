//! Read-only recovery discovery and bounded preview of the stored merge order.
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use crate::bisync::{Conflict, PendingMerge, RecordedMergeChoice, StateKey};
use crate::mobile::ApiError;
use super::super::sync_conflicts::{MergeDraft, PairContext};

pub(in super::super) fn load(pair: &PairContext, key: &StateKey, rel: &str,
    cancel: &AtomicBool) -> Result<Option<PendingMerge>, ApiError> {
    super::check(cancel)?;
    let pending = crate::bisync::pending_merge_for_key(
        &*pair.a, &pair.root_a, &*pair.b, &pair.root_b, key, rel)
        .map_err(|e| super::super::args::io_error("Gespeicherten Merge-Auftrag lesen", e))?;
    super::check(cancel)?;
    Ok(pending)
}

/// The key comes from the fresh, validated dry run; each order is validated
/// again against the original endpoints before its conflict enters the list.
pub(in super::super) fn conflicts(pair: &PairContext, key: &StateKey,
    cancel: &AtomicBool) -> Result<Vec<Conflict>, ApiError> {
    super::check(cancel)?;
    let relatives = {
        let lock = crate::bisync::PairLock::acquire(&key.lock_id)
            .map_err(|e| super::super::args::io_error("Merge-Paarsperre", e))?;
        crate::bisync::pending_merge_relatives(&lock, key)
            .map_err(|e| super::super::args::io_error("Offene Merge-Aufträge", e))?
    };
    let mut items = Vec::new();
    for rel in relatives {
        if let Some(pending) = load(pair, key, &rel, cancel)? {
            items.push(pending.conflict);
        }
    }
    Ok(items)
}

pub(super) fn draft(cid: &str, pending: PendingMerge) -> MergeDraft {
    let state = |sig: Option<crate::bisync::Sig>| sig.map_or((0, 0), |sig| (sig.size, sig.mtime_ms));
    MergeDraft { cid: cid.to_string(), rows: Vec::new(),
        state_a: state(pending.conflict.a), state_b: state(pending.conflict.b),
        text_a: String::new(), text_b: String::new(), retry: true, pending: Some(pending) }
}

pub(super) fn summary(pending: &PendingMerge) -> Value {
    let (kind, keep_a) = match pending.choice {
        RecordedMergeChoice::Write => ("write", None),
        RecordedMergeChoice::KeepBoth { keep_a } => ("keep_both", Some(keep_a)),
    };
    // This is display-only. The repeat uses the exact stored bytes in Rust.
    let preview = std::str::from_utf8(&pending.merged).ok().map(|text|
        text.chars().take(8 * 1024).collect::<String>());
    let truncated = preview.as_ref().is_some_and(|text| text.len() < pending.merged.len());
    json!({ "kind": kind, "keepA": keep_a, "confirmedA": pending.confirmed_a.is_some(),
        "confirmedB": pending.confirmed_b.is_some(), "preview": preview,
        "previewTruncated": truncated })
}
