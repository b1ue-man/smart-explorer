//! The receipts of one job in its crash log (`crash_log`). An intent goes in
//! before every publication and is durable before the caller may publish
//! (one record and one sync for a whole packet); a publication's
//! acknowledgement goes in without its own sync and becomes durable with the
//! next group commit or the job's end. A crash that loses an acknowledgement
//! leaves the file in doubt, which the resumed job decides by metadata or
//! content digest (`state`, `missing_retry`).
//!
//! The newest receipt of each key is found through an in-memory index of
//! log offsets keyed by the key's SHA-256, so a live run never reads a
//! receipt back from disk; an intent waiting for its acknowledgement stays
//! in memory. A vault job seals every record with its transfer-blob context:
//! names are never stored in plaintext.
use std::collections::HashMap;
use std::io;
use std::sync::Mutex;

use ring::digest::SHA256;
use serde::{Deserialize, Serialize};

use super::super::lock;
use super::crash_log::{Log, NewLog};
use super::receipts::{directory, validate, Receipt};
use super::{failure, legacy, JobRecord, Path, RecordContext, RecordKind, StoredEndpoint};
use crate::transfer::engine::record_io;

/// The log's name in the job's private folder.
const LOG: &str = "journal.log";
/// Receipts per record when an older journal is converted.
const MIGRATION_CHUNK: usize = 256;

/// The job a log belongs to, its first record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Header {
    version: u32,
    job: String,
    source: StoredEndpoint,
    target: StoredEndpoint,
    root: String,
}

impl Header {
    fn of(record: &JobRecord) -> Self {
        Self {
            version: 2,
            job: record.id.clone(),
            source: record.source.clone(),
            target: record.target.clone(),
            root: record.target_dir.clone(),
        }
    }
}

#[derive(Deserialize)]
enum Entry {
    Header(Header),
    Receipts(Vec<Receipt>),
}

