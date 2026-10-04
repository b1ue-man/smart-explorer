//! Fault-injecting adapters retain the actual local stage and flush contract.
use crate::vfs::{
    Backend, BackendExtensions, LocalBackend, Scheme, StageFinish, StageFinished, TargetLimits,
    VfsListing, VfsMeta, VfsResult, VolumeIdentity,
};
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub(super) struct WriteFail<'a> {
    pub(super) inner: &'a LocalBackend,
    pub(super) needle: &'a str,
}

pub(super) struct StatFail<'a> {
    pub(super) inner: &'a LocalBackend,
    pub(super) needle: &'a str,
}

pub(super) struct DeleteProbe {
    pub(super) root: String,
    pub(super) local: bool,
    pub(super) removes: Arc<AtomicUsize>,
}

impl DeleteProbe {
    fn target(&self) -> String {
        format!("{}/target.txt", self.root)
    }

    fn present(&self, path: &str) -> bool {
        path == self.target() && self.removes.load(Ordering::Relaxed) == 0
    }
}

impl Backend for DeleteProbe {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }

    fn root_display(&self) -> String {
        "delete-probe".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path != self.root {
            return Err(io::Error::new(io::ErrorKind::NotFound, "missing directory"));
        }
        if self.present(&self.target()) {
            Ok(vec![self.stat(&self.target())?])
        } else {
            Ok(Vec::new())
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path == self.root {
            return LocalBackend::new(&self.root).stat(path);
        }
        if !self.present(path) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "missing"));
        }
        Ok(VfsMeta {
            name: "target.txt".into(),
            size: 1,
            ..Default::default()
        })
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        if self.present(path) {
            Ok(Box::new(Cursor::new(vec![0u8])))
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "missing"))
        }
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked"))
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "rename"))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        if !self.present(path) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "missing"));
        }
        self.removes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "remove dir"))
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "mkdir"))
    }

    fn is_local(&self) -> bool {
        self.local
    }
}

impl Backend for WriteFail<'_> {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
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
        if path.contains(self.needle) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "write blocked",
            ))
        } else {
            self.inner.open_write(path)
        }
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        if path.contains(self.needle) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "write blocked",
            ))
        } else {
            self.inner.open_write_new(path)
        }
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        if dst.contains(self.needle) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no-replace commit blocked",
            ))
        } else {
            self.inner.rename_no_replace(src, dst)
        }
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }

    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.inner.discard_copy_stage(stage)
    }

    fn is_local(&self) -> bool {
        self.inner.is_local()
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}

impl Backend for StatFail<'_> {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path.contains(self.needle) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "stat blocked",
            ))
        } else {
            self.inner.stat(path)
        }
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(src, dst)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }

    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.inner.discard_copy_stage(stage)
    }

    fn is_local(&self) -> bool {
        self.inner.is_local()
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}

impl BackendExtensions for WriteFail<'_> {
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        crate::vfs::list_dir_tolerant(self.inner, path)
    }

    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        crate::vfs::open_read_regular(self.inner, path, id)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        crate::vfs::finish_stage(self.inner, stage, finish)
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        crate::vfs::sync_filesystem(self.inner, root)
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        crate::vfs::target_limits(self.inner, root)
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        crate::vfs::unix_mode(self.inner, path)
    }

    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        crate::vfs::volume_identity(self.inner, root)
    }
}

impl BackendExtensions for StatFail<'_> {
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        crate::vfs::list_dir_tolerant(self.inner, path)
    }

    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        crate::vfs::open_read_regular(self.inner, path, id)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        crate::vfs::finish_stage(self.inner, stage, finish)
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        crate::vfs::sync_filesystem(self.inner, root)
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        crate::vfs::target_limits(self.inner, root)
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        crate::vfs::unix_mode(self.inner, path)
    }

    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        crate::vfs::volume_identity(self.inner, root)
    }
}
