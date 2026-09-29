use crate::transfer::engine::folders::FolderRegister;
use crate::transfer::Side;
use crate::vfs::Backend;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::apply_delete::delete_guarded;
use super::apply_guard::ExpectedFile;
use super::apply_pool::run_actions;
use super::apply_retry::{run_with_retry, AttemptError};
use super::apply_transfer::{copy_conflict_sibling, copy_replace, verify_copy};
use super::paths::join;
use super::sync_flows::{PairFlows, PairSide};
use super::types::{Action, BisyncOptions, BisyncStats, Direction, Throttle, Tree};

pub(super) use super::apply_transfer::back_up;

#[derive(Default, Clone, Debug)]
pub(super) struct ApplyReport {
    pub(super) stats: BisyncStats,
    pub(super) completed: Vec<Action>,
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    act: &Action,
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    throttle: &Throttle,
    cancel: &AtomicBool,
    planned: Option<(&Tree, &Tree)>,
) -> Result<BisyncStats, AttemptError> {
    let mut st = BisyncStats::default();
    let planned_a = planned.map(|(tree, _)| tree);
    let planned_b = planned.map(|(_, tree)| tree);
    match act {
        Action::CopyAtoB(rel) => {
            let sp = join(root_a, rel);
            let dp = join(root_b, rel);
            let n = copy_replace(
                a,
                &sp,
                ExpectedFile::from_tree(planned_a, rel),
                b,
                &dp,
                ExpectedFile::from_tree(planned_b, rel),
                opts.reversible.then_some((rel.as_str(), versions_dir)),
                throttle,
                cancel,
            )?;
            if opts.verify {
                verify_copy(b, &dp, n).map_err(AttemptError::commit_attempted)?;
            }
            st.bytes += n;
            st.a_to_b += 1;
            if opts.move_files && opts.direction != Direction::Both {
                super::move_finalize::verify_and_delete_source(
                    a,
                    &sp,
                    b,
                    &dp,
                    rel,
                    opts.reversible,
                    versions_dir,
                    cancel,
                )
                .map_err(AttemptError::commit_attempted)?;
                st.deleted += 1;
            }
        }
        Action::CopyBtoA(rel) => {
            let sp = join(root_b, rel);
            let dp = join(root_a, rel);
            let n = copy_replace(
                b,
                &sp,
                ExpectedFile::from_tree(planned_b, rel),
                a,
                &dp,
                ExpectedFile::from_tree(planned_a, rel),
                opts.reversible.then_some((rel.as_str(), versions_dir)),
                throttle,
                cancel,
            )?;
            if opts.verify {
                verify_copy(a, &dp, n).map_err(AttemptError::commit_attempted)?;
            }
            st.bytes += n;
            st.b_to_a += 1;
            if opts.move_files && opts.direction != Direction::Both {
                super::move_finalize::verify_and_delete_source(
                    b,
                    &sp,
                    a,
                    &dp,
                    rel,
                    opts.reversible,
                    versions_dir,
                    cancel,
                )
                .map_err(AttemptError::commit_attempted)?;
                st.deleted += 1;
            }
        }
        Action::FinalizeMoveAtoB(rel) => {
            super::move_finalize::verify_and_delete_source(
                a,
                &join(root_a, rel),
                b,
                &join(root_b, rel),
                rel,
                opts.reversible,
                versions_dir,
                cancel,
            )
            .map_err(AttemptError::commit_attempted)?;
            st.deleted += 1;
        }
        Action::FinalizeMoveBtoA(rel) => {
            super::move_finalize::verify_and_delete_source(
                b,
                &join(root_b, rel),
                a,
                &join(root_a, rel),
                rel,
                opts.reversible,
                versions_dir,
                cancel,
            )
            .map_err(AttemptError::commit_attempted)?;
            st.deleted += 1;
        }
        Action::DeleteB(rel) => {
            delete_guarded(
                b,
                &join(root_b, rel),
                rel,
                ExpectedFile::from_tree(planned_b, rel),
                opts.reversible,
                versions_dir,
                opts.use_recycle,
                cancel,
            )?;
            st.deleted += 1;
        }
        Action::DeleteA(rel) => {
            delete_guarded(
                a,
                &join(root_a, rel),
                rel,
                ExpectedFile::from_tree(planned_a, rel),
                opts.reversible,
                versions_dir,
                opts.use_recycle,
                cancel,
            )?;
            st.deleted += 1;
        }
        Action::KeepBothAtoB(rel) => {
            let bp = join(root_b, rel);
            let (preserved, expected_b) =
                preservation_state(b, &bp, ExpectedFile::from_tree(planned_b, rel))?;
            if preserved {
                copy_conflict_sibling(b, &bp, root_b, rel, expected_b, throttle, cancel)?;
            }
            let result = copy_replace(
                a,
                &join(root_a, rel),
                ExpectedFile::from_tree(planned_a, rel),
                b,
                &bp,
                expected_b,
                None,
                throttle,
                cancel,
            );
            let copied = if preserved {
                result.map_err(|error| AttemptError::commit_attempted(error.into_io()))?
            } else {
                result?
            };
            if opts.verify {
                verify_copy(b, &bp, copied).map_err(AttemptError::commit_attempted)?;
            }
            st.bytes += copied;
            st.a_to_b += 1;
        }
        Action::KeepBothBtoA(rel) => {
            let ap = join(root_a, rel);
            let (preserved, expected_a) =
                preservation_state(a, &ap, ExpectedFile::from_tree(planned_a, rel))?;
            if preserved {
                copy_conflict_sibling(a, &ap, root_a, rel, expected_a, throttle, cancel)?;
            }
            let result = copy_replace(
                b,
                &join(root_b, rel),
                ExpectedFile::from_tree(planned_b, rel),
                a,
                &ap,
                expected_a,
                None,
                throttle,
                cancel,
            );
            let copied = if preserved {
                result.map_err(|error| AttemptError::commit_attempted(error.into_io()))?
            } else {
                result?
            };
            if opts.verify {
                verify_copy(a, &ap, copied).map_err(AttemptError::commit_attempted)?;
            }
            st.bytes += copied;
            st.b_to_a += 1;
        }
    }
    Ok(st)
}

