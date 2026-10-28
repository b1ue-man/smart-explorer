//! The crash log (v2) of one transfer job: one append-only private file in
//! the job's folder, opened once per run, so no write walks or re-checks a
//! path again. After an 8-byte magic every record is `length (u32 LE) |
//! CRC-32 of the payload (u32 LE) | payload`. A commit thread makes
//! everything written so far durable with one `sync_data` for all writers
//! waiting at that moment (group commit); a record nobody waits for becomes
//! durable with the next group commit or when the log closes.
//!
//! Reading a log stops at the first record that is incomplete, has an
//! impossible length or fails its CRC, and cuts it off: every acknowledged
//! sync covers all bytes written before it, so that record and everything
//! after it were never acknowledged as durable (a crash may keep later
//! pages and lose earlier ones only after the last completed sync).
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use super::super::lock;

/// Marks a file as a crash log of this format.
const MAGIC: [u8; 8] = *b"SEJRNL\x00\x02";
/// Length and CRC in front of every payload.
const HEAD: usize = 8;
/// Largest payload: far above a full packet of receipts, small enough that
/// a damaged length never allocates much.
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

fn failure(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                (value >> 1) ^ 0xEDB8_8320
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// CRC-32 (IEEE 802.3, the zlib/PNG checksum) of `bytes`.
pub(super) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc = CRC_TABLE[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn frame(payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return Err(failure("Dateijournaleintrag hat eine ungültige Größe"));
    }
    let mut frame = Vec::with_capacity(HEAD + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&crc32(payload).to_le_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

fn parse_head(head: &[u8; HEAD]) -> (usize, u32) {
    let length = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let crc = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    (length, crc)
}

fn write_at(file: &File, offset: u64, bytes: &[u8]) -> io::Result<()> {
    let mut handle = file;
    handle.seek(SeekFrom::Start(offset))?;
    handle.write_all(bytes)
}

/// A log written from scratch under a private stage name and promoted to
/// its final name only once complete and durable: a crash never leaves a
/// log without its first record.
pub(super) struct NewLog {
    file: Option<File>,
    stage: PathBuf,
    path: PathBuf,
}

impl NewLog {
    pub(super) fn start(path: &Path) -> io::Result<Self> {
        let stage = path.with_extension("log.new");
        match std::fs::remove_file(&stage) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let file = crate::support_dirs::create_private_file(&stage)?;
        let mut log = Self {
            file: Some(file),
            stage,
            path: path.to_path_buf(),
        };
        log.write(&MAGIC)?;
        Ok(log)
    }

    pub(super) fn record(&mut self, payload: &[u8]) -> io::Result<()> {
        let frame = frame(payload)?;
        self.write(&frame)
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| failure("Neues Dateijournal ist bereits geschlossen"))?
            .write_all(bytes)
    }

    /// Makes the log durable and gives it its final name.
    pub(super) fn finish(mut self) -> io::Result<()> {
        let file = self
            .file
            .take()
            .ok_or_else(|| failure("Neues Dateijournal ist bereits geschlossen"))?;
        file.sync_all()?;
        drop(file);
        crate::vfs::replace_local_file(&self.stage, &self.path)?;
        // Promoted: nothing is left to remove.
        self.stage = PathBuf::new();
        match self.path.parent() {
            Some(parent) => crate::support_dirs::sync_private_dir(parent),
            None => Ok(()),
        }
    }
}

impl Drop for NewLog {
    fn drop(&mut self) {
        self.file.take();
        if !self.stage.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.stage);
        }
    }
}

struct Writer {
    file: File,
    /// Where the next record starts.
    end: u64,
    /// Records written in this run; the last one's sequence number.
    seq: u64,
    /// A failed write left bytes behind that could not be cut off.
    broken: bool,
}

#[derive(Default)]
struct Durability {
    /// The highest sequence a writer waits for.
    requested: u64,
    /// Every record up to this sequence is on disk.
    durable: u64,
    /// The first failed sync: nothing written can be confirmed any more.
    failed: Option<(io::ErrorKind, String)>,
    closing: bool,
}

struct Shared {
    /// A second handle of the log, for syncs outside the writer's lock.
    sync: File,
    state: Mutex<Durability>,
    changed: Condvar,
}

