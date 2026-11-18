use super::{system_time_ms, EntryError, EntryKind, FinalLink, LocalEntry, NotRegular};
use std::{io, path::Path};

#[path = "linux/directory_handle.rs"]
mod directory_handle;
pub use directory_handle::{DirectoryHandle, QuarantinedChild};
#[path = "linux/confined.rs"]
mod confined;
pub use confined::ConfinedDirectory;

pub fn parallel_scan_allowed() -> bool {
    true
}

/// Lists `path` in inode order. The kind comes from the directory entry (`d_type`, no extra
/// call where the file system supplies it); one `lstat` per entry supplies
/// sizes and times (on Android's shared storage it is answered from the
/// attribute cache that readdirplus filled).
///
/// A failing entry is reported as `<entry path>: <error>` with the original
/// kind. The storage analysis relies on that prefix to tell a hidden
/// `Android/data` or `Android/obb` entry from a real read error.
pub fn read_directory(path: &Path) -> io::Result<impl Iterator<Item = io::Result<LocalEntry>>> {
    use std::os::unix::fs::DirEntryExt;
    // Entries in inode order before their one `lstat` each (coreutils `cp`,
    // gnulib savedir SAVEDIR_SORT_FASTREAD): a cold tree is then read close
    // to disk order instead of the directory's hash order.
    let mut entries: Vec<io::Result<std::fs::DirEntry>> = std::fs::read_dir(path)?.collect();
    entries.sort_by_key(|entry| entry.as_ref().map_or(0, |entry| entry.ino()));
    Ok(entries.into_iter().map(|entry| {
        let entry = entry?;
        let contextualize =
            |error: io::Error| EntryError::wrap(entry.file_name(), &entry.path(), error);
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
        let (allocated, hardlink) = occupancy(kind, &metadata);
        Ok(LocalEntry {
            name,
            kind,
            is_dir: ty.is_dir(),
            is_link_like: ty.is_symlink(),
            size,
            allocated,
            hardlink,
            device: Some(std::os::unix::fs::MetadataExt::dev(&metadata)),
            unreachable: false,
            mtime_ms: metadata.modified().map(system_time_ms).unwrap_or(0),
            btime_ms: metadata.created().map(system_time_ms).unwrap_or(0),
            hidden,
            system: false,
        })
    }))
}

/// Occupied bytes (`st_blocks` counts 512-byte units on Linux and Android,
/// independent of the file system block size) and, for a file with more than
/// one hard link, its `(st_dev, st_ino)` identity. Only regular files carry
/// bytes, like `size`.
pub(super) fn occupancy(
    kind: EntryKind,
    metadata: &std::fs::Metadata,
) -> (Option<u64>, Option<(u64, u64)>) {
    use std::os::unix::fs::MetadataExt;
    if kind != EntryKind::File {
        return (Some(0), None);
    }
    let hardlink = (metadata.nlink() > 1).then(|| (metadata.dev(), metadata.ino()));
    (Some(metadata.blocks().saturating_mul(512)), hardlink)
}

/// The file system (`st_dev`) of `path`, following a link at the path.
pub fn device_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .map(|metadata| std::os::unix::fs::MetadataExt::dev(&metadata))
}

/// Unix keeps no per-scan state.
pub fn begin_scan() {}

pub fn normalize_scan_root(root: &Path) -> std::path::PathBuf {
    root.to_path_buf()
}

pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn can_request_access(_root: &str) -> bool {
    false
}
#[cfg(not(target_os = "android"))]
pub fn request_access(_root: &str) -> Result<bool, String> {
    Err("Zusätzliche Leserechte müssen unter Linux am Dateisystem gewährt werden".into())
}
#[cfg(target_os = "android")]
pub fn request_access(_root: &str) -> Result<bool, String> {
    Err("Zusätzliche Leserechte müssen unter Android in den Einstellungen als „Zugriff auf alle Dateien“ gewährt werden".into())
}
pub fn run_helper_if_requested(args: &[std::ffi::OsString]) -> Option<Result<(), String>> {
    super::protocol::parse(args)
        .map(|_| Err("Der Windows-Lesehelfer ist hier nicht verfügbar".into()))
}
pub fn open_read(path: &Path) -> io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Checks the type before opening (opening a device can have side effects),
/// opens without blocking (`O_NONBLOCK`; `O_NOFOLLOW` when links are
/// refused), re-checks the opened file and clears `O_NONBLOCK` again.
pub fn open_regular(path: &Path, final_link: FinalLink) -> io::Result<std::fs::File> {
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

pub fn symlink_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    std::fs::symlink_metadata(path)
}

