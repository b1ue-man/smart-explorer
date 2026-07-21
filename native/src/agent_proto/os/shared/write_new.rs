use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use super::promotion::ensure_destination_parent_plain;
use super::session::{emit, Sink};
use super::{Frame, CHUNK};

/// Receive a stream directly into one exclusively created path. The Ready
/// response is emitted only after this process owns that exact namespace entry,
/// which lets a layered `open_write_new` uphold its ownership contract.
pub(crate) fn handle_write_new(
    sink: &Sink,
    id: u64,
    path: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
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
            Ok(Frame::End) if !cancel.load(Ordering::Relaxed) => break file.sync_all(),
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
    if let Err(error) = transfer {
        // Keep the exclusively created entry. Path-based cleanup can race a
        // concurrent rename/replacement and remove content we never created.
        return Err(error);
    }
    // A lost final acknowledgement is ambiguous to the caller. Retain the
    // completed entry; the caller received Ready and therefore owns its name.
    emit(sink, id, &Frame::Ok)
}
