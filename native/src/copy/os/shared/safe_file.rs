use crate::types::{Conflict, CopyMode};
use std::collections::hash_map::RandomState;
use std::fs::{File, OpenOptions};
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::platform;
use super::durability::{finish_direct_move, finish_staged_commit};
#[cfg(test)]
use super::move_guard::restore_quarantine;
use super::move_guard::{
    moved_target_changed_error, quarantine_source, remove_quarantine, restore_after_error,
    restore_quarantine_if_any, source_changed_error, source_snapshot_file, source_snapshot_path,
    SourceSnapshot,
};
use super::path_guard::prepare_target_parent;

const COPY_CHUNK: usize = 1024 * 1024;
const MAX_RENAME_RACE_RETRIES: usize = 1000;

#[derive(Debug)]
pub(super) enum TransferResult {
    Completed(u64),
    Skipped,
    Canceled,
}

struct CopiedSource {
    bytes: u64,
    snapshot: SourceSnapshot,
}

pub(super) fn transfer_file(
    src: &Path,
    target: &Path,
    destination_root: &Path,
    conflict: Conflict,
    mode: CopyMode,
    cancel: &AtomicBool,
) -> io::Result<TransferResult> {
    let source_metadata = std::fs::symlink_metadata(src)?;
    if platform::metadata_is_link_like(&source_metadata) || !source_metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("source is not a regular, non-link file: {}", src.display()),
        ));
    }
    let requested_target = target.to_path_buf();
    let Some(mut target) = select_initial_target(src, &requested_target, conflict)? else {
        return Ok(TransferResult::Skipped);
    };
    let mut rename_race_retries = 0usize;
    prepare_target_parent(destination_root, &target)?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(TransferResult::Canceled);
    }

    // A move first relocates the source to an unpredictable no-replace sibling.
    // From this point onward no check-then-unlink operation ever targets the
    // user-visible source path; failures restore this quarantine when possible.
    let mut quarantine = if mode == CopyMode::Move {
        Some(quarantine_source(src)?)
    } else {
        None
    };
    if cancel.load(Ordering::Relaxed) {
        restore_quarantine_if_any(quarantine.as_ref())?;
        return Ok(TransferResult::Canceled);
    }
    let source_path = quarantine
        .as_ref()
        .map(|source| source.path.clone())
        .unwrap_or_else(|| src.to_path_buf());

    if mode == CopyMode::Move {
        loop {
            match platform::move_file(&source_path, &target, conflict == Conflict::Overwrite) {
                Ok(()) => {
                    let source = quarantine.take().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "move quarantine is missing")
                    })?;
                    match source_snapshot_path(&target) {
                        Ok(snapshot) if snapshot == source.snapshot => {}
                        Ok(_) => return Err(moved_target_changed_error(&target, None)),
                        Err(error) => return Err(moved_target_changed_error(&target, Some(error))),
                    }
                    // POSIX rename may be a no-op when destination is a hard
                    // link to the same inode. Remove only our verified sibling.
                    if metadata_if_exists(&source.path)?.is_some() {
                        remove_quarantine(&source)?;
                    }
                    finish_direct_move(&target, &source, platform::sync_parent)?;
                    return Ok(TransferResult::Completed(source.snapshot.len));
                }
                Err(error) if platform::is_cross_device(&error) => break,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match conflict {
                    Conflict::Skip => {
                        restore_quarantine_if_any(quarantine.as_ref())?;
                        return Ok(TransferResult::Skipped);
                    }
                    Conflict::Rename => {
                        rename_race_retries = rename_race_retries.saturating_add(1);
                        if rename_race_retries > MAX_RENAME_RACE_RETRIES {
                            return Err(restore_after_error(
                                quarantine.as_ref(),
                                rename_race_exhausted(),
                            ));
                        }
                        target = match unique_path(&requested_target) {
                            Ok(target) => target,
                            Err(error) => {
                                return Err(restore_after_error(quarantine.as_ref(), error));
                            }
                        };
                        if let Err(error) = prepare_target_parent(destination_root, &target) {
                            return Err(restore_after_error(quarantine.as_ref(), error));
                        }
                    }
                    Conflict::Overwrite => {
                        return Err(restore_after_error(quarantine.as_ref(), error));
                    }
                },
                Err(error) => return Err(restore_after_error(quarantine.as_ref(), error)),
            }
        }
    }

    let (temp, mut staged) = match create_temp_sibling(&target) {
        Ok(staged) => staged,
        Err(error) => return Err(restore_after_error(quarantine.as_ref(), error)),
    };
    let copied = match copy_to_staged(&source_path, &mut staged, cancel) {
        Ok(Some(copied)) => copied,
        Ok(None) => {
            drop(staged);
            let _ = std::fs::remove_file(&temp);
            restore_quarantine_if_any(quarantine.as_ref())?;
            return Ok(TransferResult::Canceled);
        }
        Err(error) => {
            drop(staged);
            let _ = std::fs::remove_file(&temp);
            return Err(restore_after_error(quarantine.as_ref(), error));
        }
    };
    if cancel.load(Ordering::Relaxed) {
        drop(staged);
        let _ = std::fs::remove_file(&temp);
        restore_quarantine_if_any(quarantine.as_ref())?;
        return Ok(TransferResult::Canceled);
    }
    if !platform::path_matches_identity(&temp, platform::file_identity(&staged)?)? {
        drop(staged);
        let _ = std::fs::remove_file(&temp);
        return Err(restore_after_error(
            quarantine.as_ref(),
            io::Error::new(
                io::ErrorKind::InvalidData,
                "staged copy path changed before commit",
            ),
        ));
    }
    if let Some(source) = &quarantine {
        if copied.snapshot != source.snapshot {
            drop(staged);
            let _ = std::fs::remove_file(&temp);
            return Err(restore_after_error(
                quarantine.as_ref(),
                source_changed_error(&source.path),
            ));
        }
    }

    loop {
        match platform::commit_staged(&temp, &target, conflict == Conflict::Overwrite) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match conflict {
                Conflict::Skip => {
                    drop(staged);
                    let _ = std::fs::remove_file(&temp);
                    restore_quarantine_if_any(quarantine.as_ref())?;
                    return Ok(TransferResult::Skipped);
                }
                Conflict::Rename => {
                    rename_race_retries = rename_race_retries.saturating_add(1);
                    if rename_race_retries > MAX_RENAME_RACE_RETRIES {
                        let error = clean_staged(temp, staged, rename_race_exhausted());
                        return Err(restore_after_error(quarantine.as_ref(), error));
                    }
                    target = match unique_path(&requested_target) {
                        Ok(target) => target,
                        Err(error) => {
                            let error = clean_staged(temp, staged, error);
                            return Err(restore_after_error(quarantine.as_ref(), error));
                        }
                    };
                    if let Err(error) = prepare_target_parent(destination_root, &target) {
                        let error = clean_staged(temp, staged, error);
                        return Err(restore_after_error(quarantine.as_ref(), error));
                    }
                }
                Conflict::Overwrite => {
                    let error = clean_staged(temp, staged, error);
                    return Err(restore_after_error(quarantine.as_ref(), error));
                }
            },
            Err(error) => {
                let error = clean_staged(temp, staged, error);
                return Err(restore_after_error(quarantine.as_ref(), error));
            }
        }
    }
    drop(staged);
    finish_staged_commit(&target, src, &mut quarantine, platform::sync_parent)?;
    Ok(TransferResult::Completed(copied.bytes))
}

