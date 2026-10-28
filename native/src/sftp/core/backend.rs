use super::channel_pool::{note_failure, ChannelPool};
use super::config::{SftpConfig, SftpLoginOrigin};
use super::connection::{SftpConnection, SftpGeneration};
use super::io_adapters::Commit;
use super::io_err;
use super::link_rules::plan;
use super::metadata::{basename, to_vfs};
use super::pool_writer::SizedWriter;
use super::stage_evidence::WrittenStages;
use crate::vfs::{
    Backend, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink, Scheme, VfsMeta,
    VfsResult,
};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::OpenFlags;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::runtime::Runtime;

#[derive(Clone)]
pub struct SftpBackend {
    pub(super) rt: Arc<Runtime>,
    pub(super) connection: Arc<SftpConnection>,
    /// Extra SFTP channels for file transfers (channel_pool.rs).
    pub(super) pool: Arc<ChannelPool>,
    /// Verdicts on the folder chain of stages for ranges (segmented_stage.rs).
    pub(super) stage_folders: Arc<Mutex<HashMap<String, (bool, Instant)>>>,
    /// Evidence of committed copy stages for the receipts (stage_evidence.rs).
    pub(super) written: WrittenStages,
    root: String,
    /// Read by `url()` (UI display), consumed in the connect-UI step.
    #[allow(dead_code)]
    url: String,
}

impl SftpBackend {
    /// A connection whose login a person may have typed into: extra SSH
    /// connections for transfers only through the SSH agent (G3-10,
    /// agent_grant.rs), never with typed credentials (`connect_with_origin`).
    pub fn connect(cfg: SftpConfig) -> io::Result<SftpBackend> {
        Self::connect_with_origin(cfg, SftpLoginOrigin::Interactive)
    }

    /// Like `connect`, with how the credentials of `cfg` were obtained. When
    /// the main login needed no input (`SftpLoginOrigin`) or used the SSH
    /// agent, transfers may add extra SSH connections of the same account,
    /// one at a time while the throughput still rises (security answer
    /// E13-S15, extended to the agent by G3-10).
    pub fn connect_with_origin(
        cfg: SftpConfig,
        origin: SftpLoginOrigin,
    ) -> io::Result<SftpBackend> {
        let url = format!("sftp://{}@{}:{}{}", cfg.user, cfg.host, cfg.port, cfg.root);
        let root = cfg.root.clone();
        let (host, port, user) = (cfg.host.clone(), cfg.port, cfg.user.clone());
        let login = plan(&cfg.auth, origin);
        let connection = SftpConnection::connect(cfg)?;
        let rt = connection.runtime();
        let pool = ChannelPool::create(&connection, &host, port, &user, login);
        Ok(SftpBackend {
            rt,
            connection,
            pool,
            stage_folders: Arc::new(Mutex::new(HashMap::new())),
            written: WrittenStages::default(),
            root,
            url,
        })
    }

