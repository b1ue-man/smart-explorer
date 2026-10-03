//! The inotify watches of all roots (Linux, Android): one instance for the
//! process, one watch per directory (`IN_ONLYDIR | IN_DONT_FOLLOW |
//! IN_EXCL_UNLINK`, links never followed), shared by roots that overlap.
//! A new directory is watched first and read afterwards, so files created in
//! between are not missed. Directories the consumer filter excludes, the
//! app's own directories and (without `cross_mounts`) other file systems are
//! not watched. The watch limit (`ENOSPC`, `EMFILE`) ends the root's watch
//! with `Unavailable(WatchLimit)`; the root is armed again with backoff.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::fs_kind::coverage;
use super::paths::{is_own, join_rel};
use super::service::{emit, own_directories, RootSpec};
use super::types::{UnavailableReason, WatchEntry, WatchEvent, WatchId};

const MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MODIFY
    | libc::IN_CLOSE_WRITE
    | libc::IN_ATTRIB
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_ONLYDIR
    | libc::IN_DONT_FOLLOW
    | libc::IN_EXCL_UNLINK;
/// A missing root (unplugged, unmounted) is looked for again this often; a
/// `stat` costs nothing.
const MISSING_RETRY: Duration = Duration::from_secs(10);
/// Backoff for limits and errors, like Syncthing: one minute, doubled up to
/// an hour.
const FIRST_BACKOFF: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(3_600);
/// Read buffer: many events per `read`, far above the kernel minimum of one
/// event with a maximal name.
const READ_WORDS: usize = 8_192;

#[path = "inotify_confined.rs"]
mod confined;
#[path = "inotify_events.rs"]
mod events;

struct Dir {
    path: PathBuf,
    /// Path of this directory below each root that uses it.
    users: HashMap<WatchId, String>,
}

struct Root {
    spec: RootSpec,
    dev: u64,
    wds: HashSet<i32>,
    retry_at: Option<Instant>,
    backoff: Duration,
    reported: Option<UnavailableReason>,
}

enum WalkError {
    Limit,
    /// The root itself cannot be watched.
    Root(UnavailableReason),
}

pub(super) struct Tree {
    fd: Option<OwnedFd>,
    dirs: HashMap<i32, Dir>,
    roots: HashMap<WatchId, Root>,
    own: Vec<PathBuf>,
    buffer: Vec<u64>,
}

impl Tree {
    pub(super) fn new() -> Self {
        Self {
            fd: None,
            dirs: HashMap::new(),
            roots: HashMap::new(),
            own: own_directories(),
            buffer: vec![0; READ_WORDS],
        }
    }

    pub(super) fn raw_fd(&self) -> Option<RawFd> {
        self.fd.as_ref().map(AsRawFd::as_raw_fd)
    }

    pub(super) fn add(&mut self, spec: RootSpec) {
        let id = spec.id;
        self.roots.insert(
            id,
            Root {
                spec,
                dev: 0,
                wds: HashSet::new(),
                retry_at: None,
                backoff: FIRST_BACKOFF,
                reported: None,
            },
        );
        self.arm(id);
    }

    pub(super) fn remove(&mut self, id: WatchId) {
        self.disarm(id);
        self.roots.remove(&id);
    }

    /// Roots whose retry time has come are armed again.
    pub(super) fn rearm_due(&mut self, now: Instant) {
        let due: Vec<WatchId> = self
            .roots
            .iter()
            .filter(|(_, root)| root.retry_at.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.arm(id);
        }
    }

    pub(super) fn next_retry(&self) -> Option<Instant> {
        self.roots.values().filter_map(|root| root.retry_at).min()
    }

