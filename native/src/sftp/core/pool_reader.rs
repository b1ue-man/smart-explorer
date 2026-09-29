//! Downloads over a pool channel: `pipelined_read` on a `RawSftpSession`.
//! As the main-session reader, a channel proven dead before the first byte
//! reached the caller is replaced once; after that a failure stays visible
//! instead of silently restarting a possibly changed file.
use super::backend::SftpBackend;
use super::channel_pool::{note_failure, open_failed, ChannelLease, PoolChannel};
use super::connection::SftpConnection;
use super::io_err;
use super::pipelined_read::{PipelinedRead, ReadSource, Reply};
use super::session::SSH_WINDOW;
use crate::transfer::read_pipeline::Pipeline;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{Data, FileAttributes, OpenFlags, StatusCode};
use std::io::{self, Read};
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// One open file on a pool channel, the READ source of a pipelined reader.
pub(super) struct ChannelFile {
    rt: Arc<Runtime>,
    connection: Arc<SftpConnection>,
    channel: Arc<PoolChannel>,
    lease: Option<ChannelLease>,
    handle: String,
    /// The last failure proved the channel dead: replaying elsewhere is safe.
    retry_safe: bool,
}

impl ReadSource for ChannelFile {
    type Pending = JoinHandle<(Result<Data, SftpError>, Instant)>;

    fn issue(&mut self, offset: u64, len: u32) -> Self::Pending {
        let session = self.channel.session.clone();
        let handle = self.handle.clone();
        self.rt.spawn(async move {
            let result = session.read(handle, offset, len).await;
            (result, Instant::now())
        })
    }

    fn wait(&mut self, pending: Self::Pending) -> (io::Result<Reply>, Instant) {
        match self.rt.block_on(pending) {
            Ok((Ok(data), arrived)) => (Ok(Reply::Data(data.data)), arrived),
            Ok((Err(SftpError::Status(status)), arrived))
                if status.status_code == StatusCode::Eof =>
            {
                (Ok(Reply::Eof), arrived)
            }
            Ok((Err(error), arrived)) => {
                self.retry_safe = note_failure(&self.connection, &self.channel, &error);
                (Err(io_err(error)), arrived)
            }
            Err(error) => (
                Err(io::Error::other(format!(
                    "SFTP-Leseauftrag wurde abgebrochen: {error}"
                ))),
                Instant::now(),
            ),
        }
    }

    fn cancel(&mut self, pending: Self::Pending) {
        pending.abort();
    }

    fn close(&mut self) {
        let session = self.channel.session.clone();
        let handle = std::mem::take(&mut self.handle);
        // Like `File::drop`: the CLOSE is sent without waiting for it.
        drop(self.rt.spawn(async move {
            let _ = session.close(handle).await;
        }));
        drop(self.lease.take());
    }
}

/// A download on a pool channel that reopens once after a dead channel.
pub(super) struct PoolReader {
    backend: SftpBackend,
    path: String,
    start: u64,
    inner: PipelinedRead<ChannelFile>,
    retried: bool,
}

impl Read for PoolReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            let error = match self.inner.read(out) {
                Ok(read) => return Ok(read),
                Err(error) => error,
            };
            if self.retried || self.inner.delivered() != 0 || !self.inner.source().retry_safe {
                return Err(error);
            }
            self.retried = true;
            match self.backend.open_pipelined(&self.path, self.start)? {
                Some(inner) => self.inner = inner,
                None => return Err(error),
            }
        }
    }
}

/// More READs than one channel window cannot arrive within one round trip.
fn window_depth(chunk: u32) -> usize {
    usize::try_from(SSH_WINDOW / chunk.max(1))
        .unwrap_or(usize::MAX)
        .max(1)
}

impl SftpBackend {
    /// A pipelined download of `path` from `offset`; `None` when no pool
    /// channel is available (the main session serves then).
    pub(super) fn open_pool_reader(
        &self,
        path: &str,
        offset: u64,
    ) -> io::Result<Option<PoolReader>> {
        let Some(inner) = self.open_pipelined(path, offset)? else {
            return Ok(None);
        };
        Ok(Some(PoolReader {
            backend: self.clone(),
            path: path.to_string(),
            start: offset,
            inner,
            retried: false,
        }))
    }

    fn open_pipelined(
        &self,
        path: &str,
        offset: u64,
    ) -> io::Result<Option<PipelinedRead<ChannelFile>>> {
        let mut retried = false;
        loop {
            let Some(lease) = self.pool.lease(self)? else {
                return Ok(None);
            };
            let channel = lease.channel().clone();
            let opened = self.rt.block_on(channel.session.open(
                path,
                OpenFlags::READ,
                FileAttributes::empty(),
            ));
            let handle = match opened {
                Ok(handle) => handle.handle,
                Err(error) => {
                    // OPEN for reading changes nothing: after a proven dead
                    // channel it is repeated once on another one.
                    if note_failure(&self.connection, &channel, &error) && !retried {
                        retried = true;
                        continue;
                    }
                    return Err(open_failed(error));
                }
            };
            let chunk = channel.read_chunk();
            let pipeline = Pipeline::new(u64::from(chunk), window_depth(chunk));
            let source = ChannelFile {
                rt: self.rt.clone(),
                connection: self.connection.clone(),
                channel,
                lease: Some(lease),
                handle,
                retry_safe: false,
            };
            return Ok(Some(PipelinedRead::new(source, offset, chunk, pipeline)));
        }
    }
}
