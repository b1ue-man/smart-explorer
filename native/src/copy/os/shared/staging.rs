//! The private stage of one local copy: the source is opened and observed,
//! its bytes go into an exclusively created, unpredictable sibling of the
//! destination through the kernel (K12), and the source must be unchanged
//! afterwards. New copies are not synced to disk file by file (like Explorer
//! and cp); moves and replacements are.
use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::platform;
use super::move_guard::{
    source_changed_error, source_snapshot_file, source_snapshot_path, SourceSnapshot,
};

/// Unpredictable stage names collide only with a foreign file of that exact
/// name; the bound is caution, as in the older copy code.
const STAGE_ATTEMPTS: u32 = 1000;

/// Which side of a local copy failed.
#[derive(Debug)]
pub(crate) enum LocalFailure {
    /// Reading the source (nothing was published).
    Source(io::Error),
    /// Creating or writing the stage (nothing was published).
    Target(io::Error),
    /// Publishing (including examining the one entry it would replace) or
    /// finishing a published copy: this file's failure, never the target's.
    Publish(io::Error),
}

impl LocalFailure {
    pub(crate) fn into_error(self) -> io::Error {
        match self {
            LocalFailure::Source(error)
            | LocalFailure::Target(error)
            | LocalFailure::Publish(error) => error,
        }
    }
}

/// A complete, private copy of the source.
pub(super) struct Staged {
    pub(super) temp: PathBuf,
    pub(super) file: File,
    pub(super) bytes: u64,
    pub(super) snapshot: SourceSnapshot,
}

impl Staged {
    pub(super) fn discard(self) {
        drop(self.file);
        let _ = std::fs::remove_file(&self.temp);
    }
}

/// Copies `source` into a new stage next to `target`; `None` when canceled.
/// `durable` (moves, replacements) syncs the stage to disk.
pub(super) fn stage_copy(
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    durable: bool,
    preserve_destination_mode: bool,
    progress: &mut dyn FnMut(u64),
) -> Result<Option<Staged>, LocalFailure> {
    let (reader, before) = open_source(source)?;
    if !durable {
        if let Some(staged) = stage_by_path(source, target, &reader, &before, cancel, progress)? {
            return Ok(staged);
        }
    }
    let (temp, mut writer) = create_temp_sibling(target).map_err(LocalFailure::Target)?;
    let copied = match platform::copy_handles(&reader, &mut writer, cancel, progress) {
        Ok(Some(copied)) => copied,
        Ok(None) => {
            discard(temp, writer);
            return Ok(None);
        }
        Err(error) => {
            discard(temp, writer);
            return Err(side_of(error));
        }
    };
    let finished = unchanged(source, &reader, &before)
        .and_then(|()| {
            reader
                .metadata()
                .and_then(|metadata| {
                    platform::copy_permissions(
                        &writer,
                        &metadata,
                        preserve_destination_mode.then_some(target),
                    )
                })
                .map_err(LocalFailure::Target)
        })
        .and_then(|()| {
            if durable {
                writer.sync_all().map_err(LocalFailure::Target)
            } else {
                Ok(())
            }
        });
    if let Err(failure) = finished {
        discard(temp, writer);
        return Err(failure);
    }
    Ok(Some(Staged {
        temp,
        file: writer,
        bytes: copied,
        snapshot: before,
    }))
}

/// Opens the source of a copy: a regular file (never a link, junction or
/// special file), observed so a change during the copy is noticed.
pub(super) fn open_source(source: &Path) -> Result<(File, SourceSnapshot), LocalFailure> {
    let link_metadata =
        crate::local_access::symlink_metadata(source).map_err(LocalFailure::Source)?;
    if crate::local_access::metadata_is_link_like(source, &link_metadata)
        || !link_metadata.is_file()
    {
        return Err(LocalFailure::Source(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{}: keine reguläre Datei (Links, Reparse-Punkte und Spezialdateien werden nicht übertragen)",
                source.display()
            ),
        )));
    }
    // Opened without following a link and without waiting on a FIFO that
    // replaced the file after the check above.
    let reader = crate::local_access::open_regular(source, crate::local_access::FinalLink::Refuse)
        .map_err(LocalFailure::Source)?;
    let before = source_snapshot_file(&reader).map_err(LocalFailure::Source)?;
    if source_snapshot_path(source).map_err(LocalFailure::Source)? != before {
        return Err(LocalFailure::Source(source_changed_error(source)));
    }
    Ok((reader, before))
}

/// Platforms that copy by path in the kernel (Windows `CopyFile2`) do it
/// onto a fresh name that must not exist; `Ok(None)` = use the handles.
fn stage_by_path(
    source: &Path,
    target: &Path,
    reader: &File,
    before: &SourceSnapshot,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<Option<Option<Staged>>, LocalFailure> {
    for attempt in 0..STAGE_ATTEMPTS {
        let temp = stage_name(target, attempt);
        match platform::copy_by_path(source, &temp, cancel, progress) {
            None => return Ok(None),
            Some(Ok(None)) => return Ok(Some(None)),
            Some(Ok(Some((bytes, file)))) => {
                if bytes != before.len || unchanged(source, reader, before).is_err() {
                    discard(temp, file);
                    return Err(LocalFailure::Source(source_changed_error(source)));
                }
                return Ok(Some(Some(Staged {
                    temp,
                    file,
                    bytes,
                    snapshot: before.clone(),
                })));
            }
            Some(Err(error)) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Some(Err(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
                ) =>
            {
                return Err(LocalFailure::Target(error));
            }
            // Protected sources (readable only through `local_access`) and
            // file systems without the kernel path use the handles.
            Some(Err(_)) => return Ok(None),
        }
    }
    Ok(None)
}

pub(super) fn unchanged(
    source: &Path,
    reader: &File,
    before: &SourceSnapshot,
) -> Result<(), LocalFailure> {
    let after = source_snapshot_file(reader).map_err(LocalFailure::Source)?;
    if after != *before || source_snapshot_path(source).map_err(LocalFailure::Source)? != *before {
        return Err(LocalFailure::Source(source_changed_error(source)));
    }
    Ok(())
}

/// Errors of a combined read/write copy: the ones only a destination can
/// cause belong to the target (a full or read-only volume ends the job), the
/// rest to the source (a refused read may still be granted).
fn side_of(error: io::Error) -> LocalFailure {
    match error.kind() {
        io::ErrorKind::StorageFull
        | io::ErrorKind::QuotaExceeded
        | io::ErrorKind::ReadOnlyFilesystem
        | io::ErrorKind::FileTooLarge
        | io::ErrorKind::WriteZero => LocalFailure::Target(error),
        _ => LocalFailure::Source(error),
    }
}

fn discard(temp: PathBuf, file: File) {
    drop(file);
    let _ = std::fs::remove_file(temp);
}

fn stage_name(target: &Path, attempt: u32) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "copy".to_string());
    let tail = format!(
        ".smart-explorer-{:016x}.part",
        random_suffix(target, attempt)
    );
    parent.join(crate::vfs::fit_stage_name(".", &name, &tail))
}

pub(super) fn create_temp_sibling(target: &Path) -> io::Result<(PathBuf, File)> {
    for attempt in 0..STAGE_ATTEMPTS {
        let candidate = stage_name(target, attempt);
        match crate::vfs::create_local_copy_stage(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique staged-copy name",
    ))
}

fn random_suffix(target: &Path, attempt: u32) -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    target.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    attempt.hash(&mut hasher);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    hasher.finish()
}
