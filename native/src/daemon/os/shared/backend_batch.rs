//! Batches through the background service (`batch-v1`): the client's batch
//! frames go straight to `put_batch`/`get_batch` of the peer backend, split
//! only where the peer's limits are smaller. The service announces batches
//! only for a peer that has them, so nothing is emulated file by file.
use std::io::{self, Read};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use crate::agent_proto::{check_get_batch, check_put_batch, BatchEntry, BatchItem, Frame, Inbound};
use crate::agent_proto::{clip_text, BATCH_UNKNOWN_MARKER, CHUNK, ITEM_PATH_MAX};
use crate::vfs::{BackendHandle, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink};

use super::backend_server::{emit, error_text, Sink};
/// A waiting upload notices cancellation this fast, like the other upload
/// handlers of the protocol.
const CANCEL_POLL: Duration = Duration::from_millis(100);

fn parent_dir(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => "",
    }
}

fn peer_limits(backend: &BackendHandle, first: Option<&str>) -> io::Result<BatchLimits> {
    let dir = first.map_or("", parent_dir);
    backend.batch_limits(dir).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Gegenstelle überträgt keine Pakete",
        )
    })
}

/// Consecutive ranges within the peer's own batch limits.
fn peer_ranges(sizes: &[u64], limits: BatchLimits) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut bytes = 0u64;
    for (index, &size) in sizes.iter().enumerate() {
        let next = bytes.saturating_add(size);
        let full =
            index - start >= limits.max_files.max(1) || (index > start && next > limits.max_bytes);
        if full {
            ranges.push(start..index);
            start = index;
            bytes = size;
        } else {
            bytes = next;
        }
    }
    if start < sizes.len() {
        ranges.push(start..sizes.len());
    }
    ranges
}

fn protocol(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// Why the upload stream stopped delivering bytes.
enum Stop {
    /// The client abandoned the rest (a source read failed or it ended
    /// early); it reports the entries it never sent itself.
    Abandoned(io::ErrorKind, String),
    /// Cancel, disconnect or a protocol violation: the request fails.
    Failed(io::ErrorKind, String),
}

/// The client's entry bytes as one reader for the peer. The last chunk of an
/// entry is handed out only after its trailer confirmed it, so an entry whose
/// source changed never reaches the peer complete.
struct UploadStream<'a> {
    inbound: &'a dyn Inbound,
    cancel: &'a AtomicBool,
    sizes: Vec<u64>,
    entry: usize,
    received: u64,
    trailer_seen: bool,
    chunk: Vec<u8>,
    pos: usize,
    /// Entries at and after this index are not readable yet (range end).
    stop: usize,
    stopped: Option<Stop>,
    /// The client's final `End` was already consumed.
    ended: bool,
}

