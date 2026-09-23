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

pub(super) fn read_pair(
    endpoints: SyncEndpoints<'_>, opts: BisyncOptions, cancel: &AtomicBool,
    filter: &WalkFilter, baseline: &Baseline,
) -> Result<PairSnapshot, (String, String)> {
    let SyncEndpoints { a, root_a, b, root_b } = endpoints;
    let fold_case = !a.case_sensitive_paths(root_a) || !b.case_sensitive_paths(root_b);
    let at = walk_snapshot(a, root_a, cancel, filter, hash_mode(a, b, opts.compare),
        Some(&prev_side(baseline, true)),
        opts.delete == DeletePolicy::Mirror && opts.direction == Direction::BtoA, fold_case)
        .map_err(|error| (root_a.into(), error.to_string()))?;
    let bt = walk_snapshot(b, root_b, cancel, filter, hash_mode(b, a, opts.compare),
        Some(&prev_side(baseline, false)),
        opts.delete == DeletePolicy::Mirror && opts.direction == Direction::AtoB, fold_case)
        .map_err(|error| (root_b.into(), error.to_string()))?;
    let mut omissions = at.omissions;
    omissions.extend(bt.omissions);
    let (mut a, mut b) = (at.tree, bt.tree);
    omissions.exclude_tree(&mut a);
    omissions.exclude_tree(&mut b);
    Ok(PairSnapshot { a, b, omissions })
}
