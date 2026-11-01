//! Planning against Backend facts, with cheap on-demand content comparison,
//! followed by the rule stage of the job kind (`plan_rules`).
use std::io;
use std::sync::atomic::AtomicBool;

use super::compare::TimeRules;
use super::guards::{deletion_block, empty_side_block, unconfirmed, DeleteCounts};
use super::incremental::SyncEndpoints;
use super::keys::{KeyPolicy, PathAliases};
use super::omissions::{OmissionKind, SyncOmissions};
use super::plan_pair::plan_pair_spelled;
use super::plan_types::{PairPlan, PlanContext};
use super::run_types::{RunBlock, RunSettings};
use super::snapshot_types::{DirSet, SideSnapshot};
use super::types::{Baseline, BisyncOptions, PairSide, Sig};
use crate::vfs::Backend;

pub(super) fn context<'a>(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    base_dirs: Option<&'a DirSet>,
) -> PlanContext<'a> {
    let keys = super::pair_key_policy(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    PlanContext {
        opts,
        keys,
        base_dirs,
        times: TimeRules {
            a: crate::vfs::mtime_precision(endpoints.a, endpoints.root_a),
            b: crate::vfs::mtime_precision(endpoints.b, endpoints.root_b),
        },
        limits_a: crate::vfs::target_limits(endpoints.a, endpoints.root_a),
        limits_b: crate::vfs::target_limits(endpoints.b, endpoints.root_b),
        can_hash: [endpoints.a, endpoints.b]
            .into_iter()
            .all(|backend| backend.is_local() || backend.provides_content_hash()),
    }
}