    /// `sftp://user@host:port/root` for UI display (connect-UI step).
    #[allow(dead_code)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub(super) fn safe_sftp_on<T>(
        &self,
        operation: impl Fn(&SftpGeneration) -> Result<T, SftpError>,
    ) -> io::Result<(Arc<SftpGeneration>, T)> {
        let mut generation = self.connection.current()?;
        for attempt in 0..2 {
            match operation(&generation) {
                Ok(value) => return Ok((generation, value)),
                Err(error) => {
                    let dead = self.connection.note_sftp_error(&generation, &error);
                    if attempt == 0 && dead {
                        generation = self.connection.current()?;
                        continue;
                    }
                    return Err(io_err(error));
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "SFTP-Verbindung ließ sich auch nach dem Neuverbinden nicht nutzen",
        ))
    }

    /// Atomic replace through `posix-rename@openssh.com` (see posix_rename.rs):
    /// on a pool channel when there is one (no extra channel, so even a full
    /// pool leaves the server's session limit untouched), else on a
    /// short-lived channel of its own.
    fn posix_rename(&self, from: &str, to: &str) -> io::Result<()> {
        if let Some(lease) = self.pool.lease(self)? {
            let channel = lease.channel().clone();
            if channel.posix_rename {
                let request = super::posix_rename::rename_request(&channel.session, from, to);
                return match self.rt.block_on(request) {
                    Ok(answer) => answer,
                    Err(error) => {
                        note_failure(&self.connection, &channel, &error);
                        Err(io_err(error))
                    }
                };
            }
        }
        let (generation, channel) = self.open_session_channel()?;
        let renamed = self
            .rt
            .block_on(super::posix_rename::posix_rename(channel, from, to));
        if let Err(error) = &renamed {
            self.connection.note_io_error(&generation, error);
        }
        renamed
    }

    fn mutate_sftp<T>(
        &self,
        operation: impl FnOnce(&SftpGeneration) -> Result<T, SftpError>,
    ) -> io::Result<T> {
        let generation = self.connection.current()?;
        operation(&generation).map_err(|error| {
            self.connection.note_sftp_error(&generation, &error);
            io_err(error)
        })
    }

    /// A writer on a pool channel, or on the main session without one.
    fn writer(
        &self,
        path: &str,
        flags: OpenFlags,
        expected: Option<u64>,
        commit: Commit,
    ) -> VfsResult<Box<dyn Write + Send>> {
        if let Some(writer) = self
            .open_pool_writer(path, flags, expected, commit)
            .map_err(|error| self.target_error(path, error))?
        {
            return Ok(self.target_writer(path, Box::new(writer)));
        }
        self.main_writer(path, flags, expected, commit)
    }

    /// A copy stage on a pool channel reports what the server answered at
    /// its commit (stage_evidence.rs); on the main session it does not.
    fn stage_writer(
        &self,
        path: &str,
        size: u64,
        commit: Commit,
    ) -> VfsResult<Box<dyn Write + Send>> {
        if let Some(writer) = self
            .open_pool_stage_writer(path, size, commit)
            .map_err(|error| self.target_error(path, error))?
        {
            return Ok(self.target_writer(path, Box::new(writer)));
        }
        self.main_writer(path, create_new_flags(), Some(size), commit)
    }

    fn main_writer(
        &self,
        path: &str,
        flags: OpenFlags,
        expected: Option<u64>,
        commit: Commit,
    ) -> VfsResult<Box<dyn Write + Send>> {
        let writer = self
            .open_main_writer(path, flags, commit)
            .map_err(|error| self.target_error(path, error))?;
        let writer: Box<dyn Write + Send> = match expected {
            Some(size) => Box::new(SizedWriter::new(writer, size)),
            None => Box::new(writer),
        };
        Ok(self.target_writer(path, writer))
    }

    fn reader(&self, path: &str, start: u64) -> VfsResult<Box<dyn Read + Send>> {
        if let Some(reader) = self.open_pool_reader(path, start)? {
            return Ok(Box::new(reader));
        }
        Ok(Box::new(self.open_main_reader(path, start)?))
    }
}

/// `sftp().create(path)`: create or truncate.
fn create_flags() -> OpenFlags {
    OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE
}

/// Exclusive create: the protocol's own O_EXCL.
pub(super) fn create_new_flags() -> OpenFlags {
    OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE
}

