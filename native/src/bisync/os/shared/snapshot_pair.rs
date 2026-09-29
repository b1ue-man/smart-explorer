//! Complete observations for one planning pass, including protected omissions.
use std::sync::atomic::AtomicBool;

use super::incremental::SyncEndpoints;
use super::omissions::SyncOmissions;
use super::snapshot::{hash_mode, prev_side, walk_snapshot, WalkFilter};
use super::types::{Baseline, BisyncOptions, DeletePolicy, Direction, Tree};

pub(super) struct PairSnapshot {
    pub a: Tree,
    pub b: Tree,
    pub omissions: SyncOmissions,
}

/// Reads both sides at once: on a first sync against a remote (a Drive
/// download, say) the remote walk and the local walk overlap instead of
/// adding up. A failing side is reported as before, side A first.
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
    let mirror = opts.delete == DeletePolicy::Mirror;
    let duplicates_a = mirror && opts.direction == Direction::BtoA;
    let duplicates_b = mirror && opts.direction == Direction::AtoB;
    let (at, bt) = std::thread::scope(|scope| {
        let side_b = scope.spawn(|| {
            walk_snapshot(
                b,
                root_b,
                cancel,
                filter,
                hash_b,
                Some(&prev_b),
                duplicates_b,
                fold_case,
            )
        });
        let side_a = walk_snapshot(
            a,
            root_a,
            cancel,
            filter,
            hash_a,
            Some(&prev_a),
            duplicates_a,
            fold_case,
        );
        (side_a, side_b.join())
    });
    let at = at.map_err(|error| (root_a.to_string(), error.to_string()))?;
    let bt = match bt {
        Ok(result) => result.map_err(|error| (root_b.to_string(), error.to_string()))?,
        Err(_) => {
            return Err((
                root_b.to_string(),
                "Einlesen dieser Seite wurde unerwartet beendet".to_string(),
            ))
        }
    };
    let mut omissions = at.omissions;
    omissions.extend(bt.omissions);
    let (mut a, mut b) = (at.tree, bt.tree);
    omissions.exclude_tree(&mut a);
    omissions.exclude_tree(&mut b);
    Ok(PairSnapshot { a, b, omissions })
}
