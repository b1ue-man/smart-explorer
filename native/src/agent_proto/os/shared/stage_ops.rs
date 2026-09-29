//! Single-request stage and directory operations of the transfer engine
//! (`stage-v1`) on the agent's local filesystem, under the same rules as the
//! older frames: exclusive creation, no replacement, links never followed.
use std::io;
use std::path::Path;

use super::promotion::{ensure_destination_parent_plain, validate_destination_root};

/// Every transfer stage name carries this marker (`vfs::unique_staging_path`
/// names `<file>.se-<purpose>-<hex>`, agent stages `<file>.se-agent-…`).
/// Discarding refuses any other name, so no caller can delete user files
/// through it.
const STAGE_MARKER: &str = ".se-";

fn regular_file(path: &Path, what: &str) -> io::Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || super::local_platform::metadata_is_link_like(path, &metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} ist keine reguläre Datei: {}", path.display()),
        ));
    }
    Ok(metadata)
}

/// Server-side copy of `src` (expected length `size`) into the new private
/// stage `stage`. The stage is created exclusively; on failure it is left
/// for the client's `DiscardStage`, like any exclusively created entry.
/// Without fsync: a new copy, published later by a no-replace promotion.
pub(crate) fn copy_to_stage(src: &str, stage: &str, size: u64) -> io::Result<u64> {
    let source_path = Path::new(src);
    regular_file(source_path, "Kopierquelle")?;
    let mut source = std::fs::File::open(source_path)?;
    let before = source.metadata()?;
    let identity = super::local_platform::file_identity(&source)?;
    if !super::local_platform::path_matches_identity(source_path, identity)? {
        return Err(changed());
    }
    if before.len() != size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Kopierquelle hat {} statt {size} Bytes: {src}",
                before.len()
            ),
        ));
    }
    let stage_path = Path::new(stage);
    ensure_destination_parent_plain(stage_path)?;
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage_path)?;
    super::local_platform::secure_staging_file(&target)?;
    let copied = io::copy(&mut source, &mut target)?;
    let after = source.metadata()?;
    if copied != size || after.len() != size || after.modified().ok() != before.modified().ok() {
        return Err(changed());
    }
    Ok(copied)
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Kopierquelle wurde während der Kopie geändert",
    )
}

/// Create one directory whose parent exists. An existing real directory is
/// success unless `exclusive`; any other existing entry is a conflict.
pub(crate) fn create_dir_one(path: &str, exclusive: bool) -> io::Result<()> {
    let path = Path::new(path);
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ordner hat keinen Elternordner",
        )
    })?;
    validate_destination_root(parent)?;
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && !exclusive => {
            let metadata = std::fs::symlink_metadata(path)?;
            if metadata.is_dir() && !super::local_platform::metadata_is_link_like(path, &metadata) {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "Name ist durch einen Eintrag belegt, der kein Ordner ist: {}",
                        path.display()
                    ),
                ))
            }
        }
        Err(error) => Err(error),
    }
}

/// Remove an unpublished stage; only regular files whose name carries the
/// stage marker.
pub(crate) fn discard_stage(stage: &str) -> io::Result<()> {
    let path = Path::new(stage);
    let is_stage = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(STAGE_MARKER));
    if !is_stage {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Keine Übertragungsstufe: {stage}"),
        ));
    }
    regular_file(path, "Übertragungsstufe")?;
    std::fs::remove_file(path)
}
