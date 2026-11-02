//! Fake backends of the links batch tests (`links_task_tests`).
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A local backend without an atomic no-replace rename, like FTP, Google
/// Drive and ZIP archives.
#[derive(Default)]
pub(super) struct NoAtomicRename {
    pub(super) refused: AtomicUsize,
}

impl Backend for NoAtomicRename {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        LocalBackend::new("/").list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        LocalBackend::new("/").stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        LocalBackend::new("/").open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn std::io::Write + Send>> {
        LocalBackend::new("/").open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn std::io::Write + Send>> {
        LocalBackend::new("/").open_write_new(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        LocalBackend::new("/").rename(src, dst)
    }
    fn rename_no_replace(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        self.refused.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "backend has no atomic no-replace rename",
        ))
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        LocalBackend::new("/").promote_staged(staged, destination)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        LocalBackend::new("/").promote_staged_no_replace(staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").mkdir_all(path)
    }
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").create_dir(path)
    }
}

/// A remote with a link to a file (`/srv/link`) and a link to a directory
/// (`/srv/dirlink`): links show as links, reads and listings follow them.
pub(super) struct RemoteLinks;

impl Backend for RemoteLinks {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match path {
            "/srv/dirlink" => Ok(Vec::new()),
            _ => Err(io::ErrorKind::NotADirectory.into()),
        }
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let name = path.rsplit('/').next().unwrap_or_default();
        match path {
            "/srv/link" | "/srv/dirlink" => Ok(VfsMeta {
                name: name.to_string(),
                is_symlink: true,
                size: 9,
                mtime_ms: 1,
                ..VfsMeta::default()
            }),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        match path {
            "/srv/link" => Ok(Box::new(io::Cursor::new(b"through the link".to_vec()))),
            _ => Err(io::Error::other("Failure")),
        }
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn std::io::Write + Send>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
