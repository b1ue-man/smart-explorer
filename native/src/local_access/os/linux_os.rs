use super::{system_time_ms, EntryKind, FinalLink, LocalEntry, NotRegular};
use std::{io, path::Path};

pub(crate) fn parallel_scan_allowed() -> bool {
    true
}

/// Lists `path`. The kind comes from the directory entry (`d_type`, no extra
/// call where the file system supplies it); one `lstat` per entry supplies
/// sizes and times (on Android's shared storage it is answered from the
/// attribute cache that readdirplus filled).
///
/// A failing entry is reported as `<entry path>: <error>` with the original
/// kind. The storage analysis relies on that prefix to tell a hidden
/// `Android/data` or `Android/obb` entry from a real read error.
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
        let name = entry.file_name();
        let hidden = name.as_encoded_bytes().first() == Some(&b'.');
        Ok(LocalEntry {
            name,
            kind,
            is_dir: ty.is_dir(),
            is_link_like: ty.is_symlink(),
            size,
            unreachable: false,
            mtime_ms: metadata.modified().map(system_time_ms).unwrap_or(0),
            btime_ms: metadata.created().map(system_time_ms).unwrap_or(0),
            hidden,
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
#[cfg(not(target_os = "android"))]
pub(crate) fn request_access(_root: &str) -> Result<bool, String> {
    Err("Zusätzliche Leserechte müssen unter Linux am Dateisystem gewährt werden".into())
}
#[cfg(target_os = "android")]
pub(crate) fn request_access(_root: &str) -> Result<bool, String> {
    Err("Zusätzliche Leserechte müssen unter Android in den Einstellungen als „Zugriff auf alle Dateien“ gewährt werden".into())
}
pub(crate) fn run_helper_if_requested(args: &[std::ffi::OsString]) -> Option<Result<(), String>> {
    super::protocol::parse(args)
        .map(|_| Err("Der Windows-Lesehelfer ist hier nicht verfügbar".into()))
}
pub(crate) fn open_read(path: &Path) -> io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Checks the type before opening (opening a device can have side effects),
/// opens without blocking (`O_NONBLOCK`; `O_NOFOLLOW` when links are
/// refused), re-checks the opened file and clears `O_NONBLOCK` again.
pub(crate) fn open_regular(path: &Path, final_link: FinalLink) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let refuse_links = final_link == FinalLink::Refuse;
    let before = if refuse_links {
        std::fs::symlink_metadata(path)?
    } else {
        std::fs::metadata(path)?
    };
    regular_or_refusal(before.file_type())?;
    let mut flags = libc::O_NONBLOCK | libc::O_NOCTTY;
    if refuse_links {
        flags |= libc::O_NOFOLLOW;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(libc::ELOOP) if refuse_links => NotRegular::Link.error(),
            Some(libc::ENXIO) => NotRegular::Special.error(),
            _ => error,
        })?;
    regular_or_refusal(file.metadata()?.file_type())?;
    let descriptor = file.as_raw_fd();
    // SAFETY: `descriptor` belongs to `file`, which stays open for both calls.
    let status = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if status < 0 {
        return Err(io::Error::last_os_error());
    }
    let blocking = status & !libc::O_NONBLOCK;
    // SAFETY: as above; only the status flags of this descriptor change.
    if blocking != status && unsafe { libc::fcntl(descriptor, libc::F_SETFL, blocking) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

fn regular_or_refusal(kind: std::fs::FileType) -> io::Result<()> {
    if kind.is_file() {
        Ok(())
    } else if kind.is_symlink() {
        Err(NotRegular::Link.error())
    } else if kind.is_dir() {
        Err(NotRegular::Directory.error())
    } else {
        Err(NotRegular::Special.error())
    }
}

pub(crate) fn symlink_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    std::fs::symlink_metadata(path)
}

pub(crate) fn metadata_is_link_like(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata.is_symlink()
}

/// Everything that is neither file, folder nor symlink (FIFO, socket, block or
/// character device) is special; it has no data stream to copy.
pub(crate) fn metadata_class(_path: &Path, metadata: &std::fs::Metadata) -> super::MetadataClass {
    let kind = metadata.file_type();
    super::MetadataClass {
        link_like: kind.is_symlink(),
        special: !(kind.is_file() || kind.is_dir() || kind.is_symlink()),
    }
}