pub(super) fn prepare(
    endpoints: SyncEndpoints<'_>,
    a: &mut SideSnapshot,
    b: &mut SideSnapshot,
    base: &Baseline,
    ctx: &PlanContext<'_>,
    cancel: &AtomicBool,
) -> PairPlan {
    prepare_spelled(endpoints, a, b, base, ctx, &PathAliases::default(), cancel)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_spelled(
    endpoints: SyncEndpoints<'_>,
    a: &mut SideSnapshot,
    b: &mut SideSnapshot,
    base: &Baseline,
    ctx: &PlanContext<'_>,
    aliases: &PathAliases,
    cancel: &AtomicBool,
) -> PairPlan {
    let mut plan = plan_pair_spelled(a, b, base, ctx, aliases);
    if plan.verify.is_empty() {
        rule_stage(&mut plan);
        a.omissions.extend(plan.omissions.clone());
        b.omissions.extend(plan.omissions.clone());
        return plan;
    }
    verify_contents(endpoints, a, b, &mut plan, cancel);
    let counts = (plan.files_a, plan.files_b);
    a.omissions.extend(plan.omissions);
    let mut final_ctx = *ctx;
    final_ctx.can_hash = false;
    let mut final_plan = plan_pair_spelled(a, b, base, &final_ctx, aliases);
    final_plan.files_a = counts.0;
    final_plan.files_b = counts.1;
    rule_stage(&mut final_plan);
    // The planner takes the omissions. Later guards and checkpoint observations
    // still need them to distinguish a protected entry from an empty volume.
    a.omissions.extend(final_plan.omissions.clone());
    b.omissions.extend(final_plan.omissions.clone());
    final_plan
}

/// The MD5 of both sides for every key of `plan.verify` (`verify_pool.rs`):
/// both sides at the same time, each with its own adaptive workers. A file
/// that cannot be checked becomes an omission of its key, as before.
fn verify_contents(
    endpoints: SyncEndpoints<'_>,
    a: &mut SideSnapshot,
    b: &mut SideSnapshot,
    plan: &mut PairPlan,
    cancel: &AtomicBool,
) {
    use super::verify_pool::{run_side, Job};
    let mut failed: Vec<(usize, io::Error)> = Vec::new();
    let mut sides = Vec::new();
    for (side, snapshot, backend, root) in [
        (PairSide::A, &*a, endpoints.a, endpoints.root_a),
        (PairSide::B, &*b, endpoints.b, endpoints.root_b),
    ] {
        let mut jobs = Vec::new();
        for (key, rel) in plan.verify.iter().enumerate() {
            let spelled = plan.spellings.side_rel(rel, side);
            let Some(signature) = snapshot.tree.get(spelled) else {
                continue;
            };
            if signature.hash != 0 {
                continue;
            }
            match crate::vfs::sync_path(backend, root, spelled) {
                Ok(path) => jobs.push(Job {
                    key,
                    path,
                    planned: *signature,
                }),
                Err(error) => failed.push((key, error)),
            }
        }
        sides.push((side, backend, jobs));
    }
    let checked: Vec<(PairSide, Vec<(usize, io::Result<u64>)>)> = std::thread::scope(|scope| {
        let running: Vec<_> = sides
            .into_iter()
            .map(|(side, backend, jobs)| {
                scope.spawn(move || {
                    let check =
                        |path: &str, planned: Sig| hash_checked(backend, path, planned, cancel);
                    (side, run_side(backend, jobs, &check, cancel))
                })
            })
            .collect();
        running
            .into_iter()
            .filter_map(|side| side.join().ok())
            .collect()
    });
    for (side, results) in checked {
        let snapshot = match side {
            PairSide::A => &mut *a,
            PairSide::B => &mut *b,
        };
        for (key, result) in results {
            match result {
                Ok(hash) => {
                    let spelled = plan.spellings.side_rel(&plan.verify[key], side);
                    if let Some(signature) = snapshot.tree.get_mut(spelled) {
                        signature.hash = hash;
                    }
                }
                Err(error) => failed.push((key, error)),
            }
        }
    }
    for (key, error) in failed {
        let kind = crate::vfs::omission_reason(&error)
            .map(OmissionKind::from)
            .unwrap_or(OmissionKind::Unreadable);
        plan.omissions.record_kind(&plan.verify[key], kind, true);
    }
}

/// The rule stage of the job kind installed on this thread (C5.4), after
/// the planner and before guards, previews and apply see the plan. Without
/// installed rules the plan stays exactly as planned.
fn rule_stage(plan: &mut PairPlan) {
    if let Some(rules) = super::plan_rules::current() {
        rules.adjust(plan);
    }
}

/// Stat guards bind the checksum to exactly the observation we planned.
/// Native-hash remotes never download their file merely for this fast path.
fn hash_checked(
    backend: &dyn Backend,
    path: &str,
    planned: Sig,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    let before = backend.stat(path)?;
    if before.is_dir
        || before.is_symlink
        || before.special
        || before.size != planned.size
        || before.mtime_ms != planned.mtime_ms
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Datei seit der Planung geändert",
        ));
    }
    if let Some(hash) = before
        .content_md5
        .as_deref()
        .map(super::snapshot::md5_hex_to_u64)
        .filter(|hash| *hash != 0)
    {
        return Ok(hash);
    }
    if !backend.is_local() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Server-Prüfsumme nicht verfügbar",
        ));
    }
    // The comparison asks for the content as it is now: a fresh read.
    let reads = super::snapshot_hash::LocalReads {
        fresh: true,
        run: None,
    };
    let hash = super::snapshot_hash::hash_file_cached(backend, path, cancel, reads)?;
    let after = backend.stat(path)?;
    if after.is_dir
        || after.is_symlink
        || after.special
        || after.size != planned.size
        || after.size != before.size
        || after.mtime_ms != before.mtime_ms
        || after.id != before.id
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Datei beim Prüfsummenvergleich geändert",
        ));
    }
    Ok(hash)
}

/// Every independent guard is checked, even if an earlier guard is confirmed.
pub(super) fn blocked(
    plan: &PairPlan,
    a: &SideSnapshot,
    b: &SideSnapshot,
    base: &Baseline,
    opts: &BisyncOptions,
    settings: &RunSettings,
    deletes: DeleteCounts,
) -> Option<RunBlock> {
    for (side, snapshot) in [(PairSide::A, a), (PairSide::B, b)] {
        if let Some(block) = unconfirmed(
            empty_side_block(side, snapshot.is_empty(), base),
            &settings.confirmed,
        ) {
            return Some(block);
        }
    }
    if let Some(block) = unconfirmed(
        deletion_block(deletes, plan.files_a, plan.files_b, opts),
        &settings.confirmed,
    ) {
        return Some(block);
    }
    let mut percentage = *opts;
    percentage.max_delete = 0;
    for (side, count, files) in [
        (PairSide::A, deletes.a, plan.files_a),
        (PairSide::B, deletes.b, plan.files_b),
    ] {
        let mut side_counts = DeleteCounts::default();
        side_counts.add(side, count);
        let block = deletion_block(side_counts, files, files, &percentage);
        if let Some(block) = unconfirmed(block, &settings.confirmed) {
            return Some(block);
        }
    }
    None
}

