//! Data requests of the FUSE host (DRV-M4): open, create, read, write, the
//! publication at `close(2)`, and release.
//!
//! `flush` runs at every `close(2)` of a descriptor and publishes the
//! pending content of a writable one (whole-file spool, like the Dokany
//! cleanup); its error reaches `close(2)`. A remote change since the open
//! reads as `EBUSY`, and the engine keeps the save as a conflict copy next to
//! the file. A later `flush` without a change is `NoChanges`; `fsync`
//! publishes the same way. `release` closes the engine handle; what could
//! not be published stays in the journal for Retry.
//!
//! An open-existing descriptor starts lazy: its first read or write fetches
//! the file (a read-only drive reads large files by ranges). `O_TRUNC`
//! arrives with the open (`FUSE_ATOMIC_O_TRUNC`) and never fetches the old
//! content; `O_APPEND` writes go to the end of the spool, whatever size the
//! kernel last saw.
use std::ffi::OsStr;

use fuser::{
    Errno, FileAttr, FileHandle, FopenFlags, Generation, OpenAccMode, OpenFlags, ReplyCreate,
    ReplyData, ReplyEmpty, ReplyOpen, ReplyWrite,
};

use super::fuse_errno::{errno, name_text};
use super::fuse_fs::{FuseShared, OpenFile};
use super::fuse_ops_meta::{attr_for, stat, TTL};
use crate::mount::{FlushOutcome, OpenDisposition, OpenFileOptions};

pub(crate) fn open(shared: &FuseShared, ino: u64, flags: OpenFlags, reply: ReplyOpen) {
    match opened(shared, ino, flags) {
        Ok(number) => reply.opened(FileHandle(number), FopenFlags::empty()),
        Err(errno) => reply.error(errno),
    }
}

fn opened(shared: &FuseShared, ino: u64, flags: OpenFlags) -> Result<u64, Errno> {
    let writable = flags.acc_mode() != OpenAccMode::O_RDONLY;
    if writable && shared.read_only {
        return Err(Errno::EROFS);
    }
    let path = shared.inodes.path(ino).map_err(|error| errno(&error))?;
    let handle = if writable && flags.0 & libc::O_TRUNC != 0 {
        shared.filesystem.open_file(
            &path,
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::TruncateExisting,
            },
        )
    } else {
        let meta = stat(shared, ino, None)?;
        if meta.is_dir {
            return Err(Errno::EISDIR);
        }
        shared.filesystem.open_metadata_file(&path, meta, writable)
    }
    .map_err(|error| errno(&error))?;
    Ok(shared.insert_file(OpenFile {
        ino,
        handle,
        writable,
        append: flags.0 & libc::O_APPEND != 0,
    }))
}

pub(crate) fn create(
    shared: &FuseShared,
    parent: u64,
    name: &OsStr,
    flags: i32,
    reply: ReplyCreate,
) {
    match created(shared, parent, name, flags) {
        Ok((attr, number)) => reply.created(
            &TTL,
            &attr,
            Generation(0),
            FileHandle(number),
            FopenFlags::empty(),
        ),
        Err(errno) => reply.error(errno),
    }
}

fn created(
    shared: &FuseShared,
    parent: u64,
    name: &OsStr,
    flags: i32,
) -> Result<(FileAttr, u64), Errno> {
    if shared.read_only {
        return Err(Errno::EROFS);
    }
    let name = name_text(name)?;
    let path = shared
        .inodes
        .child_path(parent, name)
        .map_err(|error| errno(&error))?;
    let disposition = if flags & libc::O_EXCL != 0 {
        OpenDisposition::CreateNew
    } else if flags & libc::O_TRUNC != 0 {
        OpenDisposition::CreateAlways
    } else {
        OpenDisposition::OpenOrCreate
    };
    let writable = flags & libc::O_ACCMODE != libc::O_RDONLY;
    let handle = shared
        .filesystem
        .open_file(
            &path,
            OpenFileOptions {
                writable,
                disposition,
            },
        )
        .map_err(|error| errno(&error))?;
    let known = shared
        .filesystem
        .stat_handle(handle)
        .and_then(|meta| shared.inodes.remember(&path).map(|ino| (ino, meta)));
    let (ino, meta) = match known {
        Ok(known) => known,
        Err(error) => {
            let _ = shared.filesystem.close(handle);
            return Err(errno(&error));
        }
    };
    let number = shared.insert_file(OpenFile {
        ino,
        handle,
        writable,
        append: flags & libc::O_APPEND != 0,
    });
    Ok((attr_for(shared, ino, &meta), number))
}

