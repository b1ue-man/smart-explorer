//! Restore is a guarded replacement under the same pair lock as sync.
use std::io;
use std::sync::atomic::AtomicBool;
use crate::vfs::{Backend, LocalBackend, StageDurability};
use super::apply_guard::{capture, ExpectedFile};
use super::apply_retry::AttemptError;
use super::pair_lock::PairLock;
use super::types::{BisyncOptions, Throttle, VersionsLocation};
use super::versions::{RunVersions, VersionEntry, VersionReason, VersionSide, VersionStore, VersionsContext};

pub(super) fn restore(lock: &PairLock, pair: &str, entry: &VersionEntry,
    side: &VersionSide<'_>, cancel: &AtomicBool) -> io::Result<()> {
    super::transfer_stream::check(cancel)?;
    let known = super::version_ops::list(pair, std::slice::from_ref(side), cancel)?;
    if !known.contains(entry) { return Err(super::version_manifest::invalid("version is no longer listed for this pair")); }
    let expected = if entry.reason.is_some() {
        let manifest = super::version_ops::find(pair, entry, side, cancel)?;
        if !super::backend_identity_state::lock_matches(&manifest.pair, &manifest.lock, lock.id())?
            || !super::backend_identity_state::replica_matches(&manifest.pair, &manifest.replica,
                &super::version_save::replica(side.backend, side.root)?,
                if side.side == super::PairSide::A { 0 } else { 1 })? {
            return Err(super::version_manifest::invalid("version belongs to another pair or replica"));
        }
        Some(manifest.data_sig)
    } else { None };
    super::apply_boundary::guard(side.backend, side.root, &entry.rel, false)?;
    super::apply_boundary::target(side.backend, side.root, &entry.rel, Some(entry.size))?;
    let destination = crate::vfs::sync_path(side.backend, side.root, &entry.rel)?;
    let current = capture(side.backend, &destination, ExpectedFile::Unknown, "restore destination")?;
    let context = VersionsContext::new(pair, entry.job_id.clone().map_or(super::StateOwner::AdHoc, super::StateOwner::Job),
        VersionsLocation::Auto, Default::default());
    let versions = RunVersions::begin(context);
    versions.bind_lock(lock.id())?;
    // Keep the previous original with an explicit restore reason before the
    // normal transfer is allowed to publish.
    let preserved = if current.metadata.is_some() {
        Some(super::version_save::save(&versions, side, &destination, &entry.rel, &current,
            ExpectedFile::Unknown, VersionReason::Restored, cancel)?)
    } else { None };
    let result = if entry.store == VersionStore::SyncRoot {
        transfer(side.backend, &entry.stored_path, expected, side, &destination, &current, preserved.as_ref(), &entry.rel, &versions, cancel)
    } else {
        let root = super::version_ops::app_root(pair, &entry.stored_path)?;
        let backend = LocalBackend::new(root.to_str().ok_or_else(|| super::version_manifest::invalid("version path is not Unicode"))?);
        transfer(&backend, &entry.stored_path, expected.map(|mut sig| { sig.size = entry.size; sig }), side,
            &destination, &current, preserved.as_ref(), &entry.rel, &versions, cancel)
    };
    if let Err(error) = result {
        if let Some(preserved) = preserved.as_ref() {
            if let Err(rollback) = super::version_save::rollback(side, &destination, preserved) {
                return Err(io::Error::other(format!("restore failed ({error}); original rollback failed: {rollback}")));
            }
        }
        return Err(error);
    }
    versions.finish()
}
#[allow(clippy::too_many_arguments)]
fn transfer(source: &dyn Backend, source_path: &str, expected: Option<super::Sig>,
    side: &VersionSide<'_>, destination: &str, current: &super::apply_guard::CapturedFile,
    preserved: Option<&super::version_save::Preserved>, rel: &str, versions: &RunVersions, cancel: &AtomicBool,
) -> io::Result<()> {
    let destination_expected = if preserved.is_some_and(|version| version.moved) { ExpectedFile::Missing }
        else if let Some(version) = preserved { ExpectedFile::Present(version.signature) }
        else { current.metadata.as_ref().map_or(ExpectedFile::Missing, |meta| ExpectedFile::Present(
            super::Sig { size: meta.size, mtime_ms: meta.mtime_ms, hash: 0 })) };
    let outcome = super::apply_transaction::copy(source, source_path,
        expected.map_or(ExpectedFile::Unknown, ExpectedFile::Present), side, destination, destination_expected,
        rel, BisyncOptions { reversible: false, verify: true, ..Default::default() }, Some(versions),
        StageDurability::Now, &Throttle::new(0), cancel, None, |_| {}, |_| {}).map_err(AttemptError::into_io)?;
    super::apply_stage::require_durable(outcome.durable)
}
