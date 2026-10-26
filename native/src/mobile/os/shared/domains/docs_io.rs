//! File descriptors of the Android documents provider (DRV-M8, strand S2
//! C3): `openDocument` hands other apps a proxy descriptor
//! (`StorageManager.openProxyFileDescriptor`) whose callbacks reach these
//! functions over JNI (`native/android-bridge/src/io.rs`). Every function
//! returns a negated `errno` on failure (table of strand S2 C4) and never
//! panics across the bridge.
//!
//! - Modes `r`, `w`, `wt`, `rw`, `rwt`; `t` truncates when opening.
//!   Appending (`wa`, `rwa`) is refused with `EINVAL`: the platform drops
//!   the append flag and AppFuse has no `SETATTR`, so appends would land
//!   at offset 0 (NR N20).
//! - `r` opens without fetching; the first read fetches or, for a large
//!   file, reads ranges (`RangeReadPolicy::ReadOnlyHandles`). A dirty local
//!   copy always wins, so a reader sees what a writer left unpublished.
//! - `doc_size` answers from the handle without the network (vold and the
//!   system_server watchdog wait on it, NR N11).
//! - `doc_fsync` publishes and reports the error; `doc_release` never
//!   blocks: publishing and closing run on a thread of their own, and what
//!   fails stays in the journal for `docs.retryPending`.
//! - A revoked descriptor (its vault locked, its root no longer released)
//!   answers every call with `EACCES`.
use std::collections::HashMap;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::docs_engines::{self, EngineKey, Slot};
use super::docs_ids::{DocId, RootId};
use crate::mobile::runtime::lock;
use crate::mount::{HandleId, OpenDisposition, OpenFileOptions};

struct Open {
    root: RootId,
    slot: Arc<Slot>,
    handle: HandleId,
    writable: bool,
    revoked: AtomicBool,
}

fn handles() -> &'static Mutex<HashMap<i64, Arc<Open>>> {
    static HANDLES: OnceLock<Mutex<HashMap<i64, Arc<Open>>>> = OnceLock::new();
    HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT: AtomicI64 = AtomicI64::new(1);
/// Releases whose publication still runs.
static RELEASING: AtomicUsize = AtomicUsize::new(0);

/// The mode of `openDocument`: (writable, disposition).
pub(super) fn parse_mode(mode: &str) -> Result<(bool, OpenDisposition), i32> {
    match mode {
        "r" => Ok((false, OpenDisposition::OpenExisting)),
        "w" | "rw" => Ok((true, OpenDisposition::OpenExisting)),
        "wt" | "rwt" => Ok((true, OpenDisposition::TruncateExisting)),
        _ => Err(libc::EINVAL),
    }
}

fn guarded<T>(fallback: T, work: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or(fallback)
}

fn get(handle: i64) -> Result<Arc<Open>, i32> {
    let open = lock(handles()).get(&handle).cloned().ok_or(libc::EBADF)?;
    if open.revoked.load(Ordering::Acquire) {
        return Err(libc::EACCES);
    }
    let access = open.slot.check_access().and_then(|_| {
        if super::docs_vault::released(&open.root) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Dokument nicht freigegeben",
            ))
        }
    });
    if let Err(error) = access {
        if !open.revoked.swap(true, Ordering::AcqRel) {
            let _ = open.slot.fs.close(open.handle);
        }
        return Err(errno_for(&error));
    }
    Ok(open)
}

/// Opens `document` (an ID of a released root) in `mode`; the descriptor
/// number (≥ 1) or a negated `errno`.
pub fn doc_open(document: &str, mode: &str) -> i64 {
    guarded(-i64::from(libc::EIO), || match open(document, mode) {
        Ok(handle) => handle,
        Err(errno) => -i64::from(errno),
    })
}

fn open(document: &str, mode: &str) -> Result<i64, i32> {
    let (writable, disposition) = parse_mode(mode)?;
    let doc = DocId::parse(document).ok_or(libc::ENOENT)?;
    if !super::docs_vault::released(&doc.root) {
        return Err(libc::EACCES);
    }
    let (key, path) = EngineKey::of(&doc).ok_or(libc::EISDIR)?;
    if path == "/" {
        return Err(libc::EISDIR);
    }
    let slot = docs_engines::engine(&key).map_err(|error| errno_for(&error))?;
    if writable && !slot.writable {
        return Err(libc::EROFS);
    }
    let handle = if writable {
        let options = OpenFileOptions {
            writable,
            disposition,
        };
        slot.fs.open_file(&path, options)
    } else {
        // No fetch at open: `onGetSize` and the first read decide.
        slot.fs
            .stat(&path)
            .and_then(|meta| slot.fs.open_metadata_file(&path, meta, false))
    }
    .map_err(|error| errno_for(&error))?;
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let open = Open {
        root: doc.root,
        slot,
        handle,
        writable,
        revoked: AtomicBool::new(false),
    };
    lock(handles()).insert(number, Arc::new(open));
    Ok(number)
}

