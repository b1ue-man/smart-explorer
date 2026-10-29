//! The main SFTP session as the last place for a range: when the pool can
//! hold no channel at all (the server's session limit is used up, or no
//! channel could be set up) a range of a large file must still be moved, not
//! fail the transfer. Reading and writing go as one stream each over the
//! session every backend has; the ranges only lose their own SSH window.
use super::backend::SftpBackend;
use super::io_adapters::{seek_start, Commit, SftpReader, SftpWriter};
use super::io_err;
use super::pool_writer::SizedWriter;
use super::segmented_stage::HandleCheck;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::OpenFlags;
use std::io::{self, Read, Write};

/// `len` bytes of a reader positioned at the start of the range.
pub(super) struct MainRange {
    inner: SftpReader,
    remaining: u64,
}

impl Read for MainRange {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let wanted = (out.len() as u64).min(self.remaining) as usize;
        if wanted == 0 {
            return Ok(0);
        }
        let read = self.inner.read(&mut out[..wanted])?;
        self.remaining = self.remaining.saturating_sub(read as u64);
        Ok(read)
    }
}

impl SftpBackend {
    /// The main-session reader: used when no pool channel is available.
    pub(super) fn open_main_reader(&self, path: &str, start: u64) -> io::Result<SftpReader> {
        let (generation, (mut file, slot)) = self.safe_sftp_on(|generation| {
            self.rt.block_on(async {
                let slot = generation.handle_slot().await?;
                let file = generation.sftp().open(path.to_string()).await?;
                Ok::<_, SftpError>((file, slot))
            })
        })?;
        self.rt.block_on(seek_start(&mut file, start))?;
        Ok(SftpReader {
            rt: self.rt.clone(),
            connection: self.connection.clone(),
            generation,
            path: path.to_string(),
            file,
            start,
            delivered: 0,
            retried: false,
            slot: Some(slot),
        })
    }

    pub(super) fn open_main_writer(
        &self,
        path: &str,
        flags: OpenFlags,
        commit: Commit,
    ) -> io::Result<SftpWriter> {
        let generation = self.connection.current()?;
        // All slots held: congestion, the transfer backs off and comes again.
        let slot = generation.try_handle_slot().ok_or_else(|| {
            crate::vfs::congestion_error(
                "Die SFTP-Hauptsitzung hält keine weiteren Dateien offen",
                None,
            )
        })?;
        let (file, slot) = self
            .rt
            .block_on(async {
                let file = generation
                    .sftp()
                    .open_with_flags(path.to_string(), flags)
                    .await?;
                Ok::<_, SftpError>((file, slot))
            })
            .map_err(|error| {
                self.connection.note_sftp_error(&generation, &error);
                io_err(error)
            })?;
        Ok(SftpWriter {
            rt: self.rt.clone(),
            connection: self.connection.clone(),
            generation,
            file: Some(file),
            commit,
            slot: Some(slot),
        })
    }

    /// `[offset, offset + len)` of `path`: on a pool channel, else over the
    /// main session.
    pub(super) fn read_range(
        &self,
        path: &str,
        offset: u64,
        len: u64,
    ) -> io::Result<Box<dyn Read + Send>> {
        if let Some(reader) = self.open_pool_reader_range(path, offset, len)? {
            return Ok(Box::new(reader));
        }
        Ok(Box::new(MainRange {
            inner: self.open_main_reader(path, offset)?,
            remaining: len,
        }))
    }

    /// A writer for exactly `[offset, offset + len)` of the existing `stage`
    /// over the main session: the handle is looked at with `fstat` before
    /// the first byte, as on a pool channel (`check`), and `flush` fails
    /// unless `len` bytes came.
    pub(super) fn main_range_writer(
        &self,
        stage: &str,
        offset: u64,
        len: u64,
        check: HandleCheck,
    ) -> io::Result<Box<dyn Write + Send>> {
        let mut writer = self.open_main_writer(stage, OpenFlags::WRITE, Commit::Unsynced)?;
        if let Some(file) = writer.file.as_mut() {
            let attrs = self.rt.block_on(file.metadata()).map_err(io_err)?;
            check.verify(&attrs)?;
            self.rt.block_on(seek_start(file, offset))?;
        }
        Ok(Box::new(SizedWriter::new(writer, len)))
    }
}
