//! Exclusive stages, byte-bound signatures and the publication boundary.
use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::paths::parent_of;
use super::transfer_stream::{check, stream, Streamed};
use super::types::{Sig, Throttle};
use crate::vfs::{Backend, StageDurability, StageFinish};
use std::io::{self, Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;

pub(super) use super::apply_stage_bytes::stage_bytes;

pub(crate) struct Staged<'a> {
    pub(super) backend: &'a dyn Backend,
    pub(crate) path: String,
    pub(crate) bytes: Streamed,
    pub(crate) source: Sig,
    pub(crate) durability: StageDurability,
    pub(super) published: bool,
    pub(super) replacement: Option<super::replacement_journal::Binding>,
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
    if !backend.is_local() {
        return Ok(true);
    }
    crate::vfs::confirm_namespace(
        backend,
        &parent_of(path).unwrap_or_else(|| path.to_string()),
    )
}

/// Private state paths are native PathBufs, not backend-relative literal paths.
pub(super) fn native_namespace(path: &std::path::Path) -> io::Result<bool> {
    let parent = path
        .parent()
        .and_then(|parent| parent.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "native namespace has no Unicode parent",
            )
        })?;
    let backend = crate::vfs::LocalBackend::new(parent);
    crate::vfs::confirm_namespace(&backend, parent)
}

/// A parent namespace confirmation cannot flush an unfinished stage's contents.
pub(super) fn flush_unconfirmed_stage(backend: &dyn Backend, path: &str) -> io::Result<()> {
    if crate::vfs::sync_filesystem(
        backend,
        &parent_of(path).unwrap_or_else(|| path.to_string()),
    )? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem did not confirm the staged file contents",
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stage<'a>(
    source: &dyn Backend,
    source_path: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    destination: &'a dyn Backend,
    destination_path: &str,
    destination_state: &CapturedFile,
    durability: StageDurability,
    throttle: &Throttle,
    cancel: &AtomicBool,
    progress: impl FnMut(u64),
) -> io::Result<Staged<'a>> {
    stage_as(
        source,
        source_path,
        captured,
        expected,
        destination,
        destination_path,
        destination_state,
        durability,
        throttle,
        cancel,
        progress,
        Digest::Needed,
    )
}

