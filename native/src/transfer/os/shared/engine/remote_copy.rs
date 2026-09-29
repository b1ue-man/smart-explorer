//! Remote → remote: inside one account the target server copies itself when
//! it can; between different connections the bytes stream directly (a reader
//! thread keeps a few chunks ahead of the writer); a connection that cannot
//! read and write at once (FTP) bridges through a local temporary file.
use super::ops::{At, Meter, OpError, OpResult, Outcome, COPY_BUFFER, STREAM_DEPTH};
use super::publish::{self, Destination};
use super::queue::FileWork;
use super::source::RemoteCheck;
use super::Engine;
use crate::vfs::Backend;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;

/// Source and target of one copy.
#[derive(Clone, Copy)]
pub(crate) struct Pair<'a> {
    pub source: &'a dyn Backend,
    pub target: &'a dyn Backend,
}

/// Server copies land in a private stage like any other upload; a few names
/// are tried when one is taken.
const SERVER_STAGE_ATTEMPTS: usize = 8;

pub(super) fn copy(
    engine: &Engine<'_>,
    pair: Pair<'_>,
    file: &FileWork,
    parent_created: bool,
    meter: &Meter<'_>,
) -> OpResult {
    if engine.resume {
        if let Some(outcome) = super::upload::resume_check(engine, pair.target, file, file.size)? {
            return Ok(outcome);
        }
    }
    if engine.view.source.same_namespace(engine.view.target) {
        if let Some(outcome) = server_copy(engine, pair, file, meter)? {
            return Ok(outcome);
        }
    }
    let expected = pair
        .source
        .read_size(&file.source, file.size)
        .map_err(OpError::source)?;
    let same_connection = std::ptr::addr_eq(pair.source, pair.target)
        || pair.source.flow_key(&file.source) == pair.target.flow_key(engine.view.target_dir);
    if same_connection && !pair.target.concurrent_read_write() {
        bridge(engine, pair, file, parent_created, meter, expected)
    } else {
        stream(engine, pair, file, parent_created, meter, expected)
    }
}

/// `Ok(None)` when the target cannot copy on the server (stream instead).
/// A server copy is only a shortcut: when it fails for any other reason than
/// a taken stage name or congestion, its stage is discarded and the file is
/// streamed, which attributes a real problem to the side that has it (a
/// locked source must not end the job as a refusing target).
fn server_copy(
    engine: &Engine<'_>,
    pair: Pair<'_>,
    file: &FileWork,
    meter: &Meter<'_>,
) -> Result<Option<Outcome>, OpError> {
    let path = engine.folders.path_of(&file.rel);
    for _ in 0..SERVER_STAGE_ATTEMPTS {
        let stage = publish::stage_name(&path);
        match pair
            .target
            .server_copy_to_stage(&file.source, &stage, file.size)
        {
            Ok(None) => return Ok(None),
            Ok(Some(copied)) => {
                if copied != file.size {
                    publish::discard(engine, pair.target, &stage);
                    return Err(OpError::source(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{}: Quelle wurde während der Übertragung geändert",
                            file.source
                        ),
                    )));
                }
                meter.add(copied);
                if engine.stopped() {
                    publish::discard(engine, pair.target, &stage);
                    return Err(OpError::canceled());
                }
                return publish::publish(engine, pair.target, file, &stage, path).map(Some);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) if crate::vfs::congestion_of(&error).is_some() => {
                publish::discard(engine, pair.target, &stage);
                return Err(OpError::target(error));
            }
            Err(_) => {
                publish::discard(engine, pair.target, &stage);
                if engine.stopped() {
                    return Err(OpError::canceled());
                }
                return Ok(None);
            }
        }
    }
    Err(OpError::target(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Keine freie Kopierstufe",
    )))
}

