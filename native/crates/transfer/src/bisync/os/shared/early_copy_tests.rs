//! G4 MS14 invariants: a copy during the scan only ever creates a file the
//! target has proven absent and the baseline does not know (never anything
//! that derives from absence, such as a deletion or a re-copy of a file the
//! target deleted); nothing runs while a side's root is empty against its
//! history; a complete run records early copies like normal ones.
use super::super::checkpoint_review_tests::Fixture;
use super::super::snapshot::WalkFilter;
use super::super::snapshot_dir::Listed;
use super::super::types::{Baseline, BisyncOptions, PairSide, Sig};
use super::super::versions::{RunVersions, VersionsContext};
use super::super::{empty_globset, orchestration, StateOwner, VersionsLocation};
use super::EarlyLane;
use std::sync::atomic::AtomicBool;

fn sig(size: u64) -> Sig {
    Sig {
        size,
        mtime_ms: 1_700_000_000_000,
        hash: 0,
    }
}

fn listing(files: &[&str], dirs: &[&str]) -> Listed {
    Listed {
        files: files
            .iter()
            .map(|rel| (rel.to_string(), sig(3), String::new()))
            .collect(),
        dirs: dirs
            .iter()
            .map(|rel| (format!("/{rel}"), rel.to_string()))
            .collect(),
    }
}

fn versions() -> RunVersions {
    RunVersions::begin(VersionsContext::new(
        "early-copy-test",
        StateOwner::AdHoc,
        VersionsLocation::AppData,
        Default::default(),
    ))
}

fn queued(lane: &EarlyLane<'_>) -> Vec<String> {
    let state = lane.state.lock().unwrap();
    let mut rels = state
        .queue
        .iter()
        .map(|job| job.rel.clone())
        .collect::<Vec<_>>();
    rels.sort();
    rels
}

#[test]
fn speed_task_sync_early_copy_needs_target_proof_and_unknown_name() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let versions = versions();
    let mut baseline = Baseline::new();
    // Synced before and since deleted on B: the plan decides (deletion or
    // conflict); the lane must never copy it back.
    baseline.insert("known.txt".into(), (Some(sig(3)), Some(sig(3))));
    let lane = EarlyLane::new(
        fixture.endpoints(),
        BisyncOptions::default(),
        Default::default(),
        &baseline,
        [false; 2],
        false,
        &cancel,
        &versions,
    )
    .expect("the lane runs for two plain local folders");
    lane.listed(
        PairSide::A,
        "",
        &listing(&["new.txt", "known.txt", "both.txt"], &["sub"]),
    );
    lane.listed(PairSide::A, "sub", &listing(&["sub/deep.txt"], &[]));
    assert!(
        queued(&lane).is_empty(),
        "nothing is copied before the target listed anything"
    );
    lane.listed(PairSide::B, "", &listing(&["both.txt"], &[]));
    assert_eq!(
        queued(&lane),
        ["new.txt", "sub/deep.txt"],
        "only names the target proved absent and the baseline never saw"
    );
}

#[test]
fn speed_task_sync_early_copy_waits_for_the_side_empty_guard() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let versions = versions();
    let lane = EarlyLane::new(
        fixture.endpoints(),
        BisyncOptions::default(),
        Default::default(),
        &Baseline::new(),
        [false, true],
        false,
        &cancel,
        &versions,
    )
    .expect("the lane runs for two plain local folders");
    lane.listed(PairSide::A, "", &listing(&["a.txt"], &[]));
    // B held entries before and is empty now (an unmounted drive, say).
    lane.listed(PairSide::B, "", &listing(&[], &[]));
    assert!(queued(&lane).is_empty());
    assert!(lane.state.lock().unwrap().stopped);
}

#[test]
fn speed_task_sync_early_copy_is_off_for_dry_runs_and_moves() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let versions = versions();
    for opts in [
        BisyncOptions {
            dry_run: true,
            ..Default::default()
        },
        BisyncOptions {
            move_files: true,
            ..Default::default()
        },
    ] {
        let lane = EarlyLane::new(
            fixture.endpoints(),
            opts,
            Default::default(),
            &Baseline::new(),
            [false; 2],
            false,
            &cancel,
            &versions,
        );
        assert!(lane.is_none());
    }
}

#[test]
fn speed_task_sync_run_records_early_copies_and_still_deletes() {
    let fixture = Fixture::new();
    let a = std::path::Path::new(&fixture.a);
    let b = std::path::Path::new(&fixture.b);
    std::fs::create_dir_all(a.join("keep")).unwrap();
    std::fs::write(a.join("keep/old.txt"), b"old").unwrap();
    let ignore = empty_globset();
    let filter = WalkFilter::basic(true, &ignore);
    let cancel = AtomicBool::new(false);
    let store = fixture.root.join("index.sqlite");
    let opts = BisyncOptions::default();
    let first =
        orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert!(b.join("keep/old.txt").is_file());
    std::fs::remove_file(a.join("keep/old.txt")).unwrap();
    for index in 0..40 {
        let folder = a.join(format!("new{}", index % 4));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("f{index}.txt")), b"fresh").unwrap();
    }
    let second =
        orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert_eq!(second.stats.a_to_b, 40);
    assert_eq!(
        second.stats.deleted, 1,
        "the deletion still comes from the complete plan"
    );
    assert!(!b.join("keep/old.txt").exists());
    assert_eq!(std::fs::read(b.join("new1/f1.txt")).unwrap(), b"fresh");
    let third =
        orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert!(third.errors.is_empty(), "{:?}", third.errors);
    assert_eq!(
        third.stats.a_to_b + third.stats.b_to_a + third.stats.deleted,
        0,
        "early copies are in the baseline like normal ones"
    );
}
