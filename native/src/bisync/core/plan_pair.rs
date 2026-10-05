//! Planning one pair (V3): filters per pair, planning keys, one decision per
//! key, the receiving side's limits (FS6/Y154: too large or impossible names
//! are left out up front) and folders.
use std::collections::BTreeSet;

use super::keys::{KeyPolicy, PathAliases};
use super::omissions::{OmissionKind, SyncOmissions};
use super::plan_decide::{decide, Decision, Observed, Step};
use super::plan_dirs::{plan_dirs, DirSides};
use super::plan_filter::apply_pair_filter;
use super::plan_index::{destination_spelling, dirs_by_key_spelled, index_spelled, Keyed};
use super::plan_types::{PairPlan, PlanContext};
use super::snapshot_types::SideSnapshot;
use super::types::{Action, Baseline, Conflict, PairSide};

/// Plans both observed sides against the stored baseline. The snapshots are
/// adjusted to what was planned (filter decisions, protected entries
/// removed) and serve as apply's expected states afterwards.
pub fn plan_pair(
    a: &mut SideSnapshot,
    b: &mut SideSnapshot,
    base: &Baseline,
    ctx: &PlanContext<'_>,
) -> PairPlan {
    plan_pair_spelled(a, b, base, ctx, &PathAliases::default())
}

pub(crate) fn plan_pair_spelled(
    a: &mut SideSnapshot,
    b: &mut SideSnapshot,
    base: &Baseline,
    ctx: &PlanContext<'_>,
    aliases: &PathAliases,
) -> PairPlan {
    let keys = ctx.keys;
    let mut plan = PairPlan {
        files_a: (a.tree.len() + a.filtered.len()) as u64,
        files_b: (b.tree.len() + b.filtered.len()) as u64,
        ..PairPlan::default()
    };
    let mut omissions = SyncOmissions::new(keys.fold_case);
    omissions.extend(std::mem::take(&mut a.omissions));
    omissions.extend(std::mem::take(&mut b.omissions));
    apply_pair_filter(a, b, ctx.opts.direction, keys, aliases, &mut omissions);
    protect_directory_collisions(a, b, keys, aliases, &mut omissions);
    protect_target_collisions(a, b, keys, aliases, &mut omissions);
    aliases.protect_counterparts(&mut omissions, keys);
    omissions.exclude_tree(&mut a.tree);
    omissions.exclude_tree(&mut b.tree);
    let planning_base = omissions.planning_baseline(base);
    let pair = index_spelled(&a.tree, &b.tree, &planning_base, keys, aliases);
    let mut collided = BTreeSet::new();
    for (side, rel) in &pair.collisions {
        omissions.record_kind(rel, OmissionKind::NameImpossibleOnTarget, true);
        collided.insert(aliases.key(rel, *side, keys));
    }
    aliases.protect_counterparts(&mut omissions, keys);
    let dirs_a = dirs_by_key_spelled(&a.dirs, PairSide::A, keys, aliases);
    let dirs_b = dirs_by_key_spelled(&b.dirs, PairSide::B, keys, aliases);
    let mut planner = Planner {
        ctx,
        keys,
        aliases,
        dirs_a: &dirs_a,
        dirs_b: &dirs_b,
        filled_a: BTreeSet::new(),
        filled_b: BTreeSet::new(),
        plan: &mut plan,
        omissions: &mut omissions,
    };
    for (key, keyed) in &pair.entries {
        if !collided.contains(key) {
            planner.plan_key(keyed);
        }
    }
    let Planner {
        filled_a, filled_b, ..
    } = planner;
    let sides = DirSides {
        a: &dirs_a,
        b: &dirs_b,
        filled_a: &filled_a,
        filled_b: &filled_b,
    };
    let (dirs, in_sync) = plan_dirs(&sides, ctx, aliases, &omissions);
    plan.dirs = dirs;
    plan.dirs_in_sync = in_sync;
    plan.omissions = omissions;
    plan
}

