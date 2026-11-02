//! Contract V3 of the sync engine: the pure types other blocks build on.
use std::borrow::Cow;

use super::completion::{CompletedAction, CompletedKind};
use super::keys::{KeyPolicy, Spellings};
use super::limits::SyncLimits;
use super::omissions::{OmissionKind, SyncOmissions};
use super::paths::{is_engine_name, REPLICA_MARKER_NAME, VERSIONS_DIR_NAME};
use super::run_types::{
    BlockConfirmation, ReplicaRef, RunBlock, RunSettings, ScanDepth, StateKey, StateOwner,
};
use super::snapshot_types::SideSnapshot;
use super::types::{PairSide, Sig, VersionsLocation};

fn sig(size: u64) -> Sig {
    Sig {
        size,
        mtime_ms: 1_700_000_000_000,
        hash: 0,
    }
}

#[test]
fn review_task_block_confirmation_tokens_round_trip_and_cover_only_their_stop() {
    let blocks = [
        RunBlock::MassDelete {
            side: PairSide::B,
            deletes: 120,
            files: 200,
        },
        RunBlock::DeleteLimit {
            deletes: 40,
            limit: 10,
        },
        RunBlock::SideEmpty {
            side: PairSide::A,
            previous: 7,
        },
        RunBlock::ReplicaMissing { side: PairSide::B },
    ];
    for block in &blocks {
        let confirmation = block.confirmation();
        assert!(confirmation.covers(block), "{block:?}");
        assert_eq!(
            BlockConfirmation::parse(&confirmation.token()),
            Some(confirmation.clone())
        );
        assert!(!block.message().is_empty());
        assert!(!block.code().is_empty());
    }
    let allowed = BlockConfirmation::Deletes {
        side: Some(PairSide::B),
        max: 120,
    };
    assert!(!allowed.covers(&RunBlock::MassDelete {
        side: PairSide::B,
        deletes: 121,
        files: 200,
    }));
    assert!(!allowed.covers(&RunBlock::MassDelete {
        side: PairSide::A,
        deletes: 1,
        files: 200,
    }));
    assert!(!BlockConfirmation::AcceptSide { side: PairSide::A }
        .covers(&RunBlock::ReplicaMissing { side: PairSide::B }));
    assert_eq!(allowed.token(), "deletes:b:120");
    for invalid in [
        "",
        "deletes:b:x",
        "deletes:c:1",
        "accept:c",
        "accept:a:1",
        "drop:a",
    ] {
        assert_eq!(BlockConfirmation::parse(invalid), None, "{invalid}");
    }
}

#[test]
fn review_task_plan_keys_normalize_and_fold_only_where_needed() {
    let exact = KeyPolicy::for_pair(true, true);
    assert!(!exact.fold_case);
    assert!(matches!(exact.key("Photos/x.jpg"), Cow::Borrowed(_)));
    assert_eq!(exact.key("Mu\u{308}ller.pdf"), "M\u{fc}ller.pdf");
    assert_ne!(exact.key("Photos/x.jpg"), exact.key("photos/x.jpg"));

    let folded = KeyPolicy::for_pair(true, false);
    assert!(folded.fold_case);
    assert_eq!(folded.key("Photos/IMG.jpg"), folded.key("photos/img.JPG"));
    assert_eq!(folded.key("Mu\u{308}LLER"), folded.key("m\u{fc}ller"));
    assert_ne!(folded.key("Stra\u{df}e"), folded.key("STRASSE"));
}

#[test]
fn review_task_spellings_keep_each_sides_path() {
    let mut spellings = Spellings::default();
    spellings.insert("Photos/x.jpg", PairSide::A, "Photos/x.jpg");
    assert!(spellings.is_empty());
    spellings.insert("Photos/x.jpg", PairSide::B, "photos/x.jpg");
    assert_eq!(
        spellings.side_rel("Photos/x.jpg", PairSide::B),
        "photos/x.jpg"
    );
    assert_eq!(
        spellings.side_rel("Photos/x.jpg", PairSide::A),
        "Photos/x.jpg"
    );
    assert_eq!(spellings.side_rel("other", PairSide::B), "other");
}