fn settle(state: &mut Durability, target: u64, result: io::Result<()>) {
    match result {
        Ok(()) => state.durable = state.durable.max(target),
        Err(error) => {
            state
                .failed
                .get_or_insert((error.kind(), error.to_string()));
        }
    }
}

/// The commit thread: one sync for every writer that waits.
fn commit(shared: &Shared) {
    let mut state = lock(&shared.state);
    loop {
        if state.failed.is_some() {
            shared.changed.notify_all();
            return;
        }
        if state.requested > state.durable {
            let target = state.requested;
            drop(state);
            let result = shared.sync.sync_data();
            state = lock(&shared.state);
            settle(&mut state, target, result);
            shared.changed.notify_all();
            continue;
        }
        if state.closing {
            return;
        }
        state = shared
            .changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

/// Reads every complete record (with its offset) and cuts off a torn end;
/// returns where the next record goes.
fn replay(file: &File, visit: &mut dyn FnMut(u64, &[u8]) -> io::Result<()>) -> io::Result<u64> {
    let length = file.metadata()?.len();
    let mut handle = file;
    handle.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(handle);
    let mut magic = [0u8; MAGIC.len()];
    if length < MAGIC.len() as u64 || reader.read_exact(&mut magic).is_err() || magic != MAGIC {
        return Err(failure("Dateijournal hat ein unbekanntes Format"));
    }
    let mut offset = MAGIC.len() as u64;
    let mut torn = false;
    while offset < length {
        let remaining = length - offset;
        if remaining < HEAD as u64 {
            torn = true;
            break;
        }
        let mut head = [0u8; HEAD];
        reader.read_exact(&mut head)?;
        let (size, crc) = parse_head(&head);
        if size == 0 || size > MAX_PAYLOAD || size as u64 > remaining - HEAD as u64 {
            torn = true;
            break;
        }
        let mut payload = vec![0u8; size];
        reader.read_exact(&mut payload)?;
        if crc32(&payload) != crc {
            torn = true;
            break;
        }
        visit(offset, &payload)?;
        offset += (HEAD + size) as u64;
    }
    drop(reader);
    if torn {
        // Never acknowledged as durable: the next record follows the last
        // complete one.
        file.set_len(offset)?;
        file.sync_data()?;
    }
    Ok(offset)
}

/// An open crash log with its commit thread.
pub(super) struct Log {
    writer: Mutex<Writer>,
    shared: Arc<Shared>,
    committer: Option<JoinHandle<()>>,
}

impl Log {
    /// Opens the existing log at `path`, hands every complete record and its
    /// offset to `visit` and cuts off a torn end.
    pub(super) fn open(
        path: &Path,
        mut visit: impl FnMut(u64, &[u8]) -> io::Result<()>,
    ) -> io::Result<Self> {
        let file = crate::support_dirs::open_private_file_rw(path)?;
        let end = replay(&file, &mut visit)?;
        let shared = Arc::new(Shared {
            sync: file.try_clone()?,
            state: Mutex::new(Durability::default()),
            changed: Condvar::new(),
        });
        // Without a thread every waiting writer syncs for itself.
        let committer = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("transfer-journal".into())
                .spawn(move || commit(&shared))
                .ok()
        };
        Ok(Self {
            writer: Mutex::new(Writer {
                file,
                end,
                seq: 0,
                broken: false,
            }),
            shared,
            committer,
        })
    }

    /// Appends one record; `placed` learns its offset while no other record
    /// can follow it yet. Returns its sequence number for `wait`.
    pub(super) fn append(&self, payload: &[u8], placed: impl FnOnce(u64)) -> io::Result<u64> {
        let frame = frame(payload)?;
        let mut writer = lock(&self.writer);
        if writer.broken {
            return Err(failure(
                "Dateijournal ist nach einem Schreibfehler unbestätigt",
            ));
        }
        let offset = writer.end;
        if let Err(error) = write_at(&writer.file, offset, &frame) {
            // A partial record must not stay in front of the next one.
            if writer.file.set_len(offset).is_err() {
                writer.broken = true;
            }
            return Err(error);
        }
        writer.end = offset + frame.len() as u64;
        writer.seq += 1;
        placed(offset);
        Ok(writer.seq)
    }

    /// The payload of the record at `offset`, checked against its CRC.
    pub(super) fn read_at(&self, offset: u64) -> io::Result<Vec<u8>> {
        let writer = lock(&self.writer);
        if offset.saturating_add(HEAD as u64) > writer.end {
            return Err(failure("Dateijournaleintrag liegt hinter dem Journalende"));
        }
        let mut handle = &writer.file;
        handle.seek(SeekFrom::Start(offset))?;
        let mut head = [0u8; HEAD];
        handle.read_exact(&mut head)?;
        let (size, crc) = parse_head(&head);
        if size == 0 || size > MAX_PAYLOAD || offset + (HEAD + size) as u64 > writer.end {
            return Err(failure("Dateijournaleintrag hat eine ungültige Größe"));
        }
        let mut payload = vec![0u8; size];
        handle.read_exact(&mut payload)?;
        if crc32(&payload) != crc {
            return Err(failure("Dateijournaleintrag ist beschädigt"));
        }
        Ok(payload)
    }

    /// Returns once the record `seq` and everything before it are durable.
    pub(super) fn wait(&self, seq: u64) -> io::Result<()> {
        let shared = &*self.shared;
        let mut state = lock(&shared.state);
        loop {
            if state.durable >= seq {
                return Ok(());
            }
            if let Some((kind, message)) = &state.failed {
                return Err(io::Error::new(
                    *kind,
                    format!("Dateijournal dauerhaft speichern: {message}"),
                ));
            }
            if self.committer.is_none() {
                drop(state);
                let result = shared.sync.sync_data();
                state = lock(&shared.state);
                settle(&mut state, seq, result);
                continue;
            }
            if state.requested < seq {
                state.requested = seq;
                shared.changed.notify_all();
            }
            state = shared
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Makes everything written so far durable.
    pub(super) fn flush(&self) -> io::Result<()> {
        let seq = lock(&self.writer).seq;
        self.wait(seq)
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        // The records nobody waited for become durable with the job's end.
        let seq = lock(&self.writer).seq;
        {
            let mut state = lock(&self.shared.state);
            state.requested = state.requested.max(seq);
            state.closing = true;
        }
        self.shared.changed.notify_all();
        match self.committer.take() {
            Some(committer) => {
                let _ = committer.join();
            }
            None => {
                let _ = self.shared.sync.sync_data();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_log_crc_matches_the_reference_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn speed_task_log_cuts_a_torn_end_and_keeps_appending() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.log");
        let mut created = NewLog::start(&path).unwrap();
        created.record(b"first").unwrap();
        created.finish().unwrap();
        {
            let log = Log::open(&path, |_, _| Ok(())).unwrap();
            let second = log.append(b"second", |_| {}).unwrap();
            log.wait(second).unwrap();
            log.append(b"third", |_| {}).unwrap();
        }
        // Tear the last record: its tail never reached the disk.
        let length = std::fs::metadata(&path).unwrap().len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(length - 2)
            .unwrap();
        let mut seen = Vec::new();
        let log = Log::open(&path, |_, payload| {
            seen.push(payload.to_vec());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, [b"first".to_vec(), b"second".to_vec()]);
        let offset = std::cell::Cell::new(0);
        log.append(b"fourth", |at| offset.set(at)).unwrap();
        log.flush().unwrap();
        assert_eq!(log.read_at(offset.get()).unwrap(), b"fourth");
        drop(log);
        let mut seen = Vec::new();
        Log::open(&path, |_, payload| {
            seen.push(payload.to_vec());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            seen,
            [b"first".to_vec(), b"second".to_vec(), b"fourth".to_vec()]
        );
    }

    #[test]
    fn speed_task_log_damaged_record_ends_the_log() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.log");
        let mut created = NewLog::start(&path).unwrap();
        for payload in [&b"one"[..], &b"two"[..], &b"three"[..]] {
            created.record(payload).unwrap();
        }
        created.finish().unwrap();
        // Flip one byte of the second payload: it and everything after it
        // were never confirmed.
        let mut bytes = std::fs::read(&path).unwrap();
        let second = MAGIC.len() + HEAD + 3 + HEAD;
        bytes[second] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        let mut seen = Vec::new();
        drop(
            Log::open(&path, |_, payload| {
                seen.push(payload.to_vec());
                Ok(())
            })
            .unwrap(),
        );
        assert_eq!(seen, [b"one".to_vec()]);
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            (MAGIC.len() + HEAD + 3) as u64
        );
    }
}
