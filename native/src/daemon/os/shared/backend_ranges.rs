//! Ranged uploads and LZ4 reads of the background service's backend server
//! (`range-v1`, `lz4-v1`; E13-S2 to S4): the frames `SegStageOpen`,
//! `WriteAt`, `SegStageFinish` and `ReadLz4` go to `open_segmented_stage` and
//! `open_read_range` of the backend the service stands in front of.
//!
//! The same conditions as at the agent server hold:
//! * the stage name is picked here (`random_stage_name`, system randomness),
//!   the backend creates it exclusively and the object it returns is the
//!   handle every range is written through;
//! * a range is entered as taken when it is accepted and refused when it
//!   touches a taken or finished range or lies outside the stage;
//! * the stage belongs to the session that opened it, at most
//!   `CREDIT_REQUEST_LIMIT` are open, and an unfinished one is aborted when
//!   the session ends, when it makes no progress for `STAGE_STALL`, and when
//!   `DiscardStage` names it;
//! * the rights and the free space are the backend's (a Share host re-checks
//!   its write right at every range and adds up the space of its stages);
//!   this server never widens them.
//! A backend that cannot write ranges answers `SegStageOpen` with an empty
//! stage name: the client then sends the file as one stream.
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use crate::agent_proto::{
    lz4_switched_off, merge_in, overlaps, random_stage_name, BlockSender, Frame, Inbound, CHUNK,
    CREDIT_REQUEST_LIMIT,
};
use crate::vfs::{BackendHandle, SegmentedStage};

use super::{emit, Sink};

/// A stage that makes no progress for this long is aborted.
const STAGE_STALL: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_millis(100);

/// Whether this backend can be offered ranges (and with them LZ4 blocks):
/// it reads ranges. Only then does the service announce `range-v1` and
/// `lz4-v1`; the frames of both are refused otherwise.
pub(super) fn offers_ranges(backend: &BackendHandle) -> bool {
    backend.supports_range_reads(&backend.root_display())
}

/// The labels the service adds to its version text for `backend`. Never
/// `lz4-v1` (Z-P8): the service is the loopback hop between the engine and
/// the worker, where packing costs a core on each side and saves no wire
/// time; the worker's own peer streams decide LZ4 for the real line. The
/// `ReadLz4` and `DataLz4` frames stay understood for a client that sends
/// them anyway.
pub(super) fn labels(backend: &BackendHandle) -> String {
    let mut labels = String::new();
    if offers_ranges(backend) {
        labels.push('.');
        labels.push_str(crate::agent_proto::LABEL_RANGE);
    }
    labels
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

fn unknown_stage() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "Unbekannte Stufe (nicht von dieser Verbindung angelegt)",
    )
}

type Shared = Arc<RwLock<Option<Box<dyn SegmentedStage>>>>;

struct Entry {
    stage: Shared,
    size: u64,
    covered: Vec<(u64, u64)>,
    pending: Vec<(u64, u64)>,
    last_progress: Instant,
}

struct Inner {
    stages: HashMap<String, Entry>,
    opening: usize,
    closed: bool,
}

/// The stages one session of the service has open.
pub(super) struct BackendStages {
    inner: Mutex<Inner>,
}

/// Gives a stage up: the backend removes what it made.
fn abort(stage: Shared) {
    let taken = stage
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(stage) = taken {
        stage.abort();
    }
}

