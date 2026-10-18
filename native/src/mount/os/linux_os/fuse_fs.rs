//! The FUSE filesystem of a Linux drive (DRV-M4): `fuser::Filesystem` over
//! the frontend boundary `MountFilesystem` (DRV-M2), POSIX dialect.
//!
//! The two read loops of `fuser` only hand requests on: everything that can
//! reach the engine runs on the worker pool (`fuse_workers.rs`); only table
//! work (`forget`, `readdir` from the listing taken at `opendir`,
//! `releasedir`) and fixed refusals answer on the loop itself. The
//! operations live in `fuse_ops_meta.rs`, `fuse_ops_io.rs`,
//! `fuse_ops_attr.rs` and `fuse_ops_ns.rs`; this file holds their shared
//! state and the dispatch.
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use fuser::{
    Errno, FileHandle, FileType, Filesystem, INodeNo, InitFlags, KernelConfig, OpenFlags,
    RenameFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};

use super::fuse_inodes::InodeTable;
use super::fuse_workers::WorkerPool;
use super::{
    fuse_ops_attr as attr, fuse_ops_io as file_io, fuse_ops_meta as meta, fuse_ops_ns as ns,
};
use crate::mount::{HandleId, MountFilesystem};

/// Largest write request (decision C2: 1 MiB; the kernel default is 256
/// pages).
const MAX_WRITE: u32 = 1024 * 1024;

/// The user every file of the drive belongs to: the one who mounted it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Owner {
    pub(crate) uid: u32,
    pub(crate) gid: u32,
}

/// One open file: its inode, its engine handle and how it was opened.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenFile {
    pub(crate) ino: u64,
    pub(crate) handle: HandleId,
    pub(crate) writable: bool,
    pub(crate) append: bool,
}

/// One name of a directory listing taken at `opendir`.
#[derive(Clone, Debug)]
pub(crate) struct Listed {
    pub(crate) ino: u64,
    pub(crate) kind: FileType,
    pub(crate) name: String,
}

/// How the session of a drive ended (decided in `destroy`, before `fuser`
/// releases the mount).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionEnd {
    /// The host unmounted it (stop, update stop, worker end).
    Requested,
    /// Unmounted outside the host: a file manager's eject, `fusermount3 -u`,
    /// `se drive unmount`.
    Unmounted,
    /// The connection was aborted while still mounted: the kernel request
    /// timeout or `fusectl`.
    Aborted,
}

pub(crate) struct FuseShared {
    pub(crate) filesystem: Arc<dyn MountFilesystem>,
    pub(crate) inodes: InodeTable,
    pub(crate) owner: Owner,
    pub(crate) read_only: bool,
    /// The local cache folder; its disk is the drive's free space.
    pub(crate) cache_root: PathBuf,
    /// The mount folder as `mountinfo` lists it.
    pub(crate) mount_path: PathBuf,
    pub(crate) workers: WorkerPool,
    pub(crate) unmounting: AtomicBool,
    files: Mutex<HashMap<u64, OpenFile>>,
    directories: Mutex<HashMap<u64, Arc<[Listed]>>>,
    next_handle: AtomicU64,
    pub(crate) shown: super::fuse_ops_attr::Shown,
    stop: Arc<AtomicBool>,
    end: Mutex<Option<SessionEnd>>,
}

impl FuseShared {
    pub(crate) fn new(
        filesystem: Arc<dyn MountFilesystem>,
        read_only: bool,
        cache_root: PathBuf,
        mount_path: PathBuf,
    ) -> Arc<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let panicked = Arc::clone(&stop);
        Arc::new(Self {
            filesystem,
            inodes: InodeTable::new(),
            owner: Owner {
                uid: unsafe { libc::geteuid() },
                gid: unsafe { libc::getegid() },
            },
            read_only,
            cache_root,
            mount_path,
            workers: WorkerPool::new(move || {
                eprintln!("mount: a FUSE request failed internally; the drive stops");
                panicked.store(true, Ordering::Release);
            }),
            unmounting: AtomicBool::new(false),
            files: Mutex::new(HashMap::new()),
            directories: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
            shown: Default::default(),
            stop,
            end: Mutex::new(None),
        })
    }

    /// Runs `operation` on the worker pool.
    pub(crate) fn run(self: &Arc<Self>, operation: impl FnOnce(&FuseShared) + Send + 'static) {
        let shared = Arc::clone(self);
        self.workers.submit(Box::new(move || operation(&shared)));
    }

    /// Fail-stop: the host loop ends the drive and reports why.
    pub(crate) fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub(crate) fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn handle_number(&self) -> u64 {
        self.next_handle.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn insert_file(&self, file: OpenFile) -> u64 {
        let number = self.handle_number();
        if let Ok(mut files) = self.files.lock() {
            files.insert(number, file);
        }
        number
    }

    pub(crate) fn file(&self, number: u64) -> Result<OpenFile, Errno> {
        self.files
            .lock()
            .ok()
            .and_then(|files| files.get(&number).copied())
            .ok_or(Errno::EBADF)
    }

    pub(crate) fn take_file(&self, number: u64) -> Option<OpenFile> {
        self.files.lock().ok()?.remove(&number)
    }

    /// An engine handle open on `ino`, a writable one first; with
    /// `writable_only`, only a writable one.
    pub(crate) fn handle_of(&self, ino: u64, writable_only: bool) -> Option<HandleId> {
        let files = self.files.lock().ok()?;
        let mut fallback = None;
        for file in files.values().filter(|file| file.ino == ino) {
            if file.writable {
                return Some(file.handle);
            }
            if !writable_only {
                fallback.get_or_insert(file.handle);
            }
        }
        fallback
    }

    pub(crate) fn has_open_file(&self, ino: u64) -> bool {
        self.files
            .lock()
            .is_ok_and(|files| files.values().any(|file| file.ino == ino))
    }

    /// Every file still open, for the shutdown.
    pub(crate) fn drain_files(&self) -> Vec<OpenFile> {
        self.files
            .lock()
            .map(|mut files| files.drain().map(|(_, file)| file).collect())
            .unwrap_or_default()
    }

    pub(crate) fn insert_directory(&self, listing: Arc<[Listed]>) -> u64 {
        let number = self.handle_number();
        if let Ok(mut directories) = self.directories.lock() {
            directories.insert(number, listing);
        }
        number
    }

    pub(crate) fn directory(&self, number: u64) -> Option<Arc<[Listed]>> {
        self.directories.lock().ok()?.get(&number).cloned()
    }

    pub(crate) fn remove_directory(&self, number: u64) {
        if let Ok(mut directories) = self.directories.lock() {
            directories.remove(&number);
        }
    }

    /// Called from `destroy`: the mount is still listed after an abort and
    /// gone after an unmount.
    fn record_end(&self) {
        let end = if self.unmounting.load(Ordering::Acquire) {
            SessionEnd::Requested
        } else if super::fuse_abort::mount_at(&self.mount_path)
            .is_some_and(|entry| entry.fstype == super::mount_point::FUSE_TYPE)
        {
            SessionEnd::Aborted
        } else {
            SessionEnd::Unmounted
        };
        if let Ok(mut recorded) = self.end.lock() {
            recorded.get_or_insert(end);
        }
    }

    pub(crate) fn session_end(&self) -> Option<SessionEnd> {
        self.end.lock().ok().and_then(|end| *end)
    }
}

