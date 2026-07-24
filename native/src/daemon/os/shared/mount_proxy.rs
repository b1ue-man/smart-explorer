use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::vfs::{
    Backend, BackendHandle, DeleteDisposition, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult,
};

use super::ipc_protocol::MountBackendCapabilities;
use super::mount_request_gate::{MountRequestGate, MountRequestPermit};

/// Mount-only client wrapper around AgentBackend. It restores the sanitized
/// error kinds flattened by the generic framing layer and exposes only the
/// capabilities that the rooted daemon backend actually declared.
pub(super) fn wrap(inner: BackendHandle, capabilities: MountBackendCapabilities) -> BackendHandle {
    let gate = MountRequestGate::new(usize::from(capabilities.parallelism));
    Arc::new(MountProxy {
        inner,
        capabilities,
        gate,
    })
}

struct MountProxy {
    inner: BackendHandle,
    capabilities: MountBackendCapabilities,
    gate: Arc<MountRequestGate>,
}

impl Backend for MountProxy {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn state_identity(&self) -> String {
        "mount-host-proxy".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let _permit = self.gate.enter_metadata()?;
        decode(self.inner.list_dir(path))
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let _permit = self.gate.enter_metadata()?;
        decode(self.inner.stat(path))
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        let _permit = self.gate.enter_metadata()?;
        decode(self.inner.try_exists(path))
    }

    fn item_id(&self, _path: &str) -> VfsResult<Option<String>> {
        Ok(None)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let permit = self.gate.enter()?;
        let inner = decode(self.inner.open_read(path))?;
        Ok(Box::new(DecodedReader {
            inner,
            _permit: permit,
        }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let permit = self.gate.enter()?;
        let inner = decode(self.inner.open_write(path))?;
        Ok(Box::new(DecodedWriter {
            inner,
            _permit: permit,
        }))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let permit = self.gate.enter()?;
        let inner = decode(self.inner.open_write_new(path))?;
        Ok(Box::new(DecodedWriter {
            inner,
            _permit: permit,
        }))
    }

    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }

    fn copy_file(&self, source: &str, destination: &str) -> VfsResult<u64> {
        let _permit = self.gate.enter()?;
        decode(self.inner.copy_file(source, destination))
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(self.inner.rename(source, destination))
    }

    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(self.inner.rename_no_replace(source, destination))
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(self.inner.promote_staged(staged, destination))
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(self.inner.promote_staged_no_replace(staged, destination))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(self.inner.remove_file(path))
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(self.inner.remove_dir(path))
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let _permit = self.gate.enter()?;
        decode(self.inner.mkdir_all(path))
    }

    fn delete_disposition(&self) -> DeleteDisposition {
        self.capabilities.delete_disposition.into()
    }

    fn parallelism(&self) -> usize {
        usize::from(self.capabilities.parallelism)
    }

    fn rename_overwrites(&self) -> bool {
        self.capabilities.rename_overwrites
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        self.capabilities.staged_write()
    }

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        self.capabilities.case_sensitive_paths
    }

    fn open_read_id(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
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
        self.inner.invalidate_cache();
    }
}

struct DecodedReader {
    inner: Box<dyn Read + Send>,
    _permit: MountRequestPermit,
}

impl Read for DecodedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        decode(self.inner.read(buffer))
    }
}

struct DecodedWriter {
    inner: Box<dyn Write + Send>,
    _permit: MountRequestPermit,
}

impl Write for DecodedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        decode(self.inner.write(buffer))
    }

    fn flush(&mut self) -> io::Result<()> {
        decode(self.inner.flush())
    }
}

fn decode<T>(result: io::Result<T>) -> io::Result<T> {
    result.map_err(super::mount_error::decode)
}
