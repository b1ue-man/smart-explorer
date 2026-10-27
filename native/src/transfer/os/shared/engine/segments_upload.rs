//! A large local file → remote target in parts: the target offers a stage
//! that several writers fill at once (`Backend::open_segmented_stage`), each
//! part is read from the source at its offset and written to the same range
//! of the stage. Everything after the parts is the ordinary end of an upload:
//! the source must be unchanged, the stage is closed (`finish` checks its
//! length) and published without replacing anything (`publish::publish`).
//! Nothing is ever written under the destination name before that.
//! `segments_copy.rs` uses the same stage and the same ending.
use super::super::segments::{splits, Ledger};
use super::ops::{At, Meter, OpError, Outcome};
use super::publish;
use super::queue::FileWork;
use super::segments::{self, SegmentError, SegmentIo, SegmentTiming, Shared};
use super::source::{LocalSource, RangeSource};
use super::Engine;
use crate::vfs::{Backend, SegmentedStage};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A stage name is tried this many times when the target says it is taken
/// (the same count as `publish::open_stage`).
const STAGE_ATTEMPTS: usize = 8;

/// The stage, shared between the owner and the ranges that write into it.
pub(super) type SharedStage = Arc<Box<dyn SegmentedStage>>;

/// What the ranges left behind: how they ended, and the stage handle once no
/// range holds it any more.
pub(super) struct Ranges {
    pub result: Result<(), SegmentError>,
    pub stage: Option<Box<dyn SegmentedStage>>,
    /// Bytes helper workers counted themselves.
    pub moved: u64,
}

/// One range of the local source into the same range of the stage.
struct UploadIo {
    source: RangeSource,
    stage: SharedStage,
    /// A range was opened: from then on a refusal is an error, not a reason
    /// to go on as one stream.
    opened: AtomicBool,
}

impl SegmentIo for UploadIo {
    fn run(
        &self,
        start: u64,
        len: u64,
        buffer: &mut [u8],
        meter: &Meter<'_>,
        stop: &AtomicBool,
    ) -> Result<SegmentTiming, SegmentError> {
        let began = Instant::now();
        let mut file = self.source.open_at(start).map_err(SegmentError::Failed)?;
        let mut writer = match self.stage.segment(start, len) {
            Ok(writer) => writer,
            Err(error)
                if error.kind() == io::ErrorKind::Unsupported
                    && !self.opened.load(Ordering::Acquire) =>
            {
                return Err(SegmentError::Fallback)
            }
            Err(error) => return Err(SegmentError::Failed(OpError::target(error))),
        };
        self.opened.store(true, Ordering::Release);
        let ready_ms = elapsed_ms(began);
        let mut done = 0u64;
        while done < len {
            if stop.load(Ordering::Acquire) {
                return Err(SegmentError::Failed(OpError::canceled()));
            }
            let wanted = (len - done).min(buffer.len() as u64) as usize;
            let read = match meter.timed_source(|| file.read(&mut buffer[..wanted])) {
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(SegmentError::Failed(OpError::source(error))),
            };
            if read == 0 {
                // The file is shorter than when it was opened.
                return Err(SegmentError::Failed(self.source.changed()));
            }
            meter
                .timed_target(|| writer.write_all(&buffer[..read]))
                .map_err(|error| SegmentError::Failed(OpError::target(error)))?;
            meter.add(read as u64);
            done += read as u64;
        }
        meter
            .timed_target(|| writer.flush())
            .map_err(|error| SegmentError::Failed(OpError::target(error)))?;
        Ok(SegmentTiming {
            first_byte_ms: ready_ms,
            total_ms: elapsed_ms(began),
        })
    }
}

pub(super) fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

/// Opens the stage for a file of `size` bytes that will be published as
/// `path`: a private name, a new one when the target says it is taken.
/// `Ok(None)`: this target cannot (the file is streamed).
pub(super) fn open_stage(
    target: &dyn Backend,
    path: &str,
    size: u64,
) -> Result<Option<Box<dyn SegmentedStage>>, OpError> {
    let mut last = None;
    for _ in 0..STAGE_ATTEMPTS {
        let name = publish::stage_name(path);
        match target.open_segmented_stage(&name, size) {
            Ok(stage) => return Ok(stage),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            // A refusal stays one: the file waits for the peer (K13).
            Err(error) => return Err(OpError::target(error)),
        }
    }
    Err(OpError::target(last.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::AlreadyExists, "Keine freie Upload-Stufe")
    })))
}

