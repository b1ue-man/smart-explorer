//! The streamed upload of one `Write`, `WriteNew`, `WriteNewTimed` or
//! `WriteAt` request: data frames after the server's `Progress`, then `End`
//! and the commit reply (`Ok`, or `StageWritten` for a timed stage).
use super::super::mux::Mux;
use super::super::pool::ChannelLease;
use super::super::route::RequestRx;
use crate::agent_proto::{BlockSender, Frame, CHUNK};
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

/// `std::io::Write` over a streamed `Write` op.
struct AgentWriteStream {
    /// Shared by the ranges of one stage, which all run on its channel.
    lease: Arc<ChannelLease>,
    mux: Arc<Mux>,
    id: u64,
    rx: RequestRx,
    state: WriteState,
    /// Packs the blocks when the server announced `lz4-v1`.
    lz4: Option<BlockSender>,
    /// Bytes this stream must carry exactly (a range of a stage).
    expected: Option<u64>,
    sent: u64,
    /// Takes a `WriteNewTimed` commit's `StageWritten` (`stage_time.rs`).
    report: Option<StageReport>,
}

/// Receives the size, time and identity a timed stage was committed with.
pub(in crate::agent) type StageReport = Box<dyn FnOnce(u64, i64, Option<String>) + Send>;

enum WriteState {
    Open,
    Committed,
    Failed {
        kind: io::ErrorKind,
        message: String,
    },
}

impl AgentWriteStream {
    fn finish(&mut self) -> io::Result<()> {
        match &self.state {
            WriteState::Committed => return Ok(()),
            WriteState::Failed { kind, message } => {
                return Err(io::Error::new(*kind, message.clone()))
            }
            WriteState::Open => {}
        }
        if let Some(expected) = self.expected.filter(|expected| *expected != self.sent) {
            // A range that is short never reaches the server's End.
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} statt {expected} Bytes für den Bereich", self.sent),
            );
            let _ = self.mux.send(self.id, Frame::Cancel);
            self.mux.unregister(self.id);
            self.remember_failure(&error);
            return Err(error);
        }
        if let Err(error) = self.mux.send(self.id, Frame::End) {
            self.mux.unregister(self.id);
            self.remember_failure(&error);
            return Err(error);
        }
        let result = match self.rx.recv() {
            Ok(Frame::Ok) => Ok(()),
            Ok(Frame::StageWritten {
                size,
                mtime_ms,
                object,
            }) if self.report.is_some() => {
                if let Some(report) = self.report.take() {
                    report(size, mtime_ms, object);
                }
                Ok(())
            }
            Ok(Frame::Err(e)) => Err(super::super::agent_error::agent_error(e)),
            Ok(other) => {
                self.lease.invalidate(&self.mux);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected agent write-stream reply: {other:?}"),
                ))
            }
            Err(_) => {
                self.lease.invalidate(&self.mux);
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "agent write stream closed",
                ))
            }
        };
        self.mux.unregister(self.id);
        match &result {
            Ok(()) => self.state = WriteState::Committed,
            Err(error) => self.remember_failure(error),
        }
        result
    }

    fn remember_failure(&mut self, error: &io::Error) {
        self.state = WriteState::Failed {
            kind: error.kind(),
            message: error.to_string(),
        };
    }
}

impl Write for AgentWriteStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !matches!(self.state, WriteState::Open) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "agent write stream already closed",
            ));
        }
        let written = buf.len().min(CHUNK);
        if let Some(expected) = self.expected {
            if self.sent.saturating_add(written as u64) > expected {
                let error = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Quelle liefert mehr als die angekündigten {expected} Bytes"),
                );
                let _ = self.mux.send(self.id, Frame::Cancel);
                self.mux.unregister(self.id);
                self.remember_failure(&error);
                return Err(error);
            }
        }
        let chunk = &buf[..written];
        let frame = match self
            .lz4
            .as_mut()
            .and_then(|sender| sender.try_pack(chunk, self.sent))
        {
            Some(block) => Frame::DataLz4(block),
            None => Frame::Data(chunk.to_vec()),
        };
        let started = Instant::now();
        if let Err(error) = self.mux.send(self.id, frame) {
            // The server may have ended the upload; its reply names the
            // reason. Either way the upload is over: never commit it later.
            let error = match self.rx.try_recv() {
                Ok(Frame::Err(message)) => super::super::agent_error::agent_error(message),
                _ => error,
            };
            let _ = self.mux.send(self.id, Frame::Cancel);
            self.mux.unregister(self.id);
            self.remember_failure(&error);
            return Err(error);
        }
        if let Some(sender) = self.lz4.as_mut() {
            sender.record_send(written, started.elapsed(), Instant::now());
        }
        self.sent += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for AgentWriteStream {
    fn drop(&mut self) {
        if matches!(self.state, WriteState::Open) {
            // Dropping a writer is an abort, not an implicit commit. Wake the
            // server-side receiver after setting cancellation so it refuses
            // promotion; cleanup remains subject to stable ownership proof.
            let _ = self.mux.send(self.id, Frame::Cancel);
            let _ = self.mux.send(self.id, Frame::End);
            self.mux.unregister(self.id);
        }
    }
}

/// Starts the upload `request` (`Write`, `WriteNew` or `WriteAt`) on `mux` and
/// waits for the server's `Progress`: from then on the returned writer
/// streams the bytes, as `DataLz4` blocks when `lz4` and they shrink. With
/// `expected` it also commits only after exactly that many bytes.
pub(in crate::agent) fn open_write_on(
    lease: Arc<ChannelLease>,
    mux: Arc<Mux>,
    request: Frame,
    expected: Option<u64>,
    lz4: bool,
) -> io::Result<Box<dyn Write + Send>> {
    open_write_with(lease, mux, request, (expected, None), lz4)
}

pub(in crate::agent) fn open_write_with(
    lease: Arc<ChannelLease>,
    mux: Arc<Mux>,
    request: Frame,
    (expected, report): (Option<u64>, Option<StageReport>),
    lz4: bool,
) -> io::Result<Box<dyn Write + Send>> {
    let (id, rx) = mux.register();
    if let Err(error) = mux.send(id, request) {
        mux.unregister(id);
        return Err(error);
    }
    let result = match rx.recv() {
        Ok(Frame::Progress { .. }) => {
            let lz4 = lz4.then(|| {
                let mut sender = BlockSender::new();
                sender.start_file();
                sender
            });
            return Ok(Box::new(AgentWriteStream {
                lease,
                mux,
                id,
                rx,
                state: WriteState::Open,
                lz4,
                expected,
                sent: 0,
                report,
            }));
        }
        Ok(Frame::Err(error)) => Err(super::super::agent_error::agent_error(error)),
        Ok(other) => {
            lease.invalidate(&mux);
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected agent reply to write: {other:?}"),
            ))
        }
        Err(_) => {
            lease.invalidate(&mux);
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "agent write stream closed before opening",
            ))
        }
    };
    mux.unregister(id);
    result
}
