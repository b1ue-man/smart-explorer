use super::*;

fn principal(device: &str) -> PeerPrincipal {
    PeerPrincipal::new("direct", "relation", device, "public-key", "node")
}

fn nonce() -> String {
    crate::share::core::random_hex_token::<16>().unwrap()
}

fn key(device: &str) -> BatchKey {
    BatchKey::new(principal(device), nonce())
}

fn done(length: usize) -> FsBatchStatus {
    FsBatchStatus::Done {
        outcomes: vec![FsBatchOutcome::Published {
            path: "p".repeat(length),
        }],
    }
}

fn is_busy(error: &io::Error) -> bool {
    crate::vfs::congestion_of(error).is_some()
}

#[test]
fn transfer_engine_task_batch_status_follows_one_commit() {
    let owner = principal(&nonce());
    let batch = nonce();
    let key = BatchKey::new(owner.clone(), batch.clone());
    assert_eq!(query(&owner, &batch), None);
    begin(&key).unwrap();
    assert_eq!(
        begin(&key).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists,
        "a nonce is used once"
    );
    assert_eq!(query(&owner, &batch), Some(FsBatchStatus::Pending));
    assert_eq!(query(&principal("other"), &batch), None);

    let committed = done(16);
    finish(&key, committed.clone());
    assert_eq!(query(&owner, &batch), Some(committed));
    delivered(&key);
    assert_eq!(query(&owner, &batch), None);

    let aborted = BatchKey::new(owner.clone(), nonce());
    begin(&aborted).unwrap();
    finish(&aborted, FsBatchStatus::Aborted);
    assert_eq!(query(&owner, &aborted.nonce), Some(FsBatchStatus::Aborted));
    delivered(&aborted);
}

#[test]
fn transfer_engine_task_batch_status_bounds_each_principal() {
    let now = Instant::now();
    let mut table = Table::default();
    let owned: Vec<BatchKey> = (0..MAX_RECORDS_PER_PRINCIPAL)
        .map(|_| key("busy"))
        .collect();
    for key in &owned {
        table.begin(key, now).unwrap();
    }
    let refused = table.begin(&key("busy"), now).unwrap_err();
    assert!(is_busy(&refused), "{refused}");
    table
        .begin(&key("other"), now)
        .expect("another device keeps its own share");
    table.delivered(&owned[0]);
    table
        .begin(&key("busy"), now)
        .expect("a delivered record frees its place");
    assert_eq!(
        table.per_principal.get(&principal("busy")),
        Some(&MAX_RECORDS_PER_PRINCIPAL)
    );
}

#[test]
fn transfer_engine_task_batch_status_bounds_all_records() {
    let now = Instant::now();
    let mut table = Table::default();
    let mut keys = Vec::new();
    for index in 0..MAX_RECORDS {
        let key = key(&format!("device-{}", index % 8));
        table.begin(&key, now).unwrap();
        keys.push(key);
    }
    let refused = table.begin(&key("device-late"), now).unwrap_err();
    assert!(is_busy(&refused), "batches in progress are never evicted");
    table.finish(&keys[3], FsBatchStatus::Aborted, now);
    table
        .begin(&key("device-late"), now)
        .expect("the oldest finished record makes room");
    assert!(table.query(&keys[3], now).is_none());
    assert_eq!(table.records.len(), MAX_RECORDS);
}

#[test]
fn transfer_engine_task_batch_status_counts_every_record() {
    let now = Instant::now();
    let mut table = Table::default();
    let aborted = key("counted");
    table.begin(&aborted, now).unwrap();
    let pending_bytes = table.bytes;
    assert_eq!(pending_bytes, RECORD_OVERHEAD + 2 * aborted.text_len());
    table.finish(&aborted, FsBatchStatus::Aborted, now);
    assert_eq!(
        table.bytes, pending_bytes,
        "an aborted record costs its key too"
    );

    let committed = key("counted");
    table.begin(&committed, now).unwrap();
    table.finish(&committed, done(100), now);
    assert_eq!(table.bytes, 2 * pending_bytes + OUTCOME_OVERHEAD + 100);
    assert_eq!(table.per_principal.get(&principal("counted")), Some(&2));

    table.delivered(&aborted);
    table.delivered(&committed);
    assert_eq!(table.bytes, 0);
    assert!(table.per_principal.is_empty());
    assert!(table.pending.is_empty() && table.finished.is_empty());
}

#[test]
fn transfer_engine_task_batch_status_expires_in_order() {
    let start = Instant::now();
    let mut table = Table::default();
    let finished = key("expiry");
    let running = key("expiry");
    table.begin(&finished, start).unwrap();
    table.finish(&finished, done(10), start);
    table.begin(&running, start).unwrap();

    let after_finished = start + FINISHED_RETENTION + Duration::from_secs(1);
    assert!(table.query(&finished, after_finished).is_none());
    assert_eq!(
        table.query(&running, after_finished),
        Some(FsBatchStatus::Pending),
        "a running batch outlives finished ones"
    );
    let after_pending = start + PENDING_RETENTION + Duration::from_secs(1);
    assert!(table.query(&running, after_pending).is_none());
    assert_eq!(table.bytes, 0);
    assert!(table.records.is_empty() && table.per_principal.is_empty());
}

#[test]
fn transfer_engine_task_batch_status_evicts_oldest_outcome_text() {
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let mut table = Table::default();
    let older = key("text");
    let newer = key("text");
    table.begin(&older, now).unwrap();
    table.begin(&newer, now).unwrap();
    table.finish(&older, done(MAX_STORED_BYTES / 2), now);
    table.finish(&newer, done(MAX_STORED_BYTES / 2), later);
    assert!(
        table.query(&older, later).is_none(),
        "the oldest outcome text goes first"
    );
    assert!(table.query(&newer, later).is_some());
    assert!(table.bytes <= MAX_STORED_BYTES);
}