impl BackendStages {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                stages: HashMap::new(),
                opening: 0,
                closed: false,
            }),
        })
    }

    /// Aborts the stages without progress for `STAGE_STALL`.
    fn sweep(&self) {
        let expired: Vec<Shared> = {
            let mut inner = lock(&self.inner);
            let names: Vec<String> = inner
                .stages
                .iter()
                .filter(|(_, entry)| {
                    entry.pending.is_empty() && entry.last_progress.elapsed() >= STAGE_STALL
                })
                .map(|(name, _)| name.clone())
                .collect();
            names
                .iter()
                .filter_map(|name| inner.stages.remove(name))
                .map(|entry| entry.stage)
                .collect()
        };
        expired.into_iter().for_each(abort);
    }

    /// Opens the stage next to `destination`: the name is picked here, the
    /// backend creates it. `None`: the backend has no stage for ranges.
    pub(super) fn open(
        &self,
        backend: &BackendHandle,
        destination: &str,
        size: u64,
    ) -> io::Result<Option<String>> {
        self.sweep();
        {
            let mut inner = lock(&self.inner);
            if inner.closed {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Die Verbindung ist beendet",
                ));
            }
            if inner.stages.len() + inner.opening >= CREDIT_REQUEST_LIMIT {
                return Err(crate::vfs::congestion_error(
                    "Zu viele offene Stufen auf dieser Verbindung",
                    None,
                ));
            }
            inner.opening += 1;
        }
        let made = random_stage_name(destination)
            .and_then(|hint| backend.open_segmented_stage(&hint, size));
        let mut inner = lock(&self.inner);
        inner.opening = inner.opening.saturating_sub(1);
        let Some(stage) = made? else {
            return Ok(None);
        };
        let name = stage.stage_path().to_string();
        if name.is_empty() || inner.closed || inner.stages.contains_key(&name) {
            drop(inner);
            stage.abort();
            return Err(io::Error::other("Die Stufe konnte nicht angelegt werden"));
        }
        inner.stages.insert(
            name.clone(),
            Entry {
                stage: Arc::new(RwLock::new(Some(stage))),
                size,
                covered: Vec::new(),
                pending: Vec::new(),
                last_progress: Instant::now(),
            },
        );
        Ok(Some(name))
    }

    /// Accepts `[offset, offset + len)`: inside the stage, touching no taken
    /// or finished range; the backend's writer for it is made afterwards.
    fn claim(self: &Arc<Self>, name: &str, offset: u64, len: u64) -> io::Result<RangeClaim> {
        self.sweep();
        let stage = {
            let mut inner = lock(&self.inner);
            let entry = inner.stages.get_mut(name).ok_or_else(unknown_stage)?;
            let end = offset
                .checked_add(len)
                .filter(|end| len > 0 && *end <= entry.size)
                .ok_or_else(|| invalid("Bereich liegt außerhalb der Stufe"))?;
            if overlaps(&entry.covered, offset, end) || overlaps(&entry.pending, offset, end) {
                return Err(invalid(
                    "Bereich überlappt einen fertigen oder belegten Bereich",
                ));
            }
            entry.pending.push((offset, end));
            entry.last_progress = Instant::now();
            entry.stage.clone()
        };
        let mut claim = RangeClaim {
            table: self.clone(),
            name: name.to_string(),
            offset,
            len,
            written: 0,
            writer: None,
            settled: false,
        };
        let guard = stage
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(open) = guard.as_ref() else {
            return Err(unknown_stage());
        };
        // On error the claim is dropped and the range is free again.
        claim.writer = Some(open.segment(offset, len)?);
        drop(guard);
        Ok(claim)
    }

    /// Every range arrived and none runs: the backend checks the length and
    /// closes the stage, which stays for publication.
    pub(super) fn finish(&self, name: &str) -> io::Result<()> {
        let entry = {
            let mut inner = lock(&self.inner);
            let entry = inner.stages.get(name).ok_or_else(unknown_stage)?;
            if !entry.pending.is_empty() {
                return Err(invalid("Es laufen noch Bereiche"));
            }
            if entry.size != 0 && entry.covered != [(0, entry.size)] {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Nicht alle Bereiche der Stufe sind angekommen",
                ));
            }
            inner.stages.remove(name).ok_or_else(unknown_stage)?
        };
        let taken = entry
            .stage
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        taken.ok_or_else(unknown_stage)?.finish()
    }

    /// `DiscardStage` for a stage this session holds open. False: not ours.
    pub(super) fn discard(&self, name: &str) -> bool {
        let entry = lock(&self.inner).stages.remove(name);
        match entry {
            Some(entry) => {
                abort(entry.stage);
                true
            }
            None => false,
        }
    }

    /// The session ended: every unfinished stage is aborted.
    pub(super) fn close(&self) {
        let stages: Vec<Shared> = {
            let mut inner = lock(&self.inner);
            inner.closed = true;
            inner.stages.drain().map(|(_, entry)| entry.stage).collect()
        };
        stages.into_iter().for_each(abort);
    }

    fn settle(&self, name: &str, offset: u64, len: u64) -> io::Result<()> {
        let mut inner = lock(&self.inner);
        let entry = inner.stages.get_mut(name).ok_or_else(unknown_stage)?;
        let range = (offset, offset + len);
        let position = entry
            .pending
            .iter()
            .position(|pending| *pending == range)
            .ok_or_else(|| invalid("Bereich ist nicht angenommen"))?;
        entry.pending.swap_remove(position);
        merge_in(&mut entry.covered, range.0, range.1);
        entry.last_progress = Instant::now();
        Ok(())
    }

    fn unclaim(&self, name: &str, offset: u64, len: u64) {
        if let Some(entry) = lock(&self.inner).stages.get_mut(name) {
            let range = (offset, offset + len);
            if let Some(position) = entry.pending.iter().position(|pending| *pending == range) {
                entry.pending.swap_remove(position);
            }
        }
    }
}

