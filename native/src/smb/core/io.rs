//! `std::io::Write` over smb2's `FileWriter`, driven by the backend runtime
//! (never nested in another `block_on`: Backend calls run on app, scan and
//! task threads). The reader lives in reader.rs.
//!
//! The writer's `flush` is its commit boundary, like the SFTP and FTP
//! writers: pending bytes go out, then FLUSH and CLOSE (`FileWriter::finish`)
//! and only their success reports the file as written. An unsynced copy
//! stage skips the FLUSH: every WRITE answer is still awaited and must
//! confirm all accepted bytes before CLOSE (`FileWriter::abort`, which sends
//! no FLUSH). A writer dropped without a successful `flush` aborts and
//! removes the partial file, so an interrupted upload never leaves a
//! truncated file behind.
//! WRITEs are already pipelined by smb2 (`WriteBehind::Adaptive`: about
//! uplink rate × round trip in flight, within the credit window).
use super::errors;
use super::session::Generation;
use smb2::{FileWriter, Tree};
use std::io::{self, Write};
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Bytes collected before they are handed to the WRITE pipeline, so small
/// `write` calls (8 KiB from `io::copy`) do not become one WRITE each.
const WRITE_CHUNK: usize = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteState {
    Open,
    Committed,
    Failed,
}

/// What `flush` (the commit) does after the last WRITE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Commit {
    /// FLUSH, then CLOSE (`FileWriter::finish`).
    Durable,
    /// CLOSE without FLUSH: an engine copy stage whose source stays (plan W1
    /// `open_write_copy_stage_unsynced`); sync, mounts and replacements keep
    /// `Durable`.
    Unsynced,
}

/// An unsynced commit holds only when the server confirmed every accepted
/// byte: `FileWriter::abort` counts the WRITE answers that succeeded and
/// skips refused ones and those lost with the connection.
pub(super) fn unsynced_commit(accepted: u64, confirmed: u64) -> io::Result<()> {
    if confirmed == accepted {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "SMB-Server hat nur {confirmed} von {accepted} Bytes bestätigt"
    )))
}

pub(super) struct SmbWriter {
    writer: Option<FileWriter>,
    pending: Vec<u8>,
    state: WriteState,
    /// Exact length of a sized copy stage (plan W1); `flush` refuses others.
    expected: Option<u64>,
    accepted: u64,
    commit: Commit,
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
        expected: Option<u64>,
    ) -> Self {
        Self {
            writer: Some(writer),
            pending: Vec::new(),
            state: WriteState::Open,
            expected,
            accepted: 0,
            commit: Commit::Durable,
            rel: rel.to_string(),
            path: path.to_string(),
            tree,
            generation,
            rt,
        }
    }

    pub(super) fn with_commit(mut self, commit: Commit) -> Self {
        self.commit = commit;
        self
    }

    /// A sized stage whose source delivered another length: the partial
    /// file goes, as after a failed commit.
    fn wrong_length(&mut self, message: String) -> io::Error {
        self.state = WriteState::Failed;
        let writer = self.writer.take();
        self.remove_partial(writer);
        io::Error::new(io::ErrorKind::InvalidData, message)
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
        let accepted = self.accepted.saturating_add(data.len() as u64);
        if self.expected.is_some_and(|expected| accepted > expected) {
            return Err(
                self.wrong_length("Quelle ist während der Übertragung gewachsen".to_string())
            );
        }
        self.accepted = accepted;
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
        if let Some(expected) = self.expected.filter(|expected| *expected != self.accepted) {
            let message = format!(
                "Quelle hat sich während der Übertragung geändert: {} statt {expected} Bytes",
                self.accepted
            );
            return Err(self.wrong_length(message));
        }
        if !self.pending.is_empty() {
            self.send_pending()?;
        }
        let Some(writer) = self.writer.take() else {
            return Err(io::Error::other("SMB-Datei ist bereits geschlossen"));
        };
        let committed = match self.commit {
            Commit::Durable => self.rt.block_on(writer.finish()).map(|_| Ok(())),
            Commit::Unsynced => self
                .rt
                .block_on(writer.abort())
                .map(|confirmed| unsynced_commit(self.accepted, confirmed)),
        };
        match committed {
            Ok(Ok(())) => {
                self.state = WriteState::Committed;
                Ok(())
            }
            Ok(Err(error)) => {
                self.state = WriteState::Failed;
                self.remove_partial(None);
                Err(error)
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
