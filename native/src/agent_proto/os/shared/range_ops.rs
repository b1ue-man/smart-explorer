//! Ranged uploads on the SSH agent (`range-v1`, E13-S2 and E13-S4): the three
//! requests `SegStageOpen`, `WriteAt` and `SegStageFinish`, served from the
//! stage table of the connection (`RangeStages`).
//!
//! `SegStageOpen` uses the guards of `WriteNew`: the destination's folder must
//! be a plain folder tree (`ensure_destination_parent_plain`), the stage is
//! created exclusively with a name only this host picks, and
//! `secure_staging_file` gives it its private mode. The handle of that
//! creation stays in the table; every `WriteAt` writes through it and nothing
//! ever reopens the stage by path.
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::local_platform;
use super::promotion::ensure_destination_parent_plain;
use super::range_journal::Journal;
use super::range_stage::{random_stage_name, RangeStages};
use super::session::{emit, Inbound, Sink};
use super::{busy_message, lz4_switched_off, unpack, BlockSender, Frame, CHUNK};

/// A stage that makes no progress for this long is removed. Ranges of one
/// file can wait for the sender's flow permits for a while between two
/// requests, so this is longer than the stall of a single chunk.
pub(crate) const STAGE_STALL: Duration = Duration::from_secs(300);
/// Names that are free are looked for this often (64 random bits each).
const NAME_ATTEMPTS: usize = 4;
const POLL: Duration = Duration::from_millis(100);

/// This process's list of unfinished stages, kept in a private folder of
/// the user's temp folder; none when that folder cannot be made (a
/// root-confined agent cannot write there).
pub(crate) fn agent_journal() -> Option<Arc<Journal>> {
    static JOURNAL: OnceLock<Option<Arc<Journal>>> = OnceLock::new();
    JOURNAL
        .get_or_init(|| {
            let name = format!("se-range-stages-{}", local_platform::user_tag());
            Journal::open(std::env::temp_dir().join(name))
        })
        .clone()
}

/// A full table is congestion (the client backs off); everything else is
/// reported as it is.
fn wire(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::ResourceBusy {
        return io::Error::other(busy_message(None, &error.to_string()));
    }
    error
}

/// Creates the stage next to `destination` and reports its name.
pub(crate) fn handle_seg_open(
    sink: &Sink,
    id: u64,
    stages: &Arc<RangeStages>,
    destination: &str,
    size: u64,
) -> io::Result<()> {
    let target = Path::new(destination);
    ensure_destination_parent_plain(target)?;
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent folder"))?;
    let pending = stages.begin(parent, size).map_err(wire)?;
    let (name, file) = create_stage(destination)?;
    let path = PathBuf::from(&name);
    let prepared = local_platform::secure_staging_file(&file).and_then(|()| file.set_len(size));
    if let Err(error) = prepared {
        abandon(&path, file);
        return Err(error);
    }
    pending.commit(name.clone(), path, file)?;
    emit(sink, id, &Frame::SegStageReady { stage: name })
}

/// Exclusive creation under a random name; a name that is taken is replaced
/// by another random one a few times.
fn create_stage(destination: &str) -> io::Result<(String, std::fs::File)> {
    for _ in 0..NAME_ATTEMPTS {
        let name = random_stage_name(destination)?;
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&name);
        match created {
            Ok(file) => return Ok((name, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free name for the upload stage",
    ))
}

/// Removes a stage this call created and could not finish setting up, but
/// only while the path still names that very file.
fn abandon(path: &Path, file: std::fs::File) {
    let identity = local_platform::file_identity(&file);
    drop(file);
    if let Ok(identity) = identity {
        if local_platform::path_matches_identity(path, identity).unwrap_or(false) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `WriteAt`: accepts the range, answers `Progress`, receives exactly `len`
/// bytes (`Data` / `DataLz4`) up to `End` and answers `Ok`.
pub(crate) fn handle_write_at(
    sink: &Sink,
    id: u64,
    stages: &Arc<RangeStages>,
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
            Ok(Frame::DataLz4(block)) if !lz4_switched_off() => claim.write(&unpack(&block)?)?,
            Ok(Frame::Data(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "upload data frame exceeds the protocol chunk limit",
                ))
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

/// `SegStageFinish`: every range is there; the stage stays for publication.
pub(crate) fn finish(stages: &RangeStages, stage: &str) -> io::Result<Frame> {
    stages.finish(stage)?;
    Ok(Frame::Ok)
}

/// `DiscardStage` for a stage this connection still holds: file and entry
/// go. False for any other path (the ordinary `discard_stage` handles it).
pub(crate) fn discard(stages: &RangeStages, stage: &str) -> bool {
    stages.discard(stage)
}

/// `ReadLz4`: like `Read`, but a block that shrinks goes out as `DataLz4`
/// (the policy decides per block; everything else stays a plain `Data`).
pub(crate) fn handle_read_lz4(
    sink: &Sink,
    id: u64,
    path: &str,
    range: (u64, u64),
    cancel: &AtomicBool,
) -> io::Result<()> {
    if lz4_switched_off() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "LZ4 content blocks are not served by this agent",
        ));
    }
    let (offset, len) = range;
    // Never waits on a FIFO, never reads a device (links are followed).
    let mut file = local_platform::open_regular_file(Path::new(path))?;
    if offset > 0 {
        file.seek(SeekFrom::Start(offset))?;
    }
    let mut remaining = if len == 0 { u64::MAX } else { len };
    let mut sender = BlockSender::new();
    sender.start_file();
    let mut sent = 0u64;
    let mut buffer = vec![0u8; CHUNK];
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let wanted = remaining.min(buffer.len() as u64) as usize;
        let read = file.read(&mut buffer[..wanted])?;
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
        remaining -= read as u64;
        sent += read as u64;
    }
    emit(sink, id, &Frame::End)
}

/// The frames of an upload with `DataLz4` unpacked to `Data`, so the
/// handlers of `Write` and `WriteNew` take both. A block that cannot be
/// unpacked is passed on unchanged and ends the upload there as an
/// unexpected frame.
pub(crate) struct Lz4Inbound<'a>(pub(crate) &'a dyn Inbound);

impl Inbound for Lz4Inbound<'_> {
    fn recv_timeout(&self, timeout: Duration) -> Result<Frame, RecvTimeoutError> {
        match self.0.recv_timeout(timeout)? {
            Frame::DataLz4(block) if !lz4_switched_off() => match unpack(&block) {
                Ok(raw) => Ok(Frame::Data(raw)),
                Err(_) => Ok(Frame::DataLz4(block)),
            },
            other => Ok(other),
        }
    }
}
