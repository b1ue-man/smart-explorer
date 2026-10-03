//! The job's filters decided per pair (Y36/Y68/Y153): an entry that only one
//! side's filter left out never reads as a deletion. Two-way jobs leave such
//! an entry out on both sides; one-way jobs follow the source, so a
//! destination copy outside the filter is still updated or kept.
use std::collections::BTreeSet;

use super::keys::KeyPolicy;
use super::omissions::{OmissionKind, SyncOmissions};
use super::plan_decide::source_side;
use super::snapshot_types::SideSnapshot;
use super::types::{Direction, PairSide};

/// Applies the filter decisions: protected entries become `Filtered`
/// omissions (silent), destination entries a one-way source still has go
/// back into the destination's tree.
pub(super) fn apply_pair_filter(
    a: &mut SideSnapshot,
    b: &mut SideSnapshot,
    direction: Direction,
    keys: KeyPolicy,
    omissions: &mut SyncOmissions,
) {
    match source_side(direction) {
        None => {
            for side in [&mut *a, &mut *b] {
                for rel in side.filtered.keys() {
                    omissions.record_kind(rel, OmissionKind::Filtered, false);
                }
            }
        }
        Some(source) => {
            let (source_snapshot, destination) = match source {
                PairSide::A => (a, b),
                PairSide::B => (b, a),
            };
            for rel in source_snapshot.filtered.keys() {
                omissions.record_kind(rel, OmissionKind::Filtered, false);
            }
            if destination.filtered.is_empty() {
                return;
            }
            let source_keys: BTreeSet<String> = source_snapshot
                .tree
                .keys()
                .map(|rel| keys.key(rel).into_owned())
                .collect();
            for (rel, sig) in std::mem::take(&mut destination.filtered) {
                if source_keys.contains(keys.key(&rel).as_ref()) {
                    destination.tree.insert(rel, sig);
                } else {
                    omissions.record_kind(&rel, OmissionKind::Filtered, false);
                    destination.filtered.insert(rel, sig);
                }
            }
        }
    }
}
