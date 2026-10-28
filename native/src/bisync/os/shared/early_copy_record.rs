//! The scan with its early copies, and the recording of those copies where
//! the complete plan wants the same copy (G4 MS14, `early_copy.rs`).
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::AtomicBool;

use super::super::completion::{ApplySink, CompletedAction, CompletedKind};
use super::super::incremental::SyncEndpoints;
use super::super::keys::{KeyPolicy, Spellings};
use super::super::snapshot::WalkFilter;
use super::super::snapshot_pair::{read_pair, read_pair_with, PairSnapshot};
use super::super::types::{Action, Baseline, BisyncOptions, PairSide};
use super::{index, EarlyCopy, EarlyLane};

/// Reads the pair; with a lane, its workers copy new files while the walks
/// still run. The lane stops when both walks ended; running copies finish.
pub(in super::super) fn read_pair_copying(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    baseline: &Baseline,
    lane: Option<EarlyLane<'_>>,
) -> (Result<PairSnapshot, (String, String)>, Vec<EarlyCopy>) {
    let Some(lane) = lane else {
        let read = read_pair(endpoints, opts, cancel, filter, baseline);
        return (read, Vec::new());
    };
    let read = std::thread::scope(|scope| {
        for _ in 0..lane.workers() {
            let worker = || {
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| lane.work()));
            };
            let _ = std::thread::Builder::new()
                .name("sync-early".into())
                .spawn_scoped(scope, worker);
        }
        // Ends the lane also when the scan unwinds, so the scope can join.
        let _stop = StopLane(&lane);
        read_pair_with(endpoints, opts, cancel, filter, baseline, Some(&lane))
    });
    (read, lane.into_done())
}

/// Records every early copy the complete plan also wants as that action's
/// completion and removes the action; returns the copies per direction
/// (A→B, B→A). An early copy the plan does not want stays on the target as
/// a new file without a baseline record; the next run compares it.
pub(in super::super) fn record(
    early: Vec<EarlyCopy>,
    actions: &mut Vec<Action>,
    spellings: &Spellings,
    keys: KeyPolicy,
    sink: &dyn ApplySink,
) -> [u64; 2] {
    let mut copied = [0u64; 2];
    if early.is_empty() {
        return copied;
    }
    let mut by_key = early
        .into_iter()
        .map(|copy| ((index(copy.from), copy.key.clone()), copy))
        .collect::<HashMap<_, _>>();
    actions.retain(|action| {
        let (from, rel) = match action {
            Action::CopyAtoB(rel) => (PairSide::A, rel),
            Action::CopyBtoA(rel) => (PairSide::B, rel),
            _ => return true,
        };
        let lookup = (index(from), keys.key(rel).into_owned());
        let Some(copy) = by_key.get(&lookup) else {
            return true;
        };
        if spellings.side_rel(rel, from.other()) != copy.target_rel {
            return true;
        }
        let Some(copy) = by_key.remove(&lookup) else {
            return true;
        };
        sink.completed(CompletedAction {
            rel: rel.clone(),
            kind: CompletedKind::Copied { from },
            src_sig: Some(copy.source),
            dst_sig: Some(copy.destination),
            durable: copy.durable,
        });
        copied[index(from)] += 1;
        false
    });
    if !by_key.is_empty() {
        super::super::run_log::note(
            "Früh kopiert",
            &format!(
                "{} früh kopierte Dateien plant der Abgleich anders; der nächste Lauf prüft sie",
                by_key.len()
            ),
        );
    }
    copied
}

struct StopLane<'l, 'a>(&'l EarlyLane<'a>);

impl Drop for StopLane<'_, '_> {
    fn drop(&mut self) {
        self.0.stop();
    }
}
