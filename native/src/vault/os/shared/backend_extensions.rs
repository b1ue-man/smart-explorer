//! The vault backend's own `BackendExtensions` (C2.5): a layer that
//! rewrites paths never hands out the storage's extensions unchanged
//! (`vfs::BackendExtensions`). Stage operations go to the storage with the
//! mapped stage paths; tolerant listing, regular reads and limits are the
//! vault's own; everything else keeps the conservative default: no
//! archive outside the provider, no volume identity, no trash, no tree
//! removal in one call, no change signal, no host-side hash walk or
//! duplicate search, no Unix modes.
use std::io::{self, Read, Write};

use crate::vault::core::capabilities::cleartext_limits;
use crate::vault::core::path_map::components;
use crate::vfs::{
    self, BackendExtensions, StageFinish, StageFinished, TargetLimits, VfsListing, VfsResult,
};

use super::backend::VaultBackend;
use super::backend_nodes::{NodeKind, Parts};

impl BackendExtensions for VaultBackend {
    /// Reversible replacement on the storage for a short-named file whose
    /// retained sibling (`.se-replace-…`) stays next to it in the same
    /// content directory; `Ok(false)` (unsupported, nothing changed) for
    /// long names and everything else.
    fn replace_staged_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> VfsResult<bool> {
        self.replace_reversible(staged, destination, retained)
    }

    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        self.list_tolerant(path)
    }

    /// Entries only of regular files: folders and links are refused.
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.reader(path, id, true)
    }

    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer_timed(path, size, mtime_ms)
    }

    /// Time and durability of the stored stage (listed times are the
    /// stored files' times).
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let at = self.at(&self.stage_rel_required(&Self::parts(stage)?)?);
            vfs::finish_stage(&*self.inner, &at, finish)
        })();
        result.map_err(|error| self.session.relock(error))
    }

    fn sync_filesystem(&self, _root: &str) -> VfsResult<bool> {
        vfs::sync_filesystem(&*self.inner, &self.root)
    }

    /// The parent's content directory on the storage.
    fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let dir = self.dir_for(&components(parent)?)?;
            vfs::confirm_namespace(&*self.inner, &self.at(&dir.content))
        })();
        result.map_err(|error| self.session.relock(error))
    }

    fn target_limits(&self, _root: &str) -> TargetLimits {
        let inner = vfs::target_limits(&*self.inner, &self.root);
        cleartext_limits(inner, self.session.config().combo)
    }
}

impl VaultBackend {
    fn replace_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> io::Result<bool> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let from = self.at(&self.stage_rel_required(&Self::parts(staged)?)?);
            let target = Self::parts(destination)?;
            let found = match self.find_at(&target) {
                Ok(found) => found,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error),
            };
            if found.node.shortened || !matches!(found.kind, NodeKind::File { .. }) {
                return Ok(false);
            }
            let keep = Parts::of(retained)?
                .filter(|keep| keep.parent_key() == target.parent_key())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "replacement recovery path must be a unique generated sibling",
                    )
                })?;
            let keep = self.at(&self.stage_rel_required(&keep)?);
            let done = vfs::replace_staged_reversible(
                &*self.inner,
                &from,
                &self.at(&found.node.node()),
                &keep,
            )?;
            self.cache.forget_aliases(&target.parent_key());
            Ok(done)
        })();
        result.map_err(|error| self.session.relock(error))
    }
}