/// `Entry` as it is written, without copying the receipts.
#[derive(Serialize)]
enum EntryRef<'a> {
    Header(&'a Header),
    Receipts(&'a [Receipt]),
}

enum Slot {
    /// The newest receipt, kept until its acknowledgement replaces it.
    Open(Box<Receipt>),
    /// The newest receipt is member `member` of the record at `offset`.
    Logged { offset: u64, member: u32 },
}

fn digest(key: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(ring::digest::digest(&SHA256, key.as_bytes()).as_ref());
    out
}

fn encode(job: &str, context: Option<&RecordContext>, entry: &EntryRef<'_>) -> io::Result<Vec<u8>> {
    match context {
        None => serde_json::to_vec(entry).map_err(io::Error::other),
        Some(_) => {
            let plain =
                zeroize::Zeroizing::new(serde_json::to_vec(entry).map_err(io::Error::other)?);
            record_io::seal_payload(RecordKind::Job, job, context, &plain)
        }
    }
}

fn decode(job: &str, context: Option<&RecordContext>, payload: &[u8]) -> io::Result<Entry> {
    record_io::open_payload(RecordKind::Job, job, payload, context.is_some())
}

fn exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// The crash log of one job and its index.
pub(in crate::transfer::engine) struct Journal {
    log: Log,
    job: String,
    context: Option<RecordContext>,
    slots: Mutex<HashMap<[u8; 32], Slot>>,
}

impl Journal {
    /// Opens the job's log: an existing one is read (a torn end cut off),
    /// an older journal is converted first, and a missing one is created
    /// only while `create` (no mutation started yet); otherwise the job's
    /// receipts are lost and nothing may treat it as unstarted.
    pub(in crate::transfer::engine) fn open(
        job_path: &Path,
        record: &JobRecord,
        context: Option<RecordContext>,
        create: bool,
    ) -> io::Result<Self> {
        let folder = directory(job_path);
        crate::support_dirs::ensure_private_dir(&folder)?;
        let path = folder.join(LOG);
        let header = Header::of(record);
        if !exists(&path)? {
            if legacy::present(job_path)? {
                migrate(job_path, &path, record, context.as_ref(), &header)?;
            } else if create && !legacy::stray(job_path)? {
                let mut log = NewLog::start(&path)?;
                log.record(&encode(
                    &record.id,
                    context.as_ref(),
                    &EntryRef::Header(&header),
                )?)?;
                log.finish()?;
            } else {
                return Err(failure("Dateijournal ist unvollständig oder fehlt"));
            }
        }
        // Leftovers of a conversion a crash interrupted after the log was
        // promoted: the log holds them already.
        legacy::remove(job_path);
        let mut slots = HashMap::new();
        let mut headed = false;
        let log = Log::open(&path, |offset, payload| {
            match decode(&record.id, context.as_ref(), payload)? {
                Entry::Header(found) if !headed && found == header => headed = true,
                Entry::Header(_) => {
                    return Err(failure(
                        "Dateijournalkopf gehört zu einem abweichenden Auftrag",
                    ))
                }
                Entry::Receipts(_) if !headed => {
                    return Err(failure("Dateijournal beginnt ohne Auftragskopf"))
                }
                Entry::Receipts(receipts) => {
                    for (member, receipt) in receipts.iter().enumerate() {
                        validate(receipt, record)?;
                        let member = u32::try_from(member)
                            .map_err(|_| failure("Dateijournaleintrag ist zu groß"))?;
                        slots.insert(digest(&receipt.key), Slot::Logged { offset, member });
                    }
                }
            }
            Ok(())
        })?;
        if !headed {
            return Err(failure("Dateijournal ist unvollständig oder fehlt"));
        }
        Ok(Self {
            log,
            job: record.id.clone(),
            context,
            slots: Mutex::new(slots),
        })
    }

    /// Makes every receipt written so far durable.
    pub(in crate::transfer::engine) fn flush(&self) -> io::Result<()> {
        self.log.flush()
    }

    /// No receipt at all (a new job before its first intent).
    pub(super) fn is_empty(&self) -> bool {
        lock(&self.slots).is_empty()
    }

    pub(super) fn contains(&self, key: &str) -> bool {
        lock(&self.slots).contains_key(&digest(key))
    }

    /// The newest receipt of `key`.
    pub(super) fn get(&self, key: &str) -> io::Result<Option<Receipt>> {
        let (offset, member) = match lock(&self.slots).get(&digest(key)) {
            None => return Ok(None),
            Some(Slot::Open(receipt)) => return Ok(Some((**receipt).clone())),
            Some(Slot::Logged { offset, member }) => (*offset, *member),
        };
        let receipt = self.member(offset, member)?;
        if receipt.key != key {
            return Err(failure("Dateijournalschlüssel weicht ab"));
        }
        Ok(Some(receipt))
    }

    fn member(&self, offset: u64, member: u32) -> io::Result<Receipt> {
        let payload = self.log.read_at(offset)?;
        match decode(&self.job, self.context.as_ref(), &payload)? {
            Entry::Receipts(mut receipts) if (member as usize) < receipts.len() => {
                Ok(receipts.swap_remove(member as usize))
            }
            _ => Err(failure("Dateijournaleintrag fehlt an seiner Stelle")),
        }
    }

    /// Appends `receipts` as one record. Intents (`open`) stay in memory
    /// until their acknowledgement replaces them; `durable` returns only
    /// once the record and everything before it are on disk.
    pub(super) fn put(&self, receipts: &[Receipt], open: bool, durable: bool) -> io::Result<()> {
        if receipts.is_empty() {
            return Ok(());
        }
        let payload = encode(
            &self.job,
            self.context.as_ref(),
            &EntryRef::Receipts(receipts),
        )?;
        let keys: Vec<[u8; 32]> = receipts
            .iter()
            .map(|receipt| digest(&receipt.key))
            .collect();
        let sequence = self.log.append(&payload, |offset| {
            let mut slots = lock(&self.slots);
            for (member, (key, receipt)) in keys.into_iter().zip(receipts).enumerate() {
                let slot = if open {
                    Slot::Open(Box::new(receipt.clone()))
                } else {
                    Slot::Logged {
                        offset,
                        member: member as u32,
                    }
                };
                slots.insert(key, slot);
            }
        })?;
        if durable {
            self.log.wait(sequence)?;
        }
        Ok(())
    }

    /// Every newest receipt once; each record is decoded once.
    pub(super) fn visit(&self, mut visit: impl FnMut(Receipt) -> io::Result<()>) -> io::Result<()> {
        let mut open = Vec::new();
        let mut logged = Vec::new();
        for slot in lock(&self.slots).values() {
            match slot {
                Slot::Open(receipt) => open.push((**receipt).clone()),
                Slot::Logged { offset, member } => logged.push((*offset, *member)),
            }
        }
        logged.sort_unstable();
        let mut current: Option<(u64, Vec<Option<Receipt>>)> = None;
        for (offset, member) in logged {
            if current.as_ref().map(|(at, _)| *at) != Some(offset) {
                let payload = self.log.read_at(offset)?;
                let Entry::Receipts(receipts) = decode(&self.job, self.context.as_ref(), &payload)?
                else {
                    return Err(failure("Dateijournaleintrag fehlt an seiner Stelle"));
                };
                current = Some((offset, receipts.into_iter().map(Some).collect()));
            }
            let receipt = current
                .as_mut()
                .and_then(|(_, receipts)| receipts.get_mut(member as usize))
                .and_then(Option::take)
                .ok_or_else(|| failure("Dateijournaleintrag fehlt an seiner Stelle"))?;
            visit(receipt)?;
        }
        for receipt in open {
            visit(receipt)?;
        }
        Ok(())
    }
}

/// Converts the older journal into a promoted crash log; the older files
/// are removed only afterwards.
fn migrate(
    job_path: &Path,
    path: &Path,
    record: &JobRecord,
    context: Option<&RecordContext>,
    header: &Header,
) -> io::Result<()> {
    let mut log = NewLog::start(path)?;
    log.record(&encode(&record.id, context, &EntryRef::Header(header))?)?;
    let mut chunk = Vec::with_capacity(MIGRATION_CHUNK);
    legacy::visit(job_path, record, context, |receipt| {
        chunk.push(receipt);
        if chunk.len() == MIGRATION_CHUNK {
            log.record(&encode(&record.id, context, &EntryRef::Receipts(&chunk))?)?;
            chunk.clear();
        }
        Ok(())
    })?;
    if !chunk.is_empty() {
        log.record(&encode(&record.id, context, &EntryRef::Receipts(&chunk))?)?;
    }
    log.finish()
}

/// Where the crash log of the job stored at `job_path` lives (tests).
#[cfg(test)]
pub(in crate::transfer::engine) fn log_path(job_path: &Path) -> std::path::PathBuf {
    directory(job_path).join(LOG)
}