/// Whether anything consumes the MD5 of the copied bytes (§2.6 of the G4
/// analysis: checksum or provider-hash comparison, `verify`, reversible
/// replacement, version manifests). Without a consumer, two local sides
/// copy in the kernel (`copy_file_range`, reflink, `CopyFile2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Digest {
    Needed,
    Unused,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stage_as<'a>(
    source: &dyn Backend,
    source_path: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    destination: &'a dyn Backend,
    destination_path: &str,
    destination_state: &CapturedFile,
    durability: StageDurability,
    throttle: &Throttle,
    cancel: &AtomicBool,
    progress: impl FnMut(u64),
    digest: Digest,
) -> io::Result<Staged<'a>> {
    check(cancel)?;
    captured.regular("copy source")?;
    let parent = parent_of(destination_path);
    if let Some(parent) = &parent {
        super::apply_path_context::ensure_parent(destination, parent)?;
    }
    let result = stage_body(
        source,
        source_path,
        captured,
        expected,
        destination,
        destination_path,
        destination_state,
        durability,
        throttle,
        cancel,
        progress,
        digest,
    );
    if let (Err(error), Some(parent)) = (&result, &parent) {
        if error.kind() == io::ErrorKind::NotFound {
            // The parent may have vanished since this run created it.
            super::apply_path_context::forget_parent(destination, parent);
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn stage_body<'a>(
    source: &dyn Backend,
    source_path: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    destination: &'a dyn Backend,
    destination_path: &str,
    destination_state: &CapturedFile,
    durability: StageDurability,
    throttle: &Throttle,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
    digest: Digest,
) -> io::Result<Staged<'a>> {
    let metadata = captured.regular("copy source")?;
    let mut path = crate::vfs::unique_staging_path(destination, destination_path, "bisync")?;
    let authoritative = source.download_name(source_path, &metadata.name) == metadata.name;
    // A single non-duplex session cannot keep a reader and writer alive
    // together. Exports with unknown size need a measured upload length too.
    let spool = !authoritative
        || (source.namespace_identity() == destination.namespace_identity()
            && (!source.concurrent_read_write() || !destination.concurrent_read_write()));
    // Two local sides copy in the kernel; one connection copies on its
    // server (conflict siblings: SFTP copy-data, Share/SMB/WebDAV copies).
    let same_connection = std::ptr::addr_eq(source, destination);
    let kernel = digest == Digest::Unused
        && authoritative
        && ((source.is_local() && destination.is_local()) || same_connection)
        && destination_state.metadata.is_none()
        && expected.hash() == 0
        && metadata.content_md5.is_none()
        && throttle.unlimited();
    let mut kernel_copied = None;
    if kernel {
        match destination.server_copy_to_stage(source_path, &path, metadata.size, cancel) {
            Ok(copied) => kernel_copied = copied,
            Err(error) => {
                check(cancel)?;
                // A kernel copy is only a shortcut (as in the transfer
                // engine): any failure streams into a fresh stage instead,
                // which attributes a real problem to the side that has it.
                // A taken name was never this copy's own file.
                if error.kind() != io::ErrorKind::AlreadyExists {
                    let _ = destination.discard_copy_stage(&path);
                }
                path = crate::vfs::unique_staging_path(destination, destination_path, "bisync")?;
            }
        }
    }
    let mut staged = Staged {
        backend: destination,
        path,
        bytes: Streamed {
            bytes: 0,
            digest: [0; 16],
        },
        source: Sig {
            size: metadata.size,
            mtime_ms: metadata.mtime_ms,
            hash: 0,
        },
        durability,
        published: false,
        replacement: None,
    };
    let source_mode = crate::vfs::unix_mode(source, source_path)?;
    let destination_mode = if destination_state.metadata.is_some() {
        crate::vfs::unix_mode(destination, destination_path)?
    } else {
        None
    };
    let mode = match (source_mode, destination_mode) {
        (Some(source), Some(destination)) => Some(source & destination & 0o777),
        (Some(source), None) => Some(source & 0o777),
        (None, destination) => destination.map(|mode| mode & 0o777),
    };
    let transferred = if let Some(bytes) = kernel_copied {
        progress(bytes);
        // No consumer reads a digest of these bytes; size and identity are
        // proven by the kernel copy, `revalidate` and the stage capture.
        Streamed {
            bytes,
            digest: [0; 16],
        }
    } else if spool {
        let mut spool_file = tempfile::tempfile_in(crate::support_dirs::temp_dir())?;
        let result = {
            let mut reader =
                crate::vfs::open_read_regular(source, source_path, metadata.id.as_deref())?;
            stream(
                &mut *reader,
                &mut spool_file,
                cancel,
                Some(throttle),
                expected.hash(),
                &mut progress,
            )?
        };
        spool_file.flush()?;
        revalidate(source, source_path, captured, "copy source")?;
        spool_file.seek(SeekFrom::Start(0))?;
        super::apply_boundary::target(destination, destination_path, "data", Some(result.bytes))?;
        let mut writer = crate::vfs::open_write_copy_stage_timed(
            destination,
            &staged.path,
            result.bytes,
            metadata.mtime_ms,
        )?;
        let uploaded = stream(
            &mut spool_file,
            &mut *writer,
            cancel,
            None,
            result.hash(),
            |_| {},
        )?;
        writer.flush()?;
        drop(writer);
        if uploaded.bytes != result.bytes || uploaded.digest != result.digest {
            return Err(drift("spooled content changed while uploading"));
        }
        result
    } else {
        let mut reader =
            crate::vfs::open_read_regular(source, source_path, metadata.id.as_deref())?;
        let mut writer = crate::vfs::open_write_copy_stage_timed(
            destination,
            &staged.path,
            metadata.size,
            metadata.mtime_ms,
        )?;
        let result = stream(
            &mut *reader,
            &mut *writer,
            cancel,
            Some(throttle),
            expected.hash(),
            &mut progress,
        )?;
        writer.flush()?;
        drop(writer);
        drop(reader);
        result
    };
    if authoritative && transferred.bytes != metadata.size {
        return Err(drift("copy source size changed"));
    }
    if let Some(md5) = &metadata.content_md5 {
        if !md5.eq_ignore_ascii_case(&transferred.hex()) {
            return Err(drift("copy source digest changed"));
        }
    }
    revalidate(source, source_path, captured, "copy source")?;
    let state = capture(
        destination,
        &staged.path,
        ExpectedFile::Unknown,
        "copy stage",
    )?;
    let stage_meta = state.regular("copy stage")?;
    if stage_meta.size != transferred.bytes {
        return Err(drift("copy stage has the wrong size"));
    }
    if stage_meta
        .content_md5
        .as_ref()
        .is_some_and(|hash| !hash.eq_ignore_ascii_case(&transferred.hex()))
    {
        return Err(drift("copy stage has the wrong digest"));
    }
    let finished = crate::vfs::finish_stage(
        destination,
        &staged.path,
        StageFinish {
            mtime_ms: Some(metadata.mtime_ms),
            mode,
            durability,
        },
    )?;
    if destination.is_local() && durability == StageDurability::Now && !finished.durable {
        flush_unconfirmed_stage(destination, &staged.path)?;
    }
    staged.bytes = transferred;
    staged.source.hash = if kernel_copied.is_some() {
        0
    } else {
        transferred.hash()
    };
    Ok(staged)
}

