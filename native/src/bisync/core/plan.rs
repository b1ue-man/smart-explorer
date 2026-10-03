//! Compatibility entry points for the shared pair planner. Runtime planning
//! uses complete side snapshots; callers of `plan` retain the tree-only API.
use std::collections::BTreeSet;

use super::compare::{same_entry, Against, TimeRules};
use super::plan_pair::plan_pair;
use super::plan_types::PlanContext;
use super::snapshot_types::SideSnapshot;
use super::types::{Action, Baseline, BisyncOptions, Conflict, Sig, Tree};

pub(super) fn sig_eq(x: Option<Sig>, y: Option<Sig>, opts: &BisyncOptions) -> bool {
    same_entry(x, y, TimeRules::EXACT.cross(), opts, Against::OtherSide)
}

pub fn plan(
    a: &Tree,
    b: &Tree,
    base: &Baseline,
    opts: BisyncOptions,
) -> (Vec<Action>, Vec<Conflict>, Vec<String>) {
    let mut a = SideSnapshot {
        tree: a.clone(),
        ..SideSnapshot::default()
    };
    let mut b = SideSnapshot {
        tree: b.clone(),
        ..SideSnapshot::default()
    };
    let planned = plan_pair(&mut a, &mut b, base, &PlanContext::new(opts));
    let converged = planned
        .records
        .into_iter()
        .map(|(rel, _)| rel)
        .chain(planned.forget)
        .collect();
    (planned.actions, planned.conflicts, converged)
}

/// Legacy helper for callers that already hold authoritative action results.
/// Runtime runs record `CompletedAction` signatures directly, without a rewalk.
pub fn update_baseline(
    base: &Baseline,
    a: &Tree,
    b: &Tree,
    applied: &[Action],
    converged: &[String],
    conflicts: &[Conflict],
) -> Baseline {
    let conflicts: BTreeSet<&str> = conflicts.iter().map(|c| c.rel.as_str()).collect();
    let mut next = base.clone();
    for rel in applied
        .iter()
        .map(action_rel)
        .chain(converged.iter().map(String::as_str))
    {
        next.insert(rel.to_string(), (a.get(rel).copied(), b.get(rel).copied()));
    }
    next.retain(|rel, (a, b)| a.is_some() || b.is_some() || conflicts.contains(rel.as_str()));
    next
}

pub(super) fn action_rel(action: &Action) -> &str {
    match action {
        Action::CopyAtoB(rel)
        | Action::CopyBtoA(rel)
        | Action::FinalizeMoveAtoB(rel)
        | Action::FinalizeMoveBtoA(rel)
        | Action::DeleteA(rel)
        | Action::DeleteB(rel)
        | Action::KeepBothAtoB(rel)
        | Action::KeepBothBtoA(rel) => rel,
    }
}
