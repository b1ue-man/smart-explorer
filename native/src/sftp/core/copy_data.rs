//! Server-side copy inside one SFTP server: OpenSSH's `copy-data` extension
//! (PROTOCOL §4.10, docs/refs/quic-sftp-throughput.md B.10) copies between
//! two handles the server holds open, so no byte crosses the network. Both
//! handles and the request live on one pool channel, because handles belong
//! to one `sftp-server` process.
//!
//! The server copies a request synchronously and answers when it is done,
//! so the copy runs in ranges that each end well inside the channel's answer
//! deadline and keep a transfer sharing the channel waiting at most one
//! range. The last range asks for one byte beyond the expected size: a
//! source that grew shows in the stage's length, as the stream path reads at
//! most one byte more. The stage is created exclusively and never synced (an
//! engine copy keeps its source, plan W1 `open_write_copy_stage_unsynced`).
//!
//! Any status the server answers a range with, other than OK and EOF, ends
//! the attempt without a trace and the engine streams: copies inside one
//! server streamed before, and a server's copy must not turn them into
//! failures (the stream path reports a real file problem as the source's or
//! the target's). Errors for the engine stay: a request without an answer,
//! a stage that cannot be created (the stream path could not create one
//! either; a taken name is `AlreadyExists`, so the engine tries another) and
//! a failing step after the copy or in the cleanup.
use super::backend::SftpBackend;
use super::channel_pool::{note_failure, open_failed};
use super::io_err;
use crate::vfs::VfsResult;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::RawSftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags, Packet, StatusCode};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The extension's name in the VERSION reply (version "1") and the request.
pub(super) const COPY_DATA: &str = "copy-data";
/// The first range: 16 MiB take 4 s at 4 MB/s (a slow SD card copying onto
/// itself), far inside the channel's 60 s answer deadline.
pub(super) const FIRST_RANGE: u64 = 16 << 20;
/// Later ranges last about this long at the previous range's rate: a sixth
/// of the answer deadline, so the rate may drop sixfold before a range times
/// out, and a transfer sharing the channel waits at most this long.
const RANGE_TARGET: Duration = Duration::from_secs(10);
/// OpenSSH copies in 64 KiB steps (`u_char buf[64*1024]`); a shorter range
/// only adds round trips.
pub(super) const MIN_RANGE: u64 = 64 << 10;

/// How a server-side copy ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ServerCopy {
    /// The stage holds this many bytes (one more than expected when the
    /// source grew, fewer when it shrank).
    Copied(u64),
    /// The server refused the extension (its request policy, remembered
    /// for the connection); the stage was removed again.
    Refused,
    /// Nothing of the attempt exists (the source did not open, or the server
    /// declined this copy and the stage was removed): the stream path takes
    /// over.
    Stream,
}

/// The server's answer to one range.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RangeAnswer {
    Copied,
    /// A fixed length ran into the source's end (`SSH2_FX_EOF`).
    SourceEnded,
    /// `OP_UNSUPPORTED` (the answer to an unknown request) or
    /// `PERMISSION_DENIED`: both handles are open, so the file permissions
    /// were checked already and the refusal is the server's request policy
    /// (`sftp-server -P/-p`, read-only mode).
    Refused,
    /// Any other answer (FAILURE, BAD_MESSAGE, …): this copy did not happen.
    Declined,
}

/// The request body after the name (PROTOCOL §4.10): string
/// read-from-handle, uint64 read-from-offset, uint64 read-data-length,
/// string write-to-handle, uint64 write-to-offset. Big-endian; a string is
/// a uint32 length and its bytes. russh-sftp appends it unchanged.
pub(super) fn copy_data_payload(
    read: &str,
    read_offset: u64,
    length: u64,
    write: &str,
    write_offset: u64,
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(32 + read.len() + write.len());
    put_string(&mut payload, read.as_bytes());
    payload.extend_from_slice(&read_offset.to_be_bytes());
    payload.extend_from_slice(&length.to_be_bytes());
    put_string(&mut payload, write.as_bytes());
    payload.extend_from_slice(&write_offset.to_be_bytes());
    payload
}

fn put_string(payload: &mut Vec<u8>, bytes: &[u8]) {
    // Handles come from a server packet (at most 256 KiB), so they always
    // fit the uint32 length.
    let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(bytes);
}

/// The length of the range after one of `length` bytes that took `took`.
pub(super) fn next_range(length: u64, took: Duration) -> u64 {
    let millis = took.as_millis().max(1);
    let range = u128::from(length) * RANGE_TARGET.as_millis() / millis;
    u64::try_from(range).unwrap_or(u64::MAX).max(MIN_RANGE)
}

pub(super) fn range_answer(packet: &Packet) -> RangeAnswer {
    let Packet::Status(status) = packet else {
        return RangeAnswer::Declined;
    };
    match status.status_code {
        StatusCode::Ok => RangeAnswer::Copied,
        StatusCode::Eof => RangeAnswer::SourceEnded,
        StatusCode::OpUnsupported | StatusCode::PermissionDenied => RangeAnswer::Refused,
        _ => RangeAnswer::Declined,
    }
}

/// Reports a failed request to the pool (a stalled or dead channel retires).
pub(super) type Note<'a> = &'a dyn Fn(&SftpError);

/// A failed step. `answering` while the server still answers: its handles
/// are then closed before returning (a Windows server cannot delete an open
/// stage), otherwise their CLOSE is only sent.
struct Failed {
    error: io::Error,
    answering: bool,
}

fn answering(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(_) | SftpError::Limited(_))
}

fn failed(error: SftpError, note: Note<'_>) -> Failed {
    note(&error);
    Failed {
        answering: answering(&error),
        error: io_err(error),
    }
}