/// Direct stream between two connections: a reader thread fills chunks
/// (bounded, `STREAM_DEPTH`), this thread writes them.
fn stream(
    engine: &Engine<'_>,
    pair: Pair<'_>,
    file: &FileWork,
    parent_created: bool,
    meter: &Meter<'_>,
    expected: Option<u64>,
) -> OpResult {
    let reader = pair
        .source
        .open_read_id(&file.source, file.id.as_deref())
        .map_err(OpError::source)?;
    let mut destination = publish::open(engine, pair.target, file, expected, parent_created)?;
    let mut check = RemoteCheck::new(file, expected);
    let written = std::thread::scope(|scope| {
        let (sender, chunks) = crossbeam_channel::bounded::<io::Result<Vec<u8>>>(STREAM_DEPTH - 1);
        let stop = engine.stop_flag();
        let limit = expected;
        scope.spawn(move || read_ahead(reader, sender, stop, limit));
        let mut written = 0u64;
        for chunk in chunks {
            if engine.stopped() {
                return Err(OpError::canceled());
            }
            let bytes = chunk.map_err(OpError::source)?;
            check.update(file, written, &bytes)?;
            destination
                .writer()
                .write_all(&bytes)
                .map_err(OpError::target)?;
            written += bytes.len() as u64;
            meter.add(bytes.len() as u64);
        }
        Ok(written)
    });
    let written = match written {
        Ok(_) if engine.stopped() => {
            publish::abandon(engine, pair.target, destination);
            return Err(OpError::canceled());
        }
        Ok(written) => written,
        Err(error) => {
            publish::abandon(engine, pair.target, destination);
            return Err(error);
        }
    };
    if let Err(error) = check.finish(pair.source, file, written) {
        publish::abandon(engine, pair.target, destination);
        return Err(error);
    }
    publish::complete(engine, pair.target, file, destination, || Ok(()))
}

/// Reads ahead into chunks until the end, an error, the stop flag or a
/// writer that went away; at most one byte beyond `limit` is ever read.
fn read_ahead(
    mut reader: Box<dyn Read + Send>,
    sender: crossbeam_channel::Sender<io::Result<Vec<u8>>>,
    stop: &std::sync::atomic::AtomicBool,
    limit: Option<u64>,
) {
    let mut read_total = 0u64;
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let wanted = match limit {
            Some(limit) => limit
                .saturating_sub(read_total)
                .saturating_add(1)
                .min(COPY_BUFFER as u64) as usize,
            None => COPY_BUFFER,
        };
        let mut chunk = vec![0u8; wanted];
        let read = match reader.read(&mut chunk) {
            Ok(0) => return,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        };
        chunk.truncate(read);
        read_total += read as u64;
        if sender.send(Ok(chunk)).is_err() {
            return;
        }
    }
}

/// One connection that cannot read and write at once: the whole file goes
/// to a local temporary file first (checked before the writer opens), then
/// up to the target.
fn bridge(
    engine: &Engine<'_>,
    pair: Pair<'_>,
    file: &FileWork,
    parent_created: bool,
    meter: &Meter<'_>,
    expected: Option<u64>,
) -> OpResult {
    let mut spool =
        tempfile::tempfile_in(crate::support_dirs::temp_dir()).map_err(OpError::target)?;
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut check = RemoteCheck::new(file, expected);
    let mut reader = pair
        .source
        .open_read_id(&file.source, file.id.as_deref())
        .map_err(OpError::source)?;
    let mut spooled = 0u64;
    loop {
        if engine.stopped() {
            return Err(OpError::canceled());
        }
        let limit = check.limit(spooled, buffer.len());
        let read = match reader.read(&mut buffer[..limit]) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(OpError::source(error)),
        };
        if read == 0 {
            break;
        }
        check.update(file, spooled, &buffer[..read])?;
        spool.write_all(&buffer[..read]).map_err(OpError::target)?;
        spooled += read as u64;
        meter.add_hidden(read as u64);
    }
    drop(reader);
    check.finish(pair.source, file, spooled)?;
    spool.seek(SeekFrom::Start(0)).map_err(OpError::target)?;
    let mut destination = publish::open(engine, pair.target, file, Some(spooled), parent_created)?;
    let uploaded = upload_spool(engine, &mut spool, &mut destination, &mut buffer, meter);
    if let Err(error) = uploaded {
        publish::abandon(engine, pair.target, destination);
        return Err(error);
    }
    publish::complete(engine, pair.target, file, destination, || Ok(()))
}

fn upload_spool(
    engine: &Engine<'_>,
    spool: &mut std::fs::File,
    destination: &mut Destination,
    buffer: &mut [u8],
    meter: &Meter<'_>,
) -> Result<(), OpError> {
    loop {
        if engine.stopped() {
            return Err(OpError::canceled());
        }
        let read = match spool.read(buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(OpError::at(At::Target, error)),
        };
        if read == 0 {
            return Ok(());
        }
        destination
            .writer()
            .write_all(&buffer[..read])
            .map_err(OpError::target)?;
        meter.add(read as u64);
    }
}
