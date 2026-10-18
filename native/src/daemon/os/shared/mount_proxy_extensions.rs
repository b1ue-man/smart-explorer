//! The one backend extension of the drive host's proxy (DRV-M4, decision
//! C2): `finish_stage` goes through the daemon to the authorized root
//! (`rooted_backend_extensions.rs`) under the request gate, with the error
//! kinds restored like every other proxied call. Every other extension keeps
//! the conservative default; a Windows drive never calls this one.
use crate::vfs::{self, BackendExtensions, StageFinish, StageFinished, VfsResult};

use super::{decode, MountProxy};

impl BackendExtensions for MountProxy {
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let _permit = self.gate.enter()?;
        decode(vfs::finish_stage(&*self.inner, stage, finish))
    }
}
