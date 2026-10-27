//! The backend of a child of the device tree (DRV-M6, U17-15): it connects
//! at its first call, forgets a connection that broke, so the next call
//! connects again, and reports the capabilities of its last connection. A
//! device that cannot be reached answers `HostUnreachable` with the text
//! `Gerät nicht erreichbar`; the host start never waits for a device.
use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::device_tree_types::{ChildConnection, ChildLink};
use super::mount_source::PeerMountTarget;
use super::types::BackendRoot;
use crate::vfs::{
    Backend, BackendExtensions, BackendHandle, DeleteDisposition, Scheme, StageFinish,
    StageFinished, StagedWriteCapabilities, VfsMeta, VfsResult,
};

const UNREACHABLE: &str = "Gerät nicht erreichbar";

pub(super) fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn unreachable() -> io::Error {
    io::Error::new(io::ErrorKind::HostUnreachable, UNREACHABLE)
}

/// Connection losses the next call repairs by connecting again.
fn connection_lost(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::HostUnreachable
    )
}

#[derive(Clone, Copy)]
struct Facts {
    parallelism: usize,
    delete_disposition: DeleteDisposition,
    rename_overwrites: bool,
    case_sensitive: bool,
    staged: StagedWriteCapabilities,
}

impl Default for Facts {
    /// What is assumed before the first connection: the cautious values of
    /// the `Backend` trait.
    fn default() -> Self {
        Self {
            parallelism: 1,
            delete_disposition: DeleteDisposition::Permanent,
            rename_overwrites: false,
            case_sensitive: false,
            staged: StagedWriteCapabilities::default(),
        }
    }
}

/// A backend that connects at its first call and forgets a broken
/// connection; the capabilities are those of the last connection.
pub(super) struct LazyChildBackend {
    link: Arc<dyn ChildLink>,
    target: PeerMountTarget,
    root: BackendRoot,
    connection: Mutex<Option<BackendHandle>>,
    facts: Mutex<Facts>,
    /// 0 unknown, 1 the device refuses writes, 2 the device allows them.
    writable: AtomicU8,
}

impl LazyChildBackend {
    pub(super) fn new(
        link: Arc<dyn ChildLink>,
        target: PeerMountTarget,
        root: BackendRoot,
    ) -> Arc<Self> {
        Arc::new(Self {
            link,
            target,
            root,
            connection: Mutex::new(None),
            facts: Mutex::new(Facts::default()),
            writable: AtomicU8::new(0),
        })
    }

    /// A backend that starts with the connection the caller just made.
    pub(super) fn seeded(
        link: Arc<dyn ChildLink>,
        target: PeerMountTarget,
        root: BackendRoot,
        connection: ChildConnection,
    ) -> Arc<Self> {
        let backend = Self::new(link, target, root);
        let handle = backend.accept(connection);
        *guard(&backend.connection) = Some(handle);
        backend
    }

    /// The mode the last connection allows; `None` before the first one.
    pub(super) fn observed_writable(&self) -> Option<bool> {
        match self.writable.load(Ordering::Acquire) {
            0 => None,
            value => Some(value == 2),
        }
    }

    fn accept(&self, connection: ChildConnection) -> BackendHandle {
        let backend = connection.backend;
        *guard(&self.facts) = Facts {
            parallelism: backend.parallelism().max(1),
            delete_disposition: backend.delete_disposition(),
            rename_overwrites: backend.rename_overwrites(),
            case_sensitive: backend.case_sensitive_paths("/"),
            staged: backend.staged_write_capabilities("/"),
        };
        self.writable
            .store(if connection.writable { 2 } else { 1 }, Ordering::Release);
        backend
    }

    fn current(&self) -> io::Result<BackendHandle> {
        let mut slot = guard(&self.connection);
        if let Some(backend) = slot.as_ref() {
            return Ok(Arc::clone(backend));
        }
        let connection = self
            .link
            .connect(&self.target, &self.root)
            .map_err(|_| unreachable())?;
        let backend = self.accept(connection);
        *slot = Some(Arc::clone(&backend));
        Ok(backend)
    }

