//! A copy stage of known length uploaded while it is written (plan W1): one
//! PUT with `Content-Length` and `If-None-Match: *` whose body a background
//! thread pulls from a small pipe the writer fills, instead of spooling the
//! whole file to a local temp file first. The body has no size ureq knows,
//! so ureq never replays it (docs/refs/gdrive-ureq-throughput.md §8). The
//! writer fails `flush` unless exactly the announced length came; a writer
//! dropped before that aborts the body, and a server discards a PUT whose
//! body ended early.
use super::status::overload_or_full;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// Bytes waiting between the writer and the upload thread. The socket's
/// send buffer keeps the link busy while the writer refills the pipe, so a
/// quarter of an engine copy block (1 MiB, recherche §2) is enough; it is
/// the stream's own buffer like a socket's, not read-ahead.
const PIPE_CAPACITY: usize = 256 * 1024;

#[derive(Default)]
struct PipeState {
    chunks: VecDeque<Vec<u8>>,
    queued: usize,
    /// All bytes are in: the body ends after the queued chunks.
    finished: bool,
    /// The writer gave up: the body fails, so the PUT never completes.
    aborted: bool,
    /// The upload thread stopped reading (the request ended).
    closed: bool,
}

#[derive(Default)]
struct Pipe {
    state: Mutex<PipeState>,
    changed: Condvar,
}

impl Pipe {
    fn lock(&self) -> MutexGuard<'_, PipeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, PipeState>) -> MutexGuard<'a, PipeState> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

struct PipeReader {
    pipe: Arc<Pipe>,
    current: Vec<u8>,
    consumed: usize,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.consumed >= self.current.len() {
            let mut state = self.pipe.lock();
            loop {
                if let Some(chunk) = state.chunks.pop_front() {
                    state.queued -= chunk.len();
                    self.current = chunk;
                    self.consumed = 0;
                    self.pipe.changed.notify_all();
                    break;
                }
                if state.aborted {
                    // Not `Interrupted`: `io::copy` would retry that forever.
                    return Err(io::Error::other("WebDAV-Upload wurde abgebrochen"));
                }
                if state.finished {
                    return Ok(0);
                }
                state = self.pipe.wait(state);
            }
        }
        let available = &self.current[self.consumed..];
        let count = available.len().min(out.len());
        out[..count].copy_from_slice(&available[..count]);
        self.consumed += count;
        Ok(count)
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.pipe.lock().closed = true;
        self.pipe.changed.notify_all();
    }
}

fn put(agent: &ureq::Agent, url: &str, auth: &str, size: u64, mtime_ms: Option<i64>, body: PipeReader) -> io::Result<()> {
    let request = agent
        .put(url)
        .set("Content-Length", &size.to_string())
        .set("If-None-Match", "*");
    let request = match mtime_ms.map(|ms| ms.div_euclid(1_000)).filter(|seconds| *seconds > 86_400) {
        Some(seconds) => request.set("X-OC-Mtime", &seconds.to_string()).set("X-Hash", "md5"),
        None => request,
    };
    let request = if auth.is_empty() {
        request
    } else {
        request.set("Authorization", auth)
    };
    let response = request.send(body).map_err(|error| {
        if let Some(mapped) = overload_or_full(&error) {
            return mapped;
        }
        let kind = match &error {
            ureq::Error::Status(412, _) => io::ErrorKind::AlreadyExists,
            ureq::Error::Status(401 | 403, _) => io::ErrorKind::PermissionDenied,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, error.to_string())
    })?;
    // A created representation answers 201; 202 does not confirm it.
    match response.status() {
        201 => Ok(()),
        status => Err(io::Error::other(format!(
            "WebDAV PUT returned unexpected HTTP status {status}"
        ))),
    }
}

#[derive(Clone, PartialEq, Eq)]
enum PutState {
    Open,
    Committed,
    Failed(io::ErrorKind, String),
}

