//! Namespace requests of the FUSE host (DRV-M4): mkdir, unlink, rmdir,
//! rename.
//!
//! POSIX semantics over the engine: `rename` replaces an existing file
//! (through the backend's atomic replace; the editor pattern "write a temp
//! file, rename it over the original"), an empty target folder is removed
//! first, `RENAME_NOREPLACE` never replaces, `RENAME_EXCHANGE` and
//! `RENAME_WHITEOUT` are refused (`EINVAL`). An unlinked or replaced file
//! stays usable through descriptors that are already open: the engine
//! detaches its entry, and a lazily opened replaced file is fetched first.
//! Links, devices and FIFOs are never created (`fuse_fs.rs`). A namespace
//! change whose remote result cannot be verified stops the drive, as on
//! Windows; the journal keeps it for Retry.
use std::ffi::{OsStr, OsString};
use std::io;

use fuser::{Errno, FileAttr, Generation, ReplyEmpty, ReplyEntry};

use super::fuse_errno::{errno, name_text};
use super::fuse_fs::FuseShared;
use super::fuse_ops_meta::{attr_for, hidden, TTL};
use crate::mount::{NamespaceOutcome, RenameOutcome};
use crate::vfs::VfsMeta;

/// One `rename` request.
pub(crate) struct Rename {
    pub(crate) parent: u64,
    pub(crate) name: OsString,
    pub(crate) new_parent: u64,
    pub(crate) new_name: OsString,
    pub(crate) no_replace: bool,
    /// `RENAME_EXCHANGE` or `RENAME_WHITEOUT`.
    pub(crate) unsupported: bool,
}

pub(crate) fn mkdir(shared: &FuseShared, parent: u64, name: &OsStr, reply: ReplyEntry) {
    match made(shared, parent, name) {
        Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
        Err(errno) => reply.error(errno),
    }
}

fn made(shared: &FuseShared, parent: u64, name: &OsStr) -> Result<FileAttr, Errno> {
    if shared.read_only {
        return Err(Errno::EROFS);
    }
    let path = child(shared, parent, name)?;
    match shared
        .filesystem
        .mkdir(&path)
        .map_err(|error| errno(&error))?
    {
        NamespaceOutcome::Complete => {}
        NamespaceOutcome::CommittedPendingVerification { .. } => {
            eprintln!("mount: a folder was created, but the result could not be verified; the drive stops");
            shared.request_stop();
        }
    }
    let meta = shared
        .filesystem
        .stat(&path)
        .map_err(|error| errno(&error))?;
    let ino = shared
        .inodes
        .remember(&path)
        .map_err(|error| errno(&error))?;
    Ok(attr_for(shared, ino, &meta))
}

pub(crate) fn remove(
    shared: &FuseShared,
    parent: u64,
    name: &OsStr,
    directory: bool,
    reply: ReplyEmpty,
) {
    match removed(shared, parent, name, directory) {
        Ok(()) => reply.ok(),
        Err(errno) => reply.error(errno),
    }
}

fn removed(shared: &FuseShared, parent: u64, name: &OsStr, directory: bool) -> Result<(), Errno> {
    if shared.read_only {
        return Err(Errno::EROFS);
    }
    let path = child(shared, parent, name)?;
    let meta = existing(shared, &path)?.ok_or(Errno::ENOENT)?;
    if directory && !meta.is_dir {
        return Err(Errno::ENOTDIR);
    }
    if !directory && meta.is_dir {
        return Err(Errno::EISDIR);
    }
    shared
        .filesystem
        .delete(&path, directory)
        .map_err(|error| errno(&error))?;
    shared.inodes.unlinked(&path);
    Ok(())
}

pub(crate) fn rename(shared: &FuseShared, request: Rename, reply: ReplyEmpty) {
    match renamed(shared, &request) {
        Ok(()) => reply.ok(),
        Err(errno) => reply.error(errno),
    }
}

fn renamed(shared: &FuseShared, request: &Rename) -> Result<(), Errno> {
    if request.unsupported {
        return Err(Errno::EINVAL);
    }
    if shared.read_only {
        return Err(Errno::EROFS);
    }
    let from = child(shared, request.parent, &request.name)?;
    let to = child(shared, request.new_parent, &request.new_name)?;
    if from == to {
        return Ok(());
    }
    let source = existing(shared, &from)?.ok_or(Errno::ENOENT)?;
    let outcome = match existing(shared, &to)? {
        Some(_) if request.no_replace => return Err(Errno::EEXIST),
        Some(target) if target.is_dir => {
            if !source.is_dir {
                return Err(Errno::EISDIR);
            }
            // POSIX replaces an empty folder; a full one is ENOTEMPTY.
            shared
                .filesystem
                .delete(&to, true)
                .map_err(|error| errno(&error))?;
            shared.inodes.unlinked(&to);
            shared.filesystem.rename(&from, &to, false)
        }
        Some(_) => {
            if source.is_dir {
                return Err(Errno::ENOTDIR);
            }
            replace(shared, &from, &to)
        }
        None => match shared.filesystem.rename(&from, &to, false) {
            // The name appeared meanwhile: POSIX replaces it.
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && !request.no_replace
                    && !source.is_dir =>
            {
                replace(shared, &from, &to)
            }
            other => other,
        },
    }
    .map_err(|error| errno(&error))?;
    if let RenameOutcome::CommittedPendingVerification(_) = outcome {
        eprintln!("mount: a rename took effect, but its result could not be verified");
    }
    shared.inodes.renamed(&from, &to);
    Ok(())
}

/// Replacing rename of a file; descriptors still open on the target keep
/// their content (POSIX), so the engine treats them as sharing delete.
fn replace(shared: &FuseShared, from: &str, to: &str) -> io::Result<RenameOutcome> {
    shared
        .filesystem
        .rename_with_shared_destination(from, to, true, true)
}

/// The visible entry at `path`; `None` when it does not exist. A link or a
/// special file is never touched (`EPERM`).
fn existing(shared: &FuseShared, path: &str) -> Result<Option<VfsMeta>, Errno> {
    match shared.filesystem.stat_cached(path) {
        Ok(meta) if hidden(&meta) => Err(Errno::EPERM),
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(errno(&error)),
    }
}

fn child(shared: &FuseShared, parent: u64, name: &OsStr) -> Result<String, Errno> {
    shared
        .inodes
        .child_path(parent, name_text(name)?)
        .map_err(|error| errno(&error))
}
