use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub(super) type FileIdentity = (u64, u64);

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
    rename_no_replace(staged, dest)
}

pub(super) fn move_file(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        return crate::vfs::replace_local_file(src, dest);
    }
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
/// which reflinks on btrfs/xfs and copies server-side on NFS). `None` when
/// canceled.
pub(super) fn copy_handles(
    reader: &File,
    writer: &mut File,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    use std::io::Read;
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
    }
}

/// Publish or move without replacing: `renameat2(RENAME_NOREPLACE)` with the
/// link and checked-rename fallbacks for NFS, FUSE and Android storage.
fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    crate::android_fs::rename_no_replace(source, destination)
}
