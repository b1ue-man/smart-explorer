//! Test vaults of the vault backend tests: a fresh format-8 structure in a
//! temporary folder on the local backend, its session, helpers that look at
//! the stored names, and storage backends that record or pin capabilities.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use crate::cryptomator::{content_dir, encrypt_name, CipherCombo, Masterkey, VaultConfig};
use crate::vault::core::path_map::components;
use crate::vault::core::VaultSession;
use crate::vfs::{
    Backend, BackendHandle, LocalBackend, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult,
};

use super::backend::VaultBackend;

pub(super) const SITE: &str = "0123456789abcdef0123456789abcdef";

pub(super) fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A vault in a temporary folder.
pub(super) struct TestVault {
    pub(super) dir: tempfile::TempDir,
    /// The vault root (forward slashes).
    pub(super) root: String,
    pub(super) raw: [u8; 64],
    pub(super) config: VaultConfig,
    pub(super) session: Arc<VaultSession>,
    pub(super) backend: VaultBackend,
}

impl TestVault {
    pub(super) fn new(combo: CipherCombo, threshold: u32) -> Self {
        Self::on(combo, threshold, |_| -> BackendHandle {
            Arc::new(LocalBackend::new("/"))
        })
    }

    /// A vault whose storage backend `storage` builds from the vault root.
    pub(super) fn on(
        combo: CipherCombo,
        threshold: u32,
        storage: impl FnOnce(&str) -> BackendHandle,
    ) -> Self {
        let dir = tempfile::tempdir().expect("temporary folder");
        let root = format!("{}/vault", fwd(dir.path()));
        let raw: [u8; 64] = std::array::from_fn(|index| (index as u8).wrapping_mul(29) ^ 0x5a);
        let config = VaultConfig {
            jti: "4f2c1f0e-test-vault".to_string(),
            combo,
            shortening_threshold: threshold,
        };
        let key = Masterkey::from_raw(&raw);
        let root_content = content_dir(&key, "").expect("root content directory");
        std::fs::create_dir_all(format!("{root}/{root_content}")).expect("vault structure");
        let session = VaultSession::new(SITE, key, config.clone());
        let backend = VaultBackend::new(storage(&root), &root, Arc::clone(&session));
        Self {
            dir,
            root,
            raw,
            config,
            session,
            backend,
        }
    }

    /// The same vault through a second session with the same key (what a
    /// new unlock gives).
    pub(super) fn reopened(&self) -> VaultBackend {
        let session = VaultSession::new(SITE, Masterkey::from_raw(&self.raw), self.config.clone());
        VaultBackend::new(Arc::new(LocalBackend::new("/")), &self.root, session)
    }

    pub(super) fn write(&self, path: &str, data: &[u8]) {
        write_with(&self.backend, path, data);
    }

    pub(super) fn read(&self, path: &str) -> Vec<u8> {
        read_with(&self.backend, path)
    }

    /// The content directory (vault-root-relative) of a cleartext folder.
    pub(super) fn content(&self, folder: &str) -> String {
        let all = components(folder).expect("folder path");
        self.backend.dir_for(&all).expect("folder").content
    }

    /// The directory ID of a cleartext folder.
    pub(super) fn dir_id(&self, folder: &str) -> String {
        let all = components(folder).expect("folder path");
        self.backend.dir_for(&all).expect("folder").id
    }

    /// The encrypted name of `name` in the cleartext folder `folder`.
    pub(super) fn encrypted(&self, folder: &str, name: &str) -> String {
        let id = self.dir_id(folder);
        let key = Masterkey::from_raw(&self.raw);
        encrypt_name(&key, name, &id).expect("encrypted name")
    }

    /// Stored names in a vault-root-relative storage folder, sorted.
    pub(super) fn stored(&self, rel: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(format!("{}/{rel}", self.root))
            .expect("storage folder")
            .map(|entry| {
                entry
                    .expect("storage entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// The storage path of a vault-root-relative path.
    pub(super) fn at(&self, rel: &str) -> String {
        format!("{}/{rel}", self.root)
    }
}

pub(super) fn write_with(backend: &dyn Backend, path: &str, data: &[u8]) {
    let mut writer = backend.open_write(path).expect("open_write");
    writer.write_all(data).expect("write");
    writer.flush().expect("flush publishes");
}

pub(super) fn read_with(backend: &dyn Backend, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    backend
        .open_read(path)
        .expect("open_read")
        .read_to_end(&mut bytes)
        .expect("read");
    bytes
}

/// A deterministic pattern of `length` bytes.
pub(super) fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

/// The local backend, recording every changing call as `"<op> <rel>"`
/// (paths relative to the vault root).
pub(super) struct Recording {
    inner: LocalBackend,
    root: Mutex<String>,
    pub(super) log: Mutex<Vec<String>>,
}

impl Recording {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: LocalBackend::new("/"),
            root: Mutex::new(String::new()),
            log: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn set_root(&self, root: &str) {
        *self.root.lock().unwrap_or_else(PoisonError::into_inner) = format!("{root}/");
    }

    fn note(&self, operation: &str, path: &str) {
        let root = self
            .root
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let rel = path.strip_prefix(&root).unwrap_or(path);
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{operation} {rel}"));
    }

    pub(super) fn entries(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Backend for Recording {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.note("write", path);
        self.inner.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.note("write", path);
        self.inner.open_write_new(path)
    }
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.note("write", path);
        self.inner.open_write_copy_stage_sized(path, size)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.note("rename", dst);
        self.inner.rename(src, dst)
    }
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.note("rename", dst);
        self.inner.rename_no_replace(src, dst)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.note("promote", destination);
        self.inner.promote_staged(staged, destination)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.note("promote", destination);
        self.inner.promote_staged_no_replace(staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.note("remove", path);
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.note("rmdir", path);
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.note("mkdir", path);
        self.inner.mkdir_all(path)
    }
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.note("mkdir", path);
        self.inner.create_dir(path)
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.note("mkdir", path);
        self.inner.create_dir_new(path)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn staged_write_capabilities(&self, root: &str) -> StagedWriteCapabilities {
        self.inner.staged_write_capabilities(root)
    }
}
