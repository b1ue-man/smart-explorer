//! Exclusive stages, byte-bound signatures and the publication boundary.
use std::io::{self, Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;
use crate::vfs::{Backend, StageDurability, StageFinish};
use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::paths::parent_of;
use super::transfer_stream::{check, stream, Streamed};
use super::types::{Sig, Throttle};

pub(crate) struct Staged<'a> {
    backend: &'a dyn Backend,
    pub(crate) path: String,
    pub(crate) bytes: Streamed,
    pub(crate) source: Sig,
    pub(crate) durability: StageDurability,
    published: bool,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct CopyOutcome {
    pub(crate) bytes: u64,
    pub(crate) digest: [u8; 16],
    pub(crate) source: Sig,
    pub(crate) destination: Sig,
    pub(crate) durable: bool,
}

pub(crate) fn namespace(backend: &dyn Backend, path: &str) -> io::Result<bool> {
    if !backend.is_local() { return Ok(true); }
    crate::vfs::sync_filesystem(backend, &parent_of(path).unwrap_or_else(|| path.to_string()))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stage<'a>(
    source: &dyn Backend, source_path: &str, captured: &CapturedFile,
    expected: ExpectedFile, destination: &'a dyn Backend, destination_path: &str,
    destination_state: &CapturedFile, durability: StageDurability,
    throttle: &Throttle, cancel: &AtomicBool, mut progress: impl FnMut(u64),
) -> io::Result<Staged<'a>> {
    check(cancel)?;
    let metadata = captured.regular("copy source")?;
    if let Some(parent) = parent_of(destination_path) { destination.mkdir_all(&parent)?; }
    let path = crate::vfs::unique_staging_path(destination, destination_path, "bisync")?;
    let authoritative = source.download_name(source_path, &metadata.name) == metadata.name;
    // A single non-duplex session cannot keep a reader and writer alive
    // together. Exports with unknown size need a measured upload length too.
    let spool = !authoritative || (source.namespace_identity() == destination.namespace_identity()
        && (!source.concurrent_read_write() || !destination.concurrent_read_write()));
    let mut staged = Staged { backend: destination, path, bytes: Streamed { bytes: 0, digest: [0; 16] },
        source: Sig { size: metadata.size, mtime_ms: metadata.mtime_ms, hash: 0 },
        durability, published: false };
    let source_mode = crate::vfs::unix_mode(source, source_path)?;
    let destination_mode = if destination_state.metadata.is_some() {
        crate::vfs::unix_mode(destination, destination_path)?
    } else { None };
    let mode = match (source_mode, destination_mode) {
        (Some(source), Some(destination)) => Some(source & destination & 0o777),
        (Some(source), None) => Some(source & 0o777),
        (None, destination) => destination.map(|mode| mode & 0o777),
    };
    let transferred = if spool {
        let mut spool_file = tempfile::tempfile_in(crate::support_dirs::temp_dir())?;
        let result = {
            let mut reader = crate::vfs::open_read_regular(source, source_path, metadata.id.as_deref())?;
            stream(&mut *reader, &mut spool_file, cancel, Some(throttle), expected.hash(), &mut progress)?
        };
        spool_file.flush()?;
        revalidate(source, source_path, captured, "copy source")?;
        spool_file.seek(SeekFrom::Start(0))?;
        super::apply_boundary::target(destination, destination_path, "data", Some(result.bytes))?;
        let mut writer = crate::vfs::open_write_copy_stage_timed(destination, &staged.path,
            result.bytes, metadata.mtime_ms)?;
        let uploaded = stream(&mut spool_file, &mut *writer, cancel, None, result.hash(), |_| {})?;
        writer.flush()?;
        drop(writer);
        if uploaded.bytes != result.bytes || uploaded.digest != result.digest {
            return Err(drift("spooled content changed while uploading"));
        }
        result
    } else {
        let mut reader = crate::vfs::open_read_regular(source, source_path, metadata.id.as_deref())?;
        let mut writer = crate::vfs::open_write_copy_stage_timed(destination, &staged.path,
            metadata.size, metadata.mtime_ms)?;
        let result = stream(&mut *reader, &mut *writer, cancel, Some(throttle), expected.hash(), &mut progress)?;
        writer.flush()?;
        drop(writer);
        drop(reader);
        result
    };
    if authoritative && transferred.bytes != metadata.size { return Err(drift("copy source size changed")); }
    if let Some(md5) = &metadata.content_md5 {
        if !md5.eq_ignore_ascii_case(&transferred.hex()) { return Err(drift("copy source digest changed")); }
    }
    revalidate(source, source_path, captured, "copy source")?;
    let state = capture(destination, &staged.path, ExpectedFile::Unknown, "copy stage")?;
    let stage_meta = state.regular("copy stage")?;
    if stage_meta.size != transferred.bytes { return Err(drift("copy stage has the wrong size")); }
    if stage_meta.content_md5.as_ref().is_some_and(|hash| !hash.eq_ignore_ascii_case(&transferred.hex())) {
        return Err(drift("copy stage has the wrong digest"));
    }
    let finished = crate::vfs::finish_stage(destination, &staged.path, StageFinish {
        mtime_ms: Some(metadata.mtime_ms), mode, durability,
    })?;
    if destination.is_local() && durability == StageDurability::Now && !finished.durable {
        require_durable(namespace(destination, &staged.path)?)?;
    }
    staged.bytes = transferred;
    staged.source.hash = transferred.hash();
    Ok(staged)
}

