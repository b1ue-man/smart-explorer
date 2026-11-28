//! Connections an MCP session keeps open like the tabs of the explorer: one
//! slot per key (a saved login, an SSH login, the anchor of a Share mount,
//! the working root), opened by the first call that needs it and reused by
//! every later one. Concurrent first calls of one key wait for the single
//! open; different keys open in parallel. A connection that failed is
//! removed, so the next call opens it again exactly once.
use crate::vfs::{Backend, BackendHandle};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

type Slot<V> = Arc<Mutex<Option<V>>>;

pub(super) struct Slots<V> {
    slots: Mutex<HashMap<String, Slot<V>>>,
}

impl<V> Default for Slots<V> {
    fn default() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
        }
    }
}

impl<V: Clone> Slots<V> {
    /// The connection of `key`, opened with `open` when there is none; the
    /// flag tells whether this call opened it. A failed open keeps nothing.
    pub(super) fn get_or_open(
        &self,
        key: &str,
        open: impl FnOnce() -> Result<V, String>,
    ) -> Result<(V, bool), String> {
        let slot = self
            .slots
            .lock()
            .map_err(|_| poisoned())?
            .entry(key.to_string())
            .or_default()
            .clone();
        let mut current = slot.lock().map_err(|_| poisoned())?;
        if let Some(value) = current.as_ref() {
            return Ok((value.clone(), false));
        }
        let value = open()?;
        *current = Some(value.clone());
        Ok((value, true))
    }

    /// Removes every kept connection `matches` names; returns how many.
    pub(super) fn evict_where(&self, matches: impl Fn(&V) -> bool) -> usize {
        let slots: Vec<Slot<V>> = match self.slots.lock() {
            Ok(slots) => slots.values().cloned().collect(),
            Err(_) => return 0,
        };
        let mut evicted = 0;
        for slot in slots {
            // The lock of a slot is held only while it opens or hands out its
            // value; a slot still opening holds no failed connection.
            let Ok(mut current) = slot.lock() else {
                continue;
            };
            if current.as_ref().is_some_and(&matches) {
                *current = None;
                evicted += 1;
            }
        }
        evicted
    }

    /// Removes the connection of `key`.
    pub(super) fn evict(&self, key: &str) {
        let slot = match self.slots.lock() {
            Ok(slots) => slots.get(key).cloned(),
            Err(_) => None,
        };
        if let Some(slot) = slot {
            if let Ok(mut current) = slot.lock() {
                *current = None;
            }
        }
    }
}

fn poisoned() -> String {
    "connection cache is poisoned".to_string()
}

/// What a failed operation says about its connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    /// The connection itself is gone or unreachable.
    Connection,
    /// The file, folder, name or right was the problem; the connection works.
    Entry,
    /// Not decidable from the kind; a probe decides.
    Unknown,
}

pub(super) fn classify(kind: io::ErrorKind) -> Failure {
    use io::ErrorKind as K;
    match kind {
        K::ConnectionReset
        | K::ConnectionAborted
        | K::ConnectionRefused
        | K::NotConnected
        | K::BrokenPipe
        | K::UnexpectedEof
        | K::TimedOut
        | K::HostUnreachable
        | K::NetworkUnreachable
        | K::NetworkDown => Failure::Connection,
        K::NotFound
        | K::PermissionDenied
        | K::AlreadyExists
        | K::InvalidInput
        | K::InvalidData
        | K::InvalidFilename
        | K::DirectoryNotEmpty
        | K::IsADirectory
        | K::NotADirectory
        | K::ReadOnlyFilesystem
        | K::StorageFull
        | K::QuotaExceeded
        | K::FileTooLarge
        | K::CrossesDevices
        | K::Unsupported => Failure::Entry,
        _ => Failure::Unknown,
    }
}

/// Whether `backend` no longer reaches `probe`, a path that existed when the
/// connection was opened: any failure other than a missing entry or a
/// refused right counts as a broken connection.
pub(super) fn connection_broken(backend: &dyn Backend, probe: &str) -> bool {
    match backend.stat(probe) {
        Ok(_) => false,
        Err(error) => !matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
        ),
    }
}

/// The same connection object (data address only: vtable addresses of one
/// type may differ between codegen units).
pub(super) fn same_backend(left: &BackendHandle, right: &BackendHandle) -> bool {
    std::ptr::eq(
        Arc::as_ptr(left) as *const (),
        Arc::as_ptr(right) as *const (),
    )
}

#[cfg(test)]
#[path = "tab_cache_tests.rs"]
mod tests;
