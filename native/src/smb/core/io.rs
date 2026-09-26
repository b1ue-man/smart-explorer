//! `std::io::{Read, Write}` over smb2's `FileReader`/`FileWriter`, driven by
//! the backend runtime (never nested in another `block_on`: Backend calls
//! run on app, scan and task threads).
//!
//! The writer's `flush` is its commit boundary, like the SFTP and FTP
//! writers: pending bytes go out, then FLUSH and CLOSE (`FileWriter::finish`)
//! and only their success reports the file as written. A writer dropped
//! without a successful `flush` aborts (no FLUSH) and removes the partial
//! file, so an interrupted upload never leaves a truncated file behind.
use super::errors;
use super::session::Generation;
use smb2::{FileReader, FileWriter, Tree};
use std::io::{self, Read, Write};
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Bytes asked per positioned read (smb2 splits it at MaxReadSize).
const READ_CHUNK: u64 = 1 << 20;
/// Bytes collected before they are handed to the WRITE pipeline, so small
/// `write` calls (8 KiB from `io::copy`) do not become one WRITE each.
const WRITE_CHUNK: usize = 1 << 20;

pub(super) struct SmbReader {
    reader: Option<FileReader>,
    offset: u64,
    buffer: Vec<u8>,
    consumed: usize,
    path: String,
    generation: Arc<Generation>,
    rt: Arc<Runtime>,
}

impl SmbReader {
    pub(super) fn new(
        reader: FileReader,
        path: &str,
        generation: Arc<Generation>,
        rt: Arc<Runtime>,
    ) -> Self {
        Self {
            reader: Some(reader),
            offset: 0,
            buffer: Vec::new(),
            consumed: 0,
            path: path.to_string(),
            generation,
            rt,
        }
    }
}

impl Read for SmbReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.consumed >= self.buffer.len() {
            let Some(reader) = self.reader.as_ref() else {
                return Ok(0);
            };
            let chunk = match self.rt.block_on(reader.read_at(self.offset, READ_CHUNK)) {
                Ok(chunk) => chunk,
                Err(error) => {
                    self.generation.note(&error);
                    return Err(errors::map(error, "Lesen", &self.path));
                }
            };
            if chunk.is_empty() {
                return Ok(0);
            }
            self.offset += chunk.len() as u64;
            self.buffer = chunk;
            self.consumed = 0;
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
        // smb2 has no async Drop: close the handle here or it stays open
        // until the session ends. Best effort.
        if let Some(reader) = self.reader.take() {
            let _ = self.rt.block_on(reader.close());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteState {
    Open,
    Committed,
    Failed,
}

pub(super) struct SmbWriter {
    writer: Option<FileWriter>,
    pending: Vec<u8>,
    state: WriteState,
    /// Share-relative path, for removing a partial file.
    rel: String,
    path: String,
    tree: Arc<Tree>,
    generation: Arc<Generation>,
    rt: Arc<Runtime>,
}

impl SmbWriter {
    pub(super) fn new(
        writer: FileWriter,
        rel: &str,
        path: &str,
        tree: Arc<Tree>,
        generation: Arc<Generation>,
        rt: Arc<Runtime>,
    ) -> Self {
        Self {
            writer: Some(writer),
            pending: Vec::new(),
            state: WriteState::Open,
            rel: rel.to_string(),
            path: path.to_string(),
            tree,
            generation,
            rt,
        }
    }

    fn check_open(&self) -> io::Result<()> {
        match self.state {
            WriteState::Open => Ok(()),
            WriteState::Committed => Err(io::Error::other("SMB-Upload ist bereits abgeschlossen")),
            WriteState::Failed => Err(io::Error::other(
                "SMB-Upload ist fehlgeschlagen; weitere Daten werden nicht angenommen",
            )),
        }
    }

    fn failed(&mut self, error: smb2::Error) -> io::Error {
        self.state = WriteState::Failed;
        self.generation.note(&error);
        errors::map(error, "Schreiben", &self.path)
    }

    fn send_pending(&mut self) -> io::Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(io::Error::other("SMB-Datei ist bereits geschlossen"));
        };
        match self.rt.block_on(writer.write_chunk(&self.pending)) {
            Ok(()) => {
                self.pending.clear();
                Ok(())
            }
            Err(error) => Err(self.failed(error)),
        }
    }

    /// Removes the partial file this writer left (best effort).
    fn remove_partial(&self, writer: Option<FileWriter>) {
        let tree = self.tree.clone();
        let rel = self.rel.clone();
        let mut conn = self.generation.connection();
        self.rt.block_on(async move {
            if let Some(writer) = writer {
                let _ = writer.abort().await;
            }
            let _ = tree.delete_file(&mut conn, &rel).await;
        });
    }
}

impl Write for SmbWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.check_open()?;
        self.pending.extend_from_slice(data);
        if self.pending.len() >= WRITE_CHUNK {
            self.send_pending()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.state == WriteState::Committed {
            return Ok(());
        }
        self.check_open()?;
        if !self.pending.is_empty() {
            self.send_pending()?;
        }
        let Some(writer) = self.writer.take() else {
            return Err(io::Error::other("SMB-Datei ist bereits geschlossen"));
        };
        match self.rt.block_on(writer.finish()) {
            Ok(_) => {
                self.state = WriteState::Committed;
                Ok(())
            }
            Err(error) => {
                // The handle is gone with `finish`; the file content is not
                // confirmed, so it must not stay behind as if it were.
                let error = self.failed(error);
                self.remove_partial(None);
                Err(error)
            }
        }
    }
}

impl Drop for SmbWriter {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            self.remove_partial(Some(writer));
        }
    }
}