pub(super) fn stage_bytes<'a>(destination: &'a dyn Backend, path: &str, current: &CapturedFile,
    bytes: &[u8], mtime_ms: i64, cancel: &AtomicBool,
) -> io::Result<Staged<'a>> {
    check(cancel)?;
    if let Some(parent) = parent_of(path) { destination.mkdir_all(&parent)?; }
    let stage = crate::vfs::unique_staging_path(destination, path, "merge")?;
    let mut staged = Staged { backend: destination, path: stage,
        bytes: Streamed { bytes: bytes.len() as u64, digest: md5::compute(bytes).0 },
        source: Sig { size: bytes.len() as u64, mtime_ms, hash: super::snapshot_hash::md5_to_u64(&md5::compute(bytes).0) },
        durability: StageDurability::Now, published: false };
    let mode = if current.metadata.is_some() { crate::vfs::unix_mode(destination, path)? } else { Some(0o600) };
    let mut writer = crate::vfs::open_write_copy_stage_timed(destination, &staged.path, bytes.len() as u64, mtime_ms)?;
    for block in bytes.chunks(256 * 1024) { check(cancel)?; writer.write_all(block)?; }
    writer.flush()?;
    drop(writer);
    let meta = capture(destination, &staged.path, ExpectedFile::Unknown, "merged stage")?;
    if meta.regular("merged stage")?.size != bytes.len() as u64 { return Err(drift("merged stage size differs")); }
    let finished = crate::vfs::finish_stage(destination, &staged.path, StageFinish { mtime_ms: Some(mtime_ms),
        mode: mode.map(|mode| mode & 0o777), durability: StageDurability::Now })?;
    if destination.is_local() && !finished.durable { require_durable(namespace(destination, &staged.path)?)?; }
    staged.bytes.digest = md5::compute(bytes).0;
    Ok(staged)
}

impl Staged<'_> {
    pub(super) fn publish(mut self, destination: &str, current: &CapturedFile,
        verify: bool, cancel: &AtomicBool) -> io::Result<CopyOutcome> {
        check(cancel)?;
        revalidate(self.backend, destination, current, "copy destination")?;
        // After an attempted publish, a lost ACK may hide a committed
        // mutation. Retain the stage for intent recovery instead of
        // letting Drop discard possible recovery evidence.
        let result = if let Some(meta) = current.metadata.as_ref() {
            if self.backend.has_duplicate_file_names() {
                let id = meta.id.as_deref().ok_or_else(|| drift("ID-addressed destination has no stable ID"))?;
                self.published = true;
                self.backend.promote_staged_to_id(&self.path, destination, Some(id))
            } else {
                self.published = true;
                crate::vfs::promote_staged_replace(self.backend, &self.path, destination)
            }
        } else {
            self.published = true;
            crate::vfs::promote_staged_create(self.backend, &self.path, destination)
        };
        result?;
        let current = super::apply_guard::current_like(self.backend, destination, current, "published copy")?;
        let metadata = current.regular("published copy")?;
        if metadata.size != self.bytes.bytes { return Err(drift("published copy has the wrong size")); }
        if metadata.content_md5.as_ref().is_some_and(|hash| !hash.eq_ignore_ascii_case(&self.bytes.hex())) {
            return Err(drift("published copy has the wrong digest"));
        }
        if verify && metadata.content_md5.is_none() {
            let mut reader = crate::vfs::open_read_regular(self.backend, destination, metadata.id.as_deref())?;
            let finish = AtomicBool::new(false);
            let checked = stream(&mut *reader, &mut io::sink(), &finish, None, self.bytes.hash(), |_| {})?;
            if checked.bytes != self.bytes.bytes || checked.digest != self.bytes.digest {
                return Err(drift("copy verification failed"));
            }
            revalidate(self.backend, destination, &current, "published copy")?;
        }
        // Deferred is allowed only after the caller proved a root flush
        // exists; the checkpoint must actually perform it before recording.
        let durable = if self.backend.is_local() && self.durability == StageDurability::Deferred {
            false
        } else { namespace(self.backend, destination)? };
        Ok(CopyOutcome { bytes: self.bytes.bytes, digest: self.bytes.digest, source: self.source,
            destination: Sig { size: metadata.size, mtime_ms: metadata.mtime_ms,
                hash: self.bytes.hash() }, durable })
    }

    pub(super) fn publish_sibling(&mut self, candidate: &str, cancel: &AtomicBool) -> io::Result<CopyOutcome> {
        check(cancel)?;
        match self.backend.rename_no_replace(&self.path, candidate) {
            Ok(()) => self.published = true,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::AlreadyExists { self.published = true; }
                return Err(error);
            }
        }
        let captured = capture(self.backend, candidate, ExpectedFile::Unknown, "conflict copy")?;
        let meta = captured.regular("conflict copy")?;
        if meta.size != self.bytes.bytes { return Err(drift("conflict copy has the wrong size")); }
        if meta.content_md5.as_ref().is_some_and(|hash| !hash.eq_ignore_ascii_case(&self.bytes.hex())) {
            return Err(drift("conflict copy has the wrong digest"));
        }
        let durable = namespace(self.backend, candidate)?;
        Ok(CopyOutcome { bytes: self.bytes.bytes, digest: self.bytes.digest, source: self.source,
            destination: Sig { size: meta.size, mtime_ms: meta.mtime_ms, hash: self.bytes.hash() }, durable })
    }
}
impl Drop for Staged<'_> {
    fn drop(&mut self) {
        if !self.published { let _ = self.backend.discard_copy_stage(&self.path); }
    }
}

pub(super) fn require_durable(durable: bool) -> io::Result<()> {
    if durable { Ok(()) } else {
        Err(io::Error::new(io::ErrorKind::Unsupported,
            "filesystem did not confirm the published namespace; the action is not checkpointed"))
    }
}
