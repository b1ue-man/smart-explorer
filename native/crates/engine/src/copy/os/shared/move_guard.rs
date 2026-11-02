use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::platform;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SourceSnapshot {
    pub(super) identity: platform::FileIdentity,
    pub(super) len: u64,
    pub(super) modified: Option<SystemTime>,
}

pub(super) struct QuarantinedSource {
    pub(super) original: PathBuf,
    pub(super) path: PathBuf,
    pub(super) snapshot: SourceSnapshot,
    file: File,
    parent: crate::local_access::ConfinedDirectory,
    recovery_identity: Option<String>,
}

impl QuarantinedSource {
    pub(super) fn file(&self) -> &File {
        &self.file
    }
    pub(super) fn parent(&self) -> &crate::local_access::ConfinedDirectory {
        &self.parent
    }
}

pub(super) fn source_snapshot_path(path: &Path) -> io::Result<SourceSnapshot> {
    let file = crate::local_access::open_regular(path, crate::local_access::FinalLink::Refuse)?;
    source_snapshot_file(&file)
}

pub(super) fn source_snapshot_file(file: &File) -> io::Result<SourceSnapshot> {
    file_state(file).map(|(snapshot, _)| snapshot)
}

/// The snapshot of an open file and the metadata it was taken from: one
/// `fstat` serves both.
pub(super) fn file_state(file: &File) -> io::Result<(SourceSnapshot, std::fs::Metadata)> {
    let (identity, metadata) = platform::file_state(file)?;
    let snapshot = SourceSnapshot {
        identity,
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };
    Ok((snapshot, metadata))
}

/// What the entry at `path` is now, to be compared with the snapshot of the
/// file opened there: one `lstat` on Unix instead of opening the path again.
/// A link or special file at the path is refused.
pub(super) fn source_snapshot_entry(path: &Path) -> io::Result<SourceSnapshot> {
    let (identity, len, modified) = platform::path_state(path)?;
    Ok(SourceSnapshot {
        identity,
        len,
        modified,
    })
}

#[cfg(test)]
pub(super) fn quarantine_source(source: &Path) -> io::Result<QuarantinedSource> {
    quarantine_source_observed(source, None)
}

pub(super) fn quarantine_source_observed(
    source: &Path,
    observer: Option<&dyn super::publication::PublicationObserver>,
) -> io::Result<QuarantinedSource> {
    let parent_path = source
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let held_parent = crate::local_access::ConfinedDirectory::open_root(parent_path)?;
    let file = crate::local_access::open_regular(source, crate::local_access::FinalLink::Refuse)?;
    let snapshot = source_snapshot_file(&file)?;
    let source_name = source
        .file_name()
        .ok_or_else(|| source_changed_error(source))?;
    if !platform::same_file_object(&held_parent.entry(source_name)?, &file)? {
        return Err(source_changed_error(source));
    }
    let recovery_identity = match observer
        .map(|_| super::publication::local_file_identity_handle(&file))
        .transpose()
    {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => None,
        Err(error) => return Err(error),
    };
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "source".to_string());
    for attempt in 0..1000u32 {
        let tail = format!(
            ".smart-explorer-{:016x}.move",
            random_suffix(source, attempt)
        );
        let candidate = parent.join(crate::vfs::fit_stage_name(".", &name, &tail));
        if let Some(observer) = observer {
            observer.before_source_quarantine(source, &candidate)?;
        }
        match platform::move_file(source, &candidate, false) {
            Ok(()) => {
                let actual = match source_snapshot_path(&candidate) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return Err(rollback_quarantine(source, &candidate, error));
                    }
                };
                let full_identity_matches = match &recovery_identity {
                    Some(expected) => super::publication::local_file_identity(&candidate)
                        .is_ok_and(|actual| &actual == expected),
                    None => true,
                };
                let held_matches = match candidate.file_name().map(|name| {
                    held_parent
                        .entry(name)
                        .and_then(|current| platform::same_file_object(&current, &file))
                }) {
                    Some(Ok(matches)) => matches,
                    Some(Err(error)) => return Err(rollback_quarantine(source, &candidate, error)),
                    None => false,
                };
                if actual != snapshot || !full_identity_matches || !held_matches {
                    return Err(rollback_quarantine(
                        source,
                        &candidate,
                        source_changed_error(source),
                    ));
                }
                let secured = QuarantinedSource {
                    original: source.to_path_buf(),
                    path: candidate,
                    snapshot,
                    file,
                    parent: held_parent,
                    recovery_identity,
                };
                if let Err(error) = platform::sync_parent(source) {
                    return Err(restore_after_error(Some(&secured), error));
                }
                return Ok(secured);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique move quarantine",
    ))
}

fn rollback_quarantine(source: &Path, candidate: &Path, error: io::Error) -> io::Error {
    match platform::move_file(candidate, source, false) {
        Ok(()) => error,
        Err(rollback_error) => io::Error::new(
            error.kind(),
            format!(
                "{error}; quarantine rollback failed and data remains at {}: {rollback_error}",
                candidate.display()
            ),
        ),
    }
}

pub(super) fn restore_quarantine_if_any(source: Option<&QuarantinedSource>) -> io::Result<()> {
    if let Some(source) = source {
        restore_quarantine(source)?;
    }
    Ok(())
}

pub(super) fn restore_quarantine(source: &QuarantinedSource) -> io::Result<()> {
    if source_snapshot_path(&source.path)? != source.snapshot {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "secured source changed and was retained at {}",
                source.path.display()
            ),
        ));
    }
    if let Some(expected) = &source.recovery_identity {
        if super::publication::local_file_identity_handle(&source.file)? != *expected {
            return Err(source_changed_error(&source.path));
        }
    }
    super::recovery::recover_transfer_quarantine_live(
        &source.parent,
        &source.original,
        &source.path,
        &source.file,
        false,
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not restore secured source {} -> {}: {error}",
                source.path.display(),
                source.original.display()
            ),
        )
    })
}

pub(super) fn remove_quarantine(source: &QuarantinedSource) -> io::Result<()> {
    if source_snapshot_path(&source.path)? != source.snapshot {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secured source changed before removal",
        ));
    }
    if let Some(expected) = &source.recovery_identity {
        if super::publication::local_file_identity_handle(&source.file)? != *expected {
            return Err(source_changed_error(&source.path));
        }
    }
    super::recovery::recover_transfer_quarantine_live(
        &source.parent,
        &source.original,
        &source.path,
        &source.file,
        true,
    )
}

pub(super) fn restore_after_error(
    source: Option<&QuarantinedSource>,
    error: io::Error,
) -> io::Error {
    match restore_quarantine_if_any(source) {
        Ok(()) => error,
        Err(restore_error) => io::Error::new(
            error.kind(),
            format!("{error}; source restore also failed: {restore_error}"),
        ),
    }
}

pub(super) fn moved_target_changed_error(target: &Path, cause: Option<io::Error>) -> io::Error {
    let cause = cause.map(|error| format!(": {error}")).unwrap_or_default();
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "moved source could not be revalidated and was retained at {}{cause}",
            target.display()
        ),
    )
}

pub(super) fn source_changed_error(source: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("source changed during transfer: {}", source.display()),
    )
}

fn random_suffix(source: &Path, attempt: u32) -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    source.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    attempt.hash(&mut hasher);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    hasher.finish()
}
