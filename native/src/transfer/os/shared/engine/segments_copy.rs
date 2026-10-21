//! A large remote file → another remote in parts: each part is read from the
//! source (`Backend::open_read_range`) and written to the same range of a
//! segmented stage of the target, so the file moves over several streams on
//! both sides. A part takes two permits (one per side, `acquire_pair`) and
//! the memory of a remote-to-remote operation. The ending is the same as for
//! an upload (`segments_upload::conclude`), plus the check that the source is
//! still the file the listing showed. A file whose listing carries an MD5
//! (Drive, Nextcloud) stays one stream: its hash is checked on the way, which
//! parts that arrive in any order cannot do.
use super::super::segments::splits;
use super::ops::{Meter, OpError, Outcome, COPY_BUFFER};
use super::queue::FileWork;
use super::remote_copy::Pair;
use super::segments::{self, SegmentError, SegmentIo, SegmentTiming};
use super::segments_upload::{conclude, elapsed_ms, open_stage, run_ranges, SharedStage};
use super::source::unchanged_since_listing;
use super::Engine;
use crate::vfs::Backend;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// One range of the source into the same range of the target's stage.
struct CopyIo<'e> {
    source: &'e dyn Backend,
    path: String,
    id: Option<String>,
    stage: SharedStage,
    /// A range was opened: from then on a refusal is an error, not a reason
    /// to go on as one stream.
    opened: AtomicBool,
}

impl CopyIo<'_> {
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

impl SegmentIo for CopyIo<'_> {
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
        let mut done = 0u64;
        let mut first_byte_ms = None;
        while done < len {
            if stop.load(Ordering::Acquire) {
                return Err(SegmentError::Failed(OpError::canceled()));
            }
            let wanted = (len - done).min(buffer.len() as u64) as usize;
            let read = match reader.read(&mut buffer[..wanted]) {
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(SegmentError::Failed(OpError::source(error))),
            };
            if read == 0 {
                return Err(SegmentError::Failed(self.changed()));
            }
            first_byte_ms.get_or_insert_with(|| elapsed_ms(began));
            writer
                .write_all(&buffer[..read])
                .map_err(|error| SegmentError::Failed(OpError::target(error)))?;
            meter.add(read as u64);
            done += read as u64;
        }
        writer
            .flush()
            .map_err(|error| SegmentError::Failed(OpError::target(error)))?;
        Ok(SegmentTiming {
            first_byte_ms: first_byte_ms.unwrap_or(0),
            total_ms: elapsed_ms(began),
        })
    }
}

/// Copies `file` in parts when the source reads ranges, the target offers a
/// segmented stage and the file is long enough for two parts. `Ok(Some)`:
/// published. `Ok(None)`: nothing was transferred, the caller streams the
/// file. `expected` is the source's read length (`None` for exports).
pub(super) fn try_split<'e>(
    engine: &Engine<'e>,
    pair: Pair<'e>,
    file: &FileWork,
    meter: &Meter<'_>,
    expected: Option<u64>,
) -> Result<Option<Outcome>, OpError> {
    let Some(size) = expected.filter(|size| splits(*size)) else {
        return Ok(None);
    };
    if file.md5.is_some()
        || !pair.source.supports_range_reads(&file.source)
        || segments::no_ranges(pair.source, &file.source)
    {
        return Ok(None);
    }
    let path = engine.folders.path_of(&file.rel);
    let Some(stage) = open_stage(pair.target, &path, size)? else {
        return Ok(None);
    };
    let stage: SharedStage = Arc::new(stage);
    let io = CopyIo {
        source: pair.source,
        path: file.source.clone(),
        id: file.id.clone(),
        stage: stage.clone(),
        opened: AtomicBool::new(false),
    };
    let key = pair.source.flow_identity(&file.source);
    // The worker's buffer belongs to the file operation; the copy of a part
    // needs one of the size every protocol moves per call.
    let mut buffer = vec![0u8; COPY_BUFFER];
    let ranges = run_ranges(
        engine,
        file,
        size,
        &key,
        stage,
        Box::new(io),
        meter,
        &mut buffer,
    );
    if matches!(ranges.result, Err(SegmentError::Fallback)) {
        segments::mark_no_ranges(pair.source, &file.source);
    }
    conclude(
        engine,
        pair.target,
        file,
        path,
        ranges,
        || unchanged_since_listing(pair.source, file),
        meter,
    )
}
