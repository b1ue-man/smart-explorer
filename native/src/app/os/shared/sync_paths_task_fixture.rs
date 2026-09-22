//! Contract fixture: distinct backend namespaces mapped to isolated directories.
use crate::vfs::{Backend, BackendHandle, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) fn forward(path: &Path) -> String { path.to_string_lossy().replace('\\', "/") }

pub(super) struct Location {
    pub backend: BackendHandle,
    pub root: String,
    pub disk: PathBuf,
    _temp: tempfile::TempDir,
}

impl Location {
    pub fn new(scheme: Scheme) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let disk = temp.path().join("Docs Ü %20 #");
        std::fs::create_dir(&disk).unwrap();
        let (backend, root): (BackendHandle, String) = if scheme == Scheme::Local {
            let root = forward(&disk);
            (Arc::new(LocalBackend::new(&root)), root)
        } else {
            (Arc::new(MappedBackend { base: temp.path().into(), scheme, inner: LocalBackend::new("/") }),
                "/Docs Ü %20 #".into())
        };
        Self { backend, root, disk, _temp: temp }
    }
}

struct MappedBackend {
    base: PathBuf,
    scheme: Scheme,
    inner: LocalBackend,
}

impl MappedBackend {
    fn path(&self, path: &str) -> String {
        assert!(path.starts_with('/'), "backend received a locator instead of a path: {path}");
        assert!(!path.contains("://"), "URL leaked into backend I/O: {path}");
        assert!(!path.split('/').any(|part| part == ".."));
        forward(&self.base.join(path.trim_start_matches('/')))
    }
}

impl Backend for MappedBackend {
    fn scheme(&self) -> Scheme { self.scheme }
    fn root_display(&self) -> String { "/".into() }
    fn state_identity(&self) -> String { format!("{:?}:{}", self.scheme, forward(&self.base)) }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> { self.inner.list_dir(&self.path(path)) }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> { self.inner.stat(&self.path(path)) }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> { self.inner.open_read(&self.path(path)) }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> { self.inner.open_write(&self.path(path)) }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> { self.inner.open_write_new(&self.path(path)) }
    fn rename(&self, a: &str, b: &str) -> VfsResult<()> { self.inner.rename(&self.path(a), &self.path(b)) }
    fn rename_no_replace(&self, a: &str, b: &str) -> VfsResult<()> { self.inner.rename_no_replace(&self.path(a), &self.path(b)) }
    fn promote_staged(&self, a: &str, b: &str) -> VfsResult<()> { self.inner.promote_staged(&self.path(a), &self.path(b)) }
    fn promote_staged_no_replace(&self, a: &str, b: &str) -> VfsResult<()> { self.inner.promote_staged_no_replace(&self.path(a), &self.path(b)) }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> { self.inner.mkdir_all(&self.path(path)) }
    fn remove_file(&self, path: &str) -> VfsResult<()> { self.inner.remove_file(&self.path(path)) }
    fn remove_dir(&self, path: &str) -> VfsResult<()> { self.inner.remove_dir(&self.path(path)) }
}

pub(super) fn remote(backend: BackendHandle, prefix: &str) -> crate::connect::RemoteState {
    crate::connect::RemoteState {
        backend, label: prefix.into(), agent_version: None, zip_return: None,
        sftp: None, account: None, endpoint_prefix: Some(prefix.into()),
    }
}
