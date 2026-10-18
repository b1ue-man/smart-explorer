//! File actions of the surface in the desktop folder (DSK-M4): "Umbenennen"
//! and "Neuer Ordner" through the local VFS (atomic, never replacing, with
//! the platform's name rules), "In Papierkorb" through the system's recycle
//! bin (`trash`, as the sync engine recycles; never a permanent deletion)
//! and copying, moving, pasting and dropping through the copy engine
//! (`crate::copy`; a taken name gets a new one). Every name is one entry of
//! the desktop folder, so no action reaches outside it. The surface runs
//! these in a thread of its own; the shell's folder watch shows the result.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::types::{Conflict, CopyMode, CopyOptions};
use crate::vfs::{Backend, LocalBackend};

/// Name of a new folder; the next free one gets " (2)", " (3)", …
pub(crate) const NEW_FOLDER: &str = "Neuer Ordner";
const MAX_NEW_FOLDER_TRIES: u32 = 1_000;
/// How long one wait for a report of the copy engine lasts.
const COPY_POLL: Duration = Duration::from_secs(1);

/// Moves one path to the recycle bin.
pub(crate) type RecycleFn = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

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
        .rename_no_replace(&forward(&source), &forward(&target))
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
        match backend.create_dir_new(&forward(&path)) {
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
    recycle: &RecycleFn,
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
    let mut report = TransferReport::default();
    let mut by_parent: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for source in sources {
        let Some(parent) = source
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            report
                .errors
                .push((forward(source), "kein Eintrag eines Ordners".into()));
            continue;
        };
        if mode == CopyMode::Move && same_folder(parent, folder) {
            continue;
        }
        by_parent
            .entry(parent.to_path_buf())
            .or_default()
            .push(forward(source));
    }
    for (parent, paths) in by_parent {
        let (sender, reports) = crossbeam_channel::unbounded();
        let options = CopyOptions {
            root: parent,
            dest: folder.to_path_buf(),
            preserve_structure: true,
            conflict: Conflict::Rename,
            mode,
        };
        let _job = crate::copy::start_copy_from_paths(paths, options, sender);
        loop {
            match reports.recv_timeout(COPY_POLL) {
                Ok(crate::copy::CopyMsg::Done { progress, errors }) => {
                    report.files_done += progress.files_done;
                    report.errors.extend(errors);
                    break;
                }
                Ok(crate::copy::CopyMsg::Progress(_))
                | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    report.errors.push((
                        forward(folder),
                        "Kopiervorgang ohne Abschlussmeldung beendet".into(),
                    ));
                    break;
                }
            }
        }
    }
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
