//! Test stream faults and admission counters follow the stream lifetime.
use std::io::{self, Read, Write};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::{Counters, ReadFault};

pub(super) struct Guard {
    pub(super) counters: Arc<Counters>,
    pub(super) reading: bool,
}

impl Guard {
    pub(super) fn new(counters: &Arc<Counters>, reading: bool) -> Self {
        let active = counters.active.fetch_add(1, Ordering::SeqCst) + 1;
        counters.max_active.fetch_max(active, Ordering::SeqCst);
        let (mine, other) = if reading {
            (&counters.reads_open, &counters.writes_open)
        } else {
            (&counters.writes_open, &counters.reads_open)
        };
        mine.fetch_add(1, Ordering::SeqCst);
        if other.load(Ordering::SeqCst) > 0 {
            counters.overlap.store(true, Ordering::SeqCst);
        }
        Self {
            counters: counters.clone(),
            reading,
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.counters.active.fetch_sub(1, Ordering::SeqCst);
        let mine = if self.reading {
            &self.counters.reads_open
        } else {
            &self.counters.writes_open
        };
        mine.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) struct FakeReader {
    pub(super) inner: Box<dyn Read + Send>,
    pub(super) fault: Option<ReadFault>,
    pub(super) delivered: usize,
    pub(super) delay: Duration,
    pub(super) extra: Option<Vec<u8>>,
    pub(super) _guard: Guard,
}

impl Read for FakeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        if let Some(fault) = self.fault {
            let allowed = fault.after.unwrap_or(0).saturating_sub(self.delivered);
            if allowed == 0 {
                return Err(io::Error::new(fault.kind, "fixture read fault"));
            }
            let limit = allowed.min(buffer.len());
            let read = self.inner.read(&mut buffer[..limit])?;
            self.delivered += read;
            return Ok(read);
        }
        let read = self.inner.read(buffer)?;
        if read == 0 {
            if let Some(extra) = self.extra.take() {
                let length = extra.len().min(buffer.len());
                buffer[..length].copy_from_slice(&extra[..length]);
                return Ok(length);
            }
        }
        self.delivered += read;
        Ok(read)
    }
}

pub(super) struct FakeWriter {
    pub(super) inner: Box<dyn Write + Send>,
    pub(super) fault: Option<io::ErrorKind>,
    pub(super) _guard: Guard,
}

impl Write for FakeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.fault {
            return Err(io::Error::new(kind, "fixture write fault"));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
