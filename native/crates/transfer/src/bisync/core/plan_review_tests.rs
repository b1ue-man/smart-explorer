//! Behavioral acceptance of the RV1 pair planner; executed only by the
//! task-level remote suite together with its apply integrations.
use super::compare::{same_file, same_time, Against, TimeRules};
use super::plan_pair::plan_pair;
use super::plan_types::PlanContext;
use super::*;
use crate::vfs::{MtimePrecision, TargetLimits};

fn sig(size: u64, time: i64) -> Sig {
    Sig {
        size,
        mtime_ms: time,
        hash: 0,
    }
}
fn side(files: &[(&str, Sig)]) -> SideSnapshot {
    let mut snapshot = SideSnapshot::new(true);
    snapshot
        .tree
        .extend(files.iter().map(|(rel, sig)| (rel.to_string(), *sig)));
    snapshot
}
fn mirror() -> BisyncOptions {
    BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        ..Default::default()
    }
}

#[test]
fn review_task_mirror_uses_each_recorded_time_without_copy_churn() {
    let a = sig(7, 1_000);
    let b = sig(7, 91_000);
    let base = Baseline::from([("x".into(), (Some(a), Some(b)))]);
    let plan = plan_pair(
        &mut side(&[("x", a)]),
        &mut side(&[("x", b)]),
        &base,
        &PlanContext::new(mirror()),
    );
    assert!(plan.actions.is_empty());
    assert!(plan.conflicts.is_empty());
    assert!(
        plan.records.is_empty(),
        "unchanged pair keeps the acknowledged basis"
    );
}

#[test]
fn review_task_equal_hashes_converge_without_a_previous_basis() {
    let mut a = sig(9, 100);
    a.hash = 77;
    let b = Sig { mtime_ms: 500, ..a };
    let plan = plan_pair(
        &mut side(&[("x", a)]),
        &mut side(&[("x", b)]),
        &Baseline::new(),
        &PlanContext::new(BisyncOptions::default()),
    );
    assert!(plan.actions.is_empty() && plan.conflicts.is_empty());
    assert_eq!(plan.records, vec![("x".into(), (Some(a), Some(b)))]);
}

#[test]
fn review_task_checksum_never_certifies_a_missing_hash() {
    let opts = BisyncOptions {
        compare: CompareMode::Checksum,
        ..Default::default()
    };
    let signature = sig(3, 40);
    assert!(!same_file(
        signature,
        signature,
        MtimePrecision::Seconds,
        &opts,
        Against::Baseline
    ));
}

#[test]
fn review_task_fat_hour_shift_is_separate_from_the_modify_window() {
    assert!(same_time(0, 3_601_500, MtimePrecision::TwoSeconds, 60_000));
    assert!(!same_time(0, 3_630_000, MtimePrecision::TwoSeconds, 60_000));
    assert!(!same_time(0, 3_600_000, MtimePrecision::Seconds, 0));
    assert!(!same_time(
        i64::MIN,
        i64::MAX,
        MtimePrecision::Unknown,
        i64::MAX
    ));
    assert!(same_time(10_001, 10_900, MtimePrecision::Seconds, 0));
}

#[test]
fn review_task_one_way_repairs_destination_drift_and_keeps_real_conflicts() {
    let old = sig(10, 100);
    let changed = sig(11, 200);
    let base = Baseline::from([("x".into(), (Some(old), Some(old)))]);
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        ..Default::default()
    };
    let ctx = PlanContext::new(opts);
    let repair = plan_pair(
        &mut side(&[("x", old)]),
        &mut side(&[("x", changed)]),
        &base,
        &ctx,
    );
    assert_eq!(repair.actions, vec![Action::CopyAtoB("x".into())]);
    let both = plan_pair(
        &mut side(&[("x", sig(12, 300))]),
        &mut side(&[("x", changed)]),
        &base,
        &ctx,
    );
    assert!(both.actions.is_empty());
    assert_eq!(both.conflicts.len(), 1);
    let initial = plan_pair(
        &mut side(&[("x", old)]),
        &mut side(&[("x", changed)]),
        &Baseline::new(),
        &ctx,
    );
    assert_eq!(initial.conflicts.len(), 1);
}

#[test]
fn review_task_modified_file_beats_deletion_under_size_and_time_policies() {
    let old = sig(10, 100);
    let edited = sig(11, 200);
    let base = Baseline::from([("x".into(), (Some(old), Some(old)))]);
    for conflict in [
        ConflictMode::NewerWins,
        ConflictMode::OlderWins,
        ConflictMode::LargerWins,
        ConflictMode::SmallerWins,
    ] {
        let ctx = PlanContext::new(BisyncOptions {
            conflict,
            ..Default::default()
        });
        let a_survives = plan_pair(&mut side(&[("x", edited)]), &mut side(&[]), &base, &ctx);
        assert_eq!(a_survives.actions, vec![Action::CopyAtoB("x".into())]);
        let b_survives = plan_pair(&mut side(&[]), &mut side(&[("x", edited)]), &base, &ctx);
        assert_eq!(b_survives.actions, vec![Action::CopyBtoA("x".into())]);
    }
}

