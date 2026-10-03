//! Data transfers on one FTP control connection: RETR (after REST to resume
//! mid-file) as a reader, and STOR of a known length as a streaming writer.
//! Each checks the control connection out for its whole transfer and hands
//! it back when the data stream is finalized, so the control channel always
//! reads the server's closing answer before the next command.
use super::io_adapters::FtpConnection;
use std::io::{self, Read, Write};
use std::sync::Arc;
use suppaftp::{FtpError, RustlsFtpStream};

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

/// REST answered with a 5xx: the server cannot start a download mid-file.
fn rest_unsupported(error: &FtpError) -> bool {
    matches!(
        error,
        FtpError::UnexpectedResponse(response) if (500..600).contains(&response.status.code())
    )
}

impl FtpConnection {
    pub(super) fn open_reader(self: &Arc<Self>, path: &str) -> io::Result<FtpReader> {
        self.open_reader_at(path, 0)?
            .ok_or_else(|| io_err("FTP-Download ließ sich nicht starten"))
    }

    /// RETR from byte `offset`; `None` when the server has no REST (or the
    /// offset exceeds what `REST` takes on this platform).
    pub(super) fn open_reader_at(
        self: &Arc<Self>,
        path: &str,
        offset: u64,
    ) -> io::Result<Option<FtpReader>> {
        super::errors::command_path(path)?;
        let rest = match (offset, usize::try_from(offset)) {
            (0, _) => None,
            (_, Ok(rest)) => Some(rest),
            (_, Err(_)) => return Ok(None),
        };
        let (control, data) = self.checkout(true, |stream| {
            if let Some(rest) = rest {
                match stream.resume_transfer(rest) {
                    Ok(()) => {}
                    Err(error) if rest_unsupported(&error) => return Ok(None),
                    Err(error) => return Err(super::errors::map(error)),
                }
            }
            stream
                .retr_as_stream(path)
                .map(|data| Some(Box::new(data) as Box<dyn Read + Send>))
                .map_err(super::errors::map)
        })?;
        let Some(data) = data else {
            // REST was refused with an answer: the connection is in step.
            self.return_stream(control, true);
            return Ok(None);
        };
        Ok(Some(FtpReader {
            owner: self.clone(),
            control: Some(control),
            data: Some(data),
        }))
    }

    /// STOR of exactly `size` bytes, sent as they are written. STOR creates
    /// or truncates, so it is never repeated.
    pub(super) fn open_store(
        self: &Arc<Self>,
        path: &str,
        size: u64,
    ) -> io::Result<FtpStoreWriter> {
        super::errors::command_path(path)?;
        let (control, data) = self.checkout(false, |stream| {
            stream
                .put_with_stream(path)
                .map(|data| Box::new(data) as Box<dyn Write + Send>)
                .map_err(super::errors::map)
        })?;
        Ok(FtpStoreWriter {
            owner: self.clone(),
            control: Some(control),
            data: Some(data),
            expected: size,
            written: 0,
            state: StoreState::Open,
        })
    }
}

pub(super) struct FtpReader {
    owner: Arc<FtpConnection>,
    control: Option<RustlsFtpStream>,
    data: Option<Box<dyn Read + Send>>,
}

impl FtpReader {
    fn close(&mut self, completed: bool) -> io::Result<()> {
        let Some(mut control) = self.control.take() else {
            return Ok(());
        };
        let result = match self.data.take() {
            Some(data) if completed => control.finalize_retr_stream(data).map_err(super::errors::map),
            Some(data) => control.abort(data).map_err(super::errors::map),
            None => Err(io_err("FTP-Datenstrom fehlt")),
        };
        self.owner.return_stream(control, result.is_ok());
        result
    }
}

impl Read for FtpReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let Some(data) = self.data.as_mut() else {
            return Ok(0);
        };
        match data.read(buffer) {
            Ok(0) => self.close(true).map(|()| 0),
            Ok(read) => Ok(read),
            Err(error) => {
                let _ = self.close(false);
                Err(error)
            }
        }
    }
}

impl Drop for FtpReader {
    fn drop(&mut self) {
        if self.data.is_some() {
            let _ = self.close(false);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StoreState {
    Open,
    Committed,
    Failed,
}

/// A streaming STOR of a copy stage whose length is known (plan W1): no
/// local spool, and `flush` fails unless exactly that many bytes came. A
/// writer dropped or failed before its commit leaves the partial stage on the
/// server; FTP cannot prove the name is still this client's, so it is not
/// removed (`discard_copy_stage` stays unsupported).
pub(super) struct FtpStoreWriter {
    owner: Arc<FtpConnection>,
    control: Option<RustlsFtpStream>,
    data: Option<Box<dyn Write + Send>>,
    expected: u64,
    written: u64,
    state: StoreState,
}

impl FtpStoreWriter {
    /// Closes the data connection and reads the server's answer, so the
    /// control connection stays in step; `Ok` only for a confirmed upload.
    fn finish(&mut self) -> io::Result<()> {
        let Some(mut control) = self.control.take() else {
            return Err(io_err("FTP-Upload ist bereits beendet"));
        };
        let result = match self.data.take() {
            Some(data) => control.finalize_put_stream(data).map_err(super::errors::map),
            None => Err(io_err("FTP-Datenstrom fehlt")),
        };
        self.owner.return_stream(control, result.is_ok());
        result
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.state = StoreState::Failed;
        if let Err(server) = self.finish() {
            if crate::vfs::is_target_refusal(&server) { return server }
        }
        error
    }
}

impl Write for FtpStoreWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self.state {
            StoreState::Open => {}
            StoreState::Committed => return Err(io_err("Upload bereits abgeschlossen")),
            StoreState::Failed => {
                return Err(io_err(
                    "FTP-Upload ist fehlgeschlagen; weitere Daten werden nicht angenommen",
                ))
            }
        }
        if self.written.saturating_add(buffer.len() as u64) > self.expected {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                "Quelle ist während der Übertragung gewachsen",
            );
            return Err(self.fail(error));
        }
        let written = match self.data.as_mut() {
            Some(data) => data.write(buffer),
            None => Err(io_err("FTP-Datenstrom fehlt")),
        };
        match written {
            Ok(written) => {
                self.written += written as u64;
                Ok(written)
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.state {
            StoreState::Committed => return Ok(()),
            StoreState::Failed => {
                return Err(io_err(
                    "FTP-Upload ist fehlgeschlagen; der Upload wird nicht automatisch wiederholt",
                ))
            }
            StoreState::Open => {}
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
        match self.finish() {
            Ok(()) => {
                self.state = StoreState::Committed;
                Ok(())
            }
            Err(error) => {
                self.state = StoreState::Failed;
                Err(io::Error::new(
                    error.kind(),
                    format!(
                        "FTP-Uploadstatus ist nach dem fehlgeschlagenen STOR unklar; der Upload wird nicht automatisch wiederholt: {error}"
                    ),
                ))
            }
        }
    }
}

impl Drop for FtpStoreWriter {
    fn drop(&mut self) {
        if self.control.is_some() {
            let _ = self.finish();
        }
    }
}
