//! Reading one file with many READs on the wire at once, delivered strictly
//! in offset order. The source (an SFTP channel, or a script in tests) only
//! issues a READ and hands back its answer; this type decides what to ask,
//! in which order to deliver and where the file ends.
//!
//! SFTP answers a READ with data, with fewer bytes than asked, or with an
//! end-of-file status. "For normal disk files, it is guaranteed that this
//! will read the specified number of bytes, or up to end of file"
//! (draft-ietf-secsh-filexfer-02 §6.4): a short answer normally marks the
//! end. The READ after it is usually in flight already; when it reports the
//! end too, the file ends where the short answer stopped and no extra round
//! trip is spent. When data follows instead (a device, a growing file), the
//! missing range is read first, so no byte is ever skipped.
use super::pipeline::Pipeline;
use std::collections::VecDeque;
use std::io::{self, Read};
use std::time::Instant;

/// The answer to one READ.
pub(super) enum Reply {
    Data(Vec<u8>),
    Eof,
}

/// Where READs go. `wait` returns the answer and when it arrived.
pub(super) trait ReadSource {
    type Pending;
    fn issue(&mut self, offset: u64, len: u32) -> Self::Pending;
    fn wait(&mut self, pending: Self::Pending) -> (io::Result<Reply>, Instant);
    fn cancel(&mut self, pending: Self::Pending);
    /// The file is finished (end, error or drop); called once.
    fn close(&mut self);
}

enum Outcome<P> {
    Pending(P),
    Ready(io::Result<Reply>),
}

struct Request<P> {
    offset: u64,
    len: u32,
    sent: Instant,
    outcome: Outcome<P>,
}

pub(super) struct PipelinedRead<S: ReadSource> {
    source: S,
    pipeline: Pipeline,
    chunk: u32,
    /// Contiguous READs in offset order; the front one is delivered next.
    queue: VecDeque<Request<S::Pending>>,
    next_offset: u64,
    buffer: Vec<u8>,
    consumed: usize,
    /// Range a short answer left open, still to be settled.
    gap: Option<(u64, u32)>,
    delivered: u64,
    done: bool,
    /// A failed stream keeps failing instead of looking like a short file.
    failure: Option<(io::ErrorKind, String)>,
}

impl<S: ReadSource> PipelinedRead<S> {
    pub(super) fn new(source: S, start: u64, chunk: u32, pipeline: Pipeline) -> Self {
        Self {
            source,
            pipeline,
            chunk: chunk.max(1),
            queue: VecDeque::new(),
            next_offset: start,
            buffer: Vec::new(),
            consumed: 0,
            gap: None,
            delivered: 0,
            done: false,
            failure: None,
        }
    }

    /// Bytes handed to the caller so far.
    pub(super) fn delivered(&self) -> u64 {
        self.delivered
    }

    pub(super) fn source(&self) -> &S {
        &self.source
    }

    fn top_up(&mut self) {
        while !self.done && self.gap.is_none() && self.queue.len() < self.pipeline.depth() {
            let offset = self.next_offset;
            let pending = self.source.issue(offset, self.chunk);
            self.queue.push_back(Request {
                offset,
                len: self.chunk,
                sent: Instant::now(),
                outcome: Outcome::Pending(pending),
            });
            self.next_offset = offset.saturating_add(u64::from(self.chunk));
        }
    }

    fn push_front_read(&mut self, offset: u64, len: u32) {
        let pending = self.source.issue(offset, len);
        self.queue.push_front(Request {
            offset,
            len,
            sent: Instant::now(),
            outcome: Outcome::Pending(pending),
        });
    }

    /// Waits for `request` if needed and feeds its timing to the pipeline.
    fn settle(&mut self, request: Request<S::Pending>) -> Request<S::Pending> {
        let Request {
            offset,
            len,
            sent,
            outcome,
        } = request;
        let result = match outcome {
            Outcome::Ready(result) => result,
            Outcome::Pending(pending) => {
                let (result, arrived) = self.source.wait(pending);
                let full = matches!(&result, Ok(Reply::Data(data)) if data.len() == len as usize);
                self.pipeline
                    .answered(arrived.saturating_duration_since(sent), full);
                result
            }
        };
        Request {
            offset,
            len,
            sent,
            outcome: Outcome::Ready(result),
        }
    }

    fn finish(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        self.gap = None;
        while let Some(request) = self.queue.pop_front() {
            if let Outcome::Pending(pending) = request.outcome {
                self.source.cancel(pending);
            }
        }
        self.source.close();
    }

    /// Decides a pending gap: `true` when the file ends where it starts.
    fn gap_is_end(&mut self, offset: u64, len: u32) -> bool {
        let Some(next) = self.queue.pop_front() else {
            // Nothing beyond it in flight: ask for the range itself.
            self.push_front_read(offset, len);
            return false;
        };
        let next = self.settle(next);
        let beyond_is_end = matches!(&next.outcome, Outcome::Ready(Ok(Reply::Eof)))
            || matches!(&next.outcome, Outcome::Ready(Ok(Reply::Data(data))) if data.is_empty());
        if beyond_is_end {
            return true;
        }
        // Data (or an error) lies beyond: the range is real, read it first.
        self.queue.push_front(next);
        self.push_front_read(offset, len);
        false
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.failure = Some((error.kind(), error.to_string()));
        self.finish();
        error
    }

    /// Makes the next bytes available; `Ok(false)` at the end of the file.
    fn fill(&mut self) -> io::Result<bool> {
        loop {
            if let Some((kind, message)) = &self.failure {
                return Err(io::Error::new(*kind, message.clone()));
            }
            if self.done {
                return Ok(false);
            }
            if let Some((offset, len)) = self.gap.take() {
                if self.gap_is_end(offset, len) {
                    self.finish();
                    return Ok(false);
                }
            }
            self.top_up();
            let Some(request) = self.queue.pop_front() else {
                self.finish();
                return Ok(false);
            };
            let request = self.settle(request);
            let Outcome::Ready(result) = request.outcome else {
                continue;
            };
            let data = match result {
                Ok(Reply::Data(data)) if !data.is_empty() => data,
                Ok(_) => {
                    self.finish();
                    return Ok(false);
                }
                Err(error) => return Err(self.fail(error)),
            };
            let got = data.len() as u64;
            let asked = u64::from(request.len);
            if got > asked {
                return Err(self.fail(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SFTP-Server lieferte mehr Daten als angefordert",
                )));
            }
            if got < asked {
                // `got < asked <= u32::MAX`, so the rest fits in u32.
                self.gap = Some((request.offset.saturating_add(got), request.len - got as u32));
            }
            self.buffer = data;
            self.consumed = 0;
            return Ok(true);
        }
    }
}

impl<S: ReadSource> Read for PipelinedRead<S> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.consumed >= self.buffer.len() && !self.fill()? {
            return Ok(0);
        }
        let available = &self.buffer[self.consumed..];
        let count = available.len().min(out.len());
        out[..count].copy_from_slice(&available[..count]);
        self.consumed += count;
        self.delivered += count as u64;
        Ok(count)
    }
}

impl<S: ReadSource> Drop for PipelinedRead<S> {
    fn drop(&mut self) {
        self.finish();
    }
}