    fn instance(&mut self) -> Result<RawFd, UnavailableReason> {
        if let Some(fd) = self.raw_fd() {
            return Ok(fd);
        }
        // SAFETY: plain system call; the result is checked below.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(libc::EMFILE) | Some(libc::ENFILE) => UnavailableReason::WatchLimit,
                _ => UnavailableReason::Failed(format!("inotify: {error}")),
            });
        }
        // SAFETY: `fd` is a fresh descriptor owned by nobody else.
        self.fd = Some(unsafe { OwnedFd::from_raw_fd(fd) });
        Ok(fd)
    }

    fn arm(&mut self, id: WatchId) {
        self.disarm(id);
        let Some(root) = self.roots.get(&id) else {
            return;
        };
        if let Some(anchor) = root.spec.anchor.clone() {
            return self.arm_confined(id, anchor);
        }
        let path = root.spec.root.clone();
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => metadata,
            Ok(_) => return self.fail(id, UnavailableReason::Unsupported),
            Err(error) => return self.fail(id, reason_of(&error)),
        };
        if let Some(root) = self.roots.get_mut(&id) {
            root.dev = metadata.dev();
        }
        match self.walk(id, path.clone(), String::new()) {
            Ok(()) => {
                if let Some(root) = self.roots.get_mut(&id) {
                    root.retry_at = None;
                    root.backoff = FIRST_BACKOFF;
                    root.reported = None;
                }
                let kind = coverage(&path);
                // A mount attached later beneath an armed local root produces
                // no inotify mount event. Cross-mount jobs retain a poll so
                // such a subtree cannot silently lose coverage.
                let kind = if kind == super::types::Coverage::Complete
                    && self
                        .roots
                        .get(&id)
                        .is_some_and(|root| root.spec.options.cross_mounts)
                {
                    super::types::Coverage::LocalOnly
                } else {
                    kind
                };
                emit(id, vec![WatchEvent::Ready(kind)]);
            }
            Err(WalkError::Limit) => self.fail(id, UnavailableReason::WatchLimit),
            Err(WalkError::Root(reason)) => self.fail(id, reason),
        }
    }

    fn fail(&mut self, id: WatchId, reason: UnavailableReason) {
        self.disarm(id);
        let Some(root) = self.roots.get_mut(&id) else {
            return;
        };
        let now = Instant::now();
        root.retry_at = match reason {
            UnavailableReason::Unsupported => None,
            UnavailableReason::RootMissing | UnavailableReason::DeviceRemoved => {
                Some(now + MISSING_RETRY)
            }
            _ => {
                let wait = root.backoff;
                root.backoff = (wait * 2).min(MAX_BACKOFF);
                Some(now + wait)
            }
        };
        if root.reported.as_ref() != Some(&reason) {
            root.reported = Some(reason.clone());
            emit(id, vec![WatchEvent::Unavailable(reason)]);
        }
    }

    fn disarm(&mut self, id: WatchId) {
        let Some(root) = self.roots.get_mut(&id) else {
            return;
        };
        let wds: Vec<i32> = root.wds.drain().collect();
        for wd in wds {
            self.release(id, wd);
        }
    }

    /// Drops one root's use of a watch; the last user removes it.
    fn release(&mut self, id: WatchId, wd: i32) {
        let Some(dir) = self.dirs.get_mut(&wd) else {
            return;
        };
        dir.users.remove(&id);
        if dir.users.is_empty() {
            self.dirs.remove(&wd);
            if let Some(fd) = self.raw_fd() {
                // SAFETY: removes a watch of our own instance.
                unsafe { libc::inotify_rm_watch(fd, wd as _) };
            }
        }
    }

    /// Watches `start` and every admitted directory below it for root `id`.
    fn walk(&mut self, id: WatchId, start: PathBuf, start_rel: String) -> Result<(), WalkError> {
        let Some(root) = self.roots.get(&id) else {
            return Ok(());
        };
        let filter = root.spec.filter.clone();
        let cross_mounts = root.spec.options.cross_mounts;
        let dev = root.dev;
        let mut stack = vec![(start, start_rel)];
        while let Some((dir, rel)) = stack.pop() {
            let is_root = rel.is_empty();
            match self.add_dir(id, &dir, &rel) {
                Ok(()) => {}
                Err(AddError::Limit) => return Err(WalkError::Limit),
                Err(AddError::Other(reason)) if is_root => return Err(WalkError::Root(reason)),
                // A directory that vanished or cannot be read is skipped; the
                // sync walk reports it as an omission.
                Err(AddError::Other(UnavailableReason::RootMissing)) => continue,
                Err(AddError::Other(reason)) => return Err(WalkError::Root(reason)),
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(WalkError::Root(reason_of(&error))),
            };
            for entry in entries.flatten() {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                // `DirEntry::metadata` does not follow links: links stay
                // protected omissions and are never entered.
                if !metadata.is_dir() || (!cross_mounts && metadata.dev() != dev) {
                    continue;
                }
                let child = entry.path();
                let child_rel = join_rel(&rel, &entry.file_name().to_string_lossy());
                if is_own(&child, &self.own)
                    || !filter.admits(&WatchEntry {
                        rel: &child_rel,
                        is_dir: Some(true),
                    })
                {
                    continue;
                }
                stack.push((child, child_rel));
            }
        }
        Ok(())
    }

    fn add_dir(&mut self, id: WatchId, dir: &Path, rel: &str) -> Result<(), AddError> {
        let fd = self.instance().map_err(|reason| match reason {
            UnavailableReason::WatchLimit => AddError::Limit,
            other => AddError::Other(other),
        })?;
        let path = CString::new(dir.as_os_str().as_bytes())
            .map_err(|_| AddError::Other(UnavailableReason::Unsupported))?;
        // SAFETY: NUL-terminated path, descriptor of our own instance.
        let wd = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), MASK) };
        if wd < 0 {
            let error = io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(libc::ENOSPC) | Some(libc::EMFILE) => AddError::Limit,
                _ => AddError::Other(reason_of(&error)),
            });
        }
        let entry = self.dirs.entry(wd).or_insert_with(|| Dir {
            path: dir.to_path_buf(),
            users: HashMap::new(),
        });
        // The kernel returns the existing watch of a directory; its current
        // path wins (it may have been renamed meanwhile).
        // A confined watch shares the kernel wd with overlapping job roots,
        // but its /proc spelling must not replace a job's traversal path.
        if self
            .roots
            .get(&id)
            .is_some_and(|root| root.spec.anchor.is_none())
        {
            entry.path = dir.to_path_buf();
        }
        entry.users.insert(id, rel.to_string());
        if let Some(root) = self.roots.get_mut(&id) {
            root.wds.insert(wd);
        }
        Ok(())
    }
}

enum AddError {
    Limit,
    Other(UnavailableReason),
}

fn reason_of(error: &io::Error) -> UnavailableReason {
    match error.kind() {
        io::ErrorKind::NotFound => UnavailableReason::RootMissing,
        io::ErrorKind::PermissionDenied => UnavailableReason::AccessDenied,
        _ if error.raw_os_error() == Some(libc::ENOTDIR) => UnavailableReason::RootMissing,
        _ => UnavailableReason::Failed(error.to_string()),
    }
}
