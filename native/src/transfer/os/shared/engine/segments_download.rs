//! A large remote file → local folder in parts: the ranges are read from the
//! source (`Backend::open_read_range`) and written into the private `.part`
//! at their offsets. The MD5 the listing promised is computed by a prefix
//! hasher that follows the safe start. Everything after the parts (length and
//! hash check, publishing) stays in `download.rs`.
use super::super::segments::{splits, Ledger};
use super::download::Part;
use super::ops::{Meter, OpError};
use super::queue::FileWork;
use super::segments::{self, SegmentError, SegmentIo, SegmentTiming, Shared};
use super::source::RemoteCheck;
use super::Engine;
use crate::vfs::Backend;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The prefix hasher reads the part back in blocks of this size.
const HASH_BLOCK: usize = 64 * 1024;

/// One range of the source into its place in the part.
struct DownloadIo<'e> {
    source: &'e dyn Backend,
    path: String,
    id: Option<String>,
    file: Arc<File>,
    /// A range was opened: from then on a refusal is an error, not a reason
    /// to go on as one stream.
    opened: AtomicBool,
}

impl DownloadIo<'_> {
    fn changed(&self) -> OpError {
        OpError::source(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}: Quelle wurde während der Übertragung geändert",
                self.path
            ),
        ))
    }
}

impl SegmentIo for DownloadIo<'_> {
    fn run(
        &self,
        start: u64,
        len: u64,
        buffer: &mut [u8],
        meter: &Meter<'_>,
        stop: &AtomicBool,
    ) -> Result<SegmentTiming, SegmentError> {
        let began = Instant::now();
        let opened = self
            .source
            .open_read_range(&self.path, self.id.as_deref(), start, len)
            .map_err(|error| SegmentError::Failed(OpError::source(error)))?;
        let Some(mut reader) = opened else {
            return Err(if self.opened.load(Ordering::Acquire) {
                SegmentError::Failed(OpError::source(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("{}: Bereichslesen wird nicht mehr unterstützt", self.path),
                )))
            } else {
                SegmentError::Fallback
            });
        };
        self.opened.store(true, Ordering::Release);
        let mut done = 0u64;
        let mut first_byte_ms = None;
        while done < len {
            if stop.load(Ordering::Acquire) {
                return Err(SegmentError::Failed(OpError::canceled()));
            }
            let wanted = (len - done).min(buffer.len() as u64) as usize;
            let read = match meter.timed_source(|| reader.read(&mut buffer[..wanted])) {
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(SegmentError::Failed(OpError::source(error))),
            };
            if read == 0 {
                return Err(SegmentError::Failed(self.changed()));
            }
            first_byte_ms.get_or_insert_with(|| elapsed_ms(began));
            meter
                .timed_target(|| {
                    super::super::write_all_at(&self.file, &buffer[..read], start + done)
                })
                .map_err(|error| SegmentError::Failed(OpError::target(error)))?;
            meter.add(read as u64);
            done += read as u64;
        }
        Ok(SegmentTiming {
            first_byte_ms: first_byte_ms.unwrap_or(0),
            total_ms: elapsed_ms(began),
        })
    }
}

fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

/// Feeds the bytes before the safe start, read back from the part through a
/// second handle, to the change check; it runs behind the ranges.
struct PrefixHasher {
    reader: Option<File>,
    hashed: u64,
    block: Vec<u8>,
}

impl PrefixHasher {
    fn new() -> Self {
        Self {
            reader: None,
            hashed: 0,
            block: Vec::new(),
        }
    }