#[test]
fn review_task_pending_move_uses_its_acknowledged_pair() {
    let a = sig(10, 100);
    let b = sig(10, 500);
    let base = Baseline::from([("x".into(), (Some(a), Some(b)))]);
    let opts = BisyncOptions {
        move_files: true,
        ..mirror()
    };
    let plan = plan_pair(
        &mut side(&[("x", a)]),
        &mut side(&[("x", b)]),
        &base,
        &PlanContext::new(opts),
    );
    assert_eq!(plan.actions, vec![Action::FinalizeMoveAtoB("x".into())]);
    assert_eq!(super::guards::DeleteCounts::of(&plan.actions).total(), 0);
    let complete = plan_pair(
        &mut side(&[]),
        &mut side(&[("x", b)]),
        &base,
        &PlanContext::new(opts),
    );
    assert!(complete.actions.is_empty());
    assert_eq!(complete.records, vec![("x".into(), (None, Some(b)))]);
}

#[test]
fn review_task_folded_nfc_keys_keep_both_io_spellings_and_existing_parents() {
    let old = sig(5, 100);
    let mut ctx = PlanContext::new(mirror());
    ctx.keys = KeyPolicy { fold_case: true };
    let mut a = side(&[("Photos/Cafe\u{301}.txt", old), ("Photos/new.txt", old)]);
    let mut b = side(&[("photos/Café.txt", old)]);
    a.dirs.insert("Photos".into());
    b.dirs.insert("photos".into());
    let plan = plan_pair(&mut a, &mut b, &Baseline::new(), &ctx);
    assert_eq!(
        plan.actions,
        vec![Action::CopyAtoB("Photos/new.txt".into())]
    );
    assert_eq!(
        plan.spellings
            .side_rel("Photos/Cafe\u{301}.txt", PairSide::B),
        "photos/Café.txt"
    );
    assert_eq!(
        plan.spellings.side_rel("Photos/new.txt", PairSide::B),
        "photos/new.txt"
    );
    assert!(plan.dirs.is_empty());
}

#[test]
fn review_task_collisions_and_omissions_never_forget_prior_spellings() {
    let signature = sig(3, 100);
    let base = Baseline::from([
        ("Makefile".into(), (Some(signature), Some(signature))),
        ("makefile".into(), (Some(signature), Some(signature))),
        ("link/old".into(), (Some(signature), Some(signature))),
    ]);
    let mut a = side(&[
        ("Makefile", signature),
        ("makefile", signature),
        ("ok", signature),
    ]);
    let mut b = side(&[("Makefile", signature), ("link/old", signature)]);
    a.omissions.record_kind("link", OmissionKind::Link, true);
    let mut ctx = PlanContext::new(mirror());
    ctx.keys = KeyPolicy { fold_case: true };
    let plan = plan_pair(&mut a, &mut b, &base, &ctx);
    assert_eq!(plan.actions, vec![Action::CopyAtoB("ok".into())]);
    assert!(plan.records.is_empty() && plan.forget.is_empty());
    assert!(plan.omissions.protects("MAKEFILE"));
    assert!(plan.omissions.protects("link/old"));
}

#[test]
fn review_task_directory_alias_collision_protects_descendants() {
    let signature = sig(3, 100);
    let mut a = side(&[
        ("Dir/a", signature),
        ("dir/b", signature),
        ("ok", signature),
    ]);
    a.dirs.extend(["Dir".into(), "dir".into()]);
    let mut ctx = PlanContext::new(mirror());
    ctx.keys = KeyPolicy { fold_case: true };
    let plan = plan_pair(&mut a, &mut side(&[]), &Baseline::new(), &ctx);
    assert_eq!(plan.actions, vec![Action::CopyAtoB("ok".into())]);
    assert!(plan.dirs.is_empty());
    assert!(plan.omissions.protects("Dir/a") && plan.omissions.protects("dir/b"));
}

#[test]
fn review_task_filter_transitions_protect_source_and_update_included_destination() {
    let old = sig(5, 100);
    let edited = sig(8, 200);
    let base = Baseline::from([("x".into(), (Some(old), Some(old)))]);
    let mut filtered_source = side(&[]);
    filtered_source.filtered.insert("x".into(), edited);
    let protected = plan_pair(
        &mut filtered_source,
        &mut side(&[("x", old)]),
        &base,
        &PlanContext::new(mirror()),
    );
    assert!(
        protected.actions.is_empty() && protected.records.is_empty() && protected.forget.is_empty()
    );
    assert!(protected.omissions.protects("x"));
    assert!(!filtered_source.is_empty());
    let mut filtered_target = side(&[]);
    filtered_target.filtered.insert("x".into(), old);
    let update = plan_pair(
        &mut side(&[("x", edited)]),
        &mut filtered_target,
        &base,
        &PlanContext::new(mirror()),
    );
    assert_eq!(update.actions, vec![Action::CopyAtoB("x".into())]);
    assert!(filtered_target.tree.contains_key("x"));
}

