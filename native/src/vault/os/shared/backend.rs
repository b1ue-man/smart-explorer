//! The cleartext view of an unlocked Cryptomator vault (VLT-M2, C2): a
//! `vfs::Backend` layered over the backend that stores the vault, so every
//! se surface browses, reads and writes vaults on every storage se reaches
//! (local, SFTP, FTP/FTPS, SMB, WebDAV, Google Drive, Share peers).
//!
//! Paths are cleartext paths inside the vault (`/` = vault root). They map
//! to encrypted names in the content directories below `<root>/d/`; every
//! write goes into a stage of the storage and is published by the
//! storage's own promotion (C2.6). Capabilities follow C2.5 and are never
//! more generous than the storage's. After the session is locked every
//! operation, also of readers and writers already open, fails with
//! `VaultLocked` and publishes nothing (C3.2).
//!
//! Error texts never carry cleartext names (K24); errors of the storage
//! pass through with their encrypted paths.
use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::vault::core::path_map::storage_path;
use crate::vault::core::{LockListener, VaultSession};
use crate::vfs::{
    Backend, BackendExtensions, BackendHandle, DeleteDisposition, MountPathCapabilities,
    RootConfinement, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult,
};

use super::backend_write::Promote;
use super::dir_cache::DirCache;

/// One vault's cleartext view. Cheap to clone; clones share the session
/// and the directory cache.
#[derive(Clone)]
pub(crate) struct VaultBackend {
    /// The backend that stores the vault.
    pub(super) inner: BackendHandle,
    /// The vault root folder on `inner`.
    pub(super) root: Arc<str>,
    pub(super) session: Arc<VaultSession>,
    pub(super) cache: Arc<DirCache>,
}

/// Only a backend's explicit opt-in may replace its local I/O. Unknown
/// wrappers retain their permission, cancellation and provider behavior.
pub(super) fn confine_storage(inner: BackendHandle, root: &str) -> BackendHandle {
    let confined = inner
        .extensions()
        .and_then(|extension| extension.confined_local_root(root).ok().flatten());
    confined.unwrap_or(inner)
}

impl VaultBackend {
    /// The cleartext view of the vault whose root folder is `root` on
    /// `inner`, unlocked by `session` (contract of VLT-M2).
    pub(crate) fn new(inner: BackendHandle, root: &str, session: Arc<VaultSession>) -> Self {
        let inner = confine_storage(inner, root);
        let cache = Arc::new(DirCache::default());
        let listener: Arc<dyn LockListener> = cache.clone();
        session.watch(Arc::downgrade(&listener));
        Self {
            inner,
            root: Arc::from(root),
            session,
            cache,
        }
    }

    pub(crate) fn session(&self) -> &Arc<VaultSession> {
        &self.session
    }

    /// The backend that stores the vault and the vault root on it.
    pub(crate) fn storage(&self) -> (&BackendHandle, &str) {
        (&self.inner, &self.root)
    }

    /// `rel` (relative to the vault root) on the storage backend.
    pub(super) fn at(&self, rel: &str) -> String {
        storage_path(&self.root, rel)
    }
}

impl Backend for VaultBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Vault
    }

    /// Navigation starts at the vault root.
    fn root_display(&self) -> String {
        "/".to_string()
    }

    /// `vault:` + the storage's identity + `#` + vault root (C2.3): two
    /// vaults on one storage, or a vault and its storage, never share
    /// side-specific state.
    fn state_identity(&self) -> String {
        format!("vault:{}#{}", self.inner.state_identity(), self.root)
    }

    fn namespace_identity(&self) -> String {
        format!("vault:{}#{}", self.inner.namespace_identity(), self.root)
    }

    /// Own extensions: a layer that rewrites paths never hands out those
    /// of the storage (`vfs::BackendExtensions`).
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.list_strict(path)
    }

    fn has_duplicate_file_names(&self) -> bool {
        self.inner.has_duplicate_file_names()
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.stat_path(path)
    }

    /// The ID of the stored encrypted object.
    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        self.storage_item_id(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reader(path, None, false)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write_file(path, false)
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write_file(path, true)
    }

    fn open_write_copy_stage(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer(path, None, false)
    }

    fn promote_copy_stage(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote(staged, destination, Promote::Create)
    }

    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer(path, Some(size), false)
    }

    fn open_write_copy_stage_unsynced(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer(path, Some(size), true)
    }

    /// The storage copies the encrypted bytes itself (file contents are not
    /// bound to name or folder in format 8).
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<u64>> {
        self.copy_on_storage(src, stage, size, cancel)
    }

    /// Reads from any offset across chunk boundaries when the storage can
    /// start mid-file.
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        self.reader_at(path, id, offset)
    }

    /// Copies the encrypted bytes on the storage: no cleartext leaves the
    /// vault, not even into a spool (K08).
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.copy_within(src, dst)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_entry(src, dst, false)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_entry(src, dst, true)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote(staged, destination, Promote::Replace)
    }

    fn promote_staged_to_id(
        &self,
        staged: &str,
        destination: &str,
        id: Option<&str>,
    ) -> VfsResult<()> {
        self.promote(staged, destination, Promote::ToId(id.map(str::to_string)))
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote(staged, destination, Promote::Create)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.remove_entry(path, None)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.remove_folder(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.make_dirs(path)
    }

    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.make_dir(path, false)
    }

    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.make_dir(path, true)
    }

    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.discard_stage(stage)
    }

    /// An explicit refresh also forgets folder IDs and display names.
    fn invalidate_cache(&self) {
        self.cache.clear();
    }

    /// Removal is final; there is no trash inside a vault.
    fn delete_disposition(&self) -> DeleteDisposition {
        DeleteDisposition::Permanent
    }

    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }

    /// The storage's connection (C2.3: `flow_key` = inner).
    fn flow_key(&self, _path: &str) -> String {
        self.inner.flow_key(&self.root)
    }

    fn transfer_ceiling(&self, _path: &str) -> Option<usize> {
        self.inner.transfer_ceiling(&self.root)
    }

    fn concurrent_read_write(&self) -> bool {
        self.inner.concurrent_read_write()
    }

    /// A rename never replaces: names map to separate stored entries.
    fn rename_overwrites(&self) -> bool {
        false
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        self.inner.staged_write_capabilities(&self.root)
    }

    fn probe_staged_write_capabilities(&self, _root: &str) -> VfsResult<StagedWriteCapabilities> {
        self.inner.probe_staged_write_capabilities(&self.root)
    }

    /// Cleartext names of different letter case encrypt to different
    /// stored names on every storage.
    fn case_sensitive_paths(&self, _root: &str) -> bool {
        true
    }

    fn root_confinement(&self, root: &str) -> RootConfinement {
        // The retained authority is the complete vault storage root. A
        // cleartext subdirectory has no independent held storage authority.
        if crate::vault::core::path_map::components(root).is_ok_and(|parts| parts.is_empty()) {
            self.inner.root_confinement(&self.root)
        } else {
            RootConfinement::Unverified
        }
    }

    fn mount_path_capabilities(&self, root: &str) -> VfsResult<MountPathCapabilities> {
        let mut capabilities = self.inner.mount_path_capabilities(&self.root)?;
        capabilities.root_confinement = self.root_confinement(root);
        Ok(capabilities)
    }

    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.reader(path, id, false)
    }

    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        self.remove_entry(path, id)
    }
}