/// Reads at `offset` into `output`; bytes read (0 at the end) or `-errno`.
pub fn doc_read(handle: i64, offset: i64, output: &mut [u8]) -> i32 {
    guarded(-libc::EIO, || {
        let result = get(handle).and_then(|open| {
            let offset = u64::try_from(offset).map_err(|_| libc::EINVAL)?;
            open.slot
                .fs
                .read(open.handle, offset, output)
                .map_err(|error| errno_for(&error))
        });
        count(result)
    })
}

/// Writes `input` at `offset`; bytes written or `-errno`.
pub fn doc_write(handle: i64, offset: i64, input: &[u8]) -> i32 {
    guarded(-libc::EIO, || {
        let result = get(handle).and_then(|open| {
            if !open.writable {
                return Err(libc::EBADF);
            }
            let offset = u64::try_from(offset).map_err(|_| libc::EINVAL)?;
            open.slot
                .fs
                .write(open.handle, offset, input)
                .map_err(|error| errno_for(&error))
        });
        count(result)
    })
}

/// The size from the handle, without the network; `-errno` on failure.
pub fn doc_size(handle: i64) -> i64 {
    guarded(-i64::from(libc::EIO), || {
        match get(handle).and_then(|open| {
            open.slot
                .fs
                .len(open.handle)
                .map_err(|error| errno_for(&error))
        }) {
            Ok(size) => i64::try_from(size).unwrap_or(i64::MAX),
            Err(errno) => -i64::from(errno),
        }
    })
}

/// Publishes the content now; 0 or `-errno` (the app sees the error).
pub fn doc_fsync(handle: i64) -> i32 {
    guarded(-libc::EIO, || {
        let result = get(handle).and_then(|open| {
            if !open.writable {
                return Ok(0);
            }
            open.slot
                .fs
                .flush(open.handle)
                .map(|_| 0)
                .map_err(|error| errno_for(&error))
        });
        count(result)
    })
}

/// Ends the descriptor at once; publishing runs in the background and
/// `docs.changed` follows (with `commitFailed` when it failed).
pub fn doc_release(handle: i64) -> i32 {
    guarded(-libc::EIO, || {
        let Some(open) = lock(handles()).remove(&handle) else {
            return -libc::EBADF;
        };
        RELEASING.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("docs-release".into())
            .spawn(move || {
                let failed = finish(&open);
                RELEASING.fetch_sub(1, Ordering::SeqCst);
                super::docs::committed(&open.root, failed);
            });
        if spawned.is_err() {
            RELEASING.fetch_sub(1, Ordering::SeqCst);
            return -libc::EAGAIN;
        }
        0
    })
}

/// Publishes a writable handle and closes it; whether publishing failed.
/// The engine keeps a failed change in its journal. A revoked handle is
/// closed already.
fn finish(open: &Open) -> bool {
    if open.revoked.load(Ordering::Acquire) {
        return false;
    }
    if open.slot.check_access().is_err() || !super::docs_vault::released(&open.root) {
        open.revoked.store(true, Ordering::Release);
        let _ = open.slot.fs.close(open.handle);
        return open.writable;
    }
    let failed = open.writable && open.slot.fs.flush(open.handle).is_err();
    let _ = open.slot.fs.close(open.handle);
    failed
}