#[test]
fn review_task_target_limits_omit_only_the_impossible_files() {
    let mut ctx = PlanContext::new(mirror());
    ctx.limits_b = TargetLimits {
        windows_names: true,
        max_file_size: Some(10),
        ..Default::default()
    };
    let mut a = side(&[
        ("bad?.txt", sig(3, 100)),
        ("large", sig(11, 100)),
        ("ok", sig(3, 100)),
    ]);
    let plan = plan_pair(&mut a, &mut side(&[]), &Baseline::new(), &ctx);
    assert_eq!(plan.actions, vec![Action::CopyAtoB("ok".into())]);
    assert_eq!(
        plan.omissions
            .counts()
            .get(&OmissionKind::NameImpossibleOnTarget),
        Some(&1)
    );
    assert_eq!(
        plan.omissions
            .counts()
            .get(&OmissionKind::TooLargeForTarget),
        Some(&1)
    );
}

#[test]
fn review_task_directory_history_distinguishes_union_from_propagated_removal() {
    let mut b = side(&[]);
    b.dirs.insert("empty".into());
    let union = plan_pair(
        &mut side(&[]),
        &mut b.clone(),
        &Baseline::new(),
        &PlanContext::new(BisyncOptions::default()),
    );
    assert_eq!(
        union.dirs,
        vec![DirAction::Create {
            side: PairSide::A,
            rel: "empty".into()
        }]
    );
    let history = DirSet::from(["empty".into()]);
    let mut ctx = PlanContext::new(BisyncOptions::default());
    ctx.base_dirs = Some(&history);
    let remove = plan_pair(&mut side(&[]), &mut b, &Baseline::new(), &ctx);
    assert_eq!(
        remove.dirs,
        vec![DirAction::Remove {
            side: PairSide::B,
            rel: "empty".into()
        }]
    );
}

#[test]
fn review_task_percentage_stop_is_per_side_inclusive_and_move_free() {
    let opts = BisyncOptions {
        max_delete_pct: 50,
        max_delete_min: 25,
        ..Default::default()
    };
    let deletes = super::guards::DeleteCounts { a: 25, b: 0 };
    assert_eq!(
        super::guards::deletion_block(deletes, 50, 1_000, &opts),
        Some(RunBlock::MassDelete {
            side: PairSide::A,
            deletes: 25,
            files: 50
        })
    );
    assert!(super::guards::deletion_block(
        super::guards::DeleteCounts { a: 24, b: 0 },
        24,
        0,
        &opts
    )
    .is_none());
    let actions = [
        Action::FinalizeMoveAtoB("move".into()),
        Action::CopyAtoB("copy".into()),
    ];
    assert_eq!(super::guards::DeleteCounts::of(&actions).total(), 0);
}

#[test]
fn review_task_confirmed_absolute_limit_does_not_confirm_the_percentage_stop() {
    let opts = BisyncOptions {
        max_delete: 10,
        max_delete_pct: 50,
        max_delete_min: 25,
        ..Default::default()
    };
    let settings = RunSettings {
        confirmed: vec![BlockConfirmation::Deletes {
            side: None,
            max: 25,
        }],
        ..Default::default()
    };
    let mut plan = super::plan_types::PairPlan::default();
    plan.files_a = 50;
    plan.files_b = 1_000;
    let snapshot = side(&[("exists", sig(1, 1))]);
    assert_eq!(
        super::orchestration_plan::blocked(
            &plan,
            &snapshot,
            &snapshot,
            &Baseline::new(),
            &opts,
            &settings,
            super::guards::DeleteCounts { a: 25, b: 0 }
        ),
        Some(RunBlock::MassDelete {
            side: PairSide::A,
            deletes: 25,
            files: 50
        })
    );
}

#[test]
fn review_task_silent_filtered_folder_is_not_an_empty_volume() {
    let mut snapshot = side(&[]);
    snapshot
        .omissions
        .record_kind("filtered", OmissionKind::Filtered, false);
    assert!(!snapshot.is_empty());
}

#[test]
fn review_task_precision_applies_to_each_baseline_side() {
    let a = sig(3, 1_000);
    let b = sig(3, 60_000);
    let base = Baseline::from([("x".into(), (Some(a), Some(b)))]);
    let mut ctx = PlanContext::new(mirror());
    ctx.times = TimeRules {
        a: MtimePrecision::Nanos,
        b: MtimePrecision::Minutes,
    };
    let plan = plan_pair(
        &mut side(&[("x", a)]),
        &mut side(&[("x", sig(3, 119_000))]),
        &base,
        &ctx,
    );
    assert!(plan.actions.is_empty());
}
