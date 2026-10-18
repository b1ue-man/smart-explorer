//! The one backend extension a drive host reaches through its authorized
//! root (DRV-M4, decision C2): `finish_stage` gives the upload stage of a
//! save the times and mode a POSIX program set on the file, right before the
//! stage takes the file's name. The stage path is checked like every write
//! (writable drive, a child of the root, an existing plain entry below it,
//! sanitized errors). Every other extension keeps the conservative default
//! of `BackendExtensions`, so no further capability of the wrapped backend
//! reaches the host. A Windows drive never calls it: its engine stores no
//! times or modes.
use crate::vfs::{self, BackendExtensions, StageFinish, StageFinished, VfsResult};

use super::super::mount_error::encode as sanitize_error;
use super::RootedBackend;

impl BackendExtensions for RootedBackend {
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let _operation = self.operation.write()?;
        self.require_write()?;
        Self::require_child(stage)?;
        let stage = self.checked_existing_for_write(stage)?;
        vfs::finish_stage(&*self.inner, &stage, finish).map_err(sanitize_error)
    }
}
