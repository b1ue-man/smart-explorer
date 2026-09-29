use super::agent_error::agent_error;
use super::metadata::wire_to_vfs;
use super::pool::AgentPool;
#[cfg(test)]
use super::transport::HeartbeatPolicy;
use super::transport::{AgentConnection, AgentReconnect};
use crate::agent_proto::{Frame, ServerFeatures, BATCH_MAX_BYTES, BATCH_MAX_FILES};
use crate::vfs::{
    Backend, BackendHandle, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink, Scheme,
    VfsMeta, VfsResult,
};
use crossbeam_channel::Sender;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const METADATA_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub struct AgentBackend {
    pub(super) inner: BackendHandle,
    pub(super) pool: Arc<AgentPool>,
    version: String,
    root_confined: Option<String>,
}

impl AgentBackend {
    /// Hand-shake over an already-open framed stream pair.
    pub fn from_streams(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
    ) -> io::Result<Self> {
        Self::from_streams_inner((r, w), inner, None, None)
    }

    pub(super) fn from_streams_with_reconnect(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        reconnect: AgentReconnect,
    ) -> io::Result<Self> {
        Self::from_streams_inner((r, w), inner, Some(reconnect), None)
    }

    /// The SSH-deployed agent: its reconnect also opens further exec
    /// channels, so this backend may spread requests over a channel pool.
    pub(super) fn from_root_confined_streams_with_reconnect(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        reconnect: AgentReconnect,
        root: String,
    ) -> io::Result<Self> {
        Self::from_streams_inner((r, w), inner, Some(reconnect), Some(root))
    }

    #[cfg(test)]
    pub(super) fn from_streams_with_reconnect_and_heartbeat(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        reconnect: AgentReconnect,
        heartbeat: HeartbeatPolicy,
    ) -> io::Result<Self> {
        let (connection, version) =
            AgentConnection::new_with_heartbeat((r, w), Some(reconnect), heartbeat)?;
        let pool = AgentPool::single(connection, ServerFeatures::parse(&version));
        Ok(Self {
            inner,
            pool,
            version,
            root_confined: None,
        })
    }

    /// A pool over test streams: `opener` provides further channels.
    #[cfg(test)]
    pub(super) fn pooled_for_test(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
        opener: AgentReconnect,
    ) -> io::Result<Self> {
        let (connection, version) = AgentConnection::new((r, w), Some(opener.clone()))?;
        let pool = AgentPool::growable(connection, ServerFeatures::parse(&version), opener);
        Ok(Self {
            inner,
            pool,
            version,
            root_confined: None,
        })
    }

