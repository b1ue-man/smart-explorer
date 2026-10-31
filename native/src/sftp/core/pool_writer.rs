//! Uploads over a pool channel. Writes are gathered into WRITE-sized chunks
//! (a caller copying in 8 KiB steps would otherwise send 8 KiB WRITEs) and
//! kept on the wire as deep as the pipeline allows. `flush` is the commit
//! boundary, as for every backend writer: all acknowledgements, then
//! `fsync@openssh.com` when the server offers it (as `File::flush` did on the
//! main session) unless the writer is an unsynced copy stage, then CLOSE. A
//! writer dropped without `flush` still sends what it was given and closes
//! the handle, like the main-session writer.
//!
//! Every request goes onto the channel in the order it was made
//! (`enqueue`). An unsynced commit therefore sends CLOSE right behind the
//! last WRITE instead of after its answer (G3-06a): the server handles one
//! handle's requests in order, and the commit succeeds only when every WRITE
//! and the CLOSE succeeded, so nothing is published before. A copy stage's
//! writer also sends FSTAT of its handle and LSTAT of its destination there
//! (stage_evidence.rs, G3-02b). A sized writer starts at the pipeline depth
//! its channel learned (G3-06b).
use super::backend::SftpBackend;
use super::channel_pool::{note_failure, open_failed, ChannelLease, PoolChannel};
use super::connection::SftpConnection;
use super::enqueue::enqueue;
use super::io_adapters::Commit;
use super::io_err;
use super::segmented_stage::HandleCheck;
use super::session::SSH_WINDOW;
use super::stage_evidence::StageReport;
use crate::transfer::read_pipeline::Pipeline;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{Attrs, FileAttributes, OpenFlags, Status};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

#[path = "pool_writer_stamp.rs"]
mod stamp;

type WriteTask = JoinHandle<(Result<Status, SftpError>, Instant)>;
type CloseTask = JoinHandle<Result<Status, SftpError>>;
type StatTask = JoinHandle<Result<Attrs, SftpError>>;

