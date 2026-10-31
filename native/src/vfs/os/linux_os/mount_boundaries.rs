//! Mount boundaries below one walk root without a system call per folder
//! beyond one `poll`: the root is made canonical once, the mount table is
//! read once and read again only after the kernel reports a change of the
//! mount namespace (`/proc/self/mountinfo` is pollable: `POLLPRI|POLLERR`
//! after every mount or unmount, proc(5)). A folder mounted while the walk
//! runs is therefore found exactly as `mount_boundary` finds it, bind
//! mounts on the same device and autofs triggers included, without
//! touching the child.
//!
//! A folder below the root is reached without following a link (walks omit
//! links), so its canonical path is the canonical root joined with its path
//! below the root, which `mount_boundary` obtains by `canonicalize` of the
//! parent. The root itself and paths outside it take `mount_boundary`.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use super::mountinfo::{self, Mount};

const MOUNTINFO: &str = "/proc/self/mountinfo";

struct Table {
    canonical_root: PathBuf,
    /// Open mount table whose `poll` reports namespace changes; `None` =
    /// not pollable here, so every question reads the table again.
    watch: Option<File>,
    mounts: Arc<Vec<Mount>>,
}

pub(crate) struct MountBoundaries {
    root: PathBuf,
    table: RwLock<Option<Table>>,
    /// A thread saw a change the table does not hold yet. Set together with
    /// the `poll` that consumed the change (`polling`), so no other thread
    /// answers from the old table after that change was reported.
    stale: AtomicBool,
    polling: Mutex<()>,
}

impl MountBoundaries {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            table: RwLock::new(None),
            stale: AtomicBool::new(false),
            polling: Mutex::new(()),
        }
    }

    /// `mount_boundary(path)` for a folder the walk reached below its root.
    pub(crate) fn boundary(&self, path: &Path) -> io::Result<Option<crate::vfs::MountKind>> {
        let rel = match path.strip_prefix(&self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel,
            _ => return super::mount_boundary(path),
        };
        let current = {
            let table = self
                .table
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match table.as_ref() {
                Some(table) if !self.changed(table.watch.as_ref()) => {
                    Some((table.canonical_root.join(rel), table.mounts.clone()))
                }
                _ => None,
            }
        };
        let (candidate, mounts) = match current {
            Some(current) => current,
            None => self.reload(rel)?,
        };
        Ok(mountinfo::boundary(&mounts, &candidate))
    }

    /// Reads the table (and on first use the canonical root) again; errors
    /// are never kept, so the next question tries again.
    ///
    /// No answer comes from an older table after a reported change: a change
    /// is reported only by `changed`, which polls under the read lock, so it
    /// is either reported before this write lock is taken (and then in the
    /// text read below) or after the new table is in place (and then makes
    /// the next question read again). Pending events are consumed and the
    /// mark is cleared before the text is read, never after it, so a change
    /// between the read and the release is never cleared unseen.
    fn reload(&self, rel: &Path) -> io::Result<(PathBuf, Arc<Vec<Mount>>)> {
        let mut table = self
            .table
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut state = match table.take() {
            Some(state) => state,
            None => Table {
                canonical_root: std::fs::canonicalize(&self.root)?,
                // Opened before the first read: a change after it is seen.
                watch: File::open(MOUNTINFO).ok(),
                mounts: Arc::new(Vec::new()),
            },
        };
        if let Some(watch) = state.watch.as_ref() {
            // Covered by the read below; a later change is reported anew.
            poll_changed(watch);
        }
        self.stale.store(false, Ordering::Release);
        let text = match state.watch.as_mut() {
            Some(watch) => {
                let mut text = Vec::new();
                watch.seek(SeekFrom::Start(0))?;
                watch.read_to_end(&mut text)?;
                text
            }
            None => std::fs::read(MOUNTINFO)?,
        };
        state.mounts = Arc::new(mountinfo::parse(&text));
        let answer = (state.canonical_root.join(rel), state.mounts.clone());
        *table = Some(state);
        Ok(answer)
    }
}

impl MountBoundaries {
    /// Whether the mount namespace changed since the table was read (or it
    /// cannot be told: then the table is read again, as without this cache).
    fn changed(&self, watch: Option<&File>) -> bool {
        let Some(watch) = watch else {
            return true;
        };
        let _polling = self
            .polling
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.stale.load(Ordering::Acquire) {
            return true;
        }
        let changed = poll_changed(watch);
        if changed {
            self.stale.store(true, Ordering::Release);
        }
        changed
    }
}

/// Whether the kernel reported a change of the mount namespace on `watch`
/// since the last poll (the report is consumed), or it cannot be told.
fn poll_changed(watch: &File) -> bool {
    let mut target = libc::pollfd {
        fd: watch.as_raw_fd(),
        events: libc::POLLPRI,
        revents: 0,
    };
    // SAFETY: one valid pollfd for the duration of the call; no waiting.
    // The kernel reports a namespace change once per open file.
    let ready = unsafe { libc::poll(&mut target, 1, 0) };
    ready < 0 || target.revents & (libc::POLLPRI | libc::POLLERR | libc::POLLNVAL) != 0
}