fn protect_directory_collisions(
    a: &SideSnapshot,
    b: &SideSnapshot,
    keys: KeyPolicy,
    aliases: &PathAliases,
    omissions: &mut SyncOmissions,
) {
    let mut directory_keys = std::collections::BTreeMap::<String, String>::new();
    for (side, snapshot) in [(PairSide::A, a), (PairSide::B, b)] {
        let mut seen = std::collections::BTreeMap::<String, &str>::new();
        for dir in &snapshot.dirs {
            let key = aliases.key(dir, side, keys);
            if let Some(previous) = seen.insert(key.clone(), dir) {
                omissions.record_kind(previous, OmissionKind::NameImpossibleOnTarget, true);
                omissions.record_kind(dir, OmissionKind::NameImpossibleOnTarget, true);
            }
            directory_keys.insert(key, dir.clone());
        }
    }
    for (side, tree) in [(PairSide::A, &a.tree), (PairSide::B, &b.tree)] {
        for rel in tree.keys() {
            if let Some(dir) = directory_keys.get(&aliases.key(rel, side, keys)) {
                omissions.record_kind(dir, OmissionKind::NameImpossibleOnTarget, true);
                omissions.record_kind(rel, OmissionKind::NameImpossibleOnTarget, true);
            }
        }
    }
}

fn protect_target_collisions(
    a: &SideSnapshot,
    b: &SideSnapshot,
    keys: KeyPolicy,
    aliases: &PathAliases,
    omissions: &mut SyncOmissions,
) {
    if aliases.is_empty() {
        return;
    }
    for (side, source, target) in [(PairSide::A, a, b), (PairSide::B, b, a)] {
        let to = side.other();
        let target_dirs = dirs_by_key_spelled(&target.dirs, to, keys, aliases);
        for rel in source.dirs.iter().chain(source.tree.keys()) {
            let logical = aliases.logical(rel, side, keys);
            let mapped = aliases.spelling(&logical, to, keys);
            let physical = if mapped != logical.as_ref() {
                mapped
            } else {
                destination_spelling(&logical, &target_dirs, keys)
            };
            if aliases.key(&physical, to, keys) == keys.key(&logical).as_ref() {
                continue;
            }
            let logical_parts: Vec<_> = logical.split('/').collect();
            let physical_parts: Vec<_> = physical.split('/').collect();
            for end in 1..=physical_parts.len().min(logical_parts.len()) {
                let destination = physical_parts[..end].join("/");
                let wanted = logical_parts[..end].join("/");
                let owner = aliases.logical(&destination, to, keys);
                if keys.key(&owner) != keys.key(&wanted) {
                    omissions.record_kind(&wanted, OmissionKind::NameImpossibleOnTarget, true);
                    omissions.record_kind(&destination, OmissionKind::NameImpossibleOnTarget, true);
                    omissions.record_kind(&owner, OmissionKind::NameImpossibleOnTarget, true);
                    break;
                }
            }
        }
    }
}

struct Planner<'p, 'c> {
    ctx: &'p PlanContext<'c>,
    keys: KeyPolicy,
    aliases: &'p PathAliases,
    dirs_a: &'p std::collections::BTreeMap<String, String>,
    dirs_b: &'p std::collections::BTreeMap<String, String>,
    filled_a: BTreeSet<String>,
    filled_b: BTreeSet<String>,
    plan: &'p mut PairPlan,
    omissions: &'p mut SyncOmissions,
}

