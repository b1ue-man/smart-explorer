//! Fail, restore the prerequisite, then finish the same recorded pair.
use super::sync_reliability_task_fixture::{clean, forward, Pair, TIME};
use super::*;
use std::io;
use std::sync::atomic::Ordering;

#[test]
fn sync_reliability_task_resume_scan_failure_preserves_checkpoint_without_complete_index() {
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let opts = BisyncOptions {
        delete: DeletePolicy::Mirror,
        ..forward()
    };
    pair.put(PairSide::A, "file.txt", b"recorded old bytes", TIME);
    let seeded = pair.run(opts, &filter);
    clean(&seeded);
    assert!(pair.index_complete(&seeded));
    pair.put(
        PairSide::A,
        "file.txt",
        b"replacement after reconnect",
        TIME + 10_000,
    );
    let promotions = pair.b.promotions.load(Ordering::SeqCst);
    let listings = pair.a.listings.load(Ordering::SeqCst);
    *pair.a.listing_fault.lock().unwrap() = Some(io::ErrorKind::ConnectionReset);
    let failed = pair.run(opts, &filter);
    assert!(
        !failed.errors.is_empty(),
        "incomplete enumeration must fail the scan"
    );
    assert!(pair.a.listings.load(Ordering::SeqCst) > listings);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"recorded old bytes");
    assert_eq!(pair.stored(&seeded), seeded.baseline);
    assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions);
    assert!(!pair.index_complete(&failed));
    *pair.a.listing_fault.lock().unwrap() = None;
    let resumed = pair.run(opts, &filter);
    clean(&resumed);
    assert_eq!(resumed.state, seeded.state);
    assert_eq!(
        pair.bytes(PairSide::B, "file.txt"),
        b"replacement after reconnect"
    );
    assert_eq!(pair.stored(&resumed), resumed.baseline);
    assert!(pair.index_complete(&resumed));
    pair.no_op(opts, &filter);
}

#[test]
fn sync_reliability_task_resume_create_failures_retry_without_publishing_old_baseline() {
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::StorageFull,
        io::ErrorKind::ConnectionReset,
    ] {
        let pair = Pair::new();
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let opts = forward();
        pair.put(PairSide::A, "file.txt", b"old destination", TIME);
        let seeded = pair.run(opts, &filter);
        clean(&seeded);
        pair.put(
            PairSide::A,
            "file.txt",
            b"eventual full replacement",
            TIME + 10_000,
        );
        let promotions = pair.b.promotions.load(Ordering::SeqCst);
        let opens = pair.b.stage_opens.load(Ordering::SeqCst);
        *pair.b.write_fault.lock().unwrap() = Some((kind, usize::MAX));
        let failed = pair.run(opts, &filter);
        assert!(
            !failed.errors.is_empty()
                || failed.stopped.is_some()
                || failed.omissions.protects("file.txt"),
            "{kind:?}"
        );
        assert!(pair.b.stage_opens.load(Ordering::SeqCst) > opens);
        assert_eq!(
            pair.bytes(PairSide::B, "file.txt"),
            b"old destination",
            "{kind:?}"
        );
        assert_eq!(pair.stored(&seeded), seeded.baseline, "{kind:?}");
        assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions);
        assert_eq!(pair.b.active.load(Ordering::SeqCst), 0);
        *pair.b.write_fault.lock().unwrap() = None;
        let resumed = pair.run(opts, &filter);
        clean(&resumed);
        assert_eq!(resumed.state, seeded.state);
        assert_eq!(
            pair.bytes(PairSide::B, "file.txt"),
            b"eventual full replacement"
        );
        assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions + 1);
        let old = pair
            .versions(&resumed)
            .into_iter()
            .find(|entry| entry.rel == "file.txt")
            .unwrap();
        assert_eq!(pair.version_bytes(&old), b"old destination");
        pair.no_op(opts, &filter);
    }
}

