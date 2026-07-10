use crossbeam_channel::Sender;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::super::model::{FolderIndex, IndexMsg, MAX_INDEX_DEPTH};
use super::super::platform::{is_plain_directory, should_skip_meta};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

pub(super) enum WalkStop {
    Canceled,
    ReceiverClosed,
    Failed(String),
}

pub(super) fn build_index(
    roots: Vec<PathBuf>,
    tx: &Sender<IndexMsg>,
    cancel: &AtomicBool,
) -> Result<FolderIndex, WalkStop> {
    if roots.is_empty() {
        return Err(WalkStop::Failed("folder-index roots are empty".to_string()));
    }

    let mut index = FolderIndex::new();
    let mut stack = Vec::new();
    let mut last_progress = Instant::now();
    for root in roots {
        check_canceled(cancel)?;
        require_plain_directory(&root, "index root")?;
        let normalized = normalized_path(&root)?;
        if insert_path(&mut index, normalized)? {
            stack.push((root, 0usize));
        }
    }

    while let Some((dir, depth)) = stack.pop() {
        check_canceled(cancel)?;
        require_plain_directory(&dir, "queued directory")?;
        emit_progress(tx, &index, &dir, &mut last_progress, false)?;

        let entries = std::fs::read_dir(&dir)
            .map_err(|error| WalkStop::Failed(format!("cannot read {}: {error}", dir.display())))?;
        for entry in entries {
            check_canceled(cancel)?;
            let entry = entry.map_err(|error| {
                WalkStop::Failed(format!("cannot enumerate {}: {error}", dir.display()))
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                WalkStop::Failed(format!("cannot inspect {}: {error}", path.display()))
            })?;
            if !is_plain_directory(&metadata) {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|_| {
                WalkStop::Failed(format!(
                    "directory name is not valid UTF-8 under {}",
                    dir.display()
                ))
            })?;
            if should_skip_meta(&name, &metadata) {
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
                stack.push((path, child_depth));
            }
            emit_progress(tx, &index, &dir, &mut last_progress, false)?;
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