    fn forget(&self, lost: &BackendHandle) {
        let mut slot = guard(&self.connection);
        if slot.as_ref().is_some_and(|held| Arc::ptr_eq(held, lost)) {
            *slot = None;
        }
    }

    fn call<T>(&self, operation: impl FnOnce(&BackendHandle) -> io::Result<T>) -> io::Result<T> {
        let inner = self.current()?;
        let result = operation(&inner);
        if result
            .as_ref()
            .is_err_and(|error| connection_lost(error.kind()))
        {
            self.forget(&inner);
        }
        result
    }

    fn facts(&self) -> Facts {
        *guard(&self.facts)
    }
}

impl Backend for LazyChildBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn state_identity(&self) -> String {
        "device-tree-child".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.call(|inner| inner.list_dir(path))
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.call(|inner| inner.stat(path))
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        self.call(|inner| inner.try_exists(path))
    }

    fn item_id(&self, _path: &str) -> VfsResult<Option<String>> {
        Ok(None)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn std::io::Read + Send>> {
        self.call(|inner| inner.open_read(path))
    }

    fn open_read_at(
        &self,
        path: &str,
        _id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn std::io::Read + Send>>> {
        self.call(|inner| inner.open_read_at(path, None, offset))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.call(|inner| inner.open_write(path))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.call(|inner| inner.open_write_new(path))
    }

    fn download_name(&self, path: &str, name: &str) -> String {
        match self.current() {
            Ok(inner) => inner.download_name(path, name),
            Err(_) => name.to_string(),
        }
    }

    fn copy_file(&self, source: &str, destination: &str) -> VfsResult<u64> {
        self.call(|inner| inner.copy_file(source, destination))
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.call(|inner| inner.rename(source, destination))
    }

    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.call(|inner| inner.rename_no_replace(source, destination))
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.call(|inner| inner.promote_staged(staged, destination))
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.call(|inner| inner.promote_staged_no_replace(staged, destination))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.call(|inner| inner.remove_file(path))
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.call(|inner| inner.remove_dir(path))
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.call(|inner| inner.mkdir_all(path))
    }

    fn delete_disposition(&self) -> DeleteDisposition {
        self.facts().delete_disposition
    }

    fn parallelism(&self) -> usize {
        self.facts().parallelism
    }

    fn rename_overwrites(&self) -> bool {
        self.facts().rename_overwrites
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        self.facts().staged
    }

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        self.facts().case_sensitive
    }

    fn open_read_id(
        &self,
        path: &str,
        _id: Option<&str>,
    ) -> VfsResult<Box<dyn std::io::Read + Send>> {
        self.open_read(path)
    }

    fn remove_file_id(&self, path: &str, _id: Option<&str>) -> VfsResult<()> {
        self.remove_file(path)
    }

    fn is_local(&self) -> bool {
        false
    }

    fn provides_content_hash(&self) -> bool {
        false
    }

    fn invalidate_cache(&self) {
        if let Some(inner) = guard(&self.connection).clone() {
            inner.invalidate_cache();
        }
    }

    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
}

impl BackendExtensions for LazyChildBackend {
    /// The device's own backend hashes where the file is stored or answers
    /// `None`; a lost connection is forgotten like for every other call.
    fn content_digest(
        &self,
        path: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<crate::vfs::ContentDigest>> {
        self.call(|inner| crate::vfs::content_digest(&**inner, path, cancel))
    }
    fn capacity(&self, root: &str) -> VfsResult<crate::vfs::CapacitySnapshot> {
        self.call(|inner| crate::vfs::capacity(&**inner, root))
    }
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        self.call(|inner| crate::vfs::finish_stage(&**inner, stage, finish))
    }
}
