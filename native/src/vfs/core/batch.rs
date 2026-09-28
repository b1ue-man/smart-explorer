//! Many small whole files in one round trip. Backends whose peer speaks a
//! batch protocol (Share hosts, the SSH agent, the background service) accept
//! a list of files and move them in one stream instead of one request, stage
//! and commit round trip per file.
use std::io;

/// Upper bounds a backend accepts for one batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchLimits {
    pub max_files: usize,
    pub max_bytes: u64,
}

/// One new file of a `put_batch`: its requested final path and exact length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchPut {
    pub path: String,
    pub size: u64,
}

/// Result of one `put_batch` entry.
#[derive(Debug)]
pub enum BatchPutOutcome {
    /// Published under this path; it differs from the request when the name
    /// was taken and a numbered name ("name (2).ext") was chosen instead.
    Published(String),
    Failed(io::Error),
}

/// One file of a `get_batch`: its path, backend id when known (duplicate
/// names on ID-addressed providers) and the length the caller expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchGet {
    pub path: String,
    pub id: Option<String>,
    pub size: u64,
}

/// Receives the files of a `get_batch` in request order. Returning `Err` from
/// any method aborts the batch; the backend stops sending and returns it.
pub trait BatchSink {
    /// Item `index` follows with exactly `size` bytes.
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()>;
    /// Next bytes of item `index`.
    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()>;
    /// Item `index` is complete. `result` is `Err` when the source failed or
    /// changed after `begin`; the received bytes must then be discarded.
    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()>;
    /// Item `index` could not be read at all (nothing was sent for it).
    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()>;
}
