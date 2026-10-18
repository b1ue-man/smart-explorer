//! Metadata requests of the FUSE host (DRV-M4): lookup, getattr, the
//! directory listing and the free space.
//!
//! Attributes are uniform: every file belongs to the user who mounted,
//! regular files read `0644`, folders `0755` (`0444`/`0555` on a read-only
//! drive), so a program that compares owner and mode of a replacement with
//! the original (GIO `g_file_replace`) keeps its atomic save. A file with
//! pending times or mode shows the requested values until they are
//! published (decision C2). Links and special files are hidden, as on
//! Windows. Entries and attributes live 1 s in the kernel, negative entries
//! are not cached (decision C2). The free space is that of the local cache
//! disk, the hard lower bound of a whole-file drive, as on Windows.
use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    Errno, FileAttr, FileHandle, FileType, FopenFlags, Generation, INodeNo, ReplyAttr,
    ReplyDirectory, ReplyEntry, ReplyOpen, ReplyStatfs,
};

use super::fuse_errno::{errno, name_text};
use super::fuse_fs::{FuseShared, Listed};
use super::fuse_inodes::{join, split, ROOT_INO};
use crate::vfs::VfsMeta;

pub(crate) const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u32 = 4096;
const NAME_MAX: u32 = 255;

pub(crate) fn lookup(shared: &FuseShared, parent: u64, name: &OsStr, reply: ReplyEntry) {
    match looked_up(shared, parent, name) {
        Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
        Err(errno) => reply.error(errno),
    }
}

fn looked_up(shared: &FuseShared, parent: u64, name: &OsStr) -> Result<FileAttr, Errno> {
    let name = name_text(name)?;
    let path = shared
        .inodes
        .child_path(parent, name)
        .map_err(|error| errno(&error))?;
    let meta = visible(shared.filesystem.stat_cached(&path))?;
    let ino = shared
        .inodes
        .remember(&path)
        .map_err(|error| errno(&error))?;
    Ok(attr_for(shared, ino, &meta))
}

pub(crate) fn getattr(shared: &FuseShared, ino: u64, fh: Option<u64>, reply: ReplyAttr) {
    match stat(shared, ino, fh) {
        Ok(meta) => reply.attr(&TTL, &attr_for(shared, ino, &meta)),
        Err(errno) => reply.error(errno),
    }
}

/// Metadata of `ino`: through an open handle when there is one (it also
/// serves a file whose name is gone, and shows unpublished content), else
/// by path.
pub(crate) fn stat(shared: &FuseShared, ino: u64, fh: Option<u64>) -> Result<VfsMeta, Errno> {
    let handle = fh
        .and_then(|number| shared.file(number).ok())
        .map(|file| file.handle)
        .or_else(|| shared.handle_of(ino, false));
    if let Some(handle) = handle {
        if let Ok(meta) = shared.filesystem.stat_handle(handle) {
            return Ok(meta);
        }
    }
    let path = shared.inodes.path(ino).map_err(|error| errno(&error))?;
    visible(shared.filesystem.stat_cached(&path))
}

/// The attributes the kernel receives for `meta` as inode `ino`.
pub(crate) fn attr_for(shared: &FuseShared, ino: u64, meta: &VfsMeta) -> FileAttr {
    let shown = shared.shown.get(ino).unwrap_or_default();
    let mtime = system_time(shown.mtime_ms.unwrap_or(meta.mtime_ms));
    let created = if meta.btime_ms > 0 {
        system_time(meta.btime_ms)
    } else {
        mtime
    };
    let (kind, perm, nlink, size) = if meta.is_dir {
        let perm: u16 = if shared.read_only { 0o555 } else { 0o755 };
        (FileType::Directory, perm, 2, 0)
    } else {
        let uniform: u16 = if shared.read_only { 0o444 } else { 0o644 };
        let perm = shown.mode.map_or(uniform, |mode| {
            u16::try_from(mode & 0o7777).unwrap_or(uniform)
        });
        (FileType::RegularFile, perm, 1, meta.size)
    };
    FileAttr {
        ino: INodeNo(ino),
        size,
        blocks: size.div_ceil(512),
        atime: mtime,
        mtime,
        ctime: mtime,
        crtime: created,
        kind,
        perm,
        nlink,
        uid: shared.owner.uid,
        gid: shared.owner.gid,
        rdev: 0,
        blksize: BLOCK_SIZE,
        flags: 0,
    }
}

