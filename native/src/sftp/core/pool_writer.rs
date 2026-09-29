//! Uploads over a pool channel. Writes are gathered into WRITE-sized chunks
//! (a caller copying in 8 KiB steps would otherwise send 8 KiB WRITEs) and
//! kept on the wire as deep as the pipeline allows. `flush` is the commit
//! boundary, as for every backend writer: all acknowledgements, then
//! `fsync@openssh.com` when the server offers it (as `File::flush` did on the
//! main session), then CLOSE. A writer dropped without `flush` still sends
//! what it was given and closes the handle, like the main-session writer.
use super::backend::SftpBackend;
use super::channel_pool::{note_failure, open_failed, ChannelLease, PoolChannel};
use super::connection::SftpConnection;
use super::io_err;
use super::pipeline::Pipeline;
use super::session::SSH_WINDOW;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, Status};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

type WriteTask = JoinHandle<(Result<Status, SftpError>, Instant)>;

struct InflightWrite {
    sent: Instant,
    full: bool,
    task: WriteTask,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteState {
    Open,
    Committed,
    Failed,
}

pub(super) struct PoolWriter {
    rt: Arc<Runtime>,
    connection: Arc<SftpConnection>,
    channel: Arc<PoolChannel>,
    lease: Option<ChannelLease>,
    handle: String,
    handle_open: bool,
    chunk: usize,
    buffer: Vec<u8>,
    offset: u64,
    inflight: VecDeque<InflightWrite>,
    pipeline: Pipeline,
    /// Exact length of a sized copy stage (plan W1).
    expected: Option<u64>,
    accepted: u64,
    state: WriteState,
}

/// The length check of a sized stage: `Err` names what arrived.
pub(super) fn check_length(expected: Option<u64>, accepted: u64) -> io::Result<()> {
    match expected {
        Some(expected) if expected != accepted => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Quelle hat sich während der Übertragung geändert: {accepted} statt {expected} Bytes"
            ),
        )),
        _ => Ok(()),
    }
}

impl PoolWriter {
    fn failed_error() -> io::Error {
        io::Error::other("SFTP-Upload ist fehlgeschlagen; weitere Daten werden nicht angenommen")
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.state = WriteState::Failed;
        self.abandon();
        error
    }

    fn submit(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let data = std::mem::replace(&mut self.buffer, Vec::with_capacity(self.chunk));
        let full = data.len() == self.chunk;
        let length = data.len() as u64;
        let session = self.channel.session.clone();
        let handle = self.handle.clone();
        let offset = self.offset;
        let task = self.rt.spawn(async move {
            let result = session.write(handle, offset, data).await;
            (result, Instant::now())
        });
        self.inflight.push_back(InflightWrite {
            sent: Instant::now(),
            full,
            task,
        });
        self.offset = offset.saturating_add(length);
        while self.inflight.len() > self.pipeline.depth() {
            self.await_oldest()?;
        }
        Ok(())
    }

    fn await_oldest(&mut self) -> io::Result<()> {
        let Some(write) = self.inflight.pop_front() else {
            return Ok(());
        };
        match self.rt.block_on(write.task) {
            Ok((Ok(_), arrived)) => {
                self.pipeline
                    .answered(arrived.saturating_duration_since(write.sent), write.full);
                Ok(())
            }
            Ok((Err(error), _)) => {
                note_failure(&self.connection, &self.channel, &error);
                Err(io_err(error))
            }
            Err(error) => Err(io::Error::other(format!(
                "SFTP-Schreibauftrag wurde abgebrochen: {error}"
            ))),
        }
    }

    fn sftp_step(
        &mut self,
        step: impl std::future::Future<Output = Result<Status, SftpError>>,
    ) -> io::Result<()> {
        self.rt.block_on(step).map(|_| ()).map_err(|error| {
            note_failure(&self.connection, &self.channel, &error);
            io_err(error)
        })
    }

    /// Sends the rest, waits for every acknowledgement, optionally syncs and
    /// closes the handle.
    fn complete(&mut self, sync: bool, check: bool) -> io::Result<()> {
        self.submit()?;
        while !self.inflight.is_empty() {
            self.await_oldest()?;
        }
        if check {
            check_length(self.expected, self.accepted)?;
        }
        if sync && self.channel.fsync {
            let session = self.channel.session.clone();
            let handle = self.handle.clone();
            self.sftp_step(async move { session.fsync(handle).await })?;
        }
        let session = self.channel.session.clone();
        let handle = self.handle.clone();
        self.handle_open = false;
        self.sftp_step(async move { session.close(handle).await })?;
        drop(self.lease.take());
        Ok(())
    }

