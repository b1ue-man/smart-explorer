use super::{EntryKind, LocalEntry};
use std::{io, path::Path};

pub(super) fn parallel_scan_allowed() -> bool {
    true
}

pub(super) fn can_request_elevation(_root: &str) -> bool {
    false
}
pub(super) fn launch_elevated_analysis(_root: &str) -> Result<bool, String> {
    Err("Administrator-Analyse ist nur unter Windows verfügbar".into())
}
pub(super) fn verify_analysis_startup(
    _request: &crate::analytics::AnalysisStartup,
) -> Result<(), String> {
    Err("Administrator-Analyse ist nur unter Windows verfügbar".into())
}

pub(super) fn read_directory(
    path: &Path,
) -> io::Result<impl Iterator<Item = io::Result<LocalEntry>>> {
    Ok(std::fs::read_dir(path)?.map(|entry| {
        let entry = entry?;
        let contextualize = |error: io::Error| {
            io::Error::new(error.kind(), format!("{}: {error}", entry.path().display()))
        };
        let ty = entry.file_type().map_err(contextualize)?;
        let kind = if ty.is_symlink() {
            EntryKind::Link
        } else if ty.is_dir() {
            EntryKind::Directory
        } else if ty.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let size = if kind == EntryKind::File {
            entry.metadata().map_err(contextualize)?.len()
        } else {
            0
        };
        Ok(LocalEntry {
            name: entry.file_name(),
            kind,
            size,
        })
    }))
}