#[test]
fn review_task_completed_actions_map_to_baseline_entries() {
    let done = |kind| CompletedAction {
        rel: "a.txt".into(),
        kind,
        src_sig: Some(sig(1)),
        dst_sig: Some(sig(2)),
        durable: true,
    };
    let entry = |kind| done(kind).baseline_entry();
    assert_eq!(
        entry(CompletedKind::Copied { from: PairSide::A }),
        Some((Some(sig(1)), Some(sig(2))))
    );
    assert_eq!(
        entry(CompletedKind::Copied { from: PairSide::B }),
        Some((Some(sig(2)), Some(sig(1))))
    );
    assert_eq!(
        entry(CompletedKind::Moved { from: PairSide::A }),
        Some((None, Some(sig(2))))
    );
    assert_eq!(
        entry(CompletedKind::Deleted { side: PairSide::B }),
        Some((None, None))
    );
    assert_eq!(entry(CompletedKind::DirCreated { side: PairSide::A }), None);
}

#[test]
fn review_task_limits_follow_memory_but_never_fall_below_the_previous_caps() {
    assert_eq!(SyncLimits::for_memory(None), SyncLimits::FALLBACK);
    assert_eq!(
        SyncLimits::for_memory(Some(512 * 1024 * 1024)),
        SyncLimits::FALLBACK
    );
    let large = SyncLimits::for_memory(Some(64 * 1024 * 1024 * 1024));
    assert!(large.walk_entries > SyncLimits::FALLBACK.walk_entries);
    assert!(large.walk_text_bytes > SyncLimits::FALLBACK.walk_text_bytes);
    assert_eq!(large.state_entries, large.walk_entries * 2);
    assert!(large.state_file_bytes() > large.state_text_bytes);
}

#[test]
fn review_task_omissions_protect_and_report_by_kind() {
    let mut omissions = SyncOmissions::new(false);
    omissions.record_kind("locked/file.pst", OmissionKind::Unreadable, true);
    omissions.record_kind("cache", OmissionKind::Filtered, false);
    omissions.record("link", true);
    assert!(omissions.protects("cache/inner.bin"));
    assert!(omissions.protects("locked"));
    assert_eq!(
        omissions.reported().collect::<Vec<_>>(),
        [
            ("link", OmissionKind::Link),
            ("locked/file.pst", OmissionKind::Unreadable)
        ]
    );
    assert_eq!(omissions.counts().get(&OmissionKind::Unreadable), Some(&1));
    assert_eq!(omissions.counts().get(&OmissionKind::Filtered), None);
    for kind in OmissionKind::ALL {
        assert_eq!(OmissionKind::parse(kind.as_str()), Some(kind));
        assert!(!kind.label().is_empty());
    }
    assert!(!OmissionKind::Filtered.reported_by_default());
    assert!(!OmissionKind::OwnFile.reported_by_default());
    assert!(OmissionKind::Special.reported_by_default());
}

#[test]
fn review_task_side_snapshot_is_empty_without_user_entries() {
    let mut side = SideSnapshot::new(true);
    assert!(side.is_empty());
    side.omissions
        .record_kind(REPLICA_MARKER_NAME, OmissionKind::OwnFile, false);
    assert!(side.is_empty(), "the engine's own entries are no content");
    side.dirs.insert("empty folder".into());
    assert!(!side.is_empty());
    assert_eq!(side.entry_count(), 1);
}

#[test]
fn review_task_engine_names_and_run_settings() {
    assert!(is_engine_name(REPLICA_MARKER_NAME));
    assert!(is_engine_name(VERSIONS_DIR_NAME));
    assert!(!is_engine_name("se-versions"));
    for depth in [
        ScanDepth::Incremental,
        ScanDepth::VerifySources,
        ScanDepth::Full,
    ] {
        assert_eq!(ScanDepth::parse(depth.as_str()), Some(depth));
    }
    assert_eq!(RunSettings::default().owner, StateOwner::AdHoc);
    assert_eq!(
        RunSettings::for_job("job-1").owner,
        StateOwner::Job("job-1".into())
    );
    assert!(StateKey::legacy("0a", "0b").is_legacy());
    let mut owned = StateKey::legacy("0a", "0b");
    owned.replica_b = ReplicaRef::Marker("r".into());
    assert!(!owned.is_legacy());
    assert_eq!(owned.replica(PairSide::B), &ReplicaRef::Marker("r".into()));
    for location in VersionsLocation::ALL {
        assert_eq!(VersionsLocation::parse(location.as_str()), Some(location));
    }
    assert_eq!(PairSide::parse(PairSide::A.as_str()), Some(PairSide::A));
    assert_eq!(PairSide::A.other(), PairSide::B);
}