    fn from_streams_inner(
        streams: super::transport::AgentStreams,
        inner: BackendHandle,
        reconnect: Option<AgentReconnect>,
        root_confined: Option<String>,
    ) -> io::Result<Self> {
        let (connection, version) = AgentConnection::new(streams, reconnect.clone())?;
        let features = ServerFeatures::parse(&version);
        let pool = match (reconnect, root_confined.is_some()) {
            (Some(opener), true) => AgentPool::growable(connection, features, opener),
            _ => AgentPool::single(connection, features),
        };
        Ok(AgentBackend {
            inner,
            pool,
            version,
            root_confined,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub(super) fn features(&self) -> ServerFeatures {
        self.pool.features()
    }

    /// Agent channels currently open (the pool of an SSH-deployed agent).
    pub fn channel_count(&self) -> usize {
        self.pool.channel_count()
    }

    fn metadata_call(&self, request: Frame) -> VfsResult<Frame> {
        self.pool
            .lease()
            .safe_call_timeout(request, METADATA_REQUEST_TIMEOUT)
    }
}

fn unexpected(what: &str, other: &Frame) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected agent {what} reply: {other:?}"),
    )
}

impl Backend for AgentBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn state_identity(&self) -> String {
        self.inner.state_identity()
    }
    fn namespace_identity(&self) -> String {
        self.inner.namespace_identity()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.metadata_call(Frame::ListDir(path.to_string()))? {
            Frame::Dir(v) => Ok(v.into_iter().map(wire_to_vfs).collect()),
            Frame::Err(e) => Err(agent_error(e)),
            other => Err(unexpected("directory", &other)),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match self.metadata_call(Frame::Stat(path.to_string()))? {
            Frame::Meta(m) => Ok(wire_to_vfs(m)),
            Frame::Err(e) => Err(agent_error(e)),
            other => Err(unexpected("metadata", &other)),
        }
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        match self.metadata_call(Frame::TryExists(path.to_string()))? {
            Frame::Exists(exists) => Ok(exists),
            Frame::Err(error) => Err(agent_error(error)),
            other => Err(unexpected("existence", &other)),
        }
    }

    fn supports_walk_tree(&self) -> bool {
        true
    }

    fn walk_tree(
        &self,
        root: &str,
        on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        self.walk_tree_impl(root, on_progress)
    }

    fn scan_storage(
        &self,
        root: &str,
        progress: &crate::analytics::Progress,
    ) -> VfsResult<Option<crate::analytics::ScanOutcome>> {
        self.inner.scan_storage(root, progress)
    }

    fn supports_bulk_tree(&self) -> bool {
        // The remote PutTree receiver currently spools under the system temp
        // directory. A root-confined agent cannot write there under Landlock,
        // so advertise the generic per-entry fallback until that spool lives
        // inside the confined root.
        self.root_confined.is_none()
    }

    fn get_tree(&self, root: &str, dst: &Path) -> VfsResult<u64> {
        self.agent_get_tree(root, dst)
    }

    fn put_tree(&self, src: &Path, root: &str) -> VfsResult<u64> {
        self.agent_put_tree(src, root)
    }

    fn batch_limits(&self, dir: &str) -> Option<BatchLimits> {
        let _ = dir;
        self.features().batches().then_some(BatchLimits {
            max_files: BATCH_MAX_FILES,
            max_bytes: BATCH_MAX_BYTES,
        })
    }

    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        self.agent_put_batch(entries, data)
    }

    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        self.agent_get_batch(items, sink)
    }

    fn supports_search(&self) -> bool {
        true
    }

    fn search(
        &self,
        root: &str,
        spec: &crate::agent_proto::SearchSpec,
        tx: Sender<crate::vfs::SearchHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        self.agent_search(root, spec, tx, cancel)
    }

    fn supports_walk_hashed(&self) -> bool {
        true
    }

    fn walk_hashed(
        &self,
        root: &str,
        want_hash: bool,
        tx: Sender<crate::vfs::HashHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        self.agent_walk_hashed(root, want_hash, tx, cancel)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.agent_open_read_at(path, 0)
    }

    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let _ = id;
        self.open_read(path)
    }

    /// Every agent and service reads from an offset (`Read.offset`).
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let _ = id;
        self.agent_open_read_at(path, offset).map(Some)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.agent_open_write(path)
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.agent_open_write_new(path)
    }

    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<u64>> {
        if !self.features().stage {
            // An older agent: the SSH connection underneath may still copy
            // on the server (SFTP `copy-data`); a proxied peer answers `None`.
            return self.inner.server_copy_to_stage(src, stage, size, cancel);
        }
        self.agent_copy_to_stage(src, stage, size, cancel)
    }

    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.agent_unit_op(Frame::Copy {
            src: src.to_string(),
            dst: dst.to_string(),
        })?;
        self.stat(dst).map(|meta| meta.size)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Rename {
            src: src.to_string(),
            dst: dst.to_string(),
        })
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        let lease = self.pool.lease();
        let (mux, reply) = lease.mutation_call(Frame::RenameNoReplace {
            src: src.to_string(),
            dst: dst.to_string(),
        })?;
        match reply {
            Frame::Ok => Ok(()),
            Frame::Err(error) => Err(agent_error(error)),
            other => {
                lease.invalidate(&mux);
                Err(unexpected("no-replace", &other))
            }
        }
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Promote {
            staged: staged.to_string(),
            destination: destination.to_string(),
        })
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::PromoteNoReplace {
            staged: staged.to_string(),
            destination: destination.to_string(),
        })
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Remove {
            path: path.to_string(),
            recursive: false,
        })
    }

    fn remove_file_id(&self, path: &str, _id: Option<&str>) -> VfsResult<()> {
        self.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Remove {
            path: path.to_string(),
            recursive: false,
        })
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Mkdir(path.to_string()))
    }

    fn create_dir(&self, path: &str) -> VfsResult<()> {
        if self.features().stage {
            self.agent_create_dir(path, false)
        } else {
            self.mkdir_all(path)
        }
    }

    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        if self.features().stage {
            return self.agent_create_dir(path, true);
        }
        // Older servers: the trait's probing default.
        if self.try_exists(path)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                path.to_string(),
            ));
        }
        self.create_dir(path)
    }

    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        if self.features().stage {
            self.agent_discard_stage(stage)
        } else {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "stage cleanup unsupported",
            ))
        }
    }

    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }

    /// One controller per remote account: host and user for SSH, the share
    /// for Share/Room connections through the service.
    fn flow_key(&self, path: &str) -> String {
        let _ = path;
        format!("agent:{}", self.inner.namespace_identity())
    }

    fn transfer_ceiling(&self, path: &str) -> Option<usize> {
        let _ = path;
        self.pool.transfer_ceiling()
    }

    fn rename_overwrites(&self) -> bool {
        true
    }

    fn staged_write_capabilities(&self, _root: &str) -> crate::vfs::StagedWriteCapabilities {
        crate::vfs::StagedWriteCapabilities::complete()
    }

    fn root_confinement(&self, root: &str) -> crate::vfs::RootConfinement {
        if self.root_confined.as_deref() == Some(root) {
            crate::vfs::RootConfinement::Enforced
        } else {
            crate::vfs::RootConfinement::Unverified
        }
    }

    fn is_local(&self) -> bool {
        self.inner.is_local()
    }

    fn provides_content_hash(&self) -> bool {
        self.inner.provides_content_hash()
    }
}