fn preservation_state(
    backend: &dyn Backend,
    path: &str,
    expected: ExpectedFile,
) -> Result<(bool, ExpectedFile), AttemptError> {
    let expected = expected
        .concretize(backend, path, "conflict destination")
        .map_err(AttemptError::pre_commit)?;
    Ok((matches!(expected, ExpectedFile::Present(_)), expected))
}

/// Apply the planned actions, with reversible backups. Returns stats; errors are
/// counted (and the rel/message collected) rather than aborting.
///
/// Transfers run **concurrently**, as many as the flows of both connections
/// allow (`apply_pool`, `sync_flows`): latency-bound small files (the "27k
/// small files at 0.1 Mbit/s" case) run many at once on every protocol, and a
/// set `max_transfers` stays the upper bound (one remote connection on both
/// sides keeps its protocol bound, see `sync_flows`). A folder that did not
/// exist when the plan was made is created once through the side's folder
/// register before the copies into it start; the guarded transfer still makes
/// sure it exists.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    actions: &[Action],
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> BisyncStats {
    apply_with_results(
        actions,
        a,
        root_a,
        b,
        root_b,
        opts,
        versions_dir,
        errors,
        cancel,
    )
    .stats
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_with_results(
    actions: &[Action],
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    apply_inner(
        actions,
        a,
        root_a,
        b,
        root_b,
        opts,
        versions_dir,
        errors,
        cancel,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_planned_with_results(
    actions: &[Action],
    planned_a: &Tree,
    planned_b: &Tree,
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    apply_inner(
        actions,
        a,
        root_a,
        b,
        root_b,
        opts,
        versions_dir,
        errors,
        cancel,
        Some((planned_a, planned_b)),
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_inner(
    actions: &[Action],
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
    planned: Option<(&Tree, &Tree)>,
) -> ApplyReport {
    if opts.dry_run {
        let mut st = BisyncStats::default();
        for act in actions {
            match act {
                Action::CopyAtoB(_) | Action::KeepBothAtoB(_) => st.a_to_b += 1,
                Action::CopyBtoA(_) | Action::KeepBothBtoA(_) => st.b_to_a += 1,
                Action::DeleteA(_)
                | Action::DeleteB(_)
                | Action::FinalizeMoveAtoB(_)
                | Action::FinalizeMoveBtoA(_) => st.deleted += 1,
            }
        }
        return ApplyReport {
            stats: st,
            completed: Vec::new(),
        };
    }

    let flows = PairFlows::new(a, root_a, b, root_b);
    let folders_a = FolderRegister::new(Side::Remote(a), root_a, flows.flow(PairSide::A).clone());
    let folders_b = FolderRegister::new(Side::Remote(b), root_b, flows.flow(PairSide::B).clone());
    let throttle = Throttle::new(opts.bwlimit_bps);
    let retry_delay = Duration::from_secs(opts.retry_delay_secs);
    let admit = |act: &Action| {
        prepare_folder(act, planned, &folders_a, &folders_b, cancel);
        match act {
            Action::DeleteA(_) => flows.single(PairSide::A, cancel),
            Action::DeleteB(_) => flows.single(PairSide::B, cancel),
            _ => flows.transfer(cancel),
        }
    };
    let execute = |act: &Action| {
        run_with_retry(opts.retries, retry_delay, cancel, || {
            run_one(
                act,
                a,
                root_a,
                b,
                root_b,
                opts,
                versions_dir,
                &throttle,
                cancel,
                planned,
            )
        })
    };
    // The user's "max. transfers" stays the upper bound; one remote
    // connection on both sides keeps its protocol bound (`sync_flows`).
    let cap = match (opts.max_transfers, flows.shared_connection_cap(a, b)) {
        (0, shared) => shared.unwrap_or(0),
        (max, shared) => shared.map_or(max, |shared| shared.min(max)),
    };
    let merged = run_actions(actions, cap, cancel, &admit, &execute);
    let reported = merged.errors.len() as u64;
    errors.extend(merged.errors);
    if merged.stats.errors > reported {
        errors.push((
            String::new(),
            format!(
                "{} weitere Synchronisierungsfehler unterdrückt",
                merged.stats.errors - reported
            ),
        ));
    }
    ApplyReport {
        stats: merged.stats,
        completed: merged.completed,
    }
}

/// A copy into a folder that did not exist when the plan was made creates
/// that folder once through the destination side's register (parents first,
/// under a metadata permit), so many concurrent copies into a new folder never
/// race to create it on protocols that were serial before. A failure is left
/// to the guarded copy, which creates the folder itself and reports it.
fn prepare_folder(
    act: &Action,
    planned: Option<(&Tree, &Tree)>,
    folders_a: &FolderRegister<'_>,
    folders_b: &FolderRegister<'_>,
    cancel: &AtomicBool,
) {
    let (rel, into_b) = match act {
        Action::CopyAtoB(rel) | Action::KeepBothAtoB(rel) => (rel, true),
        Action::CopyBtoA(rel) | Action::KeepBothBtoA(rel) => (rel, false),
        _ => return,
    };
    let Some((folder, _)) = rel.rsplit_once('/') else {
        return;
    };
    let known = planned.map(|(tree_a, tree_b)| if into_b { tree_b } else { tree_a });
    if known.is_some_and(|tree| holds_folder(tree, folder)) {
        return;
    }
    let register = if into_b { folders_b } else { folders_a };
    let _ = register.ensure(folder, cancel);
}

/// Some planned file lies below `folder`: it existed at planning time.
fn holds_folder(tree: &Tree, folder: &str) -> bool {
    let prefix = format!("{folder}/");
    tree.range(prefix.clone()..)
        .next()
        .is_some_and(|(rel, _)| rel.starts_with(&prefix))
}