/// Waits up to `limit` for running releases (before `docs.retryPending`).
pub(super) fn wait_for_releases(limit: Duration) {
    let until = Instant::now() + limit;
    while RELEASING.load(Ordering::SeqCst) > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Open descriptors of `root`.
fn of_root(root: &RootId) -> Vec<Arc<Open>> {
    lock(handles())
        .values()
        .filter(|open| open.root == *root)
        .cloned()
        .collect()
}

/// Publishes what the open writers of `root` hold (before a vault lock).
pub(super) fn flush_root(root: &RootId) {
    for open in of_root(root).into_iter().filter(|open| open.writable) {
        let _ = open.slot.fs.flush(open.handle);
    }
}

/// Ends every descriptor of `root`: the engine handles close at once
/// (unpublished content stays in the journal) and every later call of the
/// app on such a descriptor fails with `EACCES` until it releases it.
pub(super) fn revoke(root: &RootId) {
    for open in of_root(root) {
        if !open.revoked.swap(true, Ordering::AcqRel) {
            let _ = open.slot.fs.close(open.handle);
        }
    }
}

fn count(result: Result<usize, i32>) -> i32 {
    match result {
        Ok(read) => i32::try_from(read).unwrap_or(i32::MAX),
        Err(errno) => -errno,
    }
}

/// `errno` of an engine error (table of strand S2 C4); a locked vault is
/// `EACCES`.
pub(super) fn errno_for(error: &io::Error) -> i32 {
    use io::ErrorKind as Kind;
    if crate::vault::locked_site(error).is_some() {
        return libc::EACCES;
    }
    match error.kind() {
        Kind::NotFound => libc::ENOENT,
        Kind::NotADirectory => libc::ENOTDIR,
        Kind::IsADirectory => libc::EISDIR,
        Kind::DirectoryNotEmpty => libc::ENOTEMPTY,
        Kind::PermissionDenied => libc::EACCES,
        Kind::ReadOnlyFilesystem => libc::EROFS,
        Kind::AlreadyExists => libc::EEXIST,
        Kind::InvalidInput | Kind::InvalidData => libc::EINVAL,
        Kind::TimedOut => libc::ETIMEDOUT,
        Kind::StorageFull | Kind::WriteZero => libc::ENOSPC,
        Kind::Unsupported => libc::ENOTSUP,
        Kind::OutOfMemory => libc::ENOMEM,
        Kind::WouldBlock => libc::EBUSY,
        Kind::Interrupted => libc::EINTR,
        Kind::CrossesDevices => libc::EXDEV,
        Kind::ConnectionRefused => libc::ECONNREFUSED,
        Kind::ConnectionReset => libc::ECONNRESET,
        Kind::NotConnected => libc::ENOTCONN,
        Kind::NetworkDown => libc::ENETDOWN,
        Kind::NetworkUnreachable => libc::ENETUNREACH,
        Kind::HostUnreachable => libc::EHOSTUNREACH,
        _ => libc::EIO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_task_docs_modes_truncate_only_with_t_and_refuse_append() {
        assert_eq!(parse_mode("r"), Ok((false, OpenDisposition::OpenExisting)));
        assert_eq!(parse_mode("w"), Ok((true, OpenDisposition::OpenExisting)));
        assert_eq!(parse_mode("rw"), Ok((true, OpenDisposition::OpenExisting)));
        assert_eq!(
            parse_mode("wt"),
            Ok((true, OpenDisposition::TruncateExisting))
        );
        assert_eq!(
            parse_mode("rwt"),
            Ok((true, OpenDisposition::TruncateExisting))
        );
        for refused in ["wa", "rwa", "a", "", "x"] {
            assert_eq!(parse_mode(refused), Err(libc::EINVAL), "{refused}");
        }
    }

    #[test]
    fn drive_task_docs_errno_follows_the_c4_table() {
        use io::ErrorKind as Kind;
        let table = [
            (Kind::NotFound, libc::ENOENT),
            (Kind::PermissionDenied, libc::EACCES),
            (Kind::ReadOnlyFilesystem, libc::EROFS),
            (Kind::WouldBlock, libc::EBUSY),
            (Kind::HostUnreachable, libc::EHOSTUNREACH),
            (Kind::StorageFull, libc::ENOSPC),
            (Kind::Other, libc::EIO),
        ];
        for (kind, errno) in table {
            assert_eq!(errno_for(&io::Error::new(kind, "x")), errno, "{kind:?}");
        }
        let locked = crate::vault::vault_locked("0123456789abcdef0123456789abcdef");
        assert_eq!(errno_for(&locked), libc::EACCES);
    }

    #[test]
    fn drive_task_docs_unknown_descriptor_is_ebadf() {
        let mut buffer = [0u8; 4];
        assert_eq!(doc_read(i64::MAX, 0, &mut buffer), -libc::EBADF);
        assert_eq!(doc_size(i64::MAX), -i64::from(libc::EBADF));
        assert_eq!(doc_release(i64::MAX), -libc::EBADF);
    }
}
