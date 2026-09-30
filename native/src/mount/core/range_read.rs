//! Large read-only files need no whole-file working copy on the local disk.
use super::engine::{baseline_from_meta, read_lock, require_regular, MountEngine, OpenHandle, OpenHandleKind};
use super::types::MountMode;
use std::io::{self, Read};

impl MountEngine {
    pub(super) fn uses_range_reads(&self, size: u64) -> bool {
        // RW handles retain the established pinned whole-file snapshot, including
        // paging writes and old handles surviving a delete-sharing replacement.
        self.config.mode == MountMode::ReadOnly
            && size > self.config.cache.retained_bytes().max(1024 * 1024)
    }

    pub(super) fn read_range(
        &self,
        opened: &OpenHandle,
        offset: u64,
        output: &mut [u8],
    ) -> io::Result<Option<usize>> {
        let OpenHandleKind::Metadata { callback_path, meta } = &opened.kind else {
            return Ok(None);
        };
        if opened.writable || !self.uses_range_reads(meta.size) { return Ok(None); }
        let _namespace = read_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        if self.entry_for_path(path.backend())?.is_some() {
            // A recovered dirty/conflicted copy always takes precedence over
            // the remote, including an absent recovery payload.
            return Ok(None);
        }
        let current = self.backend.stat(path.backend())?;
        require_regular(&current)?;
        let expected = baseline_from_meta(meta);
        if baseline_from_meta(&current) != expected { return Err(changed()); }
        let length = current.size.saturating_sub(offset).min(output.len() as u64) as usize;
        if length == 0 { return Ok(Some(0)); }
        let Some(mut reader) = self.backend.open_read_at(path.backend(), current.id.as_deref(), offset)? else {
            // Preserve support for backends that cannot address a byte offset.
            return Ok(None);
        };
        // read_exact retries Interrupted and rejects a premature end, so a
        // short network read cannot become false EOF in the Windows callback.
        reader.read_exact(&mut output[..length])?;
        drop(reader); // Cancel unread frames and release the request gate before stat.
        let after = self.backend.stat(path.backend())?;
        require_regular(&after)?;
        if baseline_from_meta(&after) != expected { return Err(changed()); }
        Ok(Some(length))
    }
}

fn changed() -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock,
        "remote file changed during mounted range read; reopen it before continuing")
}
