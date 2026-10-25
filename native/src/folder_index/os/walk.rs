use crossbeam_channel::Sender;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::super::model::{FolderIndex, IndexMsg, MAX_INDEX_DEPTH};
use super::super::platform::{is_plain_directory, should_skip_meta, skip_unreadable_directory};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

pub(super) enum WalkStop {
    Canceled,
    ReceiverClosed,
    Failed(String),
}

pub(super) fn build_index_excluding(roots: Vec<PathBuf>, tx: &Sender<IndexMsg>, cancel: &AtomicBool,
    excluded: &[PathBuf]) -> Result<FolderIndex, WalkStop> {
    if roots.is_empty() {
        return Err(WalkStop::Failed("folder-index roots are empty".to_string()));
    }

    let mut index = FolderIndex::new();
    let mut stack = Vec::new();
    let mut media = Vec::new();
    let mut last_progress = Instant::now();
    for root in roots {
        if excluded.iter().any(|excluded| root.starts_with(excluded)) { continue; }
        check_canceled(cancel)?;
        let _guard = media_guard(&root, cancel)?;
        require_plain_directory(&root, "index root")?;
        if let Some(guard) = &_guard { media.push((root.clone(), guard.source().clone())); }
        let normalized = normalized_path(&root)?;
        if insert_path(&mut index, normalized)? {
            let source = _guard.as_ref().map(|guard| guard.source().clone());
            stack.push((root.clone(), 0usize, root, source));
        }
    }

    while let Some((dir, depth, root, source)) = stack.pop() {
        if excluded.iter().any(|excluded| dir.starts_with(excluded)) { continue; }
        check_canceled(cancel)?;
        if let Some(source) = &source {
            if crate::source_media::probe(&root).map_err(|error| WalkStop::Failed(error.to_string()))?.as_ref() != Some(source) {
                return Err(WalkStop::Failed(format!("optical medium changed at {}", root.display())));
            }
        }
        emit_progress(tx, &index, &dir, &mut last_progress, false)?;

        let entries = match crate::source_media::GuardedIterator::new(&dir,
            crate::source_media::Priority::Background, cancel, || {
                require_plain_directory(&dir, "queued directory").map_err(|stop| match stop {
                    WalkStop::Failed(error) => io::Error::other(error),
                    _ => io::Error::new(io::ErrorKind::Interrupted, "index canceled"),
                })?;
                std::fs::read_dir(&dir)
            }) {
            Ok(entries) => entries,
            Err(error) if depth > 0 && skip_unreadable_directory(&error) => continue,
            Err(error) => {
                return Err(WalkStop::Failed(format!(
                    "cannot read {}: {error}",
                    dir.display()
                )))
            }
        };
        for entry in entries {
            check_canceled(cancel)?;
            let entry = entry.map_err(|error| {
                if cancel.load(Ordering::Acquire) { return WalkStop::Canceled; }
                WalkStop::Failed(format!("cannot enumerate {}: {error}", dir.display()))
            })?;
            let path = entry.path();
            let _guard = media_guard(&dir, cancel)?;
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if skip_unreadable_directory(&error) => continue,
                Err(error) => {
                    return Err(WalkStop::Failed(format!(
                        "cannot inspect {}: {error}",
                        path.display()
                    )))
                }
            };
            if !is_plain_directory(&metadata) {
                continue;
            }
            drop(_guard);
            let name = entry.file_name().into_string().map_err(|_| {
                WalkStop::Failed(format!(
                    "directory name is not valid UTF-8 under {}",
                    dir.display()
                ))
            })?;
            // The active app trash (Android) is a protected omission.
            if should_skip_meta(&name, &metadata) || crate::apptrash::excluded_name(&name) {
                continue;
            }
            let child_depth = depth.checked_add(1).ok_or_else(|| {
                WalkStop::Failed("folder-index depth counter overflowed".to_string())
            })?;
            if child_depth > MAX_INDEX_DEPTH {
                return Err(WalkStop::Failed(format!(
                    "folder index exceeds maximum depth {MAX_INDEX_DEPTH} at {}",
                    path.display()
                )));
            }
            let normalized = normalized_path(&path)?;
            if insert_path(&mut index, normalized)? {
                stack.push((path, child_depth, root.clone(), source.clone()));
            }
            emit_progress(tx, &index, &dir, &mut last_progress, false)?;
        }
    }

    for (root, expected) in media {
        if crate::source_media::probe(&root).map_err(|error| WalkStop::Failed(error.to_string()))?.as_ref() != Some(&expected) {
            return Err(WalkStop::Failed(format!("optical medium changed at {}", root.display())));
        }
    }
    emit_progress(tx, &index, Path::new(""), &mut last_progress, true)?;
    Ok(index)
}

fn require_plain_directory(path: &Path, context: &str) -> Result<(), WalkStop> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        WalkStop::Failed(format!(
            "cannot inspect {context} {}: {error}",
            path.display()
        ))
    })?;
    if !is_plain_directory(&metadata) {
        return Err(WalkStop::Failed(format!(
            "{context} is not a plain directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn normalized_path(path: &Path) -> Result<String, WalkStop> {
    let path = path
        .to_str()
        .ok_or_else(|| WalkStop::Failed(format!("path is not valid UTF-8: {}", path.display())))?;
    Ok(path.replace('\\', "/"))
}

fn insert_path(index: &mut FolderIndex, path: String) -> Result<bool, WalkStop> {
    index
        .try_insert(path)
        .map_err(|error: io::Error| WalkStop::Failed(error.to_string()))
}

fn check_canceled(cancel: &AtomicBool) -> Result<(), WalkStop> {
    if cancel.load(Ordering::Acquire) {
        Err(WalkStop::Canceled)
    } else {
        Ok(())
    }
}

fn emit_progress(
    tx: &Sender<IndexMsg>,
    index: &FolderIndex,
    current: &Path,
    last_progress: &mut Instant,
    force: bool,
) -> Result<(), WalkStop> {
    if !force && last_progress.elapsed() < PROGRESS_INTERVAL {
        return Ok(());
    }
    *last_progress = Instant::now();
    tx.send(IndexMsg::Progress {
        count: index.len() as u64,
        current: current.to_string_lossy().into_owned(),
    })
    .map_err(|_| WalkStop::ReceiverClosed)
}

fn media_guard(path: &Path, cancel: &AtomicBool) -> Result<Option<crate::source_media::Guard>, WalkStop> {
    crate::source_media::acquire(path, crate::source_media::Priority::Background, cancel)
        .map_err(|error| if cancel.load(Ordering::Acquire) { WalkStop::Canceled }
            else { WalkStop::Failed(format!("cannot read {}: {error}", path.display())) })
}
