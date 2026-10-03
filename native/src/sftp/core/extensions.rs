//! V1 facts and stage completion for SFTP v3. A server acknowledgement is
//! not a time or durability guarantee: read the attributes back and require
//! an advertised, successful fsync. Namespace durability remains unknown.
use super::backend::SftpBackend;
use super::channel_pool::note_failure;
use super::io_err;
use crate::vfs::{Backend, BackendExtensions, MtimePrecision, NameLimit, OmissionReason,
    StageDurability, StageFinish, StageFinished, TargetLimits, VfsListing, VfsOmission, VfsResult};
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use std::collections::HashSet;
use std::io::{self, Read, Write};

fn seconds(ms: i64) -> Option<u32> {
    u32::try_from(ms.div_euclid(1_000)).ok()
}

fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((parent, _)) => parent.into(),
        None => ".".into(),
    }
}

impl BackendExtensions for SftpBackend {
    fn replace_staged_reversible(&self, staged: &str, destination: &str, retained: &str) -> VfsResult<bool> {
        self.replace_retaining_original(staged, destination, retained).map(|()| true)
    }
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        let mut output = VfsListing::default();
        let mut names = HashSet::new();
        for entry in self.list_dir(path)? {
            if !names.insert(entry.name.clone()) {
                return Err(io::Error::new(io::ErrorKind::InvalidData,
                    "SFTP returned duplicate or lossy-colliding child names"));
            }
            if entry.name.contains('\u{fffd}') || crate::vfs::validate_child_name(&entry.name).is_err() {
                output.omitted.push(VfsOmission { rel: entry.name,
                    reason: OmissionReason::Unrepresentable,
                    detail: "SFTP child name is not safely addressable by this path interface".into() });
            } else {
                output.entries.push(entry);
            }
        }
        Ok(output)
    }

    fn open_read_regular(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let metadata = self.stat(path)?;
        if metadata.is_dir || metadata.is_symlink || metadata.special {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "SFTP read requires a regular file; links and special entries are omitted"));
        }
        // SFTP v3 has no O_NOFOLLOW; this preserves the server's OPEN
        // semantics and does not claim local handle confinement.
        self.open_read(path)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let (_, existing) = self.safe_sftp_on(|generation| {
            self.rt.block_on(generation.sftp().symlink_metadata(stage.to_string()))
        })?;
        let kind = existing.file_type();
        if kind.is_dir() || kind.is_symlink() || (kind.is_other()
            && existing.permissions.is_some_and(|mode| mode & 0o170000 != 0)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "SFTP stage is not a regular file"));
        }
        let requested = finish.mtime_ms.and_then(seconds);
        let mut attributes = FileAttributes::empty();
        if let Some(mtime) = requested {
            // The v3 ACMODTIME flag serializes BOTH fields; omitting atime
            // would write zero. Preserve the known atime, else use mtime.
            attributes.atime = Some(existing.atime.unwrap_or(mtime));
            attributes.mtime = Some(mtime);
        }
        attributes.permissions = finish.mode.map(|mode| mode & 0o777);
        if requested.is_some() || attributes.permissions.is_some() {
            let generation = self.connection.current()?;
            if let Err(error) = self.rt.block_on(generation.sftp().set_metadata(stage.to_string(), attributes)) {
                self.connection.note_sftp_error(&generation, &error);
                let error = self.target_error(stage, io_err(error));
                if !matches!(error.kind(), io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied) {
                    return Err(error);
                }
            }
        }
        let (_, actual) = self.safe_sftp_on(|generation| {
            self.rt.block_on(generation.sftp().symlink_metadata(stage.to_string()))
        })?;
        let kind = actual.file_type();
        if kind.is_dir() || kind.is_symlink() || (kind.is_other()
            && actual.permissions.is_some_and(|mode| mode & 0o170000 != 0)) {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "SFTP stage type changed during metadata completion"));
        }
        if let Some(mode) = finish.mode {
            if actual.permissions.is_none_or(|actual| actual & 0o777 & !(mode & 0o777) != 0) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                    "SFTP stage could not keep the requested restrictive Unix mode"));
            }
        }
        let applied = requested.is_some_and(|mtime| actual.mtime == Some(mtime));
        let durable = finish.durability != StageDurability::NotRequired && self.flush_stage(stage)?;
        Ok(StageFinished { mtime_applied: applied, durable })
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        let name_max = self.connection.safe_metadata(|generation| {
            Box::pin(generation.sftp().fs_info(root.to_string()))
        }).ok().flatten().and_then(|info| usize::try_from(info.name_max).ok()).filter(|max| *max > 0);
        TargetLimits { max_name: name_max.map(NameLimit::Bytes),
            mtime_precision: MtimePrecision::Seconds, ..TargetLimits::default() }
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        self.connection.safe_metadata(|generation| {
            Box::pin(generation.sftp().symlink_metadata(path.to_string()))
        }).map(|attributes| attributes.permissions.map(|mode| mode & 0o777))
    }
}

impl SftpBackend {
    fn flush_stage(&self, stage: &str) -> VfsResult<bool> {
        let Some(lease) = self.pool.lease(self)? else { return Ok(false) };
        let channel = lease.channel();
        if !channel.fsync { return Ok(false) }
        let result = self.rt.block_on(async {
            let handle = channel.session.open(stage, OpenFlags::WRITE, FileAttributes::empty()).await?.handle;
            let synced = channel.session.fsync(handle.clone()).await;
            let closed = channel.session.close(handle).await;
            synced?;
            closed?;
            Ok::<(), russh_sftp::client::error::Error>(())
        });
        let result = result.map_err(|error| {
            note_failure(&self.connection, channel, &error);
            self.target_error(stage, io_err(error))
        });
        if result.as_ref().is_err_and(|error| error.kind() == io::ErrorKind::Unsupported) {
            return Ok(false);
        }
        result?;
        Ok(true)
    }

    /// SSH_FX_FAILURE alone is ambiguous. Only a responding statvfs may
    /// refine it; failure of that diagnostic must preserve the original.
    pub(super) fn target_error(&self, path: &str, error: io::Error) -> io::Error {
        if error.kind() != io::ErrorKind::Other { return error }
        let directory = parent(path);
        let Ok(Some(info)) = self.connection.safe_metadata(|generation| {
            Box::pin(generation.sftp().fs_info(directory.clone()))
        }) else { return error };
        let kind = if info.flags & 1 != 0 { io::ErrorKind::ReadOnlyFilesystem }
            else if info.blocks_avail == 0 { io::ErrorKind::StorageFull }
            else { return error };
        io::Error::new(kind, error)
    }

    pub(super) fn target_writer(&self, path: &str, writer: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
        Box::new(TargetWriter { backend: self.clone(), path: path.into(), writer })
    }
}

struct TargetWriter { backend: SftpBackend, path: String, writer: Box<dyn Write + Send> }
impl Write for TargetWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer.write(bytes).map_err(|error| self.backend.target_error(&self.path, error))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush().map_err(|error| self.backend.target_error(&self.path, error))
    }
}