fn select_initial_target(
    src: &Path,
    target: &Path,
    conflict: Conflict,
) -> io::Result<Option<PathBuf>> {
    match metadata_if_exists(target)? {
        Some(metadata) => {
            if conflict == Conflict::Skip {
                return Ok(None);
            }
            if metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("destination is a directory: {}", target.display()),
                ));
            }
            if platform::metadata_is_link_like(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "destination is a link or reparse point: {}",
                        target.display()
                    ),
                ));
            }
            match conflict {
                Conflict::Skip => Ok(None),
                Conflict::Rename => unique_path(target).map(Some),
                Conflict::Overwrite => {
                    if platform::same_file(src, target)? {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "source and destination are the same file",
                        ))
                    } else {
                        Ok(Some(target.to_path_buf()))
                    }
                }
            }
        }
        None => Ok(Some(target.to_path_buf())),
    }
}

fn copy_to_staged(
    src: &Path,
    writer: &mut File,
    cancel: &AtomicBool,
) -> io::Result<Option<CopiedSource>> {
    let link_metadata = std::fs::symlink_metadata(src)?;
    if platform::metadata_is_link_like(&link_metadata) || !link_metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source became a link or non-regular file",
        ));
    }
    let mut reader = File::open(src)?;
    let before = source_snapshot_file(&reader)?;
    if source_snapshot_path(src)? != before {
        return Err(source_changed_error(src));
    }
    let source_permissions = reader.metadata()?.permissions();
    let mut buffer = vec![0u8; COPY_CHUNK];
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
    }
    let after = source_snapshot_file(&reader)?;
    if after != before || source_snapshot_path(src)? != before {
        return Err(source_changed_error(src));
    }
    writer.flush()?;
    writer.set_permissions(source_permissions)?;
    writer.sync_all()?;
    Ok(Some(CopiedSource {
        bytes: copied,
        snapshot: before,
    }))
}

fn create_temp_sibling(target: &Path) -> io::Result<(PathBuf, File)> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "copy".to_string());
    for attempt in 0..1000u32 {
        let candidate = parent.join(format!(
            ".{name}.smart-explorer-{:016x}.part",
            random_suffix(target, attempt)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
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

fn unique_path(target: &Path) -> io::Result<PathBuf> {
    if metadata_if_exists(target)?.is_none() {
        return Ok(target.to_path_buf());
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = target
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    for index in 2..=100_000u32 {
        let candidate = parent.join(format!("{stem} ({index}){ext}"));
        if metadata_if_exists(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique conflict name",
    ))
}

fn metadata_if_exists(path: &Path) -> io::Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn rename_race_exhausted() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "destination names kept changing during conflict-safe rename",
    )
}

fn clean_staged(temp: PathBuf, staged: File, error: io::Error) -> io::Error {
    drop(staged);
    let _ = std::fs::remove_file(temp);
    error
}

#[cfg(test)]
#[path = "safe_file_tests.rs"]
mod tests;