impl UploadStream<'_> {
    fn next_frame(&self) -> io::Result<Frame> {
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

    fn stop_with(&mut self, stop: Stop) {
        self.chunk.clear();
        self.pos = 0;
        self.stopped = Some(stop);
    }

    fn failed(&self) -> Option<io::Error> {
        match &self.stopped {
            Some(Stop::Failed(kind, message)) => Some(io::Error::new(*kind, message.clone())),
            _ => None,
        }
    }

    /// The trailer of the current entry, whose bytes all arrived.
    fn take_trailer(&mut self) -> io::Result<()> {
        match self.next_frame()? {
            Frame::ItemEnd { index, error } if index as usize == self.entry => {
                if let Some(message) = error {
                    self.stop_with(Stop::Abandoned(io::ErrorKind::InvalidData, message));
                }
                self.trailer_seen = true;
                Ok(())
            }
            _ => Err(protocol("Paket-Eintrag ohne passenden Abschluss")),
        }
    }

    /// Advance by one step; `Ok(false)` at the end of the readable range.
    fn fill(&mut self) -> io::Result<bool> {
        if self.entry >= self.stop.min(self.sizes.len()) {
            return Ok(false);
        }
        let size = self.sizes[self.entry];
        if self.received == size {
            if self.trailer_seen {
                self.entry += 1;
                self.received = 0;
                self.trailer_seen = false;
            } else {
                self.take_trailer()?;
            }
            return Ok(true);
        }
        match self.next_frame()? {
            Frame::Data(data) => {
                self.received = self
                    .received
                    .checked_add(data.len() as u64)
                    .filter(|received| *received <= size)
                    .ok_or_else(|| protocol("Paket-Eintrag überschreitet seine Länge"))?;
                self.chunk = data;
                self.pos = 0;
                if self.received == size {
                    // Withhold the final bytes until the trailer confirms them.
                    self.take_trailer()?;
                }
            }
            Frame::ItemEnd {
                index,
                error: Some(message),
            } if index as usize == self.entry => {
                self.stop_with(Stop::Abandoned(io::ErrorKind::InvalidData, message));
            }
            Frame::End => {
                self.ended = true;
                self.stop_with(Stop::Abandoned(
                    io::ErrorKind::UnexpectedEof,
                    "Paket endete vor allen Einträgen".into(),
                ));
            }
            _ => return Err(protocol("unerwarteter Frame in einem Paket-Eintrag")),
        }
        Ok(true)
    }

    /// Discard what the peer did not read up to entry `target`.
    fn skip_to(&mut self, target: usize) {
        self.stop = target;
        while self.entry < target && self.stopped.is_none() {
            self.pos = self.chunk.len();
            match self.fill() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    self.stop_with(Stop::Failed(error.kind(), error.to_string()));
                }
            }
        }
        self.chunk.clear();
        self.pos = 0;
    }

    /// Consume the client's final `End` (also after an abandoned entry).
    fn finish(&mut self) -> io::Result<()> {
        while !self.ended {
            match self.next_frame()? {
                Frame::End => self.ended = true,
                Frame::Data(_) | Frame::ItemEnd { .. } if self.stopped.is_some() => {}
                _ => {
                    return Err(protocol(
                        "unerwarteter Frame nach dem letzten Paket-Eintrag",
                    ))
                }
            }
        }
        Ok(())
    }
}

impl Read for UploadStream<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            match &self.stopped {
                Some(Stop::Abandoned(kind, message) | Stop::Failed(kind, message)) => {
                    return Err(io::Error::new(*kind, message.clone()));
                }
                None => {}
            }
            if self.pos < self.chunk.len() {
                let count = (self.chunk.len() - self.pos).min(out.len());
                out[..count].copy_from_slice(&self.chunk[self.pos..self.pos + count]);
                self.pos += count;
                return Ok(count);
            }
            match self.fill() {
                Ok(true) => {}
                Ok(false) => return Ok(0),
                Err(error) => {
                    self.stop_with(Stop::Failed(error.kind(), error.to_string()));
                    return Err(error);
                }
            }
        }
    }
}

pub(super) fn handle_put_batch_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    entries: &[BatchEntry],
    inbound: &dyn Inbound,
    cancel: &AtomicBool,
    credit: bool,
) -> io::Result<()> {
    check_put_batch(entries)?;
    let limits = peer_limits(backend, entries.first().map(|entry| entry.path.as_str()))?;
    let puts: Vec<BatchPut> = entries
        .iter()
        .map(|entry| BatchPut {
            path: entry.path.clone(),
            size: entry.size,
        })
        .collect();
    let sizes: Vec<u64> = entries.iter().map(|entry| entry.size).collect();
    let mut stream = UploadStream {
        inbound,
        cancel,
        sizes: sizes.clone(),
        entry: 0,
        received: 0,
        trailer_seen: false,
        chunk: Vec::new(),
        pos: 0,
        stop: 0,
        stopped: None,
        ended: false,
    };
    for range in peer_ranges(&sizes, limits) {
        // Realign after a peer that stopped reading early; nothing of the
        // skipped entries was complete, so none of them was published.
        stream.skip_to(range.start);
        if let Some(error) = stream.failed() {
            return Err(error);
        }
        if stream.stopped.is_some() {
            // The client stopped sending after a failed source read; it
            // reports the entries it never sent itself.
            break;
        }
        stream.stop = range.end;
        let outcomes = match backend.put_batch(&puts[range.clone()], &mut stream) {
            Ok(outcomes) if outcomes.len() == range.len() => outcomes,
            Ok(_) => {
                return emit(
                    sink,
                    id,
                    &Frame::Err(format!(
                        "{BATCH_UNKNOWN_MARKER} Gegenstelle meldete nicht jedes Paket-Ergebnis"
                    )),
                )
            }
            // The peer's outcome is unknown: say so, so the client neither
            // counts the files as failed nor repeats them blindly.
            Err(error) => {
                let text = error_text(&error, credit);
                return emit(
                    sink,
                    id,
                    &Frame::Err(format!("{BATCH_UNKNOWN_MARKER} {text}")),
                );
            }
        };
        for (offset, outcome) in outcomes.into_iter().enumerate() {
            let index =
                u32::try_from(range.start + offset).map_err(|_| protocol("Paket-Index zu groß"))?;
            let reply = match outcome {
                // A published path the protocol cannot carry leaves the
                // outcome unknown to the client; never report it as failed.
                BatchPutOutcome::Published(path) if path.len() > ITEM_PATH_MAX => {
                    return emit(
                        sink,
                        id,
                        &Frame::Err(format!(
                            "{BATCH_UNKNOWN_MARKER} Veröffentlichter Pfad ist zu lang"
                        )),
                    );
                }
                BatchPutOutcome::Published(path) => Frame::ItemPublished { index, path },
                BatchPutOutcome::Failed(error) => Frame::ItemFailed {
                    index,
                    message: clip_text(error_text(&error, credit)),
                },
            };
            emit(sink, id, &reply)?;
        }
        if let Some(error) = stream.failed() {
            return Err(error);
        }
    }
    stream.skip_to(sizes.len());
    if let Some(error) = stream.failed() {
        return Err(error);
    }
    stream.finish()?;
    emit(sink, id, &Frame::Ok)
}

