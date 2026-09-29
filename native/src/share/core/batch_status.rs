//! Outcomes of PutBatch commits by client nonce (K18e): a client whose reply
//! was lost after its commit asks once more and learns exactly which entries
//! were published under which name, so nothing is ever retried blindly.
use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::share::session::PeerPrincipal;
use crate::share::wire::{FsBatchOutcome, FsBatchStatus};

/// A client asks right after reconnecting, within one operation deadline
/// (60 s); five times that covers a slow route change with margin.
const FINISHED_RETENTION: Duration = Duration::from_secs(5 * 60);

/// A batch still receiving or publishing is kept as long as it can run: the
/// per-chunk deadline (256 KiB within 60 s) lets a 16 MiB batch take 64
/// minutes at the slowest rate a transfer survives.
const PENDING_RETENTION: Duration = Duration::from_secs(70 * 60);

/// Stored outcome text of all records: 64 of the largest batch headers
/// (256 KiB), more than the 60 batches one connection can have in flight.
const MAX_STORED_BYTES: usize = 64 * 256 * 1024;

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
}

struct Record {
    status: FsBatchStatus,
    bytes: usize,
    updated: Instant,
}

fn table() -> io::Result<MutexGuard<'static, HashMap<BatchKey, Record>>> {
    static TABLE: OnceLock<Mutex<HashMap<BatchKey, Record>>> = OnceLock::new();
    TABLE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| crate::share::core::eio("Paketstatus ist gesperrt"))
}

/// Registers a batch as it is accepted; a reused nonce is refused.
pub(super) fn begin(key: &BatchKey) -> io::Result<()> {
    let mut records = table()?;
    let now = Instant::now();
    prune(&mut records, now);
    if records.contains_key(key) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Paket-Kennung wurde bereits verwendet",
        ));
    }
    records.insert(
        key.clone(),
        Record {
            status: FsBatchStatus::Pending,
            bytes: 0,
            updated: now,
        },
    );
    Ok(())
}

/// Records how the batch ended (committed or aborted before its commit).
pub(super) fn finish(key: &BatchKey, status: FsBatchStatus) {
    let Ok(mut records) = table() else {
        return;
    };
    let now = Instant::now();
    if let Some(record) = records.get_mut(key) {
        record.bytes = stored_bytes(&status);
        record.status = status;
        record.updated = now;
    }
    prune(&mut records, now);
}

/// The client confirmed it holds the outcomes; nothing needs to remain.
pub(super) fn delivered(key: &BatchKey) {
    if let Ok(mut records) = table() {
        records.remove(key);
    }
}

/// The state of `principal`'s batch `nonce`; `None` when unknown here.
pub(super) fn query(principal: &PeerPrincipal, nonce: &str) -> Option<FsBatchStatus> {
    let key = BatchKey::new(principal.clone(), nonce.to_string());
    let records = table().ok()?;
    records.get(&key).map(|record| record.status.clone())
}

fn prune(records: &mut HashMap<BatchKey, Record>, now: Instant) {
    records.retain(|_, record| {
        let retention = match record.status {
            FsBatchStatus::Pending => PENDING_RETENTION,
            FsBatchStatus::Aborted | FsBatchStatus::Done { .. } => FINISHED_RETENTION,
        };
        now.saturating_duration_since(record.updated) < retention
    });
    let mut total: usize = records.values().map(|record| record.bytes).sum();
    while total > MAX_STORED_BYTES {
        // Oldest finished record first; pending ones store no outcome text.
        let oldest = records
            .iter()
            .filter(|(_, record)| record.bytes > 0)
            .min_by_key(|(_, record)| record.updated)
            .map(|(key, _)| key.clone());
        let Some(oldest) = oldest else {
            break;
        };
        if let Some(removed) = records.remove(&oldest) {
            total = total.saturating_sub(removed.bytes);
        }
    }
}

fn stored_bytes(status: &FsBatchStatus) -> usize {
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
mod tests {
    use super::*;

    fn principal(device: &str) -> PeerPrincipal {
        PeerPrincipal::new("direct", "relation", device, "public-key", "node")
    }

    fn nonce() -> String {
        crate::share::core::random_hex_token::<16>().unwrap()
    }

    #[test]
    fn transfer_engine_task_batch_status_follows_one_commit() {
        let owner = principal("owner");
        let nonce = nonce();
        let key = BatchKey::new(owner.clone(), nonce.clone());
        assert_eq!(query(&owner, &nonce), None);
        begin(&key).unwrap();
        assert_eq!(
            begin(&key).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists,
            "a nonce is used once"
        );
        assert_eq!(query(&owner, &nonce), Some(FsBatchStatus::Pending));
        assert_eq!(query(&principal("other"), &nonce), None);

        let done = FsBatchStatus::Done {
            outcomes: vec![FsBatchOutcome::Published {
                path: "/A/datei (2).txt".into(),
            }],
        };
        finish(&key, done.clone());
        assert_eq!(query(&owner, &nonce), Some(done));
        delivered(&key);
        assert_eq!(query(&owner, &nonce), None);

        let aborted = BatchKey::new(owner.clone(), self::nonce());
        begin(&aborted).unwrap();
        finish(&aborted, FsBatchStatus::Aborted);
        assert_eq!(query(&owner, &aborted.nonce), Some(FsBatchStatus::Aborted));
    }

    #[test]
    fn transfer_engine_task_batch_status_prunes_by_age_and_size() {
        let now = Instant::now();
        let Some(old) = now.checked_sub(FINISHED_RETENTION + Duration::from_secs(1)) else {
            return; // The monotonic clock started too recently to age a record.
        };
        let mut records = HashMap::new();
        let record = |status: FsBatchStatus, updated| Record {
            bytes: stored_bytes(&status),
            status,
            updated,
        };
        let done = |length: usize| FsBatchStatus::Done {
            outcomes: vec![FsBatchOutcome::Published {
                path: "p".repeat(length),
            }],
        };
        records.insert(
            BatchKey::new(principal("a"), nonce()),
            record(done(10), old),
        );
        records.insert(
            BatchKey::new(principal("b"), nonce()),
            record(FsBatchStatus::Pending, old),
        );
        prune(&mut records, now);
        assert_eq!(
            records.len(),
            1,
            "an old finished record expires, a pending one stays"
        );

        let half = MAX_STORED_BYTES / 2;
        let older = BatchKey::new(principal("c"), nonce());
        let newer = BatchKey::new(principal("d"), nonce());
        let before = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
        records.insert(older.clone(), record(done(half), before));
        records.insert(newer.clone(), record(done(half), now));
        prune(&mut records, now);
        assert!(
            !records.contains_key(&older),
            "the oldest outcome text goes first"
        );
        assert!(records.contains_key(&newer));
        let total: usize = records.values().map(|record| record.bytes).sum();
        assert!(total <= MAX_STORED_BYTES);
    }
}