/// Copies `source` into the new stage `stage` on `session`'s server.
pub(super) async fn copy_to_stage(
    session: &Arc<RawSftpSession>,
    source: &str,
    stage: &str,
    size: u64,
    cancel: &AtomicBool,
    note: Note<'_>,
) -> io::Result<ServerCopy> {
    let read = match session
        .open(source, OpenFlags::READ, FileAttributes::empty())
        .await
    {
        Ok(handle) => handle.handle,
        Err(error) => {
            note(&error);
            return Ok(ServerCopy::Stream);
        }
    };
    let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE;
    let write = match session.open(stage, flags, FileAttributes::empty()).await {
        Ok(handle) => handle.handle,
        Err(error) => {
            note(&error);
            let answering = answering(&error);
            release(session, vec![read], answering).await;
            return Err(stage_refused(session, stage, error, answering).await);
        }
    };
    match copy_ranges(session, &read, &write, size, cancel, note).await {
        Ok(ServerCopy::Copied(copied)) => {
            release(session, vec![read], true).await;
            match session.close(write).await {
                Ok(_) => Ok(ServerCopy::Copied(copied)),
                Err(error) => Err(failed(error, note).error),
            }
        }
        Ok(answer) => {
            release(session, vec![read, write], true).await;
            match session.remove(stage).await {
                Ok(_) => Ok(answer),
                Err(error) => Err(io::Error::other(format!(
                    "Kopierstufe „{stage}“ nach abgelehnter serverseitiger Kopie nicht entfernt: {}",
                    failed(error, note).error
                ))),
            }
        }
        Err(Failed { error, answering }) => {
            release(session, vec![read, write], answering).await;
            Err(error)
        }
    }
}

/// Copies `[0, size + 1)` in ranges: `Copied` with the stage's length, or
/// how the server declined.
async fn copy_ranges(
    session: &RawSftpSession,
    read: &str,
    write: &str,
    size: u64,
    cancel: &AtomicBool,
    note: Note<'_>,
) -> Result<ServerCopy, Failed> {
    let end = size.saturating_add(1);
    let mut offset = 0u64;
    let mut range = FIRST_RANGE;
    while offset < end {
        // Each range takes about ten seconds at most, so a cancel ends the
        // copy that soon; the caller discards the stage.
        if cancel.load(Ordering::Acquire) {
            return Err(Failed {
                error: io::Error::new(io::ErrorKind::Interrupted, "Serverkopie abgebrochen"),
                answering: true,
            });
        }
        let length = range.min(end - offset);
        let payload = copy_data_payload(read, offset, length, write, offset);
        let sent = Instant::now();
        let packet = session
            .extended(COPY_DATA, payload)
            .await
            .map_err(|error| failed(error, note))?;
        match range_answer(&packet) {
            RangeAnswer::Copied => {}
            RangeAnswer::SourceEnded => break,
            RangeAnswer::Refused => return Ok(ServerCopy::Refused),
            RangeAnswer::Declined => return Ok(ServerCopy::Stream),
        }
        offset += length;
        range = next_range(length, sent.elapsed());
    }
    let attrs = session
        .fstat(write)
        .await
        .map_err(|error| failed(error, note))?;
    match attrs.attrs.size {
        Some(copied) => Ok(ServerCopy::Copied(copied)),
        None => Err(Failed {
            error: io::Error::new(
                io::ErrorKind::InvalidData,
                "Der SFTP-Server nennt die Größe der Kopierstufe nicht",
            ),
            answering: true,
        }),
    }
}

/// The exclusive OPEN of the stage failed: `AlreadyExists` when the name is
/// taken (SFTP v3 answers EEXIST with a plain FAILURE), so the engine tries
/// another name and never removes what is there.
async fn stage_refused(
    session: &RawSftpSession,
    stage: &str,
    error: SftpError,
    answering: bool,
) -> io::Error {
    if answering && session.lstat(stage).await.is_ok() {
        return io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Kopierstufe „{stage}“ existiert bereits"),
        );
    }
    open_failed(error)
}

/// Closes `handles`: awaited while the server answers, else only sent.
async fn release(session: &Arc<RawSftpSession>, handles: Vec<String>, answering: bool) {
    for handle in handles {
        if answering {
            let _ = session.close(handle).await;
        } else {
            let session = session.clone();
            drop(tokio::spawn(async move {
                let _ = session.close(handle).await;
            }));
        }
    }
}

impl SftpBackend {
    /// `server_copy_to_stage`: `None` (stream) without a pool channel, when
    /// the server does not offer `copy-data`, refused it before or declines
    /// this copy, and when the source does not open. Nothing of the attempt
    /// exists on the server then.
    pub(super) fn copy_on_server(
        &self,
        source: &str,
        stage: &str,
        size: u64,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<u64>> {
        if self.pool.copy_data_refused() {
            return Ok(None);
        }
        // A pool that cannot lease leaves the error to the stream path,
        // which reports it on the right side and has no stage to clean up.
        let Ok(Some(lease)) = self.pool.lease(self) else {
            return Ok(None);
        };
        let channel = lease.channel().clone();
        if !channel.copy_data {
            return Ok(None);
        }
        let note = |error: &SftpError| {
            note_failure(&self.connection, &channel, error);
        };
        let copied = self.rt.block_on(copy_to_stage(
            &channel.session,
            source,
            stage,
            size,
            cancel,
            &note,
        ));
        drop(lease);
        match copied? {
            ServerCopy::Copied(copied) => Ok(Some(copied)),
            ServerCopy::Refused => {
                self.pool.refuse_copy_data();
                Ok(None)
            }
            ServerCopy::Stream => Ok(None),
        }
    }
}
