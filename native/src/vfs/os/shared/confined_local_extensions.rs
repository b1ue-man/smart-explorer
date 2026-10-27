//! Local extensions retain the same directory/file authority as data I/O.
use super::confined_local::ConfinedLocalBackend;
use super::confined_local_platform as platform;
use super::{
    Backend, BackendExtensions, CapacitySnapshot, StageDurability, StageFinish, StageFinished,
    TargetLimits, VfsListing, VfsResult, VolumeIdentity,
};
use std::io::{self, Read, Write};

impl BackendExtensions for ConfinedLocalBackend {
    fn confined_local_root(&self, root: &str) -> VfsResult<Option<super::BackendHandle>> {
        // A nested selection stays under this already-held authority. It never
        // reopens the external spelling of the original root.
        let _optical = crate::source_media::acquire_file_current(
            self.root.file(),
            crate::source_media::Priority::Foreground,
        )?;
        let held = self.restrict(root)?;
        Ok(Some(std::sync::Arc::new(held)))
    }

    fn capacity(&self, root: &str) -> VfsResult<CapacitySnapshot> {
        let directory = self.directory(root)?;
        match platform::capacity(&directory, root, self.state_identity()) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => Ok(super::capacity_failure(
                self.state_identity(),
                root,
                super::CapacitySource::Filesystem,
                &error,
            )),
        }
    }
    fn directory_reservation_supported(&self, root: &str) -> bool {
        self.parts(root).is_ok()
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        self.create_dir_new(path)
    }
    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        let _optical = crate::source_media::acquire_file_current(
            self.root.file(),
            crate::source_media::Priority::Foreground,
        )?;
        let entry = self.entry(path)?;
        if !entry.metadata()?.is_dir() || crate::local_access::classify_open_file(&entry)?.link_like
        {
            return Ok(None);
        }
        let directory = self.directory(path)?;
        let identity = if self.parts(path)?.is_empty() {
            directory.identity().map(Some)
        } else {
            directory.stable_identity()
        };
        match identity {
            Ok(identity) => Ok(identity),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        let _optical = crate::source_media::acquire_file_current(
            self.root.file(),
            crate::source_media::Priority::Foreground,
        )?;
        self.tolerant(path)
    }
    fn file_identity(&self, path: &str) -> VfsResult<Option<String>> {
        let _optical = crate::source_media::acquire_file_current(
            self.root.file(),
            crate::source_media::Priority::Foreground,
        )?;
        super::local_file_identity::handle_identity(&self.entry(path)?)
    }
    fn open_read_regular(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.open_read(path)
    }
    /// Hashed here, through the same held root as every read.
    fn content_digest(
        &self,
        path: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<super::ContentDigest>> {
        super::digest_by_local_read(self, path, cancel)
    }
    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        _mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(self.sized_stage(path, size, false)?))
    }
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let file = self.existing_file(stage, platform::stage_write_access())?;
        platform::validate_stage(&file.file)?;
        let mut result = StageFinished::default();
        if let Some(milliseconds) = finish.mtime_ms {
            let offset = std::time::Duration::from_millis(milliseconds.unsigned_abs());
            let time = if milliseconds >= 0 {
                std::time::UNIX_EPOCH.checked_add(offset)
            } else {
                std::time::UNIX_EPOCH.checked_sub(offset)
            };
            let time = time.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "stage modification time out of range",
                )
            })?;
            match file.file.set_modified(time) {
                Ok(()) => result.mtime_applied = true,
                Err(error) if metadata_optional(&error) => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(mode) = finish.mode {
            match platform::set_mode(&file.file, mode & 0o777) {
                Ok(()) => {}
                Err(error) if metadata_optional(&error) => {}
                Err(error) => return Err(error),
            }
        }
        if finish.durability != StageDurability::NotRequired {
            file.file.sync_all()?;
            result.durable = true;
        }
        Ok(result)
    }
    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        platform::sync_filesystem(&self.directory(root)?)
    }
    fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
        platform::confirm_namespace(&self.directory(parent)?)
    }
    fn target_limits(&self, root: &str) -> TargetLimits {
        self.directory(root)
            .ok()
            .and_then(|directory| platform::limits(&directory).ok())
            .unwrap_or_else(|| super::local_platform::fallback_limits())
    }
    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        let _optical = crate::source_media::acquire_file_current(
            self.root.file(),
            crate::source_media::Priority::Foreground,
        )?;
        platform::mode(&self.entry(path)?)
    }
    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        platform::volume(&self.directory(root)?)
    }
}
fn metadata_optional(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
    )
}
