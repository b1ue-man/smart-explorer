//! Partial scans preserve counterparts, confirmed state and foreign ownership.
use super::sync_reliability_task_fixture::{clean, forward, Pair, TIME};
use super::*;
use crate::vfs::OmissionReason;

#[test]
fn sync_reliability_task_protection_tolerant_children_continue_without_complete_index() {
    for reason in [
        OmissionReason::Unreadable,
        OmissionReason::Vanished,
        OmissionReason::Unrepresentable,
    ] {
        for omitted_side in [PairSide::A, PairSide::B] {
            let pair = Pair::new();
            let globs = empty_globset();
            let filter = WalkFilter::basic(true, &globs);
            let opts = BisyncOptions {
                delete: DeletePolicy::Mirror,
                compare: CompareMode::Checksum,
                ..forward()
            };
            pair.put(
                PairSide::A,
                "protected/file.txt",
                b"previous confirmed bytes",
                TIME,
            );
            pair.put(PairSide::A, "independent.txt", b"old independent", TIME);
            let seeded = pair.run(opts, &filter);
            clean(&seeded);
            assert!(pair.index_complete(&seeded));
            let saved = seeded.baseline["protected/file.txt"];
            pair.put(
                PairSide::A,
                "protected/file.txt",
                b"source changed while protected",
                TIME + 10_000,
            );
            pair.put(
                PairSide::B,
                "protected/file.txt",
                b"private target edit",
                TIME + 20_000,
            );
            pair.put(
                PairSide::B,
                "protected/foreign.txt",
                b"new foreign child",
                TIME + 20_000,
            );
            pair.put(
                PairSide::B,
                "ordinary-orphan.txt",
                b"ordinary mirror orphan",
                TIME,
            );
            pair.put(
                PairSide::A,
                "independent.txt",
                b"completed independent update",
                TIME + 10_000,
            );
            let backend = if omitted_side == PairSide::A {
                &pair.a
            } else {
                &pair.b
            };
            *backend.omission.lock().unwrap() = Some(("protected".into(), reason));
            let preview = pair.preview(opts, &filter);
            assert!(preview.error.is_none() && preview.blocked.is_none());
            assert!(preview.omissions.protects("protected/foreign.txt"));
            assert_eq!(
                preview.omissions.reported_paths().collect::<Vec<_>>(),
                ["protected"]
            );
            assert!(preview
                .actions
                .iter()
                .all(|action| !super::core::action_rel(action).starts_with("protected/")));
            let partial = pair.run(opts, &filter);
            clean(&partial);
            assert_eq!(partial.state, seeded.state);
            assert_eq!(
                partial.omissions.reported_paths().collect::<Vec<_>>(),
                ["protected"]
            );
            assert_eq!(
                pair.bytes(PairSide::B, "independent.txt"),
                b"completed independent update"
            );
            assert_eq!(
                pair.bytes(PairSide::B, "protected/file.txt"),
                b"private target edit"
            );
            assert_eq!(
                pair.bytes(PairSide::B, "protected/foreign.txt"),
                b"new foreign child"
            );
            assert!(!pair.path(PairSide::B, "ordinary-orphan.txt").exists());
            assert_eq!(partial.baseline["protected/file.txt"], saved);
            assert_eq!(pair.stored(&partial)["protected/file.txt"], saved);
            assert!(
                !pair.index_complete(&partial),
                "{reason:?}/{omitted_side:?}: partial scan cannot bootstrap"
            );
            let noop = pair.no_op(opts, &filter);
            assert!(!pair.index_complete(&noop));
            assert_eq!(noop.baseline["protected/file.txt"], saved);
            *backend.omission.lock().unwrap() = None;
            let recovered = pair.run(opts, &filter);
            clean(&recovered);
            assert!(recovered.omissions.reported_paths().next().is_none());
            assert_eq!(
                pair.bytes(PairSide::B, "protected/file.txt"),
                b"source changed while protected"
            );
            assert!(!pair.path(PairSide::B, "protected/foreign.txt").exists());
            let versions = pair.versions(&recovered);
            for bytes in [
                b"private target edit".as_slice(),
                b"new foreign child".as_slice(),
            ] {
                assert!(versions
                    .iter()
                    .any(|entry| pair.version_bytes(entry) == bytes));
            }
            assert_eq!(pair.stored(&recovered), recovered.baseline);
            assert!(pair.index_complete(&recovered));
            pair.no_op(opts, &filter);
        }
    }
}

