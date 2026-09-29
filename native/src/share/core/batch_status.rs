//! Outcomes of PutBatch commits by client nonce (K18e): a client whose reply
//! was lost after its commit asks once more and learns exactly which entries
//! were published under which name, so nothing is ever retried blindly.
//!
//! Bounded on every axis a client could inflate: records per principal and
//! in total (beyond them a new batch is answered `Busy`), and bytes, which
//! count every record's key and bookkeeping, not only its outcome text.
//! Records leave in expiry order from two ordered indexes, never through a
//! scan of the whole table under its lock.
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::share::session::PeerPrincipal;
use crate::share::wire::{FsBatchOutcome, FsBatchStatus};

use super::admission::{busy, HOST_TRANSFER_SLOTS, PRINCIPAL_TRANSFER_SLOTS};

/// A client asks right after reconnecting, within one operation deadline
/// (60 s); five times that covers a slow route change with margin.
const FINISHED_RETENTION: Duration = Duration::from_secs(5 * 60);

/// A batch still receiving or publishing is kept as long as it can run: the
/// per-chunk deadline (256 KiB within 60 s) lets a 16 MiB batch take 64
/// minutes at the slowest rate a transfer survives.
const PENDING_RETENTION: Duration = Duration::from_secs(70 * 60);

/// Records one principal may hold: its admitted batches in flight (60) plus
/// as many finished ones awaiting a status query after a lost connection.
const MAX_RECORDS_PER_PRINCIPAL: usize = 2 * PRINCIPAL_TRANSFER_SLOTS;

/// Records of all principals: every transfer the host admits at once (256)
/// plus as many finished ones awaiting their clients.
const MAX_RECORDS: usize = 2 * HOST_TRANSFER_SLOTS;

/// Bytes of all records: 64 of the largest batch headers (256 KiB), more
/// than the batches one connection can have in flight.
const MAX_STORED_BYTES: usize = 64 * 256 * 1024;

/// Bookkeeping of one record besides its strings: its map and index entries,
/// the record itself and its share of the per-principal count.
const RECORD_OVERHEAD: usize = 256;

/// Memory of one stored outcome besides its text: the string header (24
/// bytes) plus the variant and error kind.
const OUTCOME_OVERHEAD: usize = 32;

/// One batch of one authenticated principal.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct BatchKey {
    principal: PeerPrincipal,
    nonce: String,
}

impl BatchKey {
    pub(super) fn new(principal: PeerPrincipal, nonce: String) -> Self {
        Self { principal, nonce }
    }

    fn text_len(&self) -> usize {
        self.principal.text_len() + self.nonce.len()
    }
}

/// Position in an expiry index: when the record expires, and a sequence
/// number that keeps records of the same instant apart.
type Slot = (Instant, u64);

struct Record {
    status: FsBatchStatus,
    bytes: usize,
    slot: Slot,
}

#[derive(Default)]
struct Table {
    records: HashMap<BatchKey, Record>,
    /// Batches still receiving or publishing, by expiry.
    pending: BTreeMap<Slot, BatchKey>,
    /// Committed or aborted batches, by expiry; also the eviction order.
    finished: BTreeMap<Slot, BatchKey>,
    per_principal: HashMap<PeerPrincipal, usize>,
    bytes: usize,
    sequence: u64,
}