struct InflightWrite {
    sent: Instant,
    full: bool,
    /// Bytes of the WRITE, counted once it is acknowledged.
    bytes: u64,
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
    /// Fastest acknowledgement, taught to the channel at the commit.
    fastest: Option<Duration>,
    /// Exact length of a sized copy stage (plan W1).
    expected: Option<u64>,
    accepted: u64,
    commit: Commit,
    state: WriteState,
    /// A copy stage reports what the server answered at its commit.
    report: Option<StageReport>,
    /// Unix seconds the stage gets at its commit (`pool_writer_stamp.rs`).
    mtime: Option<u32>,
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
        let task = enqueue(&self.rt, async move {
            let result = session.write(handle, offset, data).await;
            (result, Instant::now())
        });
        self.inflight.push_back(InflightWrite {
            sent: Instant::now(),
            full,
            bytes: length,
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
                let latency = arrived.saturating_duration_since(write.sent);
                self.pipeline.answered(latency, write.full);
                if write.full && self.fastest.is_none_or(|fastest| latency < fastest) {
                    self.fastest = Some(latency);
                }
                self.channel.moved(write.bytes);
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

    fn await_all(&mut self) -> io::Result<()> {
        while !self.inflight.is_empty() {
            self.await_oldest()?;
        }
        Ok(())
    }

    /// Sends the rest, waits for every acknowledgement and closes the
    /// handle; at a commit (`flush`) the length is checked first and the file
    /// synced as `commit` says. An unsynced commit sends CLOSE (and a copy
    /// stage its FSTAT and LSTAT) behind the last WRITE without waiting.
    fn complete(&mut self, commit: Option<Commit>) -> io::Result<()> {
        self.submit()?;
        let Some(commit) = commit else {
            self.await_all()?;
            return self.close_now();
        };
        check_length(self.expected, self.accepted)?;
        let stamp = self.send_stamp();
        let probes = self.send_probes();
        let syncs = commit.syncs(self.channel.fsync);
        let close = (!syncs).then(|| self.send_close());
        self.await_all()?;
        match close {
            Some(close) => self.await_close(close)?,
            None => {
                let session = self.channel.session.clone();
                let handle = self.handle.clone();
                self.sftp_step(async move { session.fsync(handle).await })?;
                self.close_now()?;
            }
        }
        self.channel.learn(self.pipeline.depth(), self.fastest);
        stamp.into_iter().for_each(|task| self.await_stamp(task));
        if let Some(probes) = probes {
            self.record(probes);
        }
        Ok(())
    }

    fn close_now(&mut self) -> io::Result<()> {
        let session = self.channel.session.clone();
        let handle = self.handle.clone();
        self.handle_open = false;
        self.sftp_step(async move { session.close(handle).await })?;
        drop(self.lease.take());
        Ok(())
    }

    /// CLOSE onto the channel now; the handle counts as closed.
    fn send_close(&mut self) -> CloseTask {
        let session = self.channel.session.clone();
        let handle = self.handle.clone();
        self.handle_open = false;
        enqueue(&self.rt, async move { session.close(handle).await })
    }

    fn await_close(&mut self, close: CloseTask) -> io::Result<()> {
        let answer = self
            .rt
            .block_on(close)
            .map_err(|error| io::Error::other(format!("SFTP-CLOSE wurde abgebrochen: {error}")))?;
        answer.map_err(|error| {
            note_failure(&self.connection, &self.channel, &error);
            io_err(error)
        })?;
        drop(self.lease.take());
        Ok(())
    }

    /// FSTAT of the stage and LSTAT of its destination behind the last
    /// WRITE, for a copy stage that reports.
    fn send_probes(&self) -> Option<(StatTask, Option<StatTask>)> {
        let report = self.report.as_ref()?;
        let session = self.channel.session.clone();
        let handle = self.handle.clone();
        let stage = enqueue(&self.rt, async move { session.fstat(handle).await });
        let destination = report.destination.clone().map(|destination| {
            let session = self.channel.session.clone();
            enqueue(&self.rt, async move { session.lstat(destination).await })
        });
        Some((stage, destination))
    }

    /// Keeps the committed stage's evidence; a probe without an answer
    /// leaves the receipts to ask themselves.
    fn record(&self, (stage, destination): (StatTask, Option<StatTask>)) {
        let Some(report) = self.report.as_ref() else {
            return;
        };
        let Ok(Ok(stage)) = self.rt.block_on(stage) else {
            return;
        };
        let destination = destination.and_then(|task| self.rt.block_on(task).ok());
        report.record(&stage, destination.as_ref(), self.accepted);
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
        match self.complete(Some(self.commit)) {
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
        if self.state == WriteState::Open && self.complete(None).is_ok() {
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
    /// A pipelined upload to `path` opened with `flags`, committed as
    /// `commit` says; `None` when no pool channel is available (the main
    /// session serves then). OPEN creates or truncates, so it is never
    /// repeated.
    pub(super) fn open_pool_writer(
        &self,
        path: &str,
        flags: OpenFlags,
        expected: Option<u64>,
        commit: Commit,
    ) -> io::Result<Option<PoolWriter>> {
        self.open_pool_writer_at(path, flags, 0, expected, commit, None)
    }

    /// Like `open_pool_writer`, but the first WRITE goes to byte `offset`:
    /// one range of a file several writers fill (`segmented_stage.rs`). The
    /// flags then name neither `CREATE` nor `TRUNCATE`. With a `check` the
    /// handle is looked at with `fstat` before anything is written to it; a
    /// file that fails it is closed untouched.
    pub(super) fn open_pool_writer_at(
        &self,
        path: &str,
        flags: OpenFlags,
        offset: u64,
        expected: Option<u64>,
        commit: Commit,
        check: Option<HandleCheck>,
    ) -> io::Result<Option<PoolWriter>> {
        // A range (it comes with the handle check) never fails for lack of a
        // channel; a stream without one goes over the main session.
        let leased = match check {
            Some(_) => self.pool.lease_range(self)?,
            None => self.pool.lease(self)?,
        };
        let Some(lease) = leased else {
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
        if let Some(check) = check {
            let attrs = self.rt.block_on(channel.session.fstat(handle.clone()));
            let verdict = match attrs {
                Ok(attrs) => check.verify(&attrs.attrs),
                Err(error) => {
                    note_failure(&self.connection, &channel, &error);
                    Err(io_err(error))
                }
            };
            if let Err(error) = verdict {
                let session = channel.session.clone();
                let closing = handle.clone();
                drop(self.rt.spawn(async move {
                    let _ = session.close(closing).await;
                }));
                return Err(error);
            }
        }
        let chunk = channel.write_chunk(handle.len());
        let cap = usize::try_from(u64::from(SSH_WINDOW) / chunk.max(1) as u64)
            .unwrap_or(usize::MAX)
            .max(1);
        let pipeline = match expected {
            Some(size) => {
                let needed = size.div_ceil(chunk.max(1) as u64);
                channel.seeded_pipeline(chunk as u64, cap, needed)
            }
            None => Pipeline::new(chunk as u64, cap),
        };
        Ok(Some(PoolWriter {
            rt: self.rt.clone(),
            connection: self.connection.clone(),
            channel,
            lease: Some(lease),
            handle,
            handle_open: true,
            chunk,
            buffer: Vec::with_capacity(chunk),
            offset,
            inflight: VecDeque::new(),
            pipeline,
            fastest: None,
            expected,
            accepted: 0,
            commit,
            state: WriteState::Open,
            report: None,
            mtime: None,
        }))
    }

    /// `open_pool_writer` of a copy stage of `size` bytes, created
    /// exclusively; an engine upload stage reports its evidence at the
    /// commit (stage_evidence.rs).
    pub(super) fn open_pool_stage_writer(
        &self,
        stage: &str,
        size: u64,
        commit: Commit,
        mtime: Option<u32>,
    ) -> io::Result<Option<PoolWriter>> {
        let opened = self.open_pool_writer(
            stage,
            super::backend::create_new_flags(),
            Some(size),
            commit,
        )?;
        Ok(opened.map(|mut writer| {
            writer.report = StageReport::for_stage(stage, &self.written);
            writer.with_mtime(mtime)
        }))
    }
}
