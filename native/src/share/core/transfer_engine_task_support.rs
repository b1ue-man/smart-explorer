//! Helpers of the Share transfer v1 loopback tests.
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::vfs::{BatchGet, BatchPut, BatchPutOutcome, BatchSink};

pub(super) fn put(path: &str, size: u64) -> BatchPut {
    BatchPut {
        path: path.into(),
        size,
    }
}

pub(super) fn get(path: &str, size: u64) -> BatchGet {
    BatchGet {
        path: path.into(),
        id: None,
        size,
    }
}

pub(super) fn describe(outcomes: &[BatchPutOutcome]) -> Vec<String> {
    outcomes
        .iter()
        .map(|outcome| match outcome {
            BatchPutOutcome::Published(path) => path.clone(),
            BatchPutOutcome::Failed(error) => format!("failed {:?}", error.kind()),
        })
        .collect()
}

pub(super) fn stage_names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.contains(".se-") {
            names.push(name);
        }
    }
    Ok(names)
}

/// The host cleans up after a reset on its own worker; give it a moment.
pub(super) fn stages_after_cleanup(dir: &Path) -> io::Result<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let names = stage_names(dir)?;
        if names.is_empty() || Instant::now() >= deadline {
            return Ok(names);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn pattern(length: usize, modulus: usize) -> Vec<u8> {
    (0..length).map(|index| (index % modulus) as u8).collect()
}

#[derive(Default)]
pub(super) struct RecordingSink {
    pub(super) events: Vec<String>,
    pub(super) bytes: Vec<Vec<u8>>,
}

impl BatchSink for RecordingSink {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        self.events.push(format!("begin {index} {size}"));
        if self.bytes.len() <= index {
            self.bytes.resize(index + 1, Vec::new());
        }
        Ok(())
    }

    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        self.bytes[index].extend_from_slice(bytes);
        Ok(())
    }

    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        self.events.push(format!("end {index} {}", result.is_ok()));
        Ok(())
    }

    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()> {
        self.events
            .push(format!("failed {index} {:?}", error.kind()));
        Ok(())
    }
}

pub(super) struct RefusingSink;

impl BatchSink for RefusingSink {
    fn begin(&mut self, _index: usize, _size: u64) -> io::Result<()> {
        Ok(())
    }

    fn data(&mut self, _index: usize, _bytes: &[u8]) -> io::Result<()> {
        Err(io::Error::other("Ziel ist voll"))
    }

    fn end(&mut self, _index: usize, _result: io::Result<()>) -> io::Result<()> {
        Ok(())
    }

    fn failed(&mut self, _index: usize, _error: io::Error) -> io::Result<()> {
        Ok(())
    }
}

/// Delivers `data` until `fail_at`, then fails like a vanished source file.
pub(super) struct FailingSource {
    pub(super) data: Vec<u8>,
    pub(super) position: usize,
    pub(super) fail_at: usize,
}

impl Read for FailingSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.fail_at {
            return Err(io::Error::other("Quelle bricht ab"));
        }
        let length = out.len().min(self.fail_at - self.position);
        out[..length].copy_from_slice(&self.data[self.position..self.position + length]);
        self.position += length;
        Ok(length)
    }
}