fn table() -> MutexGuard<'static, Table> {
    static TABLE: OnceLock<Mutex<Table>> = OnceLock::new();
    TABLE
        .get_or_init(|| Mutex::new(Table::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Registers a batch as it is accepted; a reused nonce is refused, a full
/// table answers `Busy`.
pub(super) fn begin(key: &BatchKey) -> io::Result<()> {
    table().begin(key, Instant::now())
}

/// Records how the batch ended (committed or aborted before its commit).
pub(super) fn finish(key: &BatchKey, status: FsBatchStatus) {
    table().finish(key, status, Instant::now());
}

/// The client confirmed it holds the outcomes; nothing needs to remain.
pub(super) fn delivered(key: &BatchKey) {
    table().delivered(key);
}

/// The state of `principal`'s batch `nonce`; `None` when unknown here.
pub(super) fn query(principal: &PeerPrincipal, nonce: &str) -> Option<FsBatchStatus> {
    let key = BatchKey::new(principal.clone(), nonce.to_string());
    table().query(&key, Instant::now())
}

impl Table {
    fn begin(&mut self, key: &BatchKey, now: Instant) -> io::Result<()> {
        self.expire(now);
        if self.records.contains_key(key) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Paket-Kennung wurde bereits verwendet",
            ));
        }
        let owned = self.per_principal.get(&key.principal).copied();
        if owned.is_some_and(|owned| owned >= MAX_RECORDS_PER_PRINCIPAL) {
            return Err(busy("zu viele offene Pakete dieses Geräts"));
        }
        let bytes = record_bytes(key, &FsBatchStatus::Pending);
        self.make_room(1, bytes);
        if self.records.len() >= MAX_RECORDS || self.bytes + bytes > MAX_STORED_BYTES {
            return Err(busy("zu viele offene Pakete"));
        }
        let slot = self.next_slot(now, PENDING_RETENTION);
        self.pending.insert(slot, key.clone());
        let record = Record {
            status: FsBatchStatus::Pending,
            bytes,
            slot,
        };
        self.records.insert(key.clone(), record);
        *self.per_principal.entry(key.principal.clone()).or_insert(0) += 1;
        self.bytes += bytes;
        Ok(())
    }

    fn finish(&mut self, key: &BatchKey, status: FsBatchStatus, now: Instant) {
        let slot = self.next_slot(now, FINISHED_RETENTION);
        let bytes = record_bytes(key, &status);
        let Some(record) = self.records.get_mut(key) else {
            return;
        };
        let index = if matches!(record.status, FsBatchStatus::Pending) {
            &mut self.pending
        } else {
            &mut self.finished
        };
        index.remove(&record.slot);
        self.bytes = self
            .bytes
            .saturating_sub(record.bytes)
            .saturating_add(bytes);
        record.status = status;
        record.bytes = bytes;
        record.slot = slot;
        self.finished.insert(slot, key.clone());
        self.expire(now);
        self.make_room(0, 0);
    }

    fn delivered(&mut self, key: &BatchKey) {
        let Some(record) = self.records.remove(key) else {
            return;
        };
        let index = if matches!(record.status, FsBatchStatus::Pending) {
            &mut self.pending
        } else {
            &mut self.finished
        };
        index.remove(&record.slot);
        self.forget(key, &record);
    }

    fn query(&mut self, key: &BatchKey, now: Instant) -> Option<FsBatchStatus> {
        self.expire(now);
        self.records.get(key).map(|record| record.status.clone())
    }

    /// Removes the expired records from the front of both indexes.
    fn expire(&mut self, now: Instant) {
        for pending in [true, false] {
            loop {
                let index = if pending {
                    &mut self.pending
                } else {
                    &mut self.finished
                };
                let expired = index
                    .first_key_value()
                    .is_some_and(|(slot, _)| slot.0 <= now);
                if !expired {
                    break;
                }
                let Some((_, key)) = index.pop_first() else {
                    break;
                };
                if let Some(record) = self.records.remove(&key) {
                    self.forget(&key, &record);
                }
            }
        }
    }

    /// Evicts the oldest finished records until `records` more records and
    /// `bytes` more bytes fit; batches in progress are never evicted.
    fn make_room(&mut self, records: usize, bytes: usize) {
        while self.records.len() + records > MAX_RECORDS || self.bytes + bytes > MAX_STORED_BYTES {
            let Some((_, key)) = self.finished.pop_first() else {
                break;
            };
            if let Some(record) = self.records.remove(&key) {
                self.forget(&key, &record);
            }
        }
    }

    /// Counts of a record that left the map (its index entry is gone too).
    fn forget(&mut self, key: &BatchKey, record: &Record) {
        self.bytes = self.bytes.saturating_sub(record.bytes);
        if let Some(owned) = self.per_principal.get_mut(&key.principal) {
            *owned = owned.saturating_sub(1);
            if *owned == 0 {
                self.per_principal.remove(&key.principal);
            }
        }
    }

    fn next_slot(&mut self, now: Instant, retention: Duration) -> Slot {
        self.sequence = self.sequence.wrapping_add(1);
        (now.checked_add(retention).unwrap_or(now), self.sequence)
    }
}

/// Memory one record holds: bookkeeping, its key twice (record map and
/// expiry index) and the outcome text.
fn record_bytes(key: &BatchKey, status: &FsBatchStatus) -> usize {
    RECORD_OVERHEAD + 2 * key.text_len() + outcome_bytes(status)
}

fn outcome_bytes(status: &FsBatchStatus) -> usize {
    let FsBatchStatus::Done { outcomes } = status else {
        return 0;
    };
    outcomes
        .iter()
        .map(|outcome| {
            OUTCOME_OVERHEAD
                + match outcome {
                    FsBatchOutcome::Published { path } => path.len(),
                    FsBatchOutcome::Failed { msg, .. } => msg.len(),
                }
        })
        .sum()
}

#[cfg(test)]
#[path = "batch_status_tests.rs"]
mod tests;