#[test]
fn sync_reliability_task_protection_first_partial_run_never_seeds_full_index() {
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let opts = BisyncOptions {
        delete: DeletePolicy::Mirror,
        compare: CompareMode::Checksum,
        ..forward()
    };
    pair.put(
        PairSide::A,
        "protected/file.txt",
        b"temporarily unreadable source",
        TIME,
    );
    pair.put(
        PairSide::B,
        "protected/file.txt",
        b"unconfirmed counterpart",
        TIME,
    );
    pair.put(PairSide::A, "independent.txt", b"finish this file", TIME);
    *pair.a.omission.lock().unwrap() = Some(("protected".into(), OmissionReason::Unreadable));
    let partial = pair.run(opts, &filter);
    clean(&partial);
    assert_eq!(
        pair.bytes(PairSide::B, "independent.txt"),
        b"finish this file"
    );
    assert_eq!(
        pair.bytes(PairSide::B, "protected/file.txt"),
        b"unconfirmed counterpart"
    );
    assert!(!partial.baseline.contains_key("protected/file.txt"));
    assert!(!pair.stored(&partial).contains_key("protected/file.txt"));
    assert!(!pair.index_complete(&partial));
    pair.no_op(opts, &filter);
    *pair.a.omission.lock().unwrap() = None;
    let complete = pair.run(opts, &filter);
    clean(&complete);
    assert_eq!(
        pair.bytes(PairSide::B, "protected/file.txt"),
        b"temporarily unreadable source"
    );
    assert!(pair.index_complete(&complete));
    pair.no_op(opts, &filter);
}

#[test]
fn sync_reliability_task_protection_job_owners_and_reverse_pair_lock_are_isolated() {
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let first_job = crate::syncjobs::SyncJob::new(
        "first owner".into(),
        pair.roots[0].clone(),
        pair.roots[1].clone(),
    );
    let second_job = crate::syncjobs::SyncJob::new(
        "second owner".into(),
        pair.roots[0].clone(),
        pair.roots[1].clone(),
    );
    assert_ne!(first_job.id, second_job.id);
    let opts = BisyncOptions {
        direction: Direction::Both,
        compare: CompareMode::Checksum,
        ..forward()
    };
    pair.put(PairSide::A, "file.txt", b"recorded original", TIME);
    let first = pair.run_settings(opts, &filter, RunSettings::for_job(&first_job.id));
    clean(&first);
    pair.put(PairSide::A, "file.txt", b"first side edit", TIME + 10_000);
    pair.put(PairSide::B, "file.txt", b"second side edit", TIME + 20_000);
    {
        let state = first.state.as_ref().unwrap();
        let _lock = PairLock::acquire(&state.lock_id).unwrap();
        let mut reverse = RunRequest::new(
            &pair.b,
            &pair.roots[1],
            &pair.a,
            &pair.roots[0],
            opts,
            &filter,
            &pair.cancel,
        );
        reverse.settings = RunSettings::for_job(&second_job.id);
        let busy = run_with(reverse);
        assert!(busy.busy && busy.errors.is_empty());
        assert_eq!(pair.bytes(PairSide::A, "file.txt"), b"first side edit");
        assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"second side edit");
        assert_eq!(pair.stored(&first), first.baseline);
    }
    let foreign = pair.run_settings(opts, &filter, RunSettings::for_job(&second_job.id));
    assert!(foreign.errors.is_empty() && foreign.blocked.is_none());
    assert_eq!(foreign.conflicts.len(), 1);
    assert_eq!(
        foreign.state.as_ref().unwrap().owner,
        StateOwner::Job(second_job.id.clone())
    );
    assert!(
        foreign.baseline.is_empty(),
        "new owner cannot adopt the other job's checkpoint"
    );
    assert_eq!(pair.stored(&first), first.baseline);
    let winner = BisyncOptions {
        conflict: ConflictMode::SourceWins,
        ..opts
    };
    let resolved = pair.run_settings(winner, &filter, RunSettings::for_job(&first_job.id));
    clean(&resolved);
    assert_eq!(resolved.state, first.state);
    assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"first side edit");
    assert!(pair
        .versions(&resolved)
        .iter()
        .any(
            |entry| entry.job_id.as_deref() == Some(first_job.id.as_str())
                && pair.version_bytes(entry) == b"second side edit"
        ));
    let settled = pair.run_settings(opts, &filter, RunSettings::for_job(&second_job.id));
    clean(&settled);
    assert_eq!(settled.state, foreign.state);
    let noop = pair.run_settings(opts, &filter, RunSettings::for_job(&second_job.id));
    clean(&noop);
    assert_eq!(
        noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
        0
    );
    assert_eq!(pair.stored(&noop), noop.baseline);
}