/// Takes the listing of `ino` once; `readdir` serves it by index, so a
/// change during the enumeration never skips or repeats a name.
pub(crate) fn opendir(shared: &FuseShared, ino: u64, reply: ReplyOpen) {
    match listing(shared, ino) {
        Ok(listing) => {
            let number = shared.insert_directory(listing.into());
            reply.opened(FileHandle(number), FopenFlags::empty());
        }
        Err(errno) => reply.error(errno),
    }
}

fn listing(shared: &FuseShared, ino: u64) -> Result<Vec<Listed>, Errno> {
    let path = shared.inodes.path(ino).map_err(|error| errno(&error))?;
    // A cold listing also captures the folder itself: list first, so the
    // stat below needs no separate remote request.
    let entries = shared
        .filesystem
        .list_dir_cached(&path)
        .map_err(|error| errno(&error))?;
    let directory = visible(shared.filesystem.stat_cached(&path))?;
    if !directory.is_dir {
        return Err(Errno::ENOTDIR);
    }
    let parent = split(&path)
        .and_then(|(parent, _)| shared.inodes.known(parent))
        .unwrap_or(ROOT_INO);
    let mut listing = vec![
        Listed {
            ino,
            kind: FileType::Directory,
            name: ".".into(),
        },
        Listed {
            ino: parent,
            kind: FileType::Directory,
            name: "..".into(),
        },
    ];
    for entry in entries.iter().filter(|entry| !hidden(entry)) {
        listing.push(Listed {
            ino: shared.inodes.listing_ino(&join(&path, &entry.name)),
            kind: if entry.is_dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            name: entry.name.clone(),
        });
    }
    Ok(listing)
}

/// Serves the listing of `opendir`; the offset is the index of the next
/// name.
pub(crate) fn readdir(shared: &FuseShared, number: u64, offset: u64, mut reply: ReplyDirectory) {
    let Some(listing) = shared.directory(number) else {
        reply.error(Errno::EBADF);
        return;
    };
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    for (index, entry) in listing.iter().enumerate().skip(start) {
        let next = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
        if reply.add(INodeNo(entry.ino), next, entry.kind, &entry.name) {
            break;
        }
    }
    reply.ok();
}

pub(crate) fn statfs(shared: &FuseShared, reply: ReplyStatfs) {
    match disk(&shared.cache_root) {
        Ok(stat) => reply.statfs(
            widen(stat.f_blocks),
            widen(stat.f_bfree),
            widen(stat.f_bavail),
            widen(stat.f_files),
            widen(stat.f_ffree),
            u32::try_from(widen(stat.f_bsize)).unwrap_or(BLOCK_SIZE),
            NAME_MAX,
            u32::try_from(widen(stat.f_frsize)).unwrap_or(BLOCK_SIZE),
        ),
        Err(error) => reply.error(errno(&error)),
    }
}

/// Links and special files (FIFO, socket, device) are hidden.
pub(crate) fn hidden(meta: &VfsMeta) -> bool {
    meta.is_symlink || meta.special
}

fn visible(meta: io::Result<VfsMeta>) -> Result<VfsMeta, Errno> {
    let meta = meta.map_err(|error| errno(&error))?;
    if hidden(&meta) {
        Err(Errno::ENOENT)
    } else {
        Ok(meta)
    }
}

pub(crate) fn system_time(milliseconds: i64) -> SystemTime {
    let span = Duration::from_millis(milliseconds.unsigned_abs());
    let time = if milliseconds >= 0 {
        UNIX_EPOCH.checked_add(span)
    } else {
        UNIX_EPOCH.checked_sub(span)
    };
    time.unwrap_or(UNIX_EPOCH)
}

fn disk(path: &Path) -> io::Result<libc::statvfs> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "cache path contains NUL"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { stat.assume_init() })
}

/// The C library field types differ; every value fits into `u64`.
fn widen<T: TryInto<u64>>(value: T) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}
