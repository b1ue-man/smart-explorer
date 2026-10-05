//! Authorized literal paths for loading a recorded conflict's original texts.
use super::run_types::StateKey;
use crate::vfs::Backend;
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedPaths {
    pub rel_a: String,
    pub rel_b: String,
    pub path_a: String,
    pub path_b: String,
}

/// The UI may read these locators with their original Backend endpoints.
/// Apply validates the bytes again under its own held pair lock.
#[allow(clippy::too_many_arguments)]
pub fn recorded_original_paths_for_key(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    key: &StateKey,
    rel: &str,
) -> io::Result<RecordedPaths> {
    let _lock = super::pair_lock::PairLock::acquire(&key.lock_id)?;
    let endpoints = super::incremental::SyncEndpoints::new(a, root_a, b, root_b);
    super::single_recorded::validate_state(endpoints, key)?;
    super::sync_relative_path::SyncRelativePath::parse(rel)?;
    let keys = super::KeyPolicy::for_pair(
        a.case_sensitive_paths(root_a),
        b.case_sensitive_paths(root_b),
    );
    let spellings = super::state_spellings::load(key, keys)?;
    let rel_a = spellings.rel(rel, super::PairSide::A, keys);
    let rel_b = spellings.rel(rel, super::PairSide::B, keys);
    let opts = match &key.owner {
        super::StateOwner::Job(id) => crate::syncjobs::recorded_options(id)?,
        super::StateOwner::AdHoc => super::BisyncOptions::default(),
    };
    super::apply_boundary::guard(a, root_a, &rel_a, opts.cross_mounts)?;
    super::apply_boundary::guard(b, root_b, &rel_b, opts.cross_mounts)?;
    Ok(RecordedPaths {
        path_a: crate::vfs::sync_path(a, root_a, &rel_a)?,
        path_b: crate::vfs::sync_path(b, root_b, &rel_b)?,
        rel_a,
        rel_b,
    })
}
