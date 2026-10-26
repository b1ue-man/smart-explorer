//! File actions of the surface in the desktop folder (DSK-M4): "Umbenennen"
//! and "Neuer Ordner" through the local VFS (atomic, never replacing, with
//! the platform's name rules), "In Papierkorb" through the system's recycle
//! bin (`trash`, as the sync engine recycles; never a permanent deletion)
//! and copying, moving, pasting and dropping through the copy engine
//! (`crate::copy`; a taken name gets a new one). Every name is one entry of
//! the desktop folder, so no action reaches outside it. The surface runs
//! these in a thread of its own; the shell's folder watch shows the result.
use std::path::{Path, PathBuf};

use crate::types::{Conflict, CopyMode};
use crate::vfs::{Backend, LocalBackend};

/// Name of a new folder; the next free one gets " (2)", " (3)", …
pub(crate) const NEW_FOLDER: &str = "Neuer Ordner";
const MAX_NEW_FOLDER_TRIES: u32 = 1_000;

/// Moves one path to the recycle bin.
pub(crate) type RecycleFn<'a> = dyn Fn(&Path) -> Result<(), String> + Send + Sync + 'a;

/// The system recycle bin (Windows `IFileOperation`, Linux XDG trash).
pub(crate) fn system_recycle(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|error| error.to_string())
}

/// One entry name of the desktop folder: not empty, no separator, not `.` or
/// `..`, no control character. The platform's own rules follow in the VFS.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Der Name ist leer".into());
    }
    let separator = name.contains('/') || name.contains('\\');
    if name == "." || name == ".." || separator || name.chars().any(char::is_control) {
        return Err(format!("„{name}“ ist kein gültiger Name"));
    }
    Ok(())
}

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn io_path(path: &Path) -> Result<String, String> {
    crate::copy::native_path_text(path).map_err(|error| error.to_string())
}

/// Renames `name` in `folder` to `new_name`; an existing entry is never
/// replaced.
pub(crate) fn rename(folder: &Path, name: &str, new_name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    check_name(new_name)?;
    let target = folder.join(new_name);
    if name == new_name {
        return Ok(target);
    }
    let source = folder.join(name);
    LocalBackend::new("/")
        .rename_no_replace(&io_path(&source)?, &io_path(&target)?)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => format!("„{new_name}“ gibt es schon"),
            _ => format!("„{name}“ nicht umbenannt: {error}"),
        })?;
    Ok(target)
}

/// "Neuer Ordner" in `folder` under the first free name.
pub(crate) fn new_folder(folder: &Path) -> Result<PathBuf, String> {
    let backend = LocalBackend::new("/");
    for attempt in 1..=MAX_NEW_FOLDER_TRIES {
        let name = if attempt == 1 {
            NEW_FOLDER.to_string()
        } else {
            format!("{NEW_FOLDER} ({attempt})")
        };
        let path = folder.join(name);
        match backend.create_dir_new(&io_path(&path)?) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    Err("Kein freier Name für einen neuen Ordner".into())
}

/// Moves entries of `folder` to the recycle bin, each on its own. A failure
/// never falls back to deleting; it is returned as `(name, error)`.
pub(crate) fn recycle(
    folder: &Path,
    names: &[String],
    recycle: &RecycleFn<'_>,
) -> Vec<(String, String)> {
    let mut failed = Vec::new();
    for name in names {
        let result = check_name(name).and_then(|()| recycle(&folder.join(name)));
        if let Err(error) = result {
            failed.push((name.clone(), error));
        }
    }
    failed
}

/// What a copy or move into the desktop folder reached.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TransferReport {
    pub(crate) files_done: u64,
    /// `(path, error)` of every entry that failed.
    pub(crate) errors: Vec<(String, String)>,
}

/// Copies or moves `sources` into `folder` with the copy engine: each source
/// keeps its name and its tree; a taken name gets a new one. Moving an entry
/// of `folder` into `folder` changes nothing and is left out. Blocks until
/// the engine reports its end.
pub(crate) fn transfer(sources: &[PathBuf], folder: &Path, mode: CopyMode) -> TransferReport {
    let (job, errors) = transfer_job(sources, folder, mode);
    let mut report = job.map(run_transfer_job).unwrap_or_default();
    report.errors.extend(errors);
    report
}

pub(crate) fn transfer_job(
    sources: &[PathBuf],
    folder: &Path,
    mode: CopyMode,
) -> (Option<crate::transfer::TransferJob>, Vec<(String, String)>) {
    let mut paths = Vec::new();
    let mut errors = Vec::new();
    for source in sources {
        let Some(parent) = source
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            errors.push((forward(source), "kein Eintrag eines Ordners".to_string()));
            continue;
        };
        if mode == CopyMode::Move && same_folder(parent, folder) {
            continue;
        }
        match io_path(source) {
            Ok(path) => paths.push(path),
            Err(error) => errors.push((forward(source), error)),
        }
    }
    if paths.is_empty() {
        return (None, errors);
    }
    let target_dir = match io_path(folder) {
        Ok(path) => path,
        Err(error) => {
            errors.push((forward(folder), error));
            return (None, errors);
        }
    };
    let job = crate::transfer::TransferJob {
        id: crate::transfer::new_job_id(),
        recoverable: true,
        recovery: crate::transfer::RecoveryMode::New,
        source: crate::transfer::Endpoint::Local,
        target: crate::transfer::Endpoint::Local,
        target_dir: target_dir.clone(),
        items: crate::transfer::JobItems::Roots { paths, base: None },
        layout: crate::transfer::Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        folders: crate::transfer::FolderConflictPolicy::new(
            crate::transfer::FolderConflict::KeepBoth,
        ),
        mode,
        source_label: "Desktop-Auswahl".to_string(),
        target_label: target_dir,
        resume: None,
    };
    if let Err(error) = job.validate() {
        errors.push((forward(folder), error));
        return (None, errors);
    }
    (Some(job), errors)
}

pub(crate) fn run_transfer_job(job: crate::transfer::TransferJob) -> TransferReport {
    let mut report = TransferReport::default();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let state = std::sync::Mutex::new(&mut report);
    crate::transfer::run_view(
        crate::transfer::JobView::of(&job),
        &|message| {
            if let crate::transfer::TransferMsg::Done {
                progress, issues, ..
            } = message
            {
                let mut state = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.files_done = progress.files_done;
                state
                    .errors
                    .extend(issues.into_iter().map(|issue| (issue.path, issue.message)));
            }
        },
        &cancel,
    );
    drop(state);
    report
}

fn same_folder(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
#[path = "desktop_ops_tests.rs"]
mod tests;