pub fn metadata_is_link_like(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata.is_symlink()
}

/// Everything that is neither file, folder nor symlink (FIFO, socket, block or
/// character device) is special; it has no data stream to copy.
pub fn metadata_class(_path: &Path, metadata: &std::fs::Metadata) -> super::MetadataClass {
    let kind = metadata.file_type();
    super::MetadataClass {
        link_like: kind.is_symlink(),
        special: !(kind.is_file() || kind.is_dir() || kind.is_symlink()),
    }
}

pub fn classify_open_file(file: &std::fs::File) -> io::Result<super::MetadataClass> {
    Ok(metadata_class(Path::new(""), &file.metadata()?))
}

pub fn secure_private_handle(file: &std::fs::File, is_directory: bool) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = file.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || (is_directory && !metadata.is_dir())
        || (!is_directory && (!metadata.is_file() || metadata.nlink() != 1))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "object is not privately owned",
        ));
    }
    let mode = if is_directory { 0o700 } else { 0o600 };
    if metadata.mode() & 0o7777 != mode {
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    let metadata = file.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != mode
        || (!is_directory && metadata.nlink() != 1)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "provider did not retain private mode",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod belegt_tests {
    use super::*;

    /// A sparse file occupies fewer bytes than its size; a second hard link
    /// carries the shared `(st_dev, st_ino)` identity on both names.
    #[test]
    fn belegt_task_linux_blocks_and_hardlink_identity() {
        let dir = tempfile::tempdir().expect("temp dir");
        let sparse = dir.path().join("sparse");
        let file = std::fs::File::create(&sparse).expect("create");
        file.set_len(64 * 1024 * 1024).expect("extend");
        drop(file);
        let single = dir.path().join("single");
        std::fs::write(&single, vec![1u8; 10_000]).expect("write");
        std::fs::hard_link(&single, dir.path().join("twin")).expect("hard link");
        let entries: Vec<_> = read_directory(dir.path())
            .expect("list")
            .collect::<io::Result<_>>()
            .expect("entries");
        let find = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .expect("entry")
        };
        let sparse = find("sparse");
        assert_eq!(sparse.size, 64 * 1024 * 1024);
        assert!(sparse.allocated.expect("known") < sparse.size);
        assert_eq!(sparse.hardlink, None);
        let (single, twin) = (find("single"), find("twin"));
        assert!(single.allocated.expect("known") >= 10_000);
        assert!(single.hardlink.is_some());
        assert_eq!(single.hardlink, twin.hardlink);
    }

    /// The size listing of an opened folder: one `fstatat` per entry,
    /// folders without one (their file system from the opened child), the
    /// same sizes as the full listing.
    #[test]
    fn analyse_task_size_listing_stats_relative_and_skips_folders() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("f.bin"), vec![3u8; 5000]).expect("write");
        std::fs::create_dir(dir.path().join("sub")).expect("dir");
        let handle = DirectoryHandle::open_root(dir.path()).expect("root");
        let quick: Vec<_> = handle
            .read_directory_for_sizes()
            .expect("list")
            .collect::<io::Result<_>>()
            .expect("entries");
        let full: Vec<_> = handle
            .read_directory()
            .expect("list")
            .collect::<io::Result<_>>()
            .expect("entries");
        let find = |entries: &[LocalEntry], name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| {
                    (
                        entry.kind,
                        entry.size,
                        entry.allocated,
                        entry.device,
                        entry.mtime_ms,
                    )
                })
                .expect("entry")
        };
        assert_eq!(find(&quick, "f.bin"), find(&full, "f.bin"));
        let (kind, _, _, device, _) = find(&quick, "sub");
        assert_eq!(kind, EntryKind::Directory);
        assert_eq!(device, None);
        let root_device = device_of(dir.path());
        assert!(root_device.is_some());
        let child = handle
            .open_child(std::ffi::OsStr::new("sub"))
            .expect("child");
        assert_eq!(child.device_id(), root_device);
    }
}
