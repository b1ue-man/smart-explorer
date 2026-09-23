use super::{system_time_ms, EntryKind, LocalEntry};
use std::{io, path::Path};

pub(crate) fn parallel_scan_allowed() -> bool {
    true
}

pub(crate) fn read_directory(
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
        let metadata = entry.metadata().map_err(contextualize)?;
        let size = if kind == EntryKind::File {
            metadata.len()
        } else {
            0
        };
        Ok(LocalEntry {
            name: entry.file_name(),
            kind,
            is_dir: ty.is_dir(),
            is_link_like: ty.is_symlink(),
            size,
            unreachable: false,
            mtime_ms: metadata.modified().map(system_time_ms).unwrap_or(0),
            btime_ms: metadata.created().map(system_time_ms).unwrap_or(0),
            hidden: entry.file_name().to_string_lossy().starts_with('.'),
            system: false,
        })
    }))
}

pub(crate) fn normalize_scan_root(root: &Path) -> std::path::PathBuf {
    root.to_path_buf()
}

pub(crate) fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn can_request_access(_root: &str) -> bool {
    false
}
pub(crate) fn request_access(_root: &str) -> Result<bool, String> {
    Err("Zusätzliche Leserechte müssen unter Linux am Dateisystem gewährt werden".into())
}
pub(crate) fn run_helper_if_requested(args: &[std::ffi::OsString]) -> Option<Result<(), String>> {
    super::protocol::parse(args)
        .map(|_| Err("Der Windows-Lesehelfer ist hier nicht verfügbar".into()))
}
pub(crate) fn open_read(path: &Path) -> io::Result<std::fs::File> {
    std::fs::File::open(path)
}
pub(crate) fn symlink_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    std::fs::symlink_metadata(path)
}

pub(crate) fn metadata_is_link_like(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata.is_symlink()
}