/// Runs the ranges of one file under the permits of its owner and takes the
/// stage back when no range holds it any more. `stage` is the owner's handle,
/// the one `io` was built with a clone of.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_ranges<'e>(
    engine: &Engine<'e>,
    file: &FileWork,
    size: u64,
    sizer_key: &str,
    stage: SharedStage,
    io: Box<dyn SegmentIo + 'e>,
    meter: &Meter<'_>,
    buffer: &mut [u8],
) -> Ranges {
    let sizer = segments::sizer_of(sizer_key);
    let shared = Shared::new(&file.rel, Ledger::new(size, 0), sizer, io);
    let registered = engine.segments.insert(shared.clone());
    let result = segments::drive(engine, &shared, buffer, meter, &mut || {});
    drop(registered);
    settle(&shared);
    let moved = shared.moved();
    // The ranges hold the stage through `io`, which lives as long as `shared`.
    drop(shared);
    Ranges {
        result,
        stage: Arc::try_unwrap(stage).ok(),
        moved,
    }
}

/// Waits until no helper holds the file any more (they let go within moments
/// of their last part).
fn settle(shared: &Arc<Shared<'_>>) {
    while Arc::strong_count(shared) > 1 {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The end of a file in ranges. `Ok(Some)`: published. `Ok(None)`: this way
/// cannot read or write ranges, nothing was transferred (the stage is gone),
/// the caller streams the file. `verify` proves that the source is unchanged
/// before anything is published.
pub(super) fn conclude(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    path: String,
    ranges: Ranges,
    verify: impl FnOnce() -> Result<(), OpError>,
    meter: &Meter<'_>,
) -> Result<Option<Outcome>, OpError> {
    let Ranges {
        result,
        stage,
        moved,
    } = ranges;
    // Helper bytes belong to this file: a failure at any later step (held
    // stage, source check, finish, publication) gives them back with the
    // owner's (`worker::run_file`); on success nothing reads them.
    meter.absorb(moved);
    let Some(stage) = stage else {
        return Err(OpError::at(
            At::Unknown,
            io::Error::other(format!(
                "Stufe zu „{path}“ wird noch von einem Bereich gehalten und nicht veröffentlicht"
            )),
        ));
    };
    match result {
        Ok(()) => {
            if let Err(error) = verify() {
                stage.abort();
                return Err(error);
            }
            if engine.stopped() {
                stage.abort();
                return Err(OpError::canceled());
            }
            let stage_path = stage.stage_path().to_string();
            if let Err(error) = stage.finish() {
                publish::discard(engine, target, &stage_path);
                return Err(OpError::target(error));
            }
            publish::publish(engine, target, file, &stage_path, path).map(Some)
        }
        Err(SegmentError::Fallback) => {
            stage.abort();
            Ok(None)
        }
        Err(SegmentError::Failed(error)) => {
            stage.abort();
            Err(error)
        }
    }
}

/// Uploads `file` in parts when the target offers a segmented stage and the
/// file is long enough for two parts. `Ok(Some)`: published. `Ok(None)`:
/// nothing was transferred, the caller streams the file. A retry after an
/// error starts over with a new stage (an upload is not resumed).
pub(super) fn try_split(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    source: &LocalSource,
    meter: &Meter<'_>,
    buffer: &mut [u8],
) -> Result<Option<Outcome>, OpError> {
    let size = source.length();
    if source.is_optical() || !splits(size) {
        return Ok(None);
    }
    let path = engine.folders.path_of(&file.rel);
    let Some(stage) = open_stage(target, &path, size)? else {
        return Ok(None);
    };
    let stage: SharedStage = Arc::new(stage);
    let io = UploadIo {
        source: source.range_source(),
        stage: stage.clone(),
        opened: AtomicBool::new(false),
    };
    let key = target.flow_identity(&path);
    let ranges = run_ranges(engine, file, size, &key, stage, Box::new(io), meter, buffer);
    conclude(
        engine,
        target,
        file,
        path,
        ranges,
        || source.complete(size),
        meter,
    )
}