pub(super) struct StreamPut {
    pipe: Arc<Pipe>,
    upload: Option<JoinHandle<io::Result<()>>>,
    expected: u64,
    written: u64,
    state: PutState,
}

impl StreamPut {
    /// Starts the PUT of exactly `size` bytes to `url` (a new name only).
    pub(super) fn start(
        agent: ureq::Agent,
        url: String,
        auth: String,
        size: u64,
    ) -> io::Result<Self> {
        Self::start_timed(agent, url, auth, size, None)
    }

    pub(super) fn start_timed(
        agent: ureq::Agent, url: String, auth: String, size: u64, mtime_ms: Option<i64>,
    ) -> io::Result<Self> {
        let pipe = Arc::new(Pipe::default());
        let body = PipeReader {
            pipe: pipe.clone(),
            current: Vec::new(),
            consumed: 0,
        };
        let upload = std::thread::Builder::new()
            .name("webdav-put".to_string())
            .spawn(move || put(&agent, &url, &auth, size, mtime_ms, body))?;
        Ok(Self {
            pipe,
            upload: Some(upload),
            expected: size,
            written: 0,
            state: PutState::Open,
        })
    }

    /// The upload's own result once its thread ended.
    fn outcome(&mut self) -> io::Result<()> {
        match self.upload.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(io::Error::other("WebDAV-Upload endete unerwartet")),
            None => Err(io::Error::other("WebDAV-Upload ist bereits beendet")),
        }
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.state = PutState::Failed(error.kind(), error.to_string());
        self.pipe.lock().aborted = true;
        self.pipe.changed.notify_all();
        error
    }
}

impl Write for StreamPut {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match &self.state {
            PutState::Open => {}
            PutState::Committed => {
                return Err(io::Error::other("Upload bereits abgeschlossen"));
            }
            PutState::Failed(kind, message) => return Err(io::Error::new(*kind, message.clone())),
        }
        if data.is_empty() {
            return Ok(0);
        }
        if self.written.saturating_add(data.len() as u64) > self.expected {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                "Quelle ist während der Übertragung gewachsen",
            );
            return Err(self.fail(error));
        }
        let mut state = self.pipe.lock();
        while state.queued > 0 && state.queued + data.len() > PIPE_CAPACITY && !state.closed {
            state = self.pipe.wait(state);
        }
        if state.closed {
            // The request ended before its body did: the server answered early.
            drop(state);
            let error = self
                .outcome()
                .err()
                .unwrap_or_else(|| io::Error::other("WebDAV-Server beendete den Upload vorzeitig"));
            return Err(self.fail(error));
        }
        state.chunks.push_back(data.to_vec());
        state.queued += data.len();
        drop(state);
        self.pipe.changed.notify_all();
        self.written += data.len() as u64;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match &self.state {
            PutState::Open => {}
            PutState::Committed => return Ok(()),
            PutState::Failed(kind, message) => {
                return Err(io::Error::new(
                    *kind,
                    format!("WebDAV-PUT fehlgeschlagen; der Upload wird nicht automatisch wiederholt: {message}"),
                ))
            }
        }
        if self.written != self.expected {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Quelle hat sich während der Übertragung geändert: {} statt {} Bytes",
                    self.written, self.expected
                ),
            );
            return Err(self.fail(error));
        }
        self.pipe.lock().finished = true;
        self.pipe.changed.notify_all();
        match self.outcome() {
            Ok(()) => {
                self.state = PutState::Committed;
                Ok(())
            }
            Err(error) => {
                self.state = PutState::Failed(error.kind(), error.to_string());
                Err(error)
            }
        }
    }
}

impl Drop for StreamPut {
    fn drop(&mut self) {
        if self.state == PutState::Open {
            // The body fails, so the server never completes this PUT; the
            // thread ends on its own.
            self.pipe.lock().aborted = true;
            self.pipe.changed.notify_all();
        }
    }
}
