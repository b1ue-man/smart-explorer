use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use super::promotion::ensure_destination_parent_plain;
use super::session::{emit, Inbound, Sink};
use super::{Frame, CHUNK};

/// Receive a stream directly into one exclusively created path. The Ready
/// response is emitted only after this process owns that exact namespace entry,
/// which lets a layered `open_write_new` uphold its ownership contract.
/// With `mtime_ms` (`WriteNewTimed`) the file gets that time before it is
/// closed and the commit answers `StageWritten` with the size, time and file
/// identity read through the still open handle, so the client needs neither
/// a `FinishStage` nor a `Stat` and `FileIdentity` of its own.
pub(crate) fn handle_write_new(
    sink: &Sink,
    id: u64,
    path: &str,
    inbound: &dyn Inbound,
    cancel: &AtomicBool,
    mtime_ms: Option<i64>,
) -> io::Result<()> {
    ensure_destination_parent_plain(Path::new(path))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    if let Err(error) = super::local_platform::secure_staging_file(&file) {
        drop(file);
        // The path can be renamed and reused by another actor even while this
        // handle is open. Retain it: a check-then-unlink would still race.
        return Err(error);
    }
    if let Err(error) = emit(sink, id, &Frame::Progress { done: 0, total: 0 }) {
        drop(file);
        // Ready was not delivered, but the visible spelling may already have
        // been reused. There is no portable identity-bound unlink operation.
        return Err(error);
    }

    let transfer = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"));
        }
        match inbound.recv_timeout(Duration::from_millis(100)) {
            Ok(Frame::Data(data)) if data.len() <= CHUNK => {
                if let Err(error) = file.write_all(&data) {
                    break Err(error);
                }
            }
            Ok(Frame::Data(_)) => {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "upload data frame exceeds the protocol chunk limit",
                ));
            }
            Ok(Frame::End) if !cancel.load(Ordering::Relaxed) => {
                if let Some(ms) = mtime_ms {
                    // A filesystem that keeps no such time is no failure: the
                    // reported time shows it, and the client sets it again.
                    let _ = super::ext_ops::set_mtime(&file, ms);
                }
                break file.sync_all().and_then(|()| written(&file, mtime_ms));
            }
            Ok(Frame::End) => {
                break Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"));
            }
            Ok(_) => {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected frame in upload stream",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "upload aborted",
                ));
            }
        }
    };
    drop(file);
    let reply = match transfer {
        Ok(reply) => reply,
        // Keep the exclusively created entry. Path-based cleanup can race a
        // concurrent rename/replacement and remove content we never created.
        Err(error) => return Err(error),
    };
    // A lost final acknowledgement is ambiguous to the caller. Retain the
    // completed entry; the caller received Ready and therefore owns its name.
    emit(sink, id, &reply)
}

/// The commit reply: `Ok`, or for a timed stage what its open handle shows.
fn written(file: &std::fs::File, mtime_ms: Option<i64>) -> io::Result<Frame> {
    if mtime_ms.is_none() {
        return Ok(Frame::Ok);
    }
    let metadata = file.metadata()?;
    let object = match super::local_platform::stable_handle_identity(file) {
        Ok(identity) => Some(identity),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => None,
        Err(error) => return Err(error),
    };
    Ok(Frame::StageWritten {
        size: metadata.len(),
        mtime_ms: metadata
            .modified()
            .ok()
            .map(super::fs::systemtime_ms)
            .unwrap_or(0),
        object,
    })
}
