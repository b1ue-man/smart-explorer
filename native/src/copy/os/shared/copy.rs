//! Local copy and move entry points (copy dialog, clipboard, drag & drop).
//! Each runs as one streaming engine job: copying starts with the first
//! listing, files are copied in parallel per volume with kernel copies, and
//! a move on one volume renames whole folders.
use crate::transfer::{JobItems, Layout};
use crate::types::{CopyOptions, CopyProgress, FileEntry};
use crossbeam_channel::Sender;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::platform;

#[path = "bridge.rs"]
mod bridge;
#[path = "durability.rs"]
mod durability;
#[path = "move_guard.rs"]
mod move_guard;
#[path = "pairs.rs"]
mod pairs;
#[path = "path_guard.rs"]
mod path_guard;
#[path = "planning.rs"]
mod planning;
#[path = "prune.rs"]
mod prune;
#[path = "safe_file.rs"]
mod safe_file;
#[path = "staging.rs"]
mod staging;

pub use pairs::start_copy_pairs;
pub(crate) use path_guard::validate_directory_target;
pub(crate) use prune::prune_empty_dirs;
pub(crate) use safe_file::{transfer_local, LocalOutcome, LocalRequest};
pub(crate) use staging::LocalFailure;

use bridge::LocalJob;
use planning::{dedupe_entries, dedupe_paths};

pub enum CopyMsg {
    Progress(CopyProgress),
    Done {
        progress: CopyProgress,
        errors: Vec<(String, String)>,
    },
}

pub struct CopyHandle {
    pub cancel: Arc<AtomicBool>,
}

/// Moves a whole folder in one no-replace rename (same volume, destination
/// name free). Both parents are synced afterwards like every move; a failed
/// sync of an already renamed folder is not reported as a failed move.
pub(crate) fn move_folder(
    source: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<()> {
    platform::move_file(source, target, false)?;
    let _ = platform::sync_parent(target);
    let _ = platform::sync_parent(source);
    Ok(())
}

/// Forward-slash text of a local path (Windows: the normalized long form).
fn path_text(path: &std::path::Path) -> Result<String, (String, String)> {
    platform::path_text(path).map_err(|error| (path.display().to_string(), error.to_string()))
}

/// Where the selected entries land: below `root` when the structure is kept,
/// else every file directly in the target folder.
fn roots_of(
    paths: Vec<String>,
    opts: &CopyOptions,
) -> Result<(JobItems, Layout), (String, String)> {
    if opts.preserve_structure {
        let base = path_text(&opts.root)?;
        let base = base.trim_end_matches('/').to_string();
        Ok((
            JobItems::Roots {
                paths,
                base: Some(base),
            },
            Layout::Tree,
        ))
    } else {
        Ok((JobItems::Roots { paths, base: None }, Layout::Flatten))
    }
}

/// Copy selected entries. Folders are walked on the worker while copying
/// starts, so the UI never waits for a large subtree. `filter` (with its
/// root prefix) applies below selected folders; selected files always pass.
pub fn start_copy_expanded(
    seeds: Vec<FileEntry>,
    filter: Option<(crate::types::FilterDef, String)>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    bridge::spawn(tx, move || {
        let paths = dedupe_entries(seeds)
            .into_iter()
            .map(|entry| entry.path.to_string())
            .collect();
        let (items, layout) = roots_of(paths, &opts)?;
        Ok(LocalJob {
            items,
            layout,
            filter,
            target_dir: path_text(&opts.dest)?,
            conflict: opts.conflict,
            mode: opts.mode,
        })
    })
}

/// Copy/move raw clipboard paths into a destination (stats and walks them
/// on the worker thread).
pub fn start_copy_from_paths(
    paths: Vec<String>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    bridge::spawn(tx, move || {
        let paths = dedupe_paths(paths)
            .into_iter()
            .map(|path| path_text(std::path::Path::new(&path)))
            .collect::<Result<Vec<_>, _>>()?;
        let (items, layout) = roots_of(paths, &opts)?;
        Ok(LocalJob {
            items,
            layout,
            filter: None,
            target_dir: path_text(&opts.dest)?,
            conflict: opts.conflict,
            mode: opts.mode,
        })
    })
}
