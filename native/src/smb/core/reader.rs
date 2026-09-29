//! Downloads with several positioned READs of one file on the wire at once.
//! smb2's `FileReader::read_at` takes `&self` and keeps no cursor, so
//! "concurrent positioned reads over one reader are independent and pipeline
//! over the single SMB session" (smb2 stream.rs): a few of them run as tasks
//! on the backend runtime and are delivered in offset order. How many is the
//! shared pipeline's decision (it grows while answers come about as fast as
//! the fastest one), never more than the credit window funds
//! (`Connection::credit_capacity_for`); READs beyond that would only wait
//! for credits. The size seen at open bounds the READs; a short answer means
//! the file shrank since, so the stream ends there instead of skipping bytes.
use super::errors;
use super::session::Generation;
use crate::sftp::Pipeline;
use smb2::FileReader;
use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// Bytes a READ asks for when the server allows at least this much: smb2's
/// own download chunk (`DOWNLOAD_CHUNK_SIZE`, 512 KiB), chosen so a slow link
/// still delivers a chunk about every 1.4 s while a fast one needs few.
const READ_CHUNK: u32 = smb2::DOWNLOAD_CHUNK_SIZE;
/// Every dialect allows 64 KiB READs (MS-SMB2 3.3.5.4).
const MIN_MAX_READ: u32 = 65_536;

type ReadTask = JoinHandle<(smb2::Result<Vec<u8>>, Instant)>;

struct InflightRead {
    len: u64,
    sent: Instant,
    task: ReadTask,
}

pub(super) struct SmbReader {
    reader: Option<Arc<FileReader>>,
    size: u64,
    next: u64,
    chunk: u64,
    inflight: VecDeque<InflightRead>,
    pipeline: Pipeline,
    buffer: Vec<u8>,
    consumed: usize,
    ended: bool,
    /// A failed stream keeps failing instead of looking like a short file.
    failure: Option<(io::ErrorKind, String)>,
    path: String,
    generation: Arc<Generation>,
    rt: Arc<Runtime>,
}

impl SmbReader {
    /// A reader from byte `start` (a resumed download starts mid-file).
    pub(super) fn new(
        reader: FileReader,
        start: u64,
        path: &str,
        generation: Arc<Generation>,
        rt: Arc<Runtime>,
    ) -> Self {
        let conn = generation.connection();
        let max_read = conn
            .params()
            .map_or(MIN_MAX_READ, |params| params.max_read_size)
            .max(MIN_MAX_READ);
        let chunk = u64::from(READ_CHUNK.min(max_read));
        let cap = conn.credit_capacity_for(chunk);
        Self {
            size: reader.size(),
            reader: Some(Arc::new(reader)),
            next: start,
            chunk,
            inflight: VecDeque::new(),
            pipeline: Pipeline::new(chunk, cap),
            buffer: Vec::new(),
            consumed: 0,
            ended: false,
            failure: None,
            path: path.to_string(),
            generation,
            rt,
        }
    }

    fn top_up(&mut self) {
        let Some(reader) = self.reader.clone() else {
            return;
        };
        while !self.ended && self.next < self.size && self.inflight.len() < self.pipeline.depth() {
            let offset = self.next;
            let len = self.chunk.min(self.size - offset);
            let reader = reader.clone();
            let task = self.rt.spawn(async move {
                let result = reader.read_at(offset, len).await;
                (result, Instant::now())
            });
            self.inflight.push_back(InflightRead {
                len,
                sent: Instant::now(),
                task,
            });
            self.next = offset + len;
        }
    }

    /// Ends the stream: READs in flight are cancelled and awaited, so no
    /// task keeps the reader when it is closed.
    fn stop(&mut self) {
        self.ended = true;
        while let Some(read) = self.inflight.pop_front() {
            read.task.abort();
            let _ = self.rt.block_on(read.task);
        }
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.failure = Some((error.kind(), error.to_string()));
        self.stop();
        error
    }

    /// Makes the next bytes available; `Ok(false)` at the end of the file.
    fn fill(&mut self) -> io::Result<bool> {
        if let Some((kind, message)) = &self.failure {
            return Err(io::Error::new(*kind, message.clone()));
        }
        self.top_up();
        let Some(read) = self.inflight.pop_front() else {
            self.ended = true;
            return Ok(false);
        };
        let (result, arrived) = match self.rt.block_on(read.task) {
            Ok(answer) => answer,
            Err(error) => {
                let error = io::Error::other(format!("SMB-Leseauftrag wurde abgebrochen: {error}"));
                return Err(self.fail(error));
            }
        };
        let data = match result {
            Ok(data) => data,
            Err(error) => {
                self.generation.note(&error);
                let error = errors::map(error, "Lesen", &self.path);
                return Err(self.fail(error));
            }
        };
        let got = data.len() as u64;
        self.pipeline.answered(
            arrived.saturating_duration_since(read.sent),
            got == read.len,
        );
        if got < read.len {
            // Shorter than the size seen at open allows: the file shrank.
            self.stop();
        }
        if data.is_empty() {
            return Ok(false);
        }
        self.buffer = data;
        self.consumed = 0;
        Ok(true)
    }
}

impl Read for SmbReader {
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
        Ok(count)
    }
}

impl Drop for SmbReader {
    fn drop(&mut self) {
        self.stop();
        // smb2 has no async Drop: close the handle here or it stays open
        // until the session ends. Best effort.
        if let Some(reader) = self.reader.take() {
            if let Ok(reader) = Arc::try_unwrap(reader) {
                let _ = self.rt.block_on(reader.close());
            }
        }
    }
}
