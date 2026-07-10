use crate::vfs::Backend;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use super::apply_delete::delete_guarded;
use super::apply_guard::ExpectedFile;
use super::apply_retry::{run_with_retry, AttemptError};
use super::apply_transfer::{copy_conflict_sibling, copy_replace, verify_copy};
use super::paths::join;
use super::types::{Action, BisyncOptions, BisyncStats, Direction, Throttle, Tree};

pub(super) use super::apply_transfer::back_up;

const MAX_REPORTED_ERRORS: usize = 100;

#[derive(Default, Clone, Debug)]
pub(super) struct ApplyReport {
    pub(super) stats: BisyncStats,
    pub(super) completed: Vec<Action>,
}

#[derive(Default)]
struct ApplyMerge {
    stats: BisyncStats,
    errors: Vec<(String, String)>,
    completed: Vec<Action>,
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
/// Transfers run **concurrently** up to `min(a, b).parallelism()` — the slower
/// side caps it, so SFTP/FTP (which report 1) stay serial while local↔Drive
/// runs many files at once. This is the headline fix for the "27k small files
/// at 0.1 Mbit/s" case: those transfers are latency-bound, not bandwidth-bound.
/// Destination folders are created lazily by the guarded transfer; backends'
/// `mkdir_all` is concurrency-safe (Drive serializes folder creation).
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

    let mut par = a
        .parallelism()
        .min(b.parallelism())
        .max(1)
        .min(actions.len().max(1));
    if opts.max_transfers > 0 {
        par = par.min(opts.max_transfers);
    }

    let throttle = Throttle::new(opts.bwlimit_bps);
    let merged = Mutex::new(ApplyMerge::default());
    let idx = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..par {
            scope.spawn(|| {
                let mut local = BisyncStats::default();
                let mut local_errs: Vec<(String, String)> = Vec::new();
                let mut local_done: Vec<Action> = Vec::new();
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i >= actions.len() {
                        break;
                    }
                    let act = &actions[i];
                    let res = run_with_retry(
                        opts.retries,
                        Duration::from_secs(opts.retry_delay_secs),
                        cancel,
                        || {
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
                        },
                    );
                    match res {
                        Ok(s) => {
                            local.a_to_b += s.a_to_b;
                            local.b_to_a += s.b_to_a;
                            local.deleted += s.deleted;
                            local.bytes += s.bytes;
                            local_done.push(act.clone());
                        }
                        Err(error) => {
                            local.errors += 1;
                            if local_errs.len() < MAX_REPORTED_ERRORS {
                                local_errs
                                    .push((format!("{:?}", act), error.into_io().to_string()));
                            }
                        }
                    }
                }
                let mut m = merged
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                m.stats.a_to_b += local.a_to_b;
                m.stats.b_to_a += local.b_to_a;
                m.stats.deleted += local.deleted;
                m.stats.bytes += local.bytes;
                m.stats.errors += local.errors;
                let remaining = MAX_REPORTED_ERRORS.saturating_sub(m.errors.len());
                m.errors.extend(local_errs.into_iter().take(remaining));
                m.completed.extend(local_done);
            });
        }
    });

    let merged = merged
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