impl Planner<'_, '_> {
    fn plan_key(&mut self, keyed: &Keyed) {
        let rel = keyed.rel().to_string();
        let seen = Observed {
            a: keyed.sig(PairSide::A),
            b: keyed.sig(PairSide::B),
            base: keyed.base_entry(),
        };
        for side in [PairSide::A, PairSide::B] {
            if let Some(spelling) = keyed.spelling(side) {
                self.plan.spellings.insert(&rel, side, spelling);
            }
        }
        let base_rel = keyed.base.as_ref().map(|(stored, _)| stored.as_str());
        let respelled = base_rel.filter(|stored| *stored != rel);
        match decide(&seen, self.ctx) {
            Decision::Nothing => {
                self.plan.forget.extend(keyed.stale.iter().cloned());
                // Re-record an unchanged entry under the current spelling.
                if let (Some(stored), Some(entry)) = (respelled, seen.base) {
                    self.plan.forget.push(stored.to_string());
                    self.plan.records.push((rel, entry));
                }
            }
            Decision::Record => {
                self.plan.forget.extend(keyed.stale.iter().cloned());
                if let Some(stored) = respelled {
                    self.plan.forget.push(stored.to_string());
                }
                self.plan.records.push((rel, (seen.a, seen.b)));
            }
            Decision::Forget => {
                self.plan.forget.extend(keyed.stale.iter().cloned());
                if let Some(stored) = base_rel {
                    self.plan.forget.push(stored.to_string());
                }
            }
            Decision::Conflict => self.plan.conflicts.push(Conflict {
                rel,
                a: seen.a,
                b: seen.b,
                duplicates: None,
            }),
            Decision::Verify => self.plan.verify.push(rel),
            Decision::Act(step) => self.admit(step, rel, keyed, &seen),
        }
    }

    /// Adds the action unless the receiving side cannot hold the file:
    /// larger than its file-system limit, or a name it cannot store. Those
    /// are reported omissions, never run errors (Y154, FS6).
    fn admit(&mut self, step: Step, rel: String, keyed: &Keyed, seen: &Observed) {
        let incoming = match step {
            Step::Copy { from } | Step::KeepBoth { winner: from } => Some(from),
            Step::FinalizeMove { .. } | Step::Delete { .. } => None,
        };
        if let Some(from) = incoming {
            let to = from.other();
            let limits = *self.ctx.limits(to);
            let size = match from {
                PairSide::A => seen.a,
                PairSide::B => seen.b,
            }
            .map_or(0, |sig| sig.size);
            if !limits.fits_size(size) {
                self.omissions
                    .record_kind(&rel, OmissionKind::TooLargeForTarget, true);
                return;
            }
            let target = match keyed.spelling(to) {
                Some(existing) => existing.to_string(),
                None => {
                    let dirs = match to {
                        PairSide::A => self.dirs_a,
                        PairSide::B => self.dirs_b,
                    };
                    let mapped = self.aliases.spelling(&rel, to, self.keys);
                    let spelled = if mapped != rel {
                        mapped
                    } else {
                        destination_spelling(&rel, dirs, self.keys)
                    };
                    if spelled
                        .split('/')
                        .any(|name| limits.name_issue(name).is_some())
                    {
                        self.omissions.record_kind(
                            &rel,
                            OmissionKind::NameImpossibleOnTarget,
                            true,
                        );
                        return;
                    }
                    self.plan.spellings.insert(&rel, to, &spelled);
                    spelled
                }
            };
            let filled = match to {
                PairSide::A => &mut self.filled_a,
                PairSide::B => &mut self.filled_b,
            };
            for (index, _) in target.match_indices('/') {
                filled.insert(self.aliases.key(&target[..index], to, self.keys));
            }
        }
        self.plan.actions.push(action_for(step, rel));
    }
}

fn action_for(step: Step, rel: String) -> Action {
    match step {
        Step::Copy { from: PairSide::A } => Action::CopyAtoB(rel),
        Step::Copy { from: PairSide::B } => Action::CopyBtoA(rel),
        Step::FinalizeMove { from: PairSide::A } => Action::FinalizeMoveAtoB(rel),
        Step::FinalizeMove { from: PairSide::B } => Action::FinalizeMoveBtoA(rel),
        Step::Delete { side: PairSide::A } => Action::DeleteA(rel),
        Step::Delete { side: PairSide::B } => Action::DeleteB(rel),
        Step::KeepBoth {
            winner: PairSide::A,
        } => Action::KeepBothAtoB(rel),
        Step::KeepBoth {
            winner: PairSide::B,
        } => Action::KeepBothBtoA(rel),
    }
}
