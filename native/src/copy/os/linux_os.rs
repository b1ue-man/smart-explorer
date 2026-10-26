use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub(super) type FileIdentity = (u64, u64);
#[path = "linux_recovery.rs"]
mod linux_recovery;
pub(super) use linux_recovery::{recover_transfer_child, recover_transfer_child_live};
#[path = "linux_file_proof.rs"]
mod linux_file_proof;
pub(super) use linux_file_proof::stable_handle_identity;

pub(super) fn stable_path_identity(path: &Path) -> io::Result<String> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "file identity requires a child name",
        )
    })?;
    let parent_path = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = crate::local_access::ConfinedDirectory::open_root(parent_path)?;
    let selected = parent.selected_entry(name)?;
    let identity = stable_handle_identity(&selected)?;
    if stable_handle_identity(&parent.selected_entry(name)?)? != identity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file changed during identity query",
        ));
    }
    Ok(identity)
}

pub(super) fn open_publication_file(
    parent: &crate::local_access::ConfinedDirectory,
    name: &std::ffi::OsStr,
) -> io::Result<File> {
    parent.open_file(name, false, false, false, false)
}

pub(super) fn open_move_publication_file(
    parent: &crate::local_access::ConfinedDirectory,
    name: &std::ffi::OsStr,
) -> io::Result<File> {
    open_publication_file(parent, name)
}

pub(super) fn confirm_publication_namespace(
    parent: &crate::local_access::ConfinedDirectory,
) -> io::Result<()> {
    parent.file().sync_all()
}

pub(super) fn confirm_live_publication_namespace(
    parent: &crate::local_access::ConfinedDirectory,
    _target: &Path,
) -> io::Result<()> {
    confirm_publication_namespace(parent)
}

pub(super) fn hard_link(source: &Path, target: &Path) -> io::Result<()> {
    std::fs::hard_link(source, target)
}

#[path = "linux_direct.rs"]
mod linux_direct;
#[path = "linux_direct_plan.rs"]
mod linux_direct_plan;
#[path = "linux_probe.rs"]
mod linux_probe;
#[path = "linux_range.rs"]
mod linux_range;

/// A large copy around the page cache onto a drive not known to be flash:
/// measured direct requests (`linux_probe`) instead of `copy_handles`' fixed
/// thread count.
pub(super) use linux_probe::copy_probed;

/// Which file `path` names, from its metadata: no read access needed (an
/// unreadable destination can still be compared and replaced).
pub(super) fn path_identity(path: &Path) -> io::Result<FileIdentity> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

pub(super) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

pub(super) fn same_file_object(left: &File, right: &File) -> io::Result<bool> {
    Ok(file_identity(left)? == file_identity(right)?)
}

/// A pre-dispatch boundary observation, never an interpretation of a failed
/// remote rename. Different devices cannot be one rename mount; statx also
/// detects separate bind mounts of the same underlying device when available.
pub(super) fn different_rename_boundary(source: &File, target_parent: &File) -> io::Result<bool> {
    if source.metadata()?.dev() != target_parent.metadata()?.dev() {
        return Ok(true);
    }
    let source_mount = rename_mount(source)?;
    let target_mount = rename_mount(target_parent)?;
    Ok(matches!((source_mount, target_mount), (Some(left), Some(right)) if left != right))
}

fn rename_mount(file: &File) -> io::Result<Option<u64>> {
    use std::os::unix::io::AsRawFd;
    #[repr(C, align(8))]
    struct Statx([u8; 256]);
    let mut facts = Statx([0; 256]);
    // Linux's fixed statx UAPI places stx_mnt_id at byte144. Only a returned
    // STATX_MNT_ID mask constitutes a mount fact on kernels supporting it.
    let status = unsafe {
        libc::syscall(
            libc::SYS_statx,
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            0x1000u32,
            facts.0.as_mut_ptr(),
        )
    };
    if status != 0 {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP | libc::EPERM | libc::EACCES)
        ) {
            return Ok(None);
        }
        return Err(error);
    }
    let mask = u32::from_ne_bytes(
        facts.0[..4]
            .try_into()
            .map_err(|_| io::Error::other("incomplete statx mask"))?,
    );
    if mask & 0x1000 == 0 {
        return Ok(None);
    }
    let mount = u64::from_ne_bytes(
        facts.0[144..152]
            .try_into()
            .map_err(|_| io::Error::other("incomplete statx mount ID"))?,
    );
    Ok(Some(mount))
}

pub(super) fn require_local_publication_observation(parent: &File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let mut facts = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(parent.as_raw_fd(), facts.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let kind = unsafe { facts.assume_init() }.f_type as u64 & 0xffff_ffff;
    // Reviewed local kernel filesystems. NFS/FUSE/network and unqualified
    // stacks must retain both objects after an ambiguous mutation error.
    if matches!(
        kind,
        0xef53
            | 0x5846_5342
            | 0x9123_683e
            | 0x0102_1994
            | 0x4d44
            | 0x2011_bab0
            | 0xf2f5_2010
            | 0x5346_544e
    ) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem cannot establish nonpublication from local namespace observations",
        ))
    }
}

/// An open file's identity together with the metadata it was read from: one
/// `fstat` for both (the snapshot, the permissions to copy).
pub(super) fn file_state(file: &File) -> io::Result<(FileIdentity, std::fs::Metadata)> {
    let metadata = file.metadata()?;
    Ok(((metadata.dev(), metadata.ino()), metadata))
}

