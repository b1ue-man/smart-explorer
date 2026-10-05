//! Authorized process-restart inputs and pending-path guard for the engine.
use super::merge_inputs::PendingMerge;
use super::pair_lock::PairLock;
use super::run_types::StateKey;
use crate::vfs::Backend;
use std::io;

#[allow(clippy::too_many_arguments)]
pub fn pending_merge_for_key(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    key: &StateKey,
    rel: &str,
) -> io::Result<Option<PendingMerge>> {
    let _lock = PairLock::acquire(&key.lock_id)?;
    super::single_recorded::validate_state(
        super::incremental::SyncEndpoints::new(a, root_a, b, root_b),
        key,
    )?;
    super::sync_relative_path::SyncRelativePath::parse(rel)?;
    let keys = super::KeyPolicy::for_pair(
        a.case_sensitive_paths(root_a),
        b.case_sensitive_paths(root_b),
    );
    let names = super::state_spellings::load(key, keys)?;
    let opts = match &key.owner {
        super::StateOwner::Job(id) => crate::syncjobs::recorded_options(id)?,
        super::StateOwner::AdHoc => super::BisyncOptions::default(),
    };
    super::apply_boundary::guard(
        a,
        root_a,
        &names.rel(rel, super::PairSide::A, keys),
        opts.cross_mounts,
    )?;
    super::apply_boundary::guard(
        b,
        root_b,
        &names.rel(rel, super::PairSide::B, keys),
        opts.cross_mounts,
    )?;
    super::merge_recovery::load(key, rel)?
        .map(|recovery| super::merge_inputs::load(key, &recovery))
        .transpose()
}

/// The normal engine already holds this lock and has validated StateKey.
/// Protect every returned original/sibling on both sides before planning,
/// complete-index creation and any baseline-convergence checkpoint.
pub fn pending_merge_relatives(lock: &PairLock, key: &StateKey) -> io::Result<Vec<String>> {
    if lock.id() != key.lock_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pending merge has another pair lock",
        ));
    }
    super::merge_recovery::relatives(key)
}