pub(crate) struct FuseFs {
    shared: Arc<FuseShared>,
}

impl FuseFs {
    pub(crate) fn new(shared: Arc<FuseShared>) -> Self {
        Self { shared }
    }
}

impl Filesystem for FuseFs {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> io::Result<()> {
        // O_TRUNC arrives with the open itself, so a truncating open never
        // fetches the old content first.
        let _ = config.add_capabilities(InitFlags::FUSE_ATOMIC_O_TRUNC);
        let _ = config.set_max_write(MAX_WRITE);
        Ok(())
    }

    fn destroy(&mut self) {
        self.shared.record_end();
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name = name.to_os_string();
        self.shared
            .run(move |shared| meta::lookup(shared, parent.0, &name, reply));
    }

    fn forget(&self, _req: &Request, ino: INodeNo, nlookup: u64) {
        self.shared.inodes.forget(ino.0, nlookup);
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        let fh = fh.map(|fh| fh.0);
        self.shared
            .run(move |shared| meta::getattr(shared, ino.0, fh, reply));
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let change = attr::Change {
            mode,
            uid,
            gid,
            size,
            mtime_ms: mtime.map(attr::millis),
            atime: atime.is_some(),
        };
        let fh = fh.map(|fh| fh.0);
        self.shared
            .run(move |shared| attr::setattr(shared, ino.0, fh, change, reply));
    }

    fn mknod(
        &self,
        _req: &Request,
        _parent: INodeNo,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        // Special files are hidden and never created on a drive.
        reply.error(Errno::EPERM);
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name = name.to_os_string();
        self.shared
            .run(move |shared| ns::mkdir(shared, parent.0, &name, reply));
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_os_string();
        self.shared
            .run(move |shared| ns::remove(shared, parent.0, &name, false, reply));
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_os_string();
        self.shared
            .run(move |shared| ns::remove(shared, parent.0, &name, true, reply));
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let request = ns::Rename {
            parent: parent.0,
            name: name.to_os_string(),
            new_parent: newparent.0,
            new_name: newname.to_os_string(),
            no_replace: flags.contains(RenameFlags::RENAME_NOREPLACE),
            unsupported: flags
                .intersects(RenameFlags::RENAME_EXCHANGE | RenameFlags::RENAME_WHITEOUT),
        };
        self.shared
            .run(move |shared| ns::rename(shared, request, reply));
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        self.shared
            .run(move |shared| file_io::open(shared, ino.0, flags, reply));
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: ReplyData,
    ) {
        self.shared
            .run(move |shared| file_io::read(shared, fh.0, offset, size, reply));
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: fuser::WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: ReplyWrite,
    ) {
        let data = data.to_vec();
        self.shared
            .run(move |shared| file_io::write(shared, fh.0, offset, &data, reply));
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _lock_owner: fuser::LockOwner,
        reply: ReplyEmpty,
    ) {
        self.shared
            .run(move |shared| file_io::flush(shared, fh.0, reply));
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.shared
            .run(move |shared| file_io::release(shared, fh.0, reply));
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        // Durable on a drive means published.
        self.shared
            .run(move |shared| file_io::flush(shared, fh.0, reply));
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        self.shared
            .run(move |shared| meta::opendir(shared, ino.0, reply));
    }

    fn readdir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: ReplyDirectory,
    ) {
        meta::readdir(&self.shared, fh.0, offset, reply);
    }

    fn releasedir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        self.shared.remove_directory(fh.0);
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        self.shared.run(move |shared| meta::statfs(shared, reply));
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let name = name.to_os_string();
        self.shared
            .run(move |shared| file_io::create(shared, parent.0, &name, flags, reply));
    }
}