/// Identity, length and modification time of what `path` names now, from one
/// `lstat`: a link, folder or special file at the path is refused (and cannot
/// equal an opened regular file's identity anyway). Compared with the
/// snapshot of the handle after a copy; no second open is needed.
pub(super) fn path_state(
    path: &Path,
) -> io::Result<(FileIdentity, u64, Option<std::time::SystemTime>)> {
    let metadata = std::fs::symlink_metadata(path)?;
    let kind = metadata.file_type();
    if !kind.is_file() {
        let refusal = if kind.is_symlink() {
            crate::local_access::NotRegular::Link
        } else if kind.is_dir() {
            crate::local_access::NotRegular::Directory
        } else {
            crate::local_access::NotRegular::Special
        };
        return Err(refusal.error());
    }
    Ok((
        (metadata.dev(), metadata.ino()),
        metadata.len(),
        metadata.modified().ok(),
    ))
}

pub(super) fn path_matches_identity(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    let actual = match path_identity(path) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(actual == expected)
}

/// Final ordinary permissions, never set-id bits; replacing a private
/// destination cannot make its contents more readable than before.
pub(super) fn copy_permissions(
    writer: &File,
    source: &std::fs::Metadata,
    destination: Option<&Path>,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut mode = source.permissions().mode() & 0o777;
    if let Some(destination) = destination {
        match std::fs::symlink_metadata(destination) {
            Ok(metadata) if metadata.is_file() => mode &= metadata.permissions().mode(),
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "destination is not a regular file",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    match writer.set_permissions(std::fs::Permissions::from_mode(mode)) {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::Unsupported
                    | io::ErrorKind::InvalidInput
                    | io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(())
        }
        result => result,
    }
}

pub(super) fn path_text(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not valid Unicode"))
}

pub(super) fn path_key(path: &Path) -> io::Result<String> {
    path_text(path)
}

pub(super) fn commit_staged(staged: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        return crate::vfs::replace_local_file(staged, dest);
    }
    crate::copy::publication_proof::preflight_no_replace(dest)?;
    rename_no_replace(staged, dest)
}

pub(super) fn move_file(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        return crate::vfs::replace_local_file(src, dest);
    }
    crate::copy::publication_proof::preflight_no_replace(dest)?;
    rename_no_replace(src, dest)
}

pub(super) fn is_cross_device(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices || error.raw_os_error() == Some(18)
}

pub(super) fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

/// Bytes per kernel copy call: cancel and progress are seen at least every
/// ~80 ms on a 100 MB/s disk, while one syscall per 8 MiB costs nothing.
const KERNEL_CHUNK: u64 = 8 * 1024 * 1024;

/// No copy by path here: the stage is created exclusively and filled
/// through the handles (`copy_handles`).
pub(super) fn copy_by_path(
    _source: &Path,
    _stage: &Path,
    _cancel: &std::sync::atomic::AtomicBool,
    _progress: &mut dyn FnMut(u64),
) -> Option<io::Result<Option<(u64, File)>>> {
    None
}

/// Copies `reader` to the end into the exclusively created stage `writer`
/// in the kernel (`std::io::copy` between two files uses copy_file_range,
/// which reflinks on btrfs/xfs and copies server-side on NFS). With
/// `parallel > 1` and a file of at least two shortest transfer parts, that
/// many threads each copy one contiguous range at explicit offsets
/// (`linux_range`); the engine asks for it with permits to spare on a drive
/// known to be flash, or after a measured probe on the same device pair found
/// that two threads help (`copy_probed`). A file of at least a quarter of the
/// available memory on block devices goes around the page cache first
/// (`linux_direct`: O_DIRECT, at most 4 threads); a file system that takes no
/// direct I/O falls back to the ranges below before a byte has moved. Callers
/// with `parallel == 1` (moves, replacements, server copies) never copy
/// directly. `None` when canceled.
pub(super) fn copy_handles(
    reader: &File,
    writer: &mut File,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
    parallel: usize,
) -> io::Result<Option<u64>> {
    use std::io::Read;
    if parallel > 1 {
        // Every thread needs at least one transfer part of its own.
        let size = reader.metadata()?.len();
        if size >= crate::transfer::segments::SPLIT_FROM {
            // Sparse until the ranges are in; a short source is an error.
            writer.set_len(size)?;
            match linux_direct::copy_whole(reader, writer, size, parallel, cancel, progress)? {
                linux_direct::Whole::Done(copied) => return Ok(Some(copied)),
                linux_direct::Whole::Canceled => return Ok(None),
                linux_direct::Whole::Unavailable => {}
            }
            return linux_range::copy_ranges(reader, writer, size, parallel, cancel, progress);
        }
    }
    let mut copied = 0u64;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(None);
        }
        let mut chunk = Read::take(reader, KERNEL_CHUNK);
        let moved = std::io::copy(&mut chunk, writer)?;
        if moved == 0 {
            return Ok(Some(copied));
        }
        copied = copied.saturating_add(moved);
        progress(moved);
        // `io::copy` only returns early at the end of the source: a short
        // chunk is the last one, so the file needs no second round of
        // `fstat`s and a zero-length probe call.
        if moved < KERNEL_CHUNK {
            return Ok(Some(copied));
        }
    }
}

/// Publish or move without replacing: `renameat2(RENAME_NOREPLACE)` with the
/// link and checked-rename fallbacks for NFS, FUSE and Android storage.
fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    crate::android_fs::rename_no_replace(source, destination)
}