#[test]
fn sync_reliability_task_resume_safe_transient_retry_happens_before_single_publication() {
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let mut job = crate::syncjobs::SyncJob::new(
        "retry this job".into(),
        pair.roots[0].clone(),
        pair.roots[1].clone(),
    );
    job.direction = Direction::AtoB;
    job.retries = 1;
    job.retry_delay_secs = 0;
    job.verify = true;
    job.max_transfers = 1;
    job.versions_location = VersionsLocation::AppData;
    let opts = job.checked_opts(false).unwrap();
    pair.put(PairSide::A, "file.txt", b"confirmed before retry", TIME);
    let seeded = pair.run_settings(opts, &filter, RunSettings::for_job(&job.id));
    clean(&seeded);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"confirmed before retry");
    assert_eq!(pair.stored(&seeded), seeded.baseline);
    let opens = pair.b.stage_opens.load(Ordering::SeqCst);
    let promotions = pair.b.promotions.load(Ordering::SeqCst);
    // Replica markers already exist, so this fault reaches the copy stage
    // inside run_with_retry rather than first-run replica identification.
    pair.put(PairSide::A, "file.txt", b"retried complete content", TIME + 10_000);
    *pair.b.write_fault.lock().unwrap() = Some((io::ErrorKind::ConnectionReset, 1));
    let out = pair.run_settings(opts, &filter, RunSettings::for_job(&job.id));
    clean(&out);
    assert_eq!(
        out.state.as_ref().unwrap().owner,
        StateOwner::Job(job.id.clone())
    );
    assert_eq!(out.state, seeded.state);
    assert_eq!(pair.b.stage_opens.load(Ordering::SeqCst), opens + 2);
    assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions + 1);
    assert_eq!(
        pair.bytes(PairSide::B, "file.txt"),
        b"retried complete content"
    );
    assert_eq!(pair.stored(&out), out.baseline);
    let old = pair.versions(&out);
    assert_eq!(old.len(), 1);
    assert_eq!(pair.version_bytes(&old[0]), b"confirmed before retry");
    let noop = pair.run_settings(opts, &filter, RunSettings::for_job(&job.id));
    clean(&noop);
    assert_eq!(noop.state, out.state);
    assert_eq!(
        noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
        0
    );
    assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions + 1);
    assert_eq!(pair.stored(&noop), noop.baseline);
    pair.restore(&out, &old[0], PairSide::B);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"confirmed before retry");
    assert_eq!(pair.stored(&out), out.baseline);
}

#[test]
fn sync_reliability_task_resume_cancel_during_stream_preserves_old_bytes_and_checkpoint() {
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let opts = BisyncOptions {
        verify: true,
        ..forward()
    };
    pair.put(PairSide::A, "file.txt", b"old complete bytes", TIME);
    let seeded = pair.run(opts, &filter);
    clean(&seeded);
    let payload = vec![b'R'; 256 * 1_024];
    pair.put(PairSide::A, "file.txt", &payload, TIME + 10_000);
    let promotions = pair.b.promotions.load(Ordering::SeqCst);
    *pair.a.cancel_on_read.lock().unwrap() = Some(pair.cancel.clone());
    let canceled = pair.run(opts, &filter);
    assert!(canceled.canceled);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"old complete bytes");
    assert_eq!(pair.stored(&seeded), seeded.baseline);
    assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions);
    assert_eq!(pair.b.active.load(Ordering::SeqCst), 0);
    pair.cancel.store(false, Ordering::Release);
    let resumed = pair.run(opts, &filter);
    clean(&resumed);
    assert_eq!(resumed.state, seeded.state);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), payload);
    assert_eq!(pair.stored(&resumed), resumed.baseline);
    assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions + 1);
    pair.no_op(opts, &filter);
}

#[test]
fn sync_reliability_task_resume_backup_denial_blocks_mutation_then_recovers() {
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let opts = forward();
    pair.put(PairSide::A, "file.txt", b"recoverable original", TIME);
    let seeded = pair.run(opts, &filter);
    clean(&seeded);
    pair.put(
        PairSide::A,
        "file.txt",
        b"new content after rights recover",
        TIME + 10_000,
    );
    let promotions = pair.b.promotions.load(Ordering::SeqCst);
    pair.b.read_fault.store(true, Ordering::Release);
    let failed = pair.run(opts, &filter);
    assert!(!failed.errors.is_empty() || failed.omissions.protects("file.txt"));
    assert!(pair.b.read_fault_hits.load(Ordering::SeqCst) > 0);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"recoverable original");
    assert_eq!(pair.stored(&seeded), seeded.baseline);
    assert_eq!(pair.b.promotions.load(Ordering::SeqCst), promotions);
    pair.b.read_fault.store(false, Ordering::Release);
    let resumed = pair.run(opts, &filter);
    clean(&resumed);
    assert_eq!(resumed.state, seeded.state);
    assert_eq!(
        pair.bytes(PairSide::B, "file.txt"),
        b"new content after rights recover"
    );
    let old = pair
        .versions(&resumed)
        .into_iter()
        .find(|entry| entry.rel == "file.txt")
        .unwrap();
    assert_eq!(pair.version_bytes(&old), b"recoverable original");
    pair.no_op(opts, &filter);
    pair.restore(&resumed, &old, PairSide::B);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"recoverable original");
}
