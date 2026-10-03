//! Short and interrupted I/O probes for the existing vault frame fixtures.
use std::io::{self, ErrorKind, Read, Write};

#[derive(Default)]
pub(super) struct ProbeWriter {
    pub(super) bytes: Vec<u8>,
    pub(super) calls: usize,
    pub(super) flushes: usize,
    pub(super) chunk: Option<usize>,
    pub(super) interrupt_on: Option<usize>,
    pub(super) write_error: Option<ErrorKind>,
    pub(super) flush_error: Option<ErrorKind>,
}

impl Write for ProbeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.interrupt_on == Some(self.calls) {
            return Err(ErrorKind::Interrupted.into());
        }
        if let Some(kind) = self.write_error {
            return Err(kind.into());
        }
        let accepted = self.chunk.unwrap_or(bytes.len()).min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..accepted]);
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        self.flush_error.map_or(Ok(()), |kind| Err(kind.into()))
    }
}

pub(super) struct InterruptedReader<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) calls: usize,
}

impl Read for InterruptedReader<'_> {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        // Before the header, midway through it, then inside the body.
        if [1, 4, 8].contains(&self.calls) {
            return Err(ErrorKind::Interrupted.into());
        }
        let length = self.bytes.len().min(destination.len()).min(1);
        destination[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        Ok(length)
    }
}
