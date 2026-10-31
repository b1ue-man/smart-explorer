//! V1 facts and stage completion for SFTP v3. A server acknowledgement is
//! not a time or durability guarantee: read the attributes back and require
//! an advertised, successful fsync. Namespace durability remains unknown.
use super::backend::SftpBackend;
use crate::vfs::{
    Backend, BackendExtensions, MtimePrecision, NameLimit, OmissionReason, RecursiveDeleteFailure,
    RecursiveDeleteProgress, RecursiveDeleteReport, StageFinish, StageFinished, TargetLimits,
    VfsListing, VfsOmission, VfsResult,
};
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;

fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((parent, _)) => parent.into(),
        None => ".".into(),
    }
}

impl BackendExtensions for SftpBackend {
    fn directory_reservation_supported(&self, _root: &str) -> bool {
        // One SSH_FXP_MKDIR; persistent identity is a separate contract.
        true
    }

    fn creates_stages_exclusively(&self) -> bool {
        // `open_write_new`, the copy stage writers and server copy-data all
        // open with CREATE|EXCLUDE (`create_new_flags`, copy_data.rs).
        true
    }

    fn stage_proven(&self, stage: &str) -> bool {
        self.written.proven(stage).is_some()
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        super::folder_reservation::create(self, path)
    }

    /// Up to 64 `copy-data` chains at once on one channel (copy_data.rs).
    fn server_copy_batch(&self, items: &[(String, String, u64)]) -> Vec<io::Result<Option<u64>>> {
        self.copy_batch_on_server(items)
    }

    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        super::folder_reservation::identity(self, path)
    }

    fn capacity(&self, root: &str) -> VfsResult<crate::vfs::CapacitySnapshot> {
        super::capacity::query(self, root)
    }

    fn replace_staged_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> VfsResult<bool> {
        self.replace_retaining_original(staged, destination, retained)
            .map(|()| true)
    }
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        let mut output = VfsListing::default();
        let mut names = HashSet::new();
        for entry in self.list_dir(path)? {
            if !names.insert(entry.name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SFTP returned duplicate or lossy-colliding child names",
                ));
            }
            if entry.name.contains('\u{fffd}')
                || crate::vfs::validate_child_name(&entry.name).is_err()
            {
                output.omitted.push(VfsOmission {
                    rel: entry.name,
                    reason: OmissionReason::Unrepresentable,
                    detail: "SFTP child name is not safely addressable by this path interface"
                        .into(),
                });
            } else {
                output.entries.push(entry);
            }
        }
        Ok(output)
    }

    fn open_read_regular(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let metadata = self.stat(path)?;
        if metadata.is_dir || metadata.is_symlink || metadata.special {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SFTP read requires a regular file; links and special entries are omitted",
            ));
        }
        // SFTP v3 has no O_NOFOLLOW; this preserves the server's OPEN
        // semantics and does not claim local handle confinement.
        self.open_read(path)
    }

    /// One LSTAT, one OPEN, then FSETSTAT, fsync, FSTAT, CLOSE and LSTAT in
    /// flight together (MS8, stage_finish.rs).
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        self.finish_stage_now(stage, finish)
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        let name_max = self
            .connection
            .safe_metadata(|generation| Box::pin(generation.sftp().fs_info(root.to_string())))
            .ok()
            .flatten()
            .and_then(|info| usize::try_from(info.name_max).ok())
            .filter(|max| *max > 0);
        TargetLimits {
            max_name: name_max.map(NameLimit::Bytes),
            mtime_precision: MtimePrecision::Seconds,
            ..TargetLimits::default()
        }
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        self.connection
            .safe_metadata(|generation| {
                Box::pin(generation.sftp().symlink_metadata(path.to_string()))
            })
            .map(|attributes| attributes.permissions.map(|mode| mode & 0o777))
    }

    /// Every folder: one READDIR per folder and pipelined REMOVEs
    /// (tree_remove.rs) instead of a stat and a round trip per entry.
    fn supports_remove_tree(&self, _path: &str) -> VfsResult<bool> {
        Ok(true)
    }

    fn remove_tree(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        crate::vfs::remove_tree_listed(self, path, cancel, progress, |leaves| {
            self.remove_leaves(leaves)
        })
    }

    /// Packets give each file the time its entry names: an `FSETSTAT` on the
    /// open handle behind the WRITEs, before the CLOSE (batch_put.rs).
    fn batch_put_times(&self) -> bool {
        true
    }

    /// The unsynced copy stage that also gets the source's time at its
    /// commit, set on the open handle behind the last WRITE
    /// (pool_writer_stamp.rs). Without a pool channel the main-session writer
    /// keeps the time of the upload and the engine finishes the stage itself.
    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer(
            path,
            size,
            super::io_adapters::Commit::Unsynced,
            super::stage_finish::seconds(mtime_ms),
        )
    }

    /// Packets report the destinations before and the files after their
    /// publication (batch_put_evidence.rs, G3-03).
    fn transfer_evidence(&self, _path: &str) -> bool {
        true
    }

    fn put_batch_evidence(
        &self,
        entries: &[crate::vfs::BatchPut],
        data: &mut dyn Read,
        prepare: &mut crate::vfs::PreparePut<'_>,
    ) -> VfsResult<Vec<crate::vfs::EvidencePutOutcome>> {
        super::batch_put_evidence::run(self, entries, data, prepare)
    }

    fn promote_copy_stage_evidence(
        &self,
        staged: &str,
        destination: &str,
    ) -> VfsResult<Option<crate::vfs::FileEvidence>> {
        self.promote_evidenced(staged, destination)
    }

    fn take_stage_evidence(&self, stage: &str) -> Option<crate::vfs::StageEvidence> {
        self.written.take(stage)
    }
}

impl SftpBackend {
    /// SSH_FX_FAILURE alone is ambiguous. Only a responding statvfs may
    /// refine it; failure of that diagnostic must preserve the original.
    pub(super) fn target_error(&self, path: &str, error: io::Error) -> io::Error {
        if error.kind() != io::ErrorKind::Other {
            return error;
        }
        let directory = parent(path);
        let Ok(Some(info)) = self
            .connection
            .safe_metadata(|generation| Box::pin(generation.sftp().fs_info(directory.clone())))
        else {
            return error;
        };
        let kind = if info.flags & 1 != 0 {
            io::ErrorKind::ReadOnlyFilesystem
        } else if info.blocks_avail == 0 {
            io::ErrorKind::StorageFull
        } else {
            return error;
        };
        io::Error::new(kind, error)
    }

    pub(super) fn target_writer(
        &self,
        path: &str,
        writer: Box<dyn Write + Send>,
    ) -> Box<dyn Write + Send> {
        Box::new(TargetWriter {
            backend: self.clone(),
            path: path.into(),
            writer,
        })
    }
}

struct TargetWriter {
    backend: SftpBackend,
    path: String,
    writer: Box<dyn Write + Send>,
}
impl Write for TargetWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer
            .write(bytes)
            .map_err(|error| self.backend.target_error(&self.path, error))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer
            .flush()
            .map_err(|error| self.backend.target_error(&self.path, error))
    }
}
