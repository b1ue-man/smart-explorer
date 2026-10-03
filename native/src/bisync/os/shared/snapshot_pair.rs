//! Complete observations for one planning pass, including protected omissions.
use std::io;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::ScopedJoinHandle;
use std::time::Duration;

use super::incremental::SyncEndpoints;
use super::omissions::SyncOmissions;
use super::snapshot::{
    hash_mode, prev_side, walk_snapshot_with_options, HashMode, Snapshot, WalkFilter,
};
use super::snapshot_types::SideSnapshot;
use super::types::{Action, Baseline, BisyncOptions, Conflict, DeletePolicy, Direction, Tree};
use crate::vfs::Backend;

/// The reading thread passes the user's cancel on to both walks this often
/// (a canceled read ends within about this delay, as the walks' own waits).
const WATCH_SLICE: Duration = Duration::from_millis(50);
const NO_FAILURE: u8 = 0;
const SIDE_A: u8 = 1;
const SIDE_B: u8 = 2;
const ENDED_UNEXPECTEDLY: &str = "Einlesen dieser Seite wurde unerwartet beendet";

pub(super) struct PairSnapshot {
    pub a: SideSnapshot,
    pub b: SideSnapshot,
    pub omissions: SyncOmissions,
    pub repairs: Vec<Conflict>,
    pub conflicts: Vec<Conflict>,
}

impl PairSnapshot {
    pub(super) fn plan(
        &self,
        base: &Baseline,
        opts: BisyncOptions,
    ) -> (Vec<Action>, Vec<Conflict>, Vec<String>) {
        let mut base = self.omissions.planning_baseline(base);
        for conflict in &self.conflicts {
            base.remove(&conflict.rel);
        }
        let (actions, mut conflicts, converged) =
            super::core::plan(&self.a.tree, &self.b.tree, &base, opts);
        conflicts.extend(self.conflicts.iter().cloned());
        (actions, conflicts, converged)
    }
}

type SideRead = Result<Snapshot, String>;

/// Reads both sides at once: on a first sync against a remote (a Drive
/// download, say) the remote walk and the local walk overlap instead of
/// adding up. Both walks share one stop flag: a side that fails (or cannot
/// start, or panics) stops the other at once, and the user's cancel reaches
/// both. The first real failure is reported; after a cancel, side A first as
/// before.
pub(super) fn read_pair(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    baseline: &Baseline,
) -> Result<PairSnapshot, (String, String)> {
    let SyncEndpoints {
        a,
        root_a,
        b,
        root_b,
    } = endpoints;
    let fold_case = !a.case_sensitive_paths(root_a) || !b.case_sensitive_paths(root_b);
    let (hash_a, hash_b) = (hash_mode(a, b, opts.compare), hash_mode(b, a, opts.compare));
    let (prev_a, prev_b) = (prev_side(baseline, true), prev_side(baseline, false));
    let (prev_a, prev_b) = (&prev_a, &prev_b);
    let mirror = opts.delete == DeletePolicy::Mirror;
    let duplicates_a = mirror && opts.direction == Direction::BtoA;
    let duplicates_b = mirror && opts.direction == Direction::AtoB;
    let stop = AtomicBool::new(false);
    let first_failure = AtomicU8::new(NO_FAILURE);
    let failed = |side: u8| {
        if !cancel.load(Ordering::Acquire) {
            let _ = first_failure.compare_exchange(
                NO_FAILURE,
                side,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        stop.store(true, Ordering::Release);
    };
    let read = &|side: u8,
                 backend: &dyn Backend,
                 root: &str,
                 hash: HashMode,
                 prev: &Tree,
                 duplicates: bool|
     -> SideRead {
        let walked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            walk_snapshot_with_options(
                backend,
                root,
                &stop,
                filter,
                hash,
                Some(prev),
                duplicates,
                fold_case,
                opts,
            )
        }));
        let failure = match walked {
            Ok(Ok(snapshot)) => return Ok(snapshot),
            Ok(Err(error)) => error.to_string(),
            Err(_) => ENDED_UNEXPECTEDLY.to_string(),
        };
        failed(side);
        Err(failure)
    };
    let (read_a, read_b) = std::thread::scope(|scope| {
        let side_a = std::thread::Builder::new()
            .name("sync-read-a".to_string())
            .spawn_scoped(scope, move || {
                read(SIDE_A, a, root_a, hash_a, prev_a, duplicates_a)
            });
        let side_b = std::thread::Builder::new()
            .name("sync-read-b".to_string())
            .spawn_scoped(scope, move || {
                read(SIDE_B, b, root_b, hash_b, prev_b, duplicates_b)
            });
        for (side, spawned) in [(SIDE_A, side_a.is_ok()), (SIDE_B, side_b.is_ok())] {
            if !spawned {
                failed(side);
            }
        }
        while still_running(&side_a) || still_running(&side_b) {
            if cancel.load(Ordering::Acquire) {
                stop.store(true, Ordering::Release);
            }
            std::thread::sleep(WATCH_SLICE);
        }
        (ended(side_a), ended(side_b))
    });
    let blame_b = first_failure.load(Ordering::Acquire) == SIDE_B;
    let (at, bt) = match (read_a, read_b) {
        (Ok(at), Ok(bt)) => (at, bt),
        (Err(error), _) if !blame_b => return Err((root_a.to_string(), error)),
        (_, Err(error)) => return Err((root_b.to_string(), error)),
        (Err(error), Ok(_)) => return Err((root_a.to_string(), error)),
    };
    let mut omissions = at.omissions.clone();
    omissions.extend(bt.omissions.clone());
    let (mut a, mut b) = (at.tree, bt.tree);
    omissions.exclude_tree(&mut a);
    omissions.exclude_tree(&mut b);
    let (repairs, conflicts) = super::duplicate_plan::prepare(
        endpoints,
        at.duplicates,
        bt.duplicates,
        &mut a,
        &mut b,
        &mut omissions,
        opts,
        cancel,
    )?;
    let a = SideSnapshot {
        tree: a,
        filtered: at.filtered,
        dirs: at.dirs,
        omissions: omissions.clone(),
    };
    let b = SideSnapshot {
        tree: b,
        filtered: bt.filtered,
        dirs: bt.dirs,
        omissions: omissions.clone(),
    };
    Ok(PairSnapshot {
        a,
        b,
        omissions,
        repairs,
        conflicts,
    })
}

fn still_running<T>(side: &io::Result<ScopedJoinHandle<'_, T>>) -> bool {
    side.as_ref().is_ok_and(|handle| !handle.is_finished())
}

fn ended(side: io::Result<ScopedJoinHandle<'_, SideRead>>) -> SideRead {
    match side {
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|_| Err(ENDED_UNEXPECTEDLY.to_string())),
        Err(error) => Err(format!(
            "Einlesen dieser Seite konnte nicht starten: {error}"
        )),
    }
}

#[cfg(test)]
#[path = "snapshot_pair_tests.rs"]
mod tests;
