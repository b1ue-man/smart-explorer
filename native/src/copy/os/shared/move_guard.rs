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
}

pub(super) fn source_snapshot_path(path: &Path) -> io::Result<SourceSnapshot> {
    let file = File::open(path)?;
    source_snapshot_file(&file)
}

pub(super) fn source_snapshot_file(file: &File) -> io::Result<SourceSnapshot> {
    let metadata = file.metadata()?;
    Ok(SourceSnapshot {
        identity: platform::file_identity(file)?,
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

pub(super) fn quarantine_source(source: &Path) -> io::Result<QuarantinedSource> {
    let snapshot = source_snapshot_path(source)?;
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "source".to_string());
    for attempt in 0..1000u32 {
        let candidate = parent.join(format!(
            ".{name}.smart-explorer-{:016x}.move",
            random_suffix(source, attempt)
        ));
        match platform::move_file(source, &candidate, false) {
            Ok(()) => {
                let actual = match source_snapshot_path(&candidate) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return Err(rollback_quarantine(source, &candidate, error));
                    }
                };
                if actual != snapshot {
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
    platform::move_file(&source.path, &source.original, false).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not restore secured source {} -> {}: {error}",
                source.path.display(),
                source.original.display()
            ),
        )
    })?;
    platform::sync_parent(&source.original)
}

pub(super) fn remove_quarantine(source: &QuarantinedSource) -> io::Result<()> {
    if source_snapshot_path(&source.path)? != source.snapshot {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secured source changed before removal",
        ));
    }
    std::fs::remove_file(&source.path)
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
