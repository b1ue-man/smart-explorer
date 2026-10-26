//! Optional operations pass through the daemon under the request gate.
//! P14 reservation/identity and P23 capacity preserve the backend's actual
//! guarantees and encoded error kinds, like source metadata completion.
use crate::vfs::{self, BackendExtensions, StageFinish, StageFinished, VfsResult};

use super::{decode, MountProxy};

impl BackendExtensions for MountProxy {
    fn directory_reservation_supported(&self, root: &str) -> bool {
        let Ok(_permit) = self.gate.enter() else {
            return false;
        };
        vfs::directory_reservation_supported(&*self.inner, root)
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(vfs::create_dir_exclusive(&*self.inner, path))
    }
    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        let _permit = self.gate.enter()?;
        decode(vfs::directory_identity(&*self.inner, path))
    }
    fn file_identity(&self, path: &str) -> VfsResult<Option<String>> {
        let _permit = self.gate.enter()?;
        decode(vfs::file_identity(&*self.inner, path))
    }

    fn capacity(&self, root: &str) -> VfsResult<crate::vfs::CapacitySnapshot> {
        {
            let _permit = self.gate.enter()?;
            decode(vfs::capacity(&*self.inner, root))
        }
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let _permit = self.gate.enter()?;
        decode(vfs::finish_stage(&*self.inner, stage, finish))
    }
}
