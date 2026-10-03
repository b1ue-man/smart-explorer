//! Execution decisions for one action; checkpoints receive every durable part.
use super::apply_guard::ExpectedFile;
use super::apply_retry::AttemptError;
use super::checkpoint::ApplyScope;
use super::completion::{CompletedAction, CompletedKind};
use super::incremental::SyncEndpoints;
use super::types::{Action, BisyncOptions, BisyncStats, Direction, PairSide, Sig, Throttle, Tree};
use super::versions::VersionSide;
use crate::vfs::StageDurability;
use std::sync::atomic::AtomicBool;

pub(super) struct Actions<'a> {
    pub endpoints: SyncEndpoints<'a>,
    pub opts: BisyncOptions,
    pub scope: &'a ApplyScope<'a>,
    pub planned: Option<(&'a Tree, &'a Tree)>,
    pub throttle: &'a Throttle,
    pub cancel: &'a AtomicBool,
    pub deferred: [bool; 2],
}
impl Actions<'_> {
    fn side(&self, side: PairSide) -> VersionSide<'_> {
        let (backend, root) = match side {
            PairSide::A => (self.endpoints.a, self.endpoints.root_a),
            PairSide::B => (self.endpoints.b, self.endpoints.root_b),
        };
        VersionSide {
            side,
            backend,
            root,
        }
    }
    fn expected(&self, side: PairSide, rel: &str) -> ExpectedFile {
        ExpectedFile::from_tree(
            self.planned
                .map(|(a, b)| if side == PairSide::A { a } else { b }),
            self.scope.spellings.side_rel(rel, side),
        )
    }
    fn hashed(&self, side: PairSide, mut sig: Sig) -> Sig {
        let this = self.side(side);
        let other = self.side(side.other());
        if matches!(
            super::snapshot_hash::hash_mode(this.backend, other.backend, self.opts.compare),
            super::snapshot_hash::HashMode::None
        ) {
            sig.hash = 0;
        }
        sig
    }
    fn report(
        &self,
        rel: &str,
        kind: CompletedKind,
        src: Option<Sig>,
        dst: Option<Sig>,
        durable: bool,
    ) {
        self.scope.sink.completed(CompletedAction {
            rel: rel.to_string(),
            kind,
            src_sig: src,
            dst_sig: dst,
            durable,
        });
    }

    pub(super) fn run(&self, action: &Action, stats: &mut BisyncStats) -> Result<(), AttemptError> {
        super::apply_transaction::gate(self.cancel, Some(self.scope.sink))
            .map_err(AttemptError::pre_commit)?;
        let rel = super::core::action_rel(action);
        let (from, deletion, finalizing, both) = match action {
            Action::CopyAtoB(_) => (PairSide::A, false, false, false),
            Action::CopyBtoA(_) => (PairSide::B, false, false, false),
            Action::KeepBothAtoB(_) => (PairSide::A, false, false, true),
            Action::KeepBothBtoA(_) => (PairSide::B, false, false, true),
            Action::FinalizeMoveAtoB(_) => (PairSide::A, false, true, false),
            Action::FinalizeMoveBtoA(_) => (PairSide::B, false, true, false),
            Action::DeleteA(_) => (PairSide::A, true, false, false),
            Action::DeleteB(_) => (PairSide::B, true, false, false),
        };
        let source = self.side(from);
        let target = self.side(from.other());
        let source_rel = self.scope.spellings.side_rel(rel, from);
        let target_rel = self.scope.spellings.side_rel(rel, from.other());
        let sp = crate::vfs::sync_path(source.backend, source.root, source_rel)
            .map_err(AttemptError::pre_commit)?;
        let dp = crate::vfs::sync_path(target.backend, target.root, target_rel)
            .map_err(AttemptError::pre_commit)?;
        super::apply_boundary::guard(
            source.backend,
            source.root,
            source_rel,
            self.opts.cross_mounts,
        )
        .map_err(AttemptError::pre_commit)?;
        // Deletion rechecks the absent peer too; a concurrent recreation
        // never authorizes deleting a now-independent destination.
        super::apply_boundary::guard(
            target.backend,
            target.root,
            target_rel,
            self.opts.cross_mounts,
        )
        .map_err(AttemptError::pre_commit)?;
        let source_expected = self.expected(from, rel);
        let target_expected = self.expected(from.other(), rel);
        if deletion {
            let peer =
                super::apply_guard::capture(target.backend, &dp, target_expected, "deleted peer")
                    .map_err(AttemptError::pre_commit)?;
            let durable = super::apply_transaction::delete(
                &source,
                &sp,
                source_expected,
                source_rel,
                self.opts,
                Some(self.scope.versions),
                self.cancel,
                Some(self.scope.sink),
                || {
                    super::apply_boundary::guard(
                        target.backend,
                        target.root,
                        target_rel,
                        self.opts.cross_mounts,
                    )?;
                    let keys = super::KeyPolicy::for_pair(
                        source.backend.case_sensitive_paths(source.root),
                        target.backend.case_sensitive_paths(target.root),
                    );
                    if peer.metadata.is_none()
                        && !super::apply_boundary::normalized_missing(
                            target.backend,
                            target.root,
                            target_rel,
                            keys,
                            self.opts.cross_mounts,
                            self.cancel,
                        )?
                    {
                        return Err(super::apply_guard::drift(
                            "deleted peer appeared under another spelling",
                        ));
                    }
                    super::apply_guard::revalidate(target.backend, &dp, &peer, "deleted peer")
                },
            )?;
            super::apply_stage::require_durable(durable).map_err(AttemptError::commit_attempted)?;
            self.report(rel, CompletedKind::Deleted { side: from }, None, None, true);
            stats.deleted += 1;
            return Ok(());
        }
        if finalizing {
            let (_, _, source_sig, destination_sig) = super::move_finalize::checked_pair(
                source.backend,
                &sp,
                source_expected,
                target.backend,
                &dp,
                target_expected,
                self.cancel,
            )
            .map_err(AttemptError::pre_commit)?;
            let durable = super::move_finalize::finish_scoped(
                &source,
                &sp,
                source_sig,
                target.backend,
                &dp,
                destination_sig,
                source_rel,
                self.opts,
                self.scope,
                self.cancel,
            )?;
            super::apply_stage::require_durable(durable).map_err(AttemptError::commit_attempted)?;
            self.report(
                rel,
                CompletedKind::Moved { from },
                None,
                Some(self.hashed(from.other(), destination_sig)),
                true,
            );
            stats.deleted += 1;
            return Ok(());
        }
        let source_state =
            super::apply_guard::capture(source.backend, &sp, source_expected, "copy source")
                .map_err(AttemptError::pre_commit)?;
        super::apply_boundary::target(
            target.backend,
            target.root,
            target_rel,
            source_state.metadata.as_ref().map(|meta| meta.size),
        )
        .map_err(AttemptError::pre_commit)?;
        let mut opts = self.opts;
        let mut sibling = false;
        if both {
            let expected = target_expected
                .concretize(target.backend, &dp, "conflict destination")
                .map_err(AttemptError::pre_commit)?;
            if matches!(expected, ExpectedFile::Present(_)) {
                super::apply_transaction::gate(self.cancel, Some(self.scope.sink))
                    .map_err(AttemptError::pre_commit)?;
                super::apply_transfer::copy_conflict_sibling(
                    target.backend,
                    &dp,
                    target.root,
                    target_rel,
                    expected,
                    self.throttle,
                    self.cancel,
                )?;
                sibling = true;
            }
            // The confirmed conflict sibling is the reversible backup.
            opts.reversible = false;
        }
        let deferred = self.deferred[if from.other() == PairSide::A { 0 } else { 1 }];
        let durability = if deferred && matches!(target_expected, ExpectedFile::Missing) {
            StageDurability::Deferred
        } else {
            StageDurability::Now
        };
        let outcome = super::apply_transaction::copy(
            source.backend,
            &sp,
            source_expected,
            &target,
            &dp,
            target_expected,
            target_rel,
            opts,
            Some(self.scope.versions),
            durability,
            self.throttle,
            self.cancel,
            Some(self.scope.sink),
            |_| {},
            |_| {},
        )
        .map_err(|error| {
            if sibling {
                AttemptError::commit_attempted(error.into_io())
            } else {
                error
            }
        })?;
        let source_sig = self.hashed(from, outcome.source);
        let destination_sig = self.hashed(from.other(), outcome.destination);
        if !outcome.durable && durability != StageDurability::Deferred {
            return Err(AttemptError::commit_attempted(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "namespace was not confirmed; copied file is not recorded",
            )));
        }
        // A failed/canceled move deletion must not lose its completed copy.
        self.report(
            rel,
            CompletedKind::Copied { from },
            Some(source_sig),
            Some(destination_sig),
            outcome.durable,
        );
        if from == PairSide::A {
            stats.a_to_b += 1;
        } else {
            stats.b_to_a += 1;
        }
        stats.bytes = stats.bytes.saturating_add(outcome.bytes);
        if self.opts.move_files && self.opts.direction != Direction::Both {
            // A move never deletes its source before the copy is durable.
            if !outcome.durable {
                return Err(AttemptError::commit_attempted(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "source retained until the deferred destination is durably checkpointed",
                )));
            }
            let durable = super::move_finalize::finish_scoped(
                &source,
                &sp,
                outcome.source,
                target.backend,
                &dp,
                outcome.destination,
                source_rel,
                self.opts,
                self.scope,
                self.cancel,
            )?;
            super::apply_stage::require_durable(durable).map_err(AttemptError::commit_attempted)?;
            self.report(
                rel,
                CompletedKind::Moved { from },
                None,
                Some(destination_sig),
                true,
            );
            stats.deleted += 1;
        }
        Ok(())
    }
}
