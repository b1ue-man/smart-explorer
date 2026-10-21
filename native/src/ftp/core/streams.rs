//! Data transfers on one FTP control connection: RETR (after REST to resume
//! mid-file) as a reader, and STOR of a known length as a streaming writer.
//! Each checks the control connection out for its whole transfer and hands
//! it back when the data stream is finalized, so the control channel always
//! reads the server's closing answer before the next command.
use super::data_finish::UploadData;
use super::io_adapters::FtpConnection;
use std::io::{self, Read, Write};
use std::sync::Arc;
use suppaftp::{FtpError, RustlsFtpStream, Status};

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
        self.open_reader_from(path, offset, true)
    }

    /// `open_reader_at` for one range: a start that fails is never repeated
    /// on a new login (ranges never log in; answer to the question S12).
    pub(super) fn open_reader_range(
        self: &Arc<Self>,
        path: &str,
        offset: u64,
    ) -> io::Result<Option<FtpReader>> {
        self.open_reader_from(path, offset, false)
    }

    fn open_reader_from(
        self: &Arc<Self>,
        path: &str,
        offset: u64,
        replay: bool,
    ) -> io::Result<Option<FtpReader>> {
        super::errors::command_path(path)?;
        let rest = match (offset, usize::try_from(offset)) {
            (0, _) => None,
            (_, Ok(rest)) => Some(rest),
            (_, Err(_)) => return Ok(None),
        };
        let (control, data) = self.checkout(replay, |stream| {
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
        // suppaftp sends STOR right after the answer to PASV/EPSV and only
        // then connects the data socket (`data_command`), so the command and
        // the TCP connect already overlap; every reply is still awaited
        // before a dependent command (plan U46-5, docs/GOTCHAS.md).
        let mut sent = false;
        let (control, data) = self
            .checkout(false, |stream| {
                sent = true;
                let data = stream.put_with_stream(path).map_err(super::errors::map)?;
                let retained = if matches!(&data, suppaftp::DataStream::Ssl(_)) {
                    data.get_ref().try_clone().map(Some)
                } else {
                    Ok(None)
                };
                Ok(UploadData::new(data, retained))
            })
            .map_err(|error| {
                if sent {
                    stor_unconfirmed(error, path)
                } else {
                    error
                }
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

/// Whether `error` is the server turning a transfer down with 421 (service
/// not available) or 530 (not logged in): the answer to a range that ends
/// ranges on this connection for a while.
pub(super) fn range_refused(error: &io::Error) -> bool {
    super::connection::refusal_code(error).is_some()
        || error
            .get_ref()
            .and_then(|source| source.downcast_ref::<FtpError>())
            .and_then(super::errors::reply_code)
            .is_some_and(|code| matches!(code, 421 | 530))
}

/// A STOR whose `150` never came because the data connection or the answer
/// failed: the server may have created the file already, and FTP cannot
/// prove the name is still this client's, so the error names the stage. A
/// reply of the server (a refusal) is left as it is.
fn stor_unconfirmed(error: io::Error, stage: &str) -> io::Error {
    let answered = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<FtpError>())
        .and_then(super::errors::reply_code)
        .is_some();
    if answered {
        return error;
    }
    io::Error::new(
        error.kind(),
        format!(
            "{error}; STOR {stage} blieb unbestätigt, die Stufe „{stage}“ ist möglicherweise angelegt"
        ),
    )
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
        let (result, in_step) = match self.data.take() {
            Some(data) if completed => {
                let result = control
                    .finalize_retr_stream(data)
                    .map_err(super::errors::map);
                let in_step = result.is_ok();
                (result, in_step)
            }
            Some(data) => abort(&mut control, data),
            None => (Err(io_err("FTP-Datenstrom fehlt")), false),
        };
        self.owner.return_stream(control, in_step);
        result
    }
}

/// ABOR of an unfinished RETR. suppaftp 10 reads one closing reply, and a
/// second one only after 426 (6.3 always read two). When the transfer had
/// already ended, the server's own answer to ABOR (225, or a second 226) can
/// still be queued and would pass for the reply to the next command. PWD
/// answers 257, which no ABOR reply is: only then is the connection handed
/// back in step; otherwise it is reconnected before its next use.
fn abort(control: &mut RustlsFtpStream, data: Box<dyn Read + Send>) -> (io::Result<()>, bool) {
    let result = control.abort(data).map_err(super::errors::map);
    let in_step = result.is_ok()
        && control
            .custom_command("PWD", &[Status::PathCreated])
            .is_ok();
    (result, in_step)
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

/// `len` bytes of a RETR that goes on beyond them: after the last byte the
/// transfer is cut with ABOR (the connection is handed back only when it is
/// in step again, `abort`). When the file ends exactly there, the transfer
/// is finished normally instead. The reader reports 0 after `len` bytes, or
/// earlier at the end of the file.
pub(super) struct BoundedFtpReader {
    inner: FtpReader,
    remaining: u64,
    ended: bool,
}

impl BoundedFtpReader {
    pub(super) fn new(inner: FtpReader, len: u64) -> Self {
        Self {
            inner,
            remaining: len,
            ended: false,
        }
    }

    /// The range is read: one more byte tells whether the file goes on.
    fn end_of_range(&mut self) -> io::Result<usize> {
        if self.ended {
            return Ok(0);
        }
        self.ended = true;
        let mut probe = [0u8; 1];
        match self.inner.read(&mut probe) {
            // The transfer ended with the range and was finished normally.
            Ok(0) => Ok(0),
            // More follows: cut the transfer here.
            Ok(_) => self.inner.close(false).map(|()| 0),
            Err(error) => Err(error),
        }
    }
}

impl Read for BoundedFtpReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return self.end_of_range();
        }
        let wanted =
            usize::try_from(self.remaining).map_or(buffer.len(), |left| left.min(buffer.len()));
        match self.inner.read(&mut buffer[..wanted])? {
            0 => {
                // The file ended inside the range.
                self.remaining = 0;
                self.ended = true;
                Ok(0)
            }
            read => {
                self.remaining -= read as u64;
                Ok(read)
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
    data: Option<UploadData>,
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
            Some(data) => data.finish(&mut control),
            None => Err(io_err("FTP-Datenstrom fehlt")),
        };
        self.owner.return_stream(control, result.is_ok());
        result
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.state = StoreState::Failed;
        if let Err(server) = self.finish() {
            if super::data_finish::terminal_refusal(&server) {
                return server;
            }
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
