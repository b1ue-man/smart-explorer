//! Batch upload on the agent's local filesystem (`batch-v1`). Each entry is
//! streamed into its own exclusively created stage next to the destination
//! (named by the client nonce), written with exactly the declared length and
//! published without replacing; a taken name gets the next free numbered
//! name ("name (2).ext"). The client closes every entry with a trailer, so a
//! source that changed while it was read is discarded instead of published.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use super::batch_limits::{check_put_batch, numbered_name};
use super::promotion::{ensure_destination_parent_plain, promote_staged_no_replace};
use super::session::{emit, Inbound, Sink};
use super::{BatchEntry, Frame};

/// Numbered names tried, as `vfs::remote_util::REMOTE_UNIQUE_ATTEMPTS`.
const NUMBERED_ATTEMPTS: usize = 1000;
/// Stage names tried per entry; the nonce is random, so a second attempt
/// only happens when another actor took exactly that name.
const STAGE_ATTEMPTS: u32 = 16;
/// A waiting upload notices cancellation this fast, like the other upload
/// handlers of the protocol.
const CANCEL_POLL: Duration = Duration::from_millis(100);

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct FrameStream<'a> {
    inbound: &'a dyn Inbound,
    cancel: &'a AtomicBool,
}

impl FrameStream<'_> {
    fn next(&self) -> io::Result<Frame> {
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Paket-Upload abgebrochen",
                ));
            }
            match self.inbound.recv_timeout(CANCEL_POLL) {
                Ok(frame) => return Ok(frame),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Paket-Upload abgebrochen",
                    ))
                }
            }
        }
    }
}

/// A private stage; removed unless it was published.
struct EntryStage {
    path: PathBuf,
    file: Option<std::fs::File>,
    published: bool,
}

impl EntryStage {
    fn create(destination: &str, nonce: u64) -> io::Result<Self> {
        ensure_destination_parent_plain(Path::new(destination))?;
        for attempt in 0..STAGE_ATTEMPTS {
            let path = PathBuf::from(format!(
                "{destination}.se-agent-batch-{nonce:016x}-{attempt:x}.part"
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    let stage = Self {
                        path,
                        file: Some(file),
                        published: false,
                    };
                    if let Some(file) = &stage.file {
                        super::local_platform::secure_staging_file(file)?;
                    }
                    return Ok(stage);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "keine freie Paket-Stufe",
        ))
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("Paket-Stufe ist geschlossen"))?
            .write_all(data)
    }

    /// Publish without replacing (new file: no fsync, like new local copies).
    fn publish(mut self, destination: &str) -> io::Result<String> {
        drop(self.file.take());
        let path = Path::new(destination);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("Paket-Ziel hat keinen Dateinamen"))?;
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        for index in 1..=NUMBERED_ATTEMPTS {
            // The requested spelling stays verbatim; only numbered names are
            // rebuilt from its parent.
            let candidate = if index == 1 {
                PathBuf::from(destination)
            } else {
                parent.join(numbered_name(name, index))
            };
            match promote_staged_no_replace(&self.path, &candidate) {
                Ok(()) => {
                    self.published = true;
                    return candidate
                        .into_os_string()
                        .into_string()
                        .map_err(|_| invalid("Paket-Ziel ist kein UTF-8"));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Kein freier Name nach {NUMBERED_ATTEMPTS} Versuchen: {destination}"),
        ))
    }
}

impl Drop for EntryStage {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.published {
            // Exclusively created under a nonce name and never published.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Receive one entry whose first frame is `first`. The outer error ends the
/// batch (protocol, cancel, disconnect); the inner result is the outcome.
fn receive_entry(
    stream: &FrameStream<'_>,
    entry: &BatchEntry,
    index: u32,
    first: Frame,
) -> io::Result<io::Result<String>> {
    let mut stage = EntryStage::create(&entry.path, entry.nonce);
    let mut received = 0u64;
    let mut frame = first;
    loop {
        match frame {
            Frame::Data(data) => {
                received = received
                    .checked_add(data.len() as u64)
                    .filter(|received| *received <= entry.size)
                    .ok_or_else(|| invalid("Paket-Eintrag überschreitet seine Länge"))?;
                if let Ok(open) = &mut stage {
                    if let Err(error) = open.write(&data) {
                        stage = Err(error);
                    }
                }
            }
            Frame::ItemEnd {
                index: trailer,
                error,
            } if trailer == index => {
                return Ok(match error {
                    // The client's source changed or failed: discard.
                    Some(message) => Err(io::Error::new(io::ErrorKind::InvalidData, message)),
                    None if received != entry.size => {
                        return Err(invalid("Paket-Eintrag endete vor seiner Länge"));
                    }
                    None => stage.and_then(|stage| stage.publish(&entry.path)),
                });
            }
            _ => return Err(invalid("unerwarteter Frame in einem Paket-Eintrag")),
        }
        frame = stream.next()?;
    }
}

/// `BatchPut`: stream, publish and report each entry in order, then `Ok`.
/// A client that stops after a failed read ends the stream early with `End`;
/// the entries it did not send are simply not reported.
pub(crate) fn handle_put_batch(
    sink: &Sink,
    id: u64,
    entries: &[BatchEntry],
    inbound: &dyn Inbound,
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_put_batch(entries)?;
    let stream = FrameStream { inbound, cancel };
    for (position, entry) in entries.iter().enumerate() {
        let index = u32::try_from(position).map_err(|_| invalid("Paket-Index zu groß"))?;
        let first = stream.next()?;
        if matches!(first, Frame::End) {
            return emit(sink, id, &Frame::Ok);
        }
        let reply = match receive_entry(&stream, entry, index, first)? {
            Ok(path) => Frame::ItemPublished { index, path },
            Err(error) => Frame::ItemFailed {
                index,
                message: error.to_string(),
            },
        };
        emit(sink, id, &reply)?;
    }
    match stream.next()? {
        Frame::End => emit(sink, id, &Frame::Ok),
        _ => Err(invalid("unerwarteter Frame nach dem letzten Paket-Eintrag")),
    }
}