/// Hands the peer's items to the client as batch frames.
struct ForwardSink<'a> {
    sink: &'a Sink,
    id: u64,
    offset: usize,
    cancel: &'a AtomicBool,
    credit: bool,
}

impl ForwardSink<'_> {
    fn index(&self, index: usize) -> io::Result<u32> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Paket-Download abgebrochen",
            ));
        }
        u32::try_from(self.offset + index).map_err(|_| protocol("Paket-Index zu groß"))
    }
}

impl BatchSink for ForwardSink<'_> {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        let index = self.index(index)?;
        emit(self.sink, self.id, &Frame::ItemBegin { index, size })
    }

    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        self.index(index)?;
        for chunk in bytes.chunks(CHUNK) {
            emit(self.sink, self.id, &Frame::Data(chunk.to_vec()))?;
        }
        Ok(())
    }

    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        let index = self.index(index)?;
        let error = result
            .err()
            .map(|error| clip_text(error_text(&error, self.credit)));
        emit(self.sink, self.id, &Frame::ItemEnd { index, error })
    }

    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()> {
        let index = self.index(index)?;
        let message = clip_text(error_text(&error, self.credit));
        emit(self.sink, self.id, &Frame::ItemFailed { index, message })
    }
}

pub(super) fn handle_get_batch_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    items: &[BatchItem],
    cancel: &AtomicBool,
    credit: bool,
) -> io::Result<()> {
    check_get_batch(items)?;
    let limits = peer_limits(backend, items.first().map(|item| item.path.as_str()))?;
    let gets: Vec<BatchGet> = items
        .iter()
        .map(|item| BatchGet {
            path: item.path.clone(),
            id: item.id.clone(),
            size: item.size,
        })
        .collect();
    let sizes: Vec<u64> = items.iter().map(|item| item.size).collect();
    for range in peer_ranges(&sizes, limits) {
        let mut forward = ForwardSink {
            sink,
            id,
            offset: range.start,
            cancel,
            credit,
        };
        backend.get_batch(&gets[range], &mut forward)?;
    }
    emit(sink, id, &Frame::End)
}

#[cfg(test)]
mod tests {
    use super::{parent_dir, peer_ranges};
    use crate::vfs::BatchLimits;

    #[test]
    fn transfer_engine_task_service_splits_batches_by_peer_limits() {
        let limits = BatchLimits {
            max_files: 2,
            max_bytes: 10,
        };
        assert_eq!(
            peer_ranges(&[1, 1, 1, 8, 8, 20, 0], limits),
            vec![0..2, 2..4, 4..5, 5..6, 6..7]
        );
        assert_eq!(parent_dir("/a/b.txt"), "/a");
        assert_eq!(parent_dir("/b.txt"), "/");
        assert_eq!(parent_dir("b.txt"), "");
    }
}