pub(crate) fn read(shared: &FuseShared, number: u64, offset: u64, size: u32, reply: ReplyData) {
    let file = match shared.file(number) {
        Ok(file) => file,
        Err(errno) => return reply.error(errno),
    };
    let mut buffer = vec![0u8; size as usize];
    match shared.filesystem.read(file.handle, offset, &mut buffer) {
        Ok(read) => reply.data(&buffer[..read.min(buffer.len())]),
        Err(error) => reply.error(errno(&error)),
    }
}

pub(crate) fn write(shared: &FuseShared, number: u64, offset: u64, data: &[u8], reply: ReplyWrite) {
    let file = match shared.file(number) {
        Ok(file) => file,
        Err(errno) => return reply.error(errno),
    };
    if !file.writable {
        return reply.error(Errno::EBADF);
    }
    let written = if file.append {
        shared.filesystem.append(file.handle, data)
    } else {
        shared.filesystem.write(file.handle, offset, data)
    };
    match written {
        Ok(written) => reply.written(u32::try_from(written).unwrap_or(u32::MAX)),
        Err(error) => reply.error(errno(&error)),
    }
}

/// `close(2)`: the file is safe in the local write cache when this returns;
/// with write-back its upload follows in the background (G4 ML1).
pub(crate) fn flush(shared: &FuseShared, number: u64, reply: ReplyEmpty) {
    let file = match shared.file(number) {
        Ok(file) => file,
        Err(errno) => return reply.error(errno),
    };
    match save(shared, file, true) {
        Ok(()) => reply.ok(),
        Err(errno) => reply.error(errno),
    }
}

/// `fsync(2)`: durable on a drive means published, so this waits for it.
pub(crate) fn fsync(shared: &FuseShared, number: u64, reply: ReplyEmpty) {
    let file = match shared.file(number) {
        Ok(file) => file,
        Err(errno) => return reply.error(errno),
    };
    match publish(shared, file) {
        Ok(()) => reply.ok(),
        Err(errno) => reply.error(errno),
    }
}

/// Publishes the pending content behind a writable descriptor.
pub(crate) fn publish(shared: &FuseShared, file: OpenFile) -> Result<(), Errno> {
    save(shared, file, false)
}

/// `on_close`: the engine may queue the upload instead of waiting for it.
fn save(shared: &FuseShared, file: OpenFile, on_close: bool) -> Result<(), Errno> {
    if !file.writable {
        return Ok(());
    }
    let outcome = if on_close {
        shared.filesystem.flush_on_close(file.handle)
    } else {
        shared.filesystem.flush(file.handle)
    };
    match outcome {
        Ok(FlushOutcome::NoChanges) => Ok(()),
        Ok(FlushOutcome::Committed) => {
            shared.shown.remove(file.ino);
            Ok(())
        }
        Ok(FlushOutcome::CommittedPendingVerification(_)) => {
            // The remote already took the save: an error would invite the
            // program to repeat it. The journal keeps the open verification.
            eprintln!("mount: a save was published, but its result could not be verified");
            shared.shown.remove(file.ino);
            Ok(())
        }
        Ok(FlushOutcome::Conflict(_)) => {
            eprintln!(
                "mount: the remote file changed while it was open; the save stays as a conflict copy"
            );
            Err(Errno::EBUSY)
        }
        Err(error) => Err(errno(&error)),
    }
}

pub(crate) fn release(shared: &FuseShared, number: u64, reply: ReplyEmpty) {
    if let Some(file) = shared.take_file(number) {
        let _ = shared.filesystem.close(file.handle);
        if !shared.has_open_file(file.ino) {
            shared.shown.remove(file.ino);
        }
    }
    reply.ok();
}

/// At the end of a session: every descriptor the kernel never released
/// (aborted or lazily unmounted with open files) is published, like the
/// Dokany cleanup at unmount, when `publish_first`, and closed.
pub(crate) fn close_all(shared: &FuseShared, publish_first: bool) {
    for file in shared.drain_files() {
        if publish_first {
            let _ = publish(shared, file);
        }
        let _ = shared.filesystem.close(file.handle);
    }
}
