//! Folders (FS5, Y51/Y70/Y108): empty folders are carried over, a folder one
//! side removed is removed on the other (apply removes it only while it is
//! empty), a mirror gives the destination the source's folders. Without a
//! folder history (ad-hoc syncs) folders are only ever added.
use std::collections::{BTreeMap, BTreeSet};

use super::completion::DirAction;
use super::keys::PathAliases;
use super::omissions::SyncOmissions;
use super::plan_decide::{may_send, source_side};
use super::plan_index::destination_spelling;
use super::plan_types::PlanContext;
use super::snapshot_types::DirSet;
use super::types::{DeletePolicy, PairSide};

/// Folders of both sides by planning key (each in its side's spelling) and
/// the keys below which planned copies create folders anyway.
pub(super) struct DirSides<'d> {
    pub(super) a: &'d BTreeMap<String, String>,
    pub(super) b: &'d BTreeMap<String, String>,
    pub(super) filled_a: &'d BTreeSet<String>,
    pub(super) filled_b: &'d BTreeSet<String>,
}

impl DirSides<'_> {
    fn dirs(&self, side: PairSide) -> &BTreeMap<String, String> {
        match side {
            PairSide::A => self.a,
            PairSide::B => self.b,
        }
    }

    fn filled(&self, side: PairSide) -> &BTreeSet<String> {
        match side {
            PairSide::A => self.filled_a,
            PairSide::B => self.filled_b,
        }
    }
}

/// Folder actions (creations parents first, then removals deepest first) and
/// the folders both sides already have.
pub(super) fn plan_dirs(
    sides: &DirSides<'_>,
    ctx: &PlanContext<'_>,
    aliases: &PathAliases,
    omissions: &SyncOmissions,
) -> (Vec<DirAction>, DirSet) {
    let history: Option<BTreeSet<String>> = ctx.base_dirs.map(|dirs| {
        dirs.iter()
            .map(|dir| ctx.keys.key(dir).into_owned())
            .collect()
    });
    let mut creates = Vec::new();
    let mut removes = Vec::new();
    let mut in_sync = DirSet::new();
    let keys: BTreeSet<&String> = sides.a.keys().chain(sides.b.keys()).collect();
    for key in keys {
        let (in_a, in_b) = (sides.a.get(key), sides.b.get(key));
        let (present, spelling) = match (in_a, in_b) {
            (Some(spelling), Some(_)) => {
                if !omissions.protects(spelling) {
                    in_sync.insert(
                        aliases
                            .logical(spelling, PairSide::A, ctx.keys)
                            .into_owned(),
                    );
                }
                continue;
            }
            (Some(spelling), None) => (PairSide::A, spelling),
            (None, Some(spelling)) => (PairSide::B, spelling),
            (None, None) => continue,
        };
        if omissions.protects(spelling) {
            continue;
        }
        let missing = present.other();
        let was_on_both = history
            .as_ref()
            .is_some_and(|history| history.contains(key));
        let create = match one_sided(present, was_on_both, ctx) {
            Some(Change::Create) => true,
            Some(Change::Remove) => {
                removes.push(DirAction::Remove {
                    side: present,
                    rel: spelling.clone(),
                });
                false
            }
            None => false,
        };
        if !create || sides.filled(missing).contains(key) {
            continue;
        }
        let logical = aliases.logical(spelling, present, ctx.keys);
        let mapped = aliases.spelling(&logical, missing, ctx.keys);
        let rel = if mapped != logical.as_ref() {
            mapped
        } else {
            destination_spelling(&logical, sides.dirs(missing), ctx.keys)
        };
        let limits = ctx.limits(missing);
        if rel.split('/').all(|name| limits.name_issue(name).is_none()) {
            creates.push(DirAction::Create { side: missing, rel });
        }
    }
    removes.reverse();
    creates.extend(removes);
    (creates, in_sync)
}

enum Change {
    /// Create the folder on the side that misses it.
    Create,
    /// Remove it from the side that has it.
    Remove,
}

/// A folder only `present` has.
fn one_sided(present: PairSide, was_on_both: bool, ctx: &PlanContext<'_>) -> Option<Change> {
    let missing = present.other();
    let direction = ctx.opts.direction;
    if ctx.opts.delete == DeletePolicy::Mirror {
        if let Some(source) = source_side(direction) {
            return Some(if source == present {
                Change::Create
            } else {
                Change::Remove
            });
        }
    }
    if !was_on_both {
        return may_send(direction, present).then_some(Change::Create);
    }
    // Both had it at the last run: `missing` removed it.
    if may_send(direction, missing) {
        return (ctx.opts.delete != DeletePolicy::NoDelete).then_some(Change::Remove);
    }
    // The destination of a one-way job lost it (Y40): recreate it.
    may_send(direction, present).then_some(Change::Create)
}
