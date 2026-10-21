//! A stage several writers fill at once (`SegmentedStage`) for SMB. smb2
//! 0.26.0 writes at an offset of a handle it opens by path
//! (`Tree::create_file_writer_at`, smb2 `client/stream.rs:1102`, `FileOpenIf`
//! without truncation), so one range is one handle with its own WRITE
//! pipeline; the stage itself is created exclusively once.
//!
//! The ranges reopen the stage by path, and `FileOpenIf` would even create a
//! missing name: the same question as for SFTP (S7), and an SMB client has no
//! way to see who owns the folders above. Until the security agent has
//! answered it for SMB the switch below is off and `open_segmented_stage`
//! answers `None`: uploads stream over one handle as before.
use super::backend::SmbBackend;
use super::streams::{Commit, SmbWriter};
use super::url::entry_path;
use crate::vfs::{Backend, SegmentedStage, VfsResult};
use std::io::{self, Write};

/// Reopening a stage by path is not yet judged safe for SMB: off.
pub(super) const SMB_SEGMENTED_WRITE: bool = false;

/// The stage of one upload: `size` bytes in `stage`.
struct SmbSegmentedStage {
    backend: SmbBackend,
    stage: String,
    size: u64,
}

impl SmbBackend {
    /// Creates `stage` exclusively when ranges are allowed; `Ok(None)` when
    /// not (the file streams).
    pub(super) fn segmented_stage(
        &self,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
        if !SMB_SEGMENTED_WRITE {
            return Ok(None);
        }
        let mut created = self.write_file(stage, true, Some(0), Commit::Unsynced)?;
        created.flush()?;
        Ok(Some(Box::new(SmbSegmentedStage {
            backend: self.clone(),
            stage: stage.to_string(),
            size,
        })))
    }

    /// Writer for `[offset, offset + len)` of the existing `stage`: its own
    /// handle, `flush` fails unless exactly `len` bytes came.
    fn range_writer(&self, stage: &str, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        let target = entry_path(stage)?;
        let (generation, tree) = self.session.target(&target.share)?;
        let opened = self.session.block_on(tree.create_file_writer_at(
            generation.connection(),
            &target.rel,
            offset,
        ));
        let writer = opened.map_err(|error| {
            generation.note(&error);
            super::errors::map(error, "Bereich öffnen", stage)
        })?;
        let writer = SmbWriter::new(
            writer,
            &target.rel,
            stage,
            tree,
            generation,
            self.session.runtime(),
            Some(len),
        );
        Ok(Box::new(writer.with_commit(Commit::Unsynced)))
    }
}

impl SegmentedStage for SmbSegmentedStage {
    fn stage_path(&self) -> &str {
        &self.stage
    }

    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        if offset.checked_add(len).is_none_or(|end| end > self.size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Bereich {offset}+{len} liegt hinter dem Ende der Stufe"),
            ));
        }
        self.backend.range_writer(&self.stage, offset, len)
    }

    fn finish(self: Box<Self>) -> VfsResult<()> {
        let size = self.backend.stat(&self.stage)?.size;
        if size != self.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Stufe „{}“ hat {size} statt {} Bytes: die Bereiche sind unvollständig",
                    self.stage, self.size
                ),
            ));
        }
        Ok(())
    }

    fn abort(self: Box<Self>) {
        let _ = self.backend.discard_copy_stage(&self.stage);
    }
}