impl Staged<'_> {
    pub(super) fn bind(
        &mut self,
        versions: &super::versions::RunVersions,
        side: &super::versions::VersionSide<'_>,
        rel: &str,
        checkpoint_allowed: bool,
    ) -> io::Result<()> {
        if side.backend.state_identity() != self.backend.state_identity() {
            return Err(drift("replacement binding belongs to another backend"));
        }
        self.replacement = Some(super::replacement_journal::Binding::new(
            versions,
            side,
            rel,
            checkpoint_allowed,
        )?);
        Ok(())
    }

    /// The stage's bytes were streamed and hashed here (not copied by the
    /// kernel or a server without a digest consumer).
    pub(super) fn digest_known(&self) -> bool {
        self.bytes.digest != [0; 16]
    }

    pub(super) fn require_backup(&mut self, signature: Sig) -> io::Result<()> {
        let binding = self
            .replacement
            .as_mut()
            .ok_or_else(|| drift("replacement backup has no binding"))?;
        binding.backup_signature = Some(signature);
        Ok(())
    }

    pub(super) fn publish(
        mut self,
        destination: &str,
        current: &CapturedFile,
        verify: bool,
        cancel: &AtomicBool,
    ) -> io::Result<CopyOutcome> {
        super::replacement_publish::publish(&mut self, destination, current, verify, cancel)
    }

    pub(super) fn publish_sibling(
        &mut self,
        candidate: &str,
        cancel: &AtomicBool,
    ) -> io::Result<CopyOutcome> {
        check(cancel)?;
        match self.backend.rename_no_replace(&self.path, candidate) {
            Ok(()) => self.published = true,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    self.published = true;
                }
                return Err(error);
            }
        }
        let captured = capture(
            self.backend,
            candidate,
            ExpectedFile::Unknown,
            "conflict copy",
        )?;
        let meta = captured.regular("conflict copy")?;
        if meta.size != self.bytes.bytes {
            return Err(drift("conflict copy has the wrong size"));
        }
        if self.digest_known()
            && meta
                .content_md5
                .as_ref()
                .is_some_and(|hash| !hash.eq_ignore_ascii_case(&self.bytes.hex()))
        {
            return Err(drift("conflict copy has the wrong digest"));
        }
        let durable = namespace(self.backend, candidate)?;
        Ok(CopyOutcome {
            bytes: self.bytes.bytes,
            digest: self.bytes.digest,
            source: self.source,
            destination: Sig {
                size: meta.size,
                mtime_ms: meta.mtime_ms,
                hash: if self.digest_known() {
                    self.bytes.hash()
                } else {
                    0
                },
            },
            durable,
        })
    }
}
impl Drop for Staged<'_> {
    fn drop(&mut self) {
        if !self.published {
            let _ = self.backend.discard_copy_stage(&self.path);
        }
    }
}

pub(super) fn require_durable(durable: bool) -> io::Result<()> {
    if durable {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem did not confirm the published namespace; the action is not checkpointed",
        ))
    }
}

#[cfg(test)]
#[path = "apply_stage_kernel_tests.rs"]
mod kernel_tests;
