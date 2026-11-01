//! Optional operations pass through the daemon under the request gate.
//! P14 reservation/identity and P23 capacity preserve the backend's actual
//! guarantees and encoded error kinds, like source metadata completion.
//! A content digest is asked of the daemon (`query::FILE_DIGEST`), which
//! has it computed where the file is stored; an older daemon yields `None`.
use std::sync::atomic::AtomicBool;

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

    /// Under the same request gate as a read; only the digest crosses.
    fn content_digest(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<vfs::ContentDigest>> {
        let _permit = self.gate.enter()?;
        decode(vfs::content_digest(&*self.inner, path, cancel))
    }

    /// The sync's MD5 formed next to the data, under the same gate.
    fn content_md5(&self, path: &str, cancel: &AtomicBool) -> VfsResult<Option<String>> {
        let _permit = self.gate.enter()?;
        decode(vfs::content_md5(&*self.inner, path, cancel))
    }

    /// The MD5s of many files of the inner side together, under the same gate.
    fn content_md5_many(
        &self,
        files: &[(&str, &vfs::VfsMeta)],
        cancel: &AtomicBool,
    ) -> VfsResult<Option<Vec<Option<String>>>> {
        let _permit = self.gate.enter()?;
        decode(vfs::content_md5_many(&*self.inner, files, cancel))
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
