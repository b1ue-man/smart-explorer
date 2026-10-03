//! Reading and applying inotify events (RV1, V4): the kernel's records are
//! parsed unaligned from the read buffer, mapped to changes below every root
//! that watches the directory, and new or moved directories are watched or
//! dropped so the tree stays complete and paths stay current.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use super::super::paths::{is_own, join_rel};
use super::super::service::emit;
use super::super::types::{Change, EventKind, UnavailableReason, WatchEntry, WatchEvent, WatchId};
use super::{Tree, WalkError};

impl Tree {
    /// Reads every queued event and hands them to the consumers.
    pub(crate) fn read_events(&mut self) {
        let Some(fd) = self.raw_fd() else {
            return;
        };
        let mut out: HashMap<WatchId, Vec<WatchEvent>> = HashMap::new();
        let capacity = self.buffer.len() * std::mem::size_of::<u64>();
        loop {
            // SAFETY: the buffer is writable for `capacity` bytes.
            let read = unsafe { libc::read(fd, self.buffer.as_mut_ptr().cast(), capacity) };
            let Ok(length) = usize::try_from(read) else {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.kind() != io::ErrorKind::WouldBlock {
                    self.overflow_all(&mut out);
                }
                break;
            };
            if length == 0 {
                break;
            }
            // SAFETY: `length` bytes of the buffer were just written.
            let bytes =
                unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), length) }
                    .to_vec();
            self.parse(&bytes, &mut out);
        }
        for (id, events) in out {
            emit(id, events);
        }
    }

    fn parse(&mut self, bytes: &[u8], out: &mut HashMap<WatchId, Vec<WatchEvent>>) {
        let header = std::mem::size_of::<libc::inotify_event>();
        let mut offset = 0;
        while offset + header <= bytes.len() {
            // SAFETY: a whole header lies in the buffer; read unaligned.
            let event: libc::inotify_event =
                unsafe { std::ptr::read_unaligned(bytes.as_ptr().add(offset).cast()) };
            let start = offset + header;
            let Some(end) = usize::try_from(event.len)
                .ok()
                .and_then(|length| start.checked_add(length))
                .filter(|end| *end <= bytes.len())
            else {
                break;
            };
            let raw = &bytes[start..end];
            let name = &raw[..raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len())];
            self.handle(event.wd, event.mask, OsStr::from_bytes(name), out);
            offset = end;
        }
    }

    fn handle(
        &mut self,
        wd: i32,
        mask: u32,
        name: &OsStr,
        out: &mut HashMap<WatchId, Vec<WatchEvent>>,
    ) {
        if mask & libc::IN_Q_OVERFLOW != 0 {
            self.overflow_all(out);
            return;
        }
        let Some(dir) = self.dirs.get(&wd) else {
            return;
        };
        let users: Vec<(WatchId, String)> = dir
            .users
            .iter()
            .map(|(id, rel)| (*id, rel.clone()))
            .collect();
        let dir_path = dir.path.clone();
        if mask & libc::IN_IGNORED != 0 {
            self.dirs.remove(&wd);
            for (id, rel) in &users {
                if let Some(root) = self.roots.get_mut(id) {
                    root.wds.remove(&wd);
                }
                if rel.is_empty() {
                    self.fail(*id, UnavailableReason::RootMissing);
                }
            }
            return;
        }
        if mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_UNMOUNT) != 0 {
            for (id, rel) in &users {
                if rel.is_empty() {
                    self.fail(*id, UnavailableReason::RootMissing);
                }
            }
            return;
        }
        let Some(kind) = kind_of(mask) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let is_dir = mask & libc::IN_ISDIR != 0;
        let display = name.to_string_lossy();
        for (id, dir_rel) in users {
            let rel = join_rel(&dir_rel, &display);
            if is_dir && mask & libc::IN_MOVED_FROM != 0 {
                self.drop_subtree(id, &rel);
            }
            if is_dir && mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 {
                self.extend(id, dir_path.join(name), rel.clone());
            }
            out.entry(id).or_default().push(WatchEvent::Change(Change {
                rel,
                kind,
                is_dir: Some(is_dir),
            }));
        }
    }

    /// A directory appeared (created or moved in): watch it and below.
    fn extend(&mut self, id: WatchId, path: PathBuf, rel: String) {
        let Some(root) = self.roots.get(&id) else {
            return;
        };
        // Confined roots deliberately expose only the held directory. A
        // path-based child walk could follow a replaced intermediate name.
        if root.spec.anchor.is_some() {
            return;
        }
        if is_own(&path, &self.own)
            || !root.spec.filter.admits(&WatchEntry {
                rel: &rel,
                is_dir: Some(true),
            })
        {
            return;
        }
        let cross_mounts = root.spec.options.cross_mounts;
        let dev = root.dev;
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && (cross_mounts || metadata.dev() == dev) => {}
            _ => return,
        }
        if let Err(WalkError::Limit) = self.walk(id, path, rel) {
            self.fail(id, UnavailableReason::WatchLimit);
        }
    }

    /// A directory moved away: its watches would report under a stale path.
    fn drop_subtree(&mut self, id: WatchId, rel: &str) {
        let prefix = format!("{rel}/");
        let stale: Vec<i32> = self
            .dirs
            .iter()
            .filter(|(_, dir)| {
                dir.users
                    .get(&id)
                    .is_some_and(|dir_rel| dir_rel == rel || dir_rel.starts_with(&prefix))
            })
            .map(|(wd, _)| *wd)
            .collect();
        for wd in stale {
            if let Some(root) = self.roots.get_mut(&id) {
                root.wds.remove(&wd);
            }
            self.release(id, wd);
        }
    }

    fn overflow_all(&mut self, out: &mut HashMap<WatchId, Vec<WatchEvent>>) {
        // A lost CREATE/MOVE directory event also means a missing kernel
        // watch. Recreate the whole instance, not just the consumer baseline.
        self.fd = None;
        self.dirs.clear();
        let now = std::time::Instant::now();
        for (id, root) in &mut self.roots {
            root.wds.clear();
            if root.retry_at.is_none() && root.reported.is_none() {
                out.entry(*id).or_default().push(WatchEvent::Overflow);
                root.retry_at = Some(now);
            }
        }
    }
}

fn kind_of(mask: u32) -> Option<EventKind> {
    Some(if mask & libc::IN_CREATE != 0 {
        EventKind::Created
    } else if mask & libc::IN_DELETE != 0 {
        EventKind::Removed
    } else if mask & libc::IN_MOVED_FROM != 0 {
        EventKind::RenamedFrom
    } else if mask & libc::IN_MOVED_TO != 0 {
        EventKind::RenamedTo
    } else if mask & (libc::IN_MODIFY | libc::IN_CLOSE_WRITE) != 0 {
        EventKind::Modified
    } else if mask & libc::IN_ATTRIB != 0 {
        EventKind::Metadata
    } else {
        return None;
    })
}