pub(super) fn absorb_omissions(
    a: &mut SideSnapshot,
    b: &mut SideSnapshot,
    omissions: SyncOmissions,
) {
    omissions.exclude_tree(&mut a.tree);
    omissions.exclude_tree(&mut b.tree);
    a.omissions.extend(omissions);
}

/// Recovery inputs own both originals and any reserved KeepBoth sibling.
/// The caller keeps this lock through observation, planning and publication.
pub(super) fn protect_pending(
    lock: &super::PairLock,
    key: &super::StateKey,
    endpoints: SyncEndpoints<'_>,
    keys: KeyPolicy,
    snapshot: &mut super::snapshot_pair::PairSnapshot,
) -> io::Result<()> {
    protect_pending_spelled(
        lock,
        key,
        endpoints,
        keys,
        &PathAliases::default(),
        snapshot,
    )
}

pub(super) fn protect_pending_spelled(
    lock: &super::PairLock,
    key: &super::StateKey,
    endpoints: SyncEndpoints<'_>,
    keys: KeyPolicy,
    aliases: &PathAliases,
    snapshot: &mut super::snapshot_pair::PairSnapshot,
) -> io::Result<()> {
    let mut omissions = SyncOmissions::new(keys.fold_case);
    for rel in pending_paths(lock, key, endpoints)? {
        omissions.record_kind(&rel, OmissionKind::Unreadable, true);
    }
    aliases.protect_counterparts(&mut omissions, keys);
    snapshot
        .repairs
        .retain(|repair| !omissions.protects(&repair.rel));
    snapshot
        .conflicts
        .retain(|conflict| !omissions.protects(&conflict.rel));
    for side in [&mut snapshot.a, &mut snapshot.b] {
        omissions.exclude_tree(&mut side.tree);
        side.filtered.retain(|rel, _| !omissions.protects(rel));
        side.dirs.retain(|rel| !omissions.protects(rel));
        side.omissions.extend(omissions.clone());
    }
    snapshot.omissions.extend(omissions);
    Ok(())
}

pub(super) fn pending_paths(
    lock: &super::PairLock,
    key: &super::StateKey,
    endpoints: SyncEndpoints<'_>,
) -> io::Result<Vec<String>> {
    let mut paths = std::collections::BTreeSet::new();
    let reversed_endpoints =
        SyncEndpoints::new(endpoints.b, endpoints.root_b, endpoints.a, endpoints.root_a);
    let reversed_key = super::StateKey {
        pair_id: super::pair_id_for(endpoints.b, endpoints.root_b, endpoints.a, endpoints.root_a),
        replica_a: key.replica_b.clone(),
        replica_b: key.replica_a.clone(),
        ..key.clone()
    };
    for (key, endpoints) in [(key, endpoints), (&reversed_key, reversed_endpoints)] {
        for owner_key in super::replica_state::pending_owner_keys(key)? {
            paths.extend(super::pending_merge_relatives(lock, &owner_key)?);
        }
        paths.extend(super::replacement_recovery::relatives(
            lock, key, endpoints,
        )?);
    }
    Ok(paths.into_iter().collect())
}

pub(super) fn planned_signatures(plan: &PairPlan, a: &SideSnapshot, b: &SideSnapshot) -> Baseline {
    plan.actions
        .iter()
        .map(super::core::action_rel)
        .map(|rel| {
            (
                rel.to_string(),
                (
                    a.tree
                        .get(plan.spellings.side_rel(rel, PairSide::A))
                        .copied(),
                    b.tree
                        .get(plan.spellings.side_rel(rel, PairSide::B))
                        .copied(),
                ),
            )
        })
        .collect()
}

pub(super) fn keys(endpoints: SyncEndpoints<'_>) -> KeyPolicy {
    super::pair_key_policy(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b)
}
