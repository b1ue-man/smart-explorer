//! One mirror holds the bisync pair lock through backups, publication and retention.
use crate::bisync::{PairLock, StateOwner, Versioning, VersionsLocation};
use crate::bisync::versions::{RunVersions, VersionsContext, VersionSide};
use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct MirrorRun { pub(super) versions: RunVersions, lock: PairLock, pair: String }
impl MirrorRun {
    pub(super) fn begin(src: &dyn Backend, src_root: &str, dst: &dyn Backend, dst_root: &str) -> io::Result<Self> {
        let lock = PairLock::acquire(&crate::bisync::pair_lock_id(src, src_root, dst, dst_root))?;
        let pair = crate::bisync::pair_id_for(src, src_root, dst, dst_root);
        let versions = RunVersions::begin(VersionsContext::new(&pair, StateOwner::AdHoc,
            VersionsLocation::Auto, Versioning::default()));
        versions.bind_lock(lock.id())?;
        Ok(Self { versions, lock, pair })
    }
    pub(super) fn finish(&self, dst: &dyn Backend, dst_root: &str, cancel: &AtomicBool) -> io::Result<()> {
        self.versions.finish()?;
        if cancel.load(Ordering::Acquire) { return Ok(()); }
        crate::bisync::versions::prune_after_run(&self.lock, &self.pair,
            &[VersionSide { side: crate::bisync::PairSide::B, backend: dst, root: dst_root }],
            &self.versions.context().versioning, cancel)
    }
}
