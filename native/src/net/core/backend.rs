use std::io::{Read, Write};

use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};

use super::imp::NetConnection;

/// Local-filesystem semantics backed by a retained authenticated WNet lease.
/// Windows' SMB redirector owns wire keepalive/reconnect; retaining the lease
/// prevents Smart Explorer itself from cancelling the session while in use.
pub struct UncBackend {
    local: LocalBackend,
    connection: NetConnection,
}

struct UncReader {
    inner: Box<dyn Read + Send>,
    _connection: NetConnection,
}

impl Read for UncReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

struct UncWriter {
    inner: Box<dyn Write + Send>,
    _connection: NetConnection,
}

impl Write for UncWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl UncBackend {
    pub fn new(root: &str, connection: NetConnection) -> Self {
        Self {
            local: LocalBackend::new(root),
            connection,
        }
    }
}

impl Backend for UncBackend {
    fn scheme(&self) -> Scheme {
        self.local.scheme()
    }

    fn root_display(&self) -> String {
        self.local.root_display()
    }

    fn state_identity(&self) -> String {
        self.local.state_identity()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.local.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.local.stat(path)
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        self.local.try_exists(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(UncReader {
            inner: self.local.open_read(path)?,
            _connection: self.connection.clone(),
        }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(UncWriter {
            inner: self.local.open_write(path)?,
            _connection: self.connection.clone(),
        }))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(UncWriter {
            inner: self.local.open_write_new(path)?,
            _connection: self.connection.clone(),
        }))
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.local.copy_file(src, dst)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.local.rename(src, dst)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.local.rename_no_replace(src, dst)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.local.promote_staged(staged, destination)
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.local.promote_staged_no_replace(staged, destination)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.local.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.local.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.local.mkdir_all(path)
    }

    fn rename_overwrites(&self) -> bool {
        self.local.rename_overwrites()
    }

    fn staged_write_capabilities(&self, root: &str) -> crate::vfs::StagedWriteCapabilities {
        self.local.staged_write_capabilities(root)
    }

    fn is_local(&self) -> bool {
        self.local.is_local()
    }
}