    /// Stops everything in flight and closes the handle without waiting.
    fn abandon(&mut self) {
        while let Some(write) = self.inflight.pop_front() {
            write.task.abort();
        }
        if self.handle_open {
            self.handle_open = false;
            let session = self.channel.session.clone();
            let handle = std::mem::take(&mut self.handle);
            drop(self.rt.spawn(async move {
                let _ = session.close(handle).await;
            }));
        }
        drop(self.lease.take());
    }
}

impl Write for PoolWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self.state {
            WriteState::Open => {}
            WriteState::Committed => {
                return Err(io::Error::other("SFTP-Upload ist bereits abgeschlossen"))
            }
            WriteState::Failed => return Err(Self::failed_error()),
        }
        let accepted = self.accepted.saturating_add(data.len() as u64);
        if self.expected.is_some_and(|expected| accepted > expected) {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                "Quelle ist während der Übertragung gewachsen",
            );
            return Err(self.fail(error));
        }
        let mut rest = data;
        while !rest.is_empty() {
            let take = (self.chunk - self.buffer.len()).min(rest.len());
            self.buffer.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.buffer.len() >= self.chunk {
                if let Err(error) = self.submit() {
                    return Err(self.fail(error));
                }
            }
        }
        self.accepted = accepted;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.state {
            WriteState::Committed => return Ok(()),
            WriteState::Failed => return Err(Self::failed_error()),
            WriteState::Open => {}
        }
        match self.complete(true, true) {
            Ok(()) => {
                self.state = WriteState::Committed;
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }
}

impl Drop for PoolWriter {
    fn drop(&mut self) {
        if self.state == WriteState::Open && self.complete(false, false).is_ok() {
            return;
        }
        self.abandon();
    }
}

/// A main-session writer of a sized stage: the same length check as the
/// pool writer, so the stage contract holds without a pool channel.
pub(super) struct SizedWriter<W: Write> {
    inner: W,
    expected: u64,
    accepted: u64,
}

impl<W: Write> SizedWriter<W> {
    pub(super) fn new(inner: W, expected: u64) -> Self {
        Self {
            inner,
            expected,
            accepted: 0,
        }
    }
}

impl<W: Write> Write for SizedWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let accepted = self.accepted.saturating_add(data.len() as u64);
        if accepted > self.expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Quelle ist während der Übertragung gewachsen",
            ));
        }
        let written = self.inner.write(data)?;
        self.accepted = self.accepted.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        check_length(Some(self.expected), self.accepted)?;
        self.inner.flush()
    }
}

impl SftpBackend {
    /// A pipelined upload to `path` opened with `flags`; `None` when no pool
    /// channel is available (the main session serves then). OPEN creates or
    /// truncates, so it is never repeated.
    pub(super) fn open_pool_writer(
        &self,
        path: &str,
        flags: OpenFlags,
        expected: Option<u64>,
    ) -> io::Result<Option<PoolWriter>> {
        let Some(lease) = self.pool.lease(self)? else {
            return Ok(None);
        };
        let channel = lease.channel().clone();
        let opened = self
            .rt
            .block_on(channel.session.open(path, flags, FileAttributes::empty()));
        let handle = match opened {
            Ok(handle) => handle.handle,
            Err(error) => {
                note_failure(&self.connection, &channel, &error);
                return Err(open_failed(error));
            }
        };
        let chunk = channel.write_chunk(handle.len());
        let cap = usize::try_from(u64::from(SSH_WINDOW) / chunk.max(1) as u64)
            .unwrap_or(usize::MAX)
            .max(1);
        Ok(Some(PoolWriter {
            rt: self.rt.clone(),
            connection: self.connection.clone(),
            channel,
            lease: Some(lease),
            handle,
            handle_open: true,
            chunk,
            buffer: Vec::with_capacity(chunk),
            offset: 0,
            inflight: VecDeque::new(),
            pipeline: Pipeline::new(chunk as u64, cap),
            expected,
            accepted: 0,
            state: WriteState::Open,
        }))
    }
}
