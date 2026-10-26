//! Optional operations retain the selected mount root and operation gate.
//! Reservation requires a writable child and checked destination; identity
//! reads the uncached object behind a checked existing path. Capacity keeps
//! the mount's read-only fact. Source metadata completion checks its stage
//! as an existing writable child. Provider errors remain sanitized.
use crate::vfs::{self, BackendExtensions, StageFinish, StageFinished, VfsResult};

use super::super::mount_error::encode as sanitize_error;
use super::RootedBackend;

impl BackendExtensions for RootedBackend {
    fn directory_reservation_supported(&self, root: &str) -> bool {
        let Ok(_operation) = self.operation.read() else {
            return false;
        };
        let Ok(root) = self.checked_existing(root) else {
            return false;
        };
        // Read-only is a separate access fact, not a missing primitive.
        vfs::directory_reservation_supported(&*self.raw_inner, &root)
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(path)?;
        let path = self.checked_destination(path)?;
        vfs::create_dir_exclusive(&*self.inner, &path).map_err(sanitize_error)
    }
    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        let _operation = self.operation.read()?;
        let path = self.checked_existing(path)?;
        vfs::directory_identity(&*self.raw_inner, &path).map_err(sanitize_error)
    }
    fn file_identity(&self, path: &str) -> VfsResult<Option<String>> {
        let _operation = self.operation.read()?;
        let path = self.checked_existing(path)?;
        vfs::file_identity(&*self.raw_inner, &path).map_err(sanitize_error)
    }

    fn capacity(&self, root: &str) -> VfsResult<vfs::CapacitySnapshot> {
        let _operation = self.operation.read()?;
        let path = self.checked_existing(root)?;
        let mut snapshot = vfs::capacity(&*self.inner, &path).map_err(sanitize_error)?;
        snapshot.read_only |= self.read_only;
        Ok(snapshot)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(stage)?;
        let stage = self.checked_existing_for_write(stage)?;
        vfs::finish_stage(&*self.inner, &stage, finish).map_err(sanitize_error)
    }
}
