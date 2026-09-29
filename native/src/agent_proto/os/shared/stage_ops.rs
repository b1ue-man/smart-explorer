//! Single-request stage and directory operations of the transfer engine
//! (`stage-v1`) on the agent's local filesystem, under the same rules as the
//! older frames: exclusive creation, no replacement, links never followed.
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::promotion::{ensure_destination_parent_plain, validate_destination_root};

/// One kernel copy call of a server-side copy (`copy_file_range` where std
/// has it). Cancellation is checked between blocks, so a stop takes effect
/// after at most 8 MiB (below 0.1 s at 100 MB/s) at few calls per file.
const COPY_BLOCK: u64 = 8 * 1024 * 1024;

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
/// stage `stage`, created exclusively. A failed or canceled copy closes the
/// stage and removes it again: this call created it, nobody else can own
/// that name yet. A taken name is never touched. Without fsync: a new copy,
/// published later by a no-replace promotion.
pub(crate) fn copy_to_stage(
    src: &str,
    stage: &str,
    size: u64,
    cancel: &AtomicBool,
) -> io::Result<u64> {
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
    let target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage_path)?;
    let copied = fill_stage(&mut source, target, size, &before, cancel);
    if copied.is_err() {
        let _ = std::fs::remove_file(stage_path);
    }
    copied
}

/// Copy block by block into `target`, which is closed when this returns.
fn fill_stage(
    source: &mut std::fs::File,
    mut target: std::fs::File,
    size: u64,
    before: &std::fs::Metadata,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    super::local_platform::secure_staging_file(&target)?;
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Server-Kopie abgebrochen",
            ));
        }
        // One byte past `size` is enough to see a source that grew.
        let limit = size
            .saturating_add(1)
            .saturating_sub(copied)
            .min(COPY_BLOCK);
        let block = io::copy(&mut (&mut *source).take(limit), &mut target)?;
        if block == 0 {
            break;
        }
        copied = copied.saturating_add(block);
        if copied > size {
            return Err(changed());
        }
    }
    drop(target);
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

fn lower_hex(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `<file>.se-<purpose>-<16 hex>`: `vfs::unique_staging_path` and the
/// engine's copy stages (`<file>.se-upload-<16 hex>`).
fn is_unique_stage(name: &str) -> bool {
    let Some((head, suffix)) = name.rsplit_once('-') else {
        return false;
    };
    let Some(marker) = head.rfind(".se-") else {
        return false;
    };
    let purpose = &head[marker + ".se-".len()..];
    suffix.len() == 16
        && lower_hex(suffix)
        && marker > 0
        && !purpose.is_empty()
        && purpose
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `<file>.se-agent-batch-<16 hex>-<hex>.part`: this agent's batch stages.
fn is_batch_stage(name: &str) -> bool {
    let Some(rest) = name.strip_suffix(".part") else {
        return false;
    };
    let Some((head, attempt)) = rest.rsplit_once('-') else {
        return false;
    };
    let Some((head, nonce)) = head.rsplit_once('-') else {
        return false;
    };
    let Some(file) = head.strip_suffix(".se-agent-batch") else {
        return false;
    };
    !file.is_empty()
        && nonce.len() == 16
        && lower_hex(nonce)
        && attempt.len() <= 8
        && lower_hex(attempt)
}

/// Whether `name` is exactly a stage name the transfer engine or this agent
/// generates; nothing else may be discarded, so no caller can delete user
/// files through `DiscardStage`.
pub(crate) fn is_discardable_stage(name: &str) -> bool {
    is_unique_stage(name) || is_batch_stage(name)
}

/// Remove an unpublished stage: only a regular file with a generated stage
/// name.
pub(crate) fn discard_stage(stage: &str) -> io::Result<()> {
    let path = Path::new(stage);
    let is_stage = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_discardable_stage);
    if !is_stage {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Keine Übertragungsstufe: {stage}"),
        ));
    }
    regular_file(path, "Übertragungsstufe")?;
    std::fs::remove_file(path)
}