impl Backend for SftpBackend {
    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        Some(self)
    }

    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn state_identity(&self) -> String {
        self.url.clone()
    }
    fn namespace_identity(&self) -> String {
        self.url
            .strip_suffix(&self.root)
            .unwrap_or(&self.url)
            .to_string()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        // Browsing is what follows a burst of transfers: close its idle
        // channels here instead of at the next transfer.
        self.pool.retire_idle();
        // russh-sftp bounds each READDIR request. An advancing large listing
        // must not also fit the ordinary 20-s single-metadata budget.
        let (_, dir) = self.safe_sftp_on(|generation| {
            self.rt
                .block_on(generation.sftp().read_dir(path.to_string()))
        })?;
        let mut out = Vec::new();
        for e in dir {
            let name = e.file_name();
            let meta = e.metadata();
            out.push(to_vfs(name, &meta));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let meta = self.connection.safe_metadata(|generation| {
            Box::pin(generation.sftp().symlink_metadata(path.to_string()))
        })?;
        Ok(to_vfs(basename(path), &meta))
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        self.connection
            .safe_metadata(|generation| Box::pin(generation.sftp().try_exists(path.to_string())))
    }

    /// Pipelined on a pool channel: many READs in flight instead of one per
    /// round trip (docs/refs/quic-sftp-throughput.md B).
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reader(path, 0)
    }

    /// Every READ names its offset, so a resumed download starts anywhere.
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let _ = id;
        self.reader(path, offset).map(Some)
    }

    /// Ranges of one file go over several pool channels (and extra SSH
    /// connections), each with its own server window; true while the pool
    /// can hand out a channel.
    fn supports_range_reads(&self, path: &str) -> bool {
        let _ = path;
        self.pool.available(self)
    }

    /// `[offset, offset + len)` on a pool channel of its own, a shared one or,
    /// only when the pool can hold no channel, over the main session: a
    /// range never fails for lack of a channel (main_range.rs).
    fn open_read_range(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
        len: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let _ = id;
        self.read_range(path, offset, len).map(Some)
    }

    /// Ranges of an upload over several pool channels, only where the
    /// folders of the stage cannot be swapped by another account (security
    /// answer S7, segmented_stage.rs).
    fn open_segmented_stage(
        &self,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn crate::vfs::SegmentedStage>>> {
        self.segmented_stage(stage, size)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.writer(path, create_flags(), None, Commit::Durable)
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.writer(path, create_new_flags(), None, Commit::Durable)
    }

    /// The exclusive stage streams anyway; the known length is enforced at
    /// `flush`, so a source that changed size never publishes.
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer(path, size, Commit::Durable)
    }

    /// The sized stage committed without `fsync@openssh.com`: all
    /// acknowledgements and CLOSE still come before `flush` returns.
    fn open_write_copy_stage_unsynced(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.stage_writer(path, size, Commit::Unsynced)
    }

    /// Inside one server the server copies itself (`copy-data`,
    /// copy_data.rs); `None` streams.
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<u64>> {
        self.copy_on_server(src, stage, size, cancel)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().rename(src.to_string(), dst.to_string()))
        })
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        // russh-sftp speaks SFTP v3. SSH_FXP_RENAME in that protocol must fail
        // when newpath already exists, so the request itself is the atomic
        // no-replace gate rather than a racy client-side existence probe.
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().rename(src.to_string(), dst.to_string()))
        })
    }

    /// An existing file is replaced atomically by `posix-rename@openssh.com`;
    /// `rename_overwrites` stays false because the extension is per server.
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        crate::vfs::promote_staged_with(self, staged, destination, |from, to| {
            self.posix_rename(from, to)
        })
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().remove_file(path.to_string()))
        })
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().remove_dir(path.to_string()))
        })
    }

    /// From the deepest existing folder down (MS7, transfer_ops.rs).
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.make_dirs(path)
    }

    /// One MKDIR; an existing real folder (not a link) is success.
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.create_one_dir(path, false)
    }

    /// One MKDIR that must create the name: the server's `mkdir(2)` refuses
    /// an existing one atomically; the entry is only inspected afterwards to
    /// tell `AlreadyExists` from other refusals.
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.create_one_dir(path, true)
    }

    /// Stages are created with an exclusive OPEN, so the name is this
    /// client's until it is published.
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.written.forget(stage);
        self.discard_stage(stage)
    }

    /// Packets of small files on one pool channel (batch_ops.rs); `None`
    /// while no pool channel can serve.
    fn batch_limits(&self, dir: &str) -> Option<BatchLimits> {
        let _ = dir;
        super::batch_ops::advertised(self)
    }

    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        super::batch_put::run(self, entries, data)
    }

    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        super::batch_get::run(self, items, sink)
    }

    /// The connections in use, or the 15-minute block of extra connections
    /// (E13-S15); `None` with one connection and no block.
    fn transfer_hint(&self) -> Option<String> {
        self.pool.hint()
    }

    /// Transfers the channels carry at once once the server's session limit
    /// is known, so the flow control does not climb into refusals.
    fn transfer_ceiling(&self, path: &str) -> Option<usize> {
        let _ = path;
        self.pool.capacity()
    }

    fn parallelism(&self) -> usize {
        // Conservative: one SFTP session, sequential remote walk. Safe default
        // until a real-server concurrency spike (plan §"open questions").
        1
    }

    /// All transfers of one SSH connection share one controller, also the
    /// clones of this backend.
    fn flow_key(&self, path: &str) -> String {
        let _ = path;
        format!(
            "{}#{:p}",
            self.namespace_identity(),
            Arc::as_ptr(&self.connection)
        )
    }

    /// Readers and writers each hold their own handle and requests carry ids,
    /// so a download and an upload on the same server run side by side
    /// (without a spool through the local disk).
    fn concurrent_read_write(&self) -> bool {
        true
    }

    fn staged_write_capabilities(&self, _root: &str) -> crate::vfs::StagedWriteCapabilities {
        crate::vfs::StagedWriteCapabilities {
            create: true,
            replace: false,
            namespace_replace: false,
        }
    }
}
