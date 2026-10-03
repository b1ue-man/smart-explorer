//! Recorded line merge and keep-both without replacing changed originals.
use super::apply_guard::drift;
use super::merge_precheck::{checked_original, digest};
use super::merge_recovery::{self, Recovery};
use super::pair_lock::PairLock;
use super::resolve_conflict::ResolvePhase;
use super::run_types::StateKey;
use super::types::{Baseline, Conflict, PairSide, Sig, VersionsLocation};
use super::versions::{RunVersions, VersionSide, VersionsContext};
use crate::vfs::Backend;
use std::io;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy)]
pub struct OriginalContent<'a> {
    pub signature: Option<Sig>,
    pub bytes: Option<&'a [u8]>,
}
#[derive(Clone, Copy)]
pub enum MergeChoice<'a> {
    Write(&'a [u8]),
    KeepBoth { keep_a: bool },
}
#[derive(Clone, Debug)]
pub struct MergeFile {
    pub rel: String,
    pub a: Option<Sig>,
    pub b: Option<Sig>,
}
#[derive(Clone, Debug, Default)]
pub struct MergeReport {
    pub a: Option<Sig>,
    pub b: Option<Sig>,
    pub confirmed_a: bool,
    pub confirmed_b: bool,
    pub baseline: Baseline,
    pub preserved: Vec<MergeFile>,
}
#[derive(Debug)]
pub struct MergeFailure {
    pub error: io::Error,
    pub partial: MergeReport,
}
impl std::fmt::Display for MergeFailure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}", self.error)
    }
}
impl std::error::Error for MergeFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn merge_recorded_for_key(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    key: &StateKey,
    conflict: &Conflict,
    original_a: OriginalContent<'_>,
    original_b: OriginalContent<'_>,
    choice: MergeChoice<'_>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(ResolvePhase),
) -> Result<MergeReport, MergeFailure> {
    let mut report = MergeReport::default();
    let result = (|| {
        // Every observation and revalidation below owns this same lock.
        let lock = PairLock::acquire(&key.lock_id)?;
        let endpoints = super::incremental::SyncEndpoints::new(a, root_a, b, root_b);
        super::single_recorded::validate_state(endpoints, key)?;
        if conflict.duplicates.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "select one duplicate variant before line merging",
            ));
        }
        if original_a.signature != conflict.a || original_b.signature != conflict.b {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "original texts do not belong to this conflict",
            ));
        }
        let keys = super::KeyPolicy::for_pair(
            a.case_sensitive_paths(root_a),
            b.case_sensitive_paths(root_b),
        );
        let (_, records, _) = super::checkpoint_journal::Journal::load(key, keys)?;
        report.baseline = records.baseline;
        let names = super::state_spellings::load(key, keys)?;
        let rel_a = names.rel(&conflict.rel, PairSide::A, keys);
        let rel_b = names.rel(&conflict.rel, PairSide::B, keys);
        let path_a = crate::vfs::sync_path(a, root_a, &rel_a)?;
        let path_b = crate::vfs::sync_path(b, root_b, &rel_b)?;
        let mut opts = match &key.owner {
            super::StateOwner::Job(id) => crate::syncjobs::recorded_options(id)?,
            super::StateOwner::AdHoc => Default::default(),
        };
        opts.reversible = true;
        opts.move_files = false;
        opts.dry_run = false;
        for (backend, root, rel) in [(a, root_a, rel_a.as_str()), (b, root_b, rel_b.as_str())] {
            super::apply_boundary::guard(backend, root, rel, opts.cross_mounts)?;
        }
        let winner = match choice {
            MergeChoice::Write(bytes) => bytes,
            MergeChoice::KeepBoth { keep_a: true } => original_a
                .bytes
                .ok_or_else(|| drift("keep-both source is absent"))?,
            MergeChoice::KeepBoth { keep_a: false } => original_b
                .bytes
                .ok_or_else(|| drift("keep-both source is absent"))?,
        };
        let kind = match choice {
            MergeChoice::Write(_) => "write",
            MergeChoice::KeepBoth { keep_a: true } => "keep-a",
            MergeChoice::KeepBoth { keep_a: false } => "keep-b",
        };
        let digest_a = original_a.bytes.map(digest);
        let digest_b = original_b.bytes.map(digest);
        let saved_recovery = merge_recovery::load(key, &conflict.rel)?;
        let recovering = saved_recovery.is_some();
        let mut recovery = match saved_recovery {
            Some(recovery)
                if recovery.kind == kind
                    && recovery.original_a == digest_a
                    && recovery.original_b == digest_b
                    && recovery.merged == digest(winner) =>
            {
                recovery
            }
            Some(_) => {
                return Err(drift(
                    "an unfinished merge has different input; refresh the comparison",
                ))
            }
            None => {
                let context = VersionsContext::new(
                    &key.pair_id,
                    key.owner.clone(),
                    VersionsLocation::Auto,
                    opts.versioning,
                );
                Recovery {
                    pair: key.pair_id.clone(),
                    lock: key.lock_id.clone(),
                    rel: conflict.rel.clone(),
                    kind: kind.into(),
                    original_a: digest_a,
                    original_b: digest_b,
                    merged: digest(winner),
                    run: context.run_id,
                    started_ms: context.started_ms,
                    a: None,
                    b: None,
                    done_a: false,
                    done_b: false,
                    sibling: None,
                    sibling_a: None,
                    sibling_b: None,
                }
            }
        };
        if recovering {
            super::merge_recovery::recover_original(
                a,
                root_a,
                &rel_a,
                &path_a,
                original_a,
                &recovery,
                PairSide::A,
                key,
                opts.cross_mounts,
                cancel,
            )?;
            super::merge_recovery::recover_original(
                b,
                root_b,
                &rel_b,
                &path_b,
                original_b,
                &recovery,
                PairSide::B,
                key,
                opts.cross_mounts,
                cancel,
            )?;
        }
        progress(ResolvePhase::Preparing);
        let (state_a, sig_a, done_a) = checked_original(
            a,
            &path_a,
            original_a,
            &recovery,
            PairSide::A,
            winner,
            recovering,
            cancel,
        )?;
        let (state_b, sig_b, done_b) = checked_original(
            b,
            &path_b,
            original_b,
            &recovery,
            PairSide::B,
            winner,
            recovering,
            cancel,
        )?;
        report.a = sig_a;
        report.b = sig_b;
        recovery.done_a |= done_a;
        recovery.done_b |= done_b;
        if done_a {
            recovery.a = sig_a;
        }
        if done_b {
            recovery.b = sig_b;
        }
        if let MergeChoice::KeepBoth { keep_a } = choice {
            let (backend, path) = if keep_a {
                (a, path_a.as_str())
            } else {
                (b, path_b.as_str())
            };
            crate::vfs::finish_stage(
                backend,
                path,
                crate::vfs::StageFinish {
                    durability: crate::vfs::StageDurability::Now,
                    ..Default::default()
                },
            )?;
            super::apply_stage::require_durable(super::apply_stage::namespace(backend, path)?)?;
            if keep_a {
                recovery.done_a = true;
                recovery.a = sig_a;
            } else {
                recovery.done_b = true;
                recovery.b = sig_b;
            }
        }
        report.confirmed_a = recovery.done_a;
        report.confirmed_b = recovery.done_b;
        let mut context = VersionsContext::new(
            &key.pair_id,
            key.owner.clone(),
            VersionsLocation::Auto,
            opts.versioning,
        );
        context.run_id = recovery.run.clone();
        context.started_ms = recovery.started_ms;
        let versions = RunVersions::begin(context);
        versions.bind_lock(lock.id())?;
        if !recovering {
            super::merge_inputs::save(key, &recovery, original_a, original_b, winner)?;
        }
        merge_recovery::save(key, &recovery)?;
        let work = super::merge_execution::execute(
            super::merge_execution::MergeWork {
                endpoints,
                key,
                conflict,
                opts,
                original_a,
                original_b,
                choice,
                winner,
                state_a: &state_a,
                state_b: &state_b,
                path_a: &path_a,
                path_b: &path_b,
                rel_a: &rel_a,
                rel_b: &rel_b,
                recovering,
                versions: &versions,
            },
            &mut recovery,
            &mut report,
            cancel,
            &mut progress,
            &lock,
        );
        // Preserve recovery versions after a partial failure. Only a
        // fully recorded merge enters retention under the same pair lock.
        let finalize = versions.finish().and_then(|()| {
            if work.is_err() {
                return Ok(());
            }
            super::versions::prune_after_run(
                &lock,
                &key.pair_id,
                &[
                    VersionSide {
                        side: PairSide::A,
                        backend: a,
                        root: root_a,
                    },
                    VersionSide {
                        side: PairSide::B,
                        backend: b,
                        root: root_b,
                    },
                ],
                &opts.versioning,
                &AtomicBool::new(false),
            )
        });
        work.and(finalize)
    })();
    match result {
        Ok(()) => Ok(report),
        Err(error) => Err(MergeFailure {
            error,
            partial: report,
        }),
    }
}

#[cfg(test)]
#[path = "merge_recorded_task_tests.rs"]
mod task_tests;