impl Drop for BackendStages {
    fn drop(&mut self) {
        self.close();
    }
}

/// One accepted range with the backend's writer for it. Dropping it before
/// `complete` frees the range again.
struct RangeClaim {
    table: Arc<BackendStages>,
    name: String,
    offset: u64,
    len: u64,
    written: u64,
    writer: Option<Box<dyn Write + Send>>,
    settled: bool,
}

impl RangeClaim {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let end = self
            .written
            .checked_add(data.len() as u64)
            .filter(|end| *end <= self.len)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Mehr Bytes als der Bereich angekündigt hat",
                )
            })?;
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("Bereichsschreiber fehlt"))?;
        writer.write_all(data)?;
        self.written = end;
        Ok(())
    }

    /// Exactly `len` bytes came and the backend committed them (`flush`).
    fn complete(mut self) -> io::Result<()> {
        if self.written != self.len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Weniger Bytes als der Bereich angekündigt hat",
            ));
        }
        if let Some(writer) = self.writer.as_mut() {
            writer.flush()?;
        }
        self.table.settle(&self.name, self.offset, self.len)?;
        self.settled = true;
        Ok(())
    }
}

impl Drop for RangeClaim {
    fn drop(&mut self) {
        if !self.settled {
            self.writer = None;
            self.table.unclaim(&self.name, self.offset, self.len);
        }
    }
}

/// `WriteAt`: accepts the range, answers `Progress`, takes exactly `len`
/// bytes (`Data`, or `DataLz4` unpacked) up to `End` and answers `Ok`.
pub(super) fn write_at(
    sink: &Sink,
    id: u64,
    stages: &Arc<BackendStages>,
    range: (&str, u64, u64),
    inbound: &dyn Inbound,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (stage, offset, len) = range;
    let mut claim = stages.claim(stage, offset, len)?;
    emit(sink, id, &Frame::Progress { done: 0, total: 0 })?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"));
        }
        match inbound.recv_timeout(POLL) {
            Ok(Frame::Data(data)) if data.len() <= CHUNK => claim.write(&data)?,
            Ok(Frame::DataLz4(block)) if !lz4_switched_off() => {
                claim.write(&crate::agent_proto::unpack(&block)?)?
            }
            Ok(Frame::End) if !cancel.load(Ordering::Relaxed) => break,
            Ok(Frame::End) => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"))
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected frame in upload stream",
                ))
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "upload aborted",
                ))
            }
        }
    }
    claim.complete()?;
    emit(sink, id, &Frame::Ok)
}

/// The reader for `ReadLz4`: from `offset`, at most `len` bytes (0 = to the
/// end); a backend that cannot start mid-file is read and its prefix dropped.
fn open_reader(
    backend: &BackendHandle,
    path: &str,
    (offset, len): (u64, u64),
    cancel: &AtomicBool,
) -> io::Result<Option<Box<dyn Read + Send>>> {
    let reader: Box<dyn Read + Send> = if offset == 0 {
        backend.open_read(path)?
    } else if let Some(reader) = backend.open_read_at(path, None, offset)? {
        reader
    } else {
        let mut reader = backend.open_read(path)?;
        let mut skipped = vec![0u8; CHUNK];
        let mut remaining = offset;
        while remaining > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let want = remaining.min(skipped.len() as u64) as usize;
            let read = match reader.read(&mut skipped[..want]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if read == 0 {
                break;
            }
            remaining -= read as u64;
        }
        reader
    };
    Ok(Some(if len == 0 {
        reader
    } else {
        Box::new(reader.take(len))
    }))
}

/// `ReadLz4`: like `Read`; a block that shrinks goes out as `DataLz4` (the
/// policy decides per block), everything else as the plain `Data` it was.
pub(super) fn read_lz4(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    path: &str,
    range: (u64, u64),
    cancel: &AtomicBool,
) -> io::Result<()> {
    let Some(mut reader) = open_reader(backend, path, range, cancel)? else {
        return Ok(());
    };
    let mut sender = BlockSender::new();
    sender.start_file();
    let mut sent = 0u64;
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let read = match reader.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        let frame = match sender.try_pack(chunk, sent) {
            Some(block) => Frame::DataLz4(block),
            None => Frame::Data(chunk.to_vec()),
        };
        let started = Instant::now();
        emit(sink, id, &frame)?;
        sender.record_send(read, started.elapsed(), Instant::now());
        sent += read as u64;
    }
    emit(sink, id, &Frame::End)
}

#[cfg(test)]
#[path = "backend_ranges_tests.rs"]
mod tests;
