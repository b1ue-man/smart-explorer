//! Optional V1 contracts of the Drive provider; all names below are literal.
use super::core::{norm, split_parent};
use super::GDriveBackend;
use crate::vfs::{
    BackendExtensions, ChangeSignalMode, ChangeSubscription, MtimePrecision, StageFinish,
    StageFinished, TargetLimits, VfsListing, VfsResult,
};
use std::io::{self, Read, Write};
use std::time::Duration;

impl BackendExtensions for GDriveBackend {
    fn previous_state_identities(&self) -> VfsResult<Vec<String>> {
        // Old norm() trimmed the outer title. Such a root was ambiguous and
        // cannot prove that its former baseline belongs to the literal root.
        if self.root.trim() != self.root {
            return Ok(Vec::new());
        }
        self.legacy_state_identity().map(|identity| vec![identity])
    }
    fn sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String> {
        if literal_name.is_empty()
            || matches!(literal_name, "." | "..")
            || literal_name.contains('/')
            || literal_name.contains('\0')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Drive sync name is not one component",
            ));
        }
        self.sync_child_locator(parent, literal_name)
    }

    fn sync_stat(&self, path: &str) -> VfsResult<crate::vfs::VfsMeta> {
        self.sync_meta(path)
    }

    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        self.sync_listing(path)
    }

    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let id = match id.filter(|id| !id.is_empty()) {
            Some(id) => id.to_string(),
            None => self.resolve(path)?,
        };
        let mime = self.mime_for(path, &id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Drive object type is unknown")
        })?;
        if mime == super::api::FOLDER_MIME || mime.starts_with("application/vnd.google-apps.") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Drive regular read refuses a folder, shortcut or Google-native object",
            ));
        }
        crate::vfs::Backend::open_read_id(self, path, Some(&id))
    }

    fn target_limits(&self, _root: &str) -> TargetLimits {
        TargetLimits {
            mtime_precision: MtimePrecision::Millis,
            ..TargetLimits::default()
        }
    }

    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        super::sized_writer::open_stage_timed(self, path, size, mtime_ms)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let key = norm(stage);
        let id = self
            .owned_stages_guard()?
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Drive stage is not owned by this backend",
                )
            })?;
        let (parent, name) = split_parent(&key);
        let parent_id = self.resolve(&parent)?;
        let object = self.exact_named_object(&parent_id, &super::names::decode(name)?, &id)?;
        if object.mime_type.starts_with("application/vnd.google-apps.") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Drive stage is not binary content",
            ));
        }
        let mtime_applied = match finish.mtime_ms {
            Some(time) => self.set_modified_time(&id, time)?,
            None => false,
        };
        // Drive accepts complete server-side media commits but exposes no
        // fsync/namespace durability guarantee.
        Ok(StageFinished {
            mtime_applied,
            durable: false,
        })
    }

    fn change_signal_mode(&self, _root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        Ok(Some(ChangeSignalMode::Poll))
    }

    fn change_signal(
        &self,
        _root: &str,
        interval: Duration,
        tx: crossbeam_channel::Sender<crate::vfs::ChangeNotice>,
    ) -> VfsResult<Option<ChangeSubscription>> {
        let mut cursor = self.start_page_token()?;
        let backend = self.clone();
        crate::connect::poll_subscription(interval, tx, true, move |canceled| {
            let batch = backend.drive_changes_since_poll(&cursor, canceled)?;
            if canceled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Drive feed poll canceled",
                ));
            }
            if batch.reset {
                cursor = backend.start_page_token()?;
                return Ok(crate::connect::PollNotice::Overflow);
            }
            cursor = batch.new_cursor.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Drive change page has no terminal cursor",
                )
            })?;
            Ok(if batch.changes.is_empty() {
                crate::connect::PollNotice::Quiet
            } else {
                crate::connect::PollNotice::Changed
            })
        })
        .map(Some)
    }
}
