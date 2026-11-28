//! The readers under the searcher of a content search: one counts the bytes
//! (progress, totals) and ends a long file when the search stops, one checks
//! whether a file is an ANSI candidate (no BOM, not valid UTF-8), one keeps
//! the bytes of a reader that cannot be opened again.
use std::cell::{Cell, RefCell};
use std::io::{self, Read};

use super::content_run::ContentRun;

/// The text of the read error that ends a file because the whole search
/// stopped (canceled, `max_hits` reached, receiver gone); never reported.
pub(super) const STOPPED: &str = "content search stopped";

/// Counts what is read (into the run and `read`) and stops between two reads.
pub(super) struct CountingReader<'r, 'q, R> {
    inner: R,
    run: &'r ContentRun<'q>,
    read: &'r Cell<u64>,
}

impl<'r, 'q, R> CountingReader<'r, 'q, R> {
    pub(super) fn new(inner: R, run: &'r ContentRun<'q>, read: &'r Cell<u64>) -> Self {
        Self { inner, run, read }
    }
}

impl<R: Read> Read for CountingReader<'_, '_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.run.halted() {
            return Err(io::Error::other(STOPPED));
        }
        let read = self.inner.read(buffer)?;
        self.read.set(self.read.get() + read as u64);
        self.run.add_bytes(read as u64);
        Ok(read)
    }
}

/// Bytes of a file read through one reader (no second open) kept for its
/// Windows-1252 pass; a longer file gets no such pass.
pub(super) const ANSI_KEEP_BYTES: usize = 16 * 1024 * 1024;

/// Passes the bytes of a single reader on and keeps them (`kept`, `None`
/// once more than `ANSI_KEEP_BYTES` came) for the Windows-1252 pass.
pub(super) struct Tee<'t> {
    pub(super) inner: &'t mut dyn Read,
    pub(super) kept: &'t RefCell<Option<Vec<u8>>>,
    pub(super) read: &'t Cell<u64>,
}

impl Read for Tee<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.read.set(self.read.get() + read as u64);
        let mut kept = self.kept.borrow_mut();
        match kept.as_mut() {
            Some(bytes) if bytes.len() + read <= ANSI_KEEP_BYTES => {
                bytes.extend_from_slice(&buffer[..read]);
            }
            Some(_) => *kept = None,
            None => {}
        }
        Ok(read)
    }
}

/// Watches the raw bytes of a file: a BOM at its start, and whether all of it
/// is valid UTF-8 (sequences split between two reads included).
pub(super) struct Utf8Probe<R> {
    inner: R,
    active: bool,
    started: bool,
    bom: bool,
    invalid: bool,
    /// The bytes of a sequence the last read ended in.
    carry: Vec<u8>,
}

impl<R> Utf8Probe<R> {
    /// `active` = the probe looks at the bytes (only a pattern beyond ASCII
    /// needs the answer).
    pub(super) fn new(inner: R, active: bool) -> Self {
        Self {
            inner,
            active,
            started: false,
            bom: false,
            invalid: false,
            carry: Vec::new(),
        }
    }

    /// No BOM and not valid UTF-8 (a sequence cut by the end of the file
    /// counts as invalid).
    pub(super) fn ansi_candidate(&self) -> bool {
        self.active && !self.bom && (self.invalid || !self.carry.is_empty())
    }

    fn inspect(&mut self, bytes: &[u8]) {
        if !self.started && !bytes.is_empty() {
            self.started = true;
            self.bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF])
                || bytes.starts_with(&[0xFF, 0xFE])
                || bytes.starts_with(&[0xFE, 0xFF]);
        }
        if self.invalid || self.bom {
            return;
        }
        let joined;
        let bytes = if self.carry.is_empty() {
            bytes
        } else {
            let mut carried = std::mem::take(&mut self.carry);
            carried.extend_from_slice(bytes);
            joined = carried;
            &joined[..]
        };
        match std::str::from_utf8(bytes) {
            Ok(_) => {}
            Err(error) if error.error_len().is_none() => {
                self.carry = bytes[error.valid_up_to()..].to_vec();
            }
            Err(_) => self.invalid = true,
        }
    }
}

impl<R: Read> Read for Utf8Probe<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        if self.active {
            self.inspect(&buffer[..read]);
        }
        Ok(read)
    }
}