    fn advance(
        &mut self,
        part: &Part,
        upto: u64,
        check: &mut RemoteCheck,
        file: &FileWork,
    ) -> Result<(), OpError> {
        if self.hashed >= upto {
            return Ok(());
        }
        if self.reader.is_none() {
            self.reader = Some(File::open(part.path()).map_err(OpError::target)?);
            self.block = vec![0u8; HASH_BLOCK];
        }
        let Some(reader) = self.reader.as_mut() else {
            return Ok(());
        };
        reader
            .seek(SeekFrom::Start(self.hashed))
            .map_err(OpError::target)?;
        while self.hashed < upto {
            let wanted = (upto - self.hashed).min(HASH_BLOCK as u64) as usize;
            let read = reader
                .read(&mut self.block[..wanted])
                .map_err(OpError::target)?;
            if read == 0 {
                return Err(OpError::target(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Teildatei ist kürzer als geschrieben",
                )));
            }
            check.update(file, self.hashed, &self.block[..read])?;
            self.hashed += read as u64;
        }
        Ok(())
    }
}

/// Transfers `file` in parts when that is possible and pays: the length is
/// known and at least two shortest parts, the source reads ranges and has not
/// refused them lately. `Ok(true)`: every byte is in the part (and hashed);
/// the caller verifies and publishes. `Ok(false)`: nothing was transferred,
/// the caller streams the file (the part's pointer sits on `written`).
/// `Err`: the attempt failed; `part.written()` is the safe start the retry
/// continues from, and the caller keeps the part for it.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_split<'e>(
    engine: &Engine<'e>,
    source: &'e dyn Backend,
    file: &FileWork,
    part: &mut Part,
    check: &mut RemoteCheck,
    expected: Option<u64>,
    meter: &Meter<'_>,
    buffer: &mut [u8],
) -> Result<bool, OpError> {
    let Some(size) = expected.filter(|size| splits(*size)) else {
        return Ok(false);
    };
    if !source.supports_range_reads(&file.source) || segments::no_ranges(source, &file.source) {
        return Ok(false);
    }
    let Some(part_file) = part.shared_file() else {
        return Ok(false);
    };
    part.prepare_segments(size)?;
    let resume_at = part.written().min(size);
    let io = DownloadIo {
        source,
        path: file.source.clone(),
        id: file.id.clone(),
        file: part_file,
        opened: AtomicBool::new(false),
    };
    let sizer = segments::sizer_of(&source.flow_identity(&file.source));
    let shared = Shared::new(&file.rel, Ledger::new(size, resume_at), sizer, Box::new(io));
    let registered = engine.segments.insert(shared.clone());
    meter.credit(resume_at);
    let hashing = check.hashing();
    let mut hasher = PrefixHasher::new();
    let result = {
        let mut on_progress = || {
            // Before the first byte of a range is in, the resumed bytes are
            // not hashed: a fallback to one stream hashes them itself.
            let prefix = shared.prefix();
            if hashing && prefix > resume_at {
                if let Err(error) = meter.timed_cpu(|| hasher.advance(part, prefix, check, file)) {
                    shared.fail(error);
                }
            }
        };
        segments::drive(engine, &shared, buffer, meter, &mut on_progress)
    };
    drop(registered);
    settle(&shared);
    match result {
        Ok(()) => {
            part.set_written(size);
            if hashing {
                hasher.advance(part, size, check, file)?;
            }
            Ok(true)
        }
        Err(SegmentError::Fallback) => {
            // Nothing was read: the resumed bytes are credited and hashed
            // again by the single stream.
            segments::mark_no_ranges(source, &file.source);
            meter.uncredit(resume_at);
            *check = RemoteCheck::new(file, expected);
            part.seek_to_written()?;
            Ok(false)
        }
        Err(SegmentError::Failed(error)) => {
            part.set_written(shared.prefix());
            meter.absorb(shared.moved());
            part.seek_to_written()?;
            Err(error)
        }
    }
}

/// Waits until no helper holds the file any more (they let go within moments
/// of their last part): on Windows a part with an open handle cannot be
/// renamed or removed.
fn settle(shared: &Arc<Shared<'_>>) {
    while Arc::strong_count(shared) > 1 {
        std::thread::sleep(Duration::from_millis(1));
    }
}
