//! `setattr` of the FUSE host: `truncate`, `chmod`, `chown`, `utimensat`
//! (DRV-M4, decision C2).
//!
//! - Size: `truncate` of the open handle; a path without an open handle is
//!   opened, truncated and published at once (`truncate(2)` is complete).
//! - Modification time and mode of a file with pending content (a dirty
//!   entry behind an open handle) are stored and handed to `finish_stage`
//!   at the publication; `getattr` shows them until then. A target that
//!   keeps its own times or no modes is no error.
//! - Times and mode of a folder or of a file without pending content:
//!   success without effect (there is no contract to set them later, and an
//!   error would break `cp -a` and `rsync -a` at every folder).
//! - Access time: no effect (the drive is mounted `noatime`).
//! - `chown` to the user who mounted and to his group: success; any other
//!   owner or group: `EPERM`, as POSIX for an unprivileged user.
//! - A read-only drive refuses every change with `EROFS`.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use fuser::{Errno, ReplyAttr, TimeOrNow};

use super::fuse_errno::errno;
use super::fuse_fs::FuseShared;
use super::fuse_ops_meta::{attr_for, stat, TTL};
use crate::mount::{FlushOutcome, OpenDisposition, OpenFileOptions, PendingAttributes};

/// The changes of one `setattr` request.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Change {
    pub(crate) mode: Option<u32>,
    pub(crate) uid: Option<u32>,
    pub(crate) gid: Option<u32>,
    pub(crate) size: Option<u64>,
    pub(crate) mtime_ms: Option<i64>,
    /// An access time was requested (no effect, but still a change).
    pub(crate) atime: bool,
}

impl Change {
    fn any(&self) -> bool {
        self.mode.is_some()
            || self.uid.is_some()
            || self.gid.is_some()
            || self.size.is_some()
            || self.mtime_ms.is_some()
            || self.atime
    }
}

/// Times and mode `getattr` shows for a file whose pending ones the engine
/// keeps, by inode, until they are published or the last descriptor closes.
#[derive(Default)]
pub(crate) struct Shown(Mutex<HashMap<u64, PendingAttributes>>);

impl Shown {
    pub(crate) fn get(&self, ino: u64) -> Option<PendingAttributes> {
        self.0.lock().ok()?.get(&ino).copied()
    }

    /// A later value replaces an earlier one field by field.
    pub(crate) fn merge(&self, ino: u64, attributes: PendingAttributes) {
        if let Ok(mut shown) = self.0.lock() {
            let stored = shown.entry(ino).or_default();
            if attributes.mtime_ms.is_some() {
                stored.mtime_ms = attributes.mtime_ms;
            }
            if attributes.mode.is_some() {
                stored.mode = attributes.mode;
            }
        }
    }

    pub(crate) fn remove(&self, ino: u64) {
        if let Ok(mut shown) = self.0.lock() {
            shown.remove(&ino);
        }
    }
}

/// A requested time in milliseconds since the epoch.
pub(crate) fn millis(time: TimeOrNow) -> i64 {
    let time = match time {
        TimeOrNow::SpecificTime(time) => time,
        TimeOrNow::Now => SystemTime::now(),
    };
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_millis())
            .map(|millis| -millis)
            .unwrap_or(i64::MIN),
    }
}

pub(crate) fn setattr(
    shared: &FuseShared,
    ino: u64,
    fh: Option<u64>,
    change: Change,
    reply: ReplyAttr,
) {
    match applied(shared, ino, fh, change) {
        Ok(()) => match stat(shared, ino, fh) {
            Ok(meta) => reply.attr(&TTL, &attr_for(shared, ino, &meta)),
            Err(errno) => reply.error(errno),
        },
        Err(errno) => reply.error(errno),
    }
}

fn applied(shared: &FuseShared, ino: u64, fh: Option<u64>, change: Change) -> Result<(), Errno> {
    if !change.any() {
        return Ok(());
    }
    if shared.read_only {
        return Err(Errno::EROFS);
    }
    if change.uid.is_some_and(|uid| uid != shared.owner.uid)
        || change.gid.is_some_and(|gid| gid != shared.owner.gid)
    {
        return Err(Errno::EPERM);
    }
    let handle = fh
        .and_then(|number| shared.file(number).ok())
        .filter(|file| file.writable)
        .map(|file| file.handle)
        .or_else(|| shared.handle_of(ino, true));
    if let Some(size) = change.size {
        match handle {
            Some(handle) => shared
                .filesystem
                .truncate(handle, size)
                .map_err(|error| errno(&error))?,
            None => truncate_path(shared, ino, size)?,
        }
    }
    if change.mtime_ms.is_none() && change.mode.is_none() {
        return Ok(());
    }
    let meta = stat(shared, ino, fh)?;
    if meta.is_dir {
        return Ok(());
    }
    let Some(handle) = handle else {
        // No pending content: success without effect (decision C2).
        return Ok(());
    };
    let attributes = PendingAttributes {
        mtime_ms: change.mtime_ms,
        mode: change.mode,
    };
    shared
        .filesystem
        .set_pending_attributes(handle, attributes)
        .map_err(|error| errno(&error))?;
    // Shown only where the engine kept them: a dirty file.
    let own = shared
        .inodes
        .path(ino)
        .ok()
        .and_then(|path| backend_path(shared, &path));
    let dirty = own.is_some_and(|own| {
        shared
            .filesystem
            .dirty_entries()
            .is_ok_and(|entries| entries.iter().any(|(path, _)| *path == own))
    });
    if dirty {
        shared.shown.merge(ino, attributes);
    }
    Ok(())
}

/// `truncate(2)` by path: opened, truncated and published at once.
fn truncate_path(shared: &FuseShared, ino: u64, size: u64) -> Result<(), Errno> {
    let path = shared.inodes.path(ino).map_err(|error| errno(&error))?;
    let disposition = if size == 0 {
        OpenDisposition::TruncateExisting
    } else {
        OpenDisposition::OpenExisting
    };
    let handle = shared
        .filesystem
        .open_file(
            &path,
            OpenFileOptions {
                writable: true,
                disposition,
            },
        )
        .map_err(|error| errno(&error))?;
    let result = shared
        .filesystem
        .truncate(handle, size)
        .and_then(|()| shared.filesystem.flush(handle));
    let _ = shared.filesystem.close(handle);
    match result {
        Ok(FlushOutcome::Conflict(_)) => Err(Errno::EBUSY),
        Ok(_) => Ok(()),
        Err(error) => Err(errno(&error)),
    }
}

/// The backend path of a callback path, as `dirty_entries` names it.
fn backend_path(shared: &FuseShared, callback: &str) -> Option<String> {
    shared
        .filesystem
        .project(callback)
        .ok()
        .map(|projected| projected.backend().to_string())
}
