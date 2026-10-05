use super::sync_reliability_task_fixture::{
    assert_complete_at, assert_persisted, contains_bytes, legacy_folded_state, run_backends,
    state_baseline, DriveFixture,
};
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};

#[test]
fn sync_reliability_task_names_legacy_folded_same_spelling_reopens_without_state_loss() {
    legacy_case_roundtrip("OldTree/Note.md");
}

#[test]
fn sync_reliability_task_names_legacy_folded_different_spellings_keep_exact_counterpart() {
    legacy_case_roundtrip("oldtree/note.md");
}

#[test]
fn sync_reliability_task_names_legacy_alias_target_collision_preserves_pair_until_explicit_rename() {
    let left = DriveFixture::new("Job");
    let right = DriveFixture::new("Job");
    let rel_a = "Notebook/note.md";
    let rel_b = "notebook/note.md";
    let id_a = left.write_file(rel_a, b"old alias pair");
    let id_b = right.write_file(rel_b, b"old alias pair");
    left.write_file("healthy.txt", b"healthy before");
    let mut seed = run_backends(
        &left.backend,
        &left.root,
        &right.backend,
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&seed, "alias collision actual seed Both");
    let folder = vfs::sync_child_path(&right.backend, &right.root, "Notebook").unwrap();
    right.backend.remove_dir(&folder).unwrap();
    let folder = vfs::sync_child_path(&left.backend, &left.root, "notebook").unwrap();
    left.backend.remove_dir(&folder).unwrap();
    seed.baseline = legacy_folded_state(&seed, rel_a, rel_b);
    left.restore_pre_registry_state();
    right.restore_pre_registry_state();
    reopened_noop(&left, &right, &seed, "alias collision .171 reopened no-op");
    let healthy = right.drive.named(&right.root_object_id(), "healthy.txt");
    assert_eq!(healthy.len(), 1);
    let healthy_id = healthy[0]["id"].as_str().unwrap().to_string();
    let new_id = left.write_file(rel_b, b"new independent lower tree");
    let folders = left.drive.named(&left.root_object_id(), "notebook");
    assert_eq!(folders.len(), 1);
    let new_folder = folders[0]["id"].as_str().unwrap().to_string();
    assert_eq!(left.aliases(&left.root)[&new_folder], "notebook");
    let old_mutations = (left.mutations_to(&id_a), right.mutations_to(&id_b));
    left.write_file("healthy.txt", b"healthy after");
    let a = left.restart();
    let b = right.restart();
    let partial = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_alias_protected(&partial, &seed, rel_a, rel_b);
    assert_eq!(partial.stats.a_to_b, 1);
    assert_eq!(partial.stats.b_to_a + partial.stats.deleted, 0);
    assert_eq!(left.drive.bytes(&id_a), b"old alias pair");
    assert_eq!(right.drive.bytes(&id_b), b"old alias pair");
    assert_eq!(left.drive.bytes(&new_id), b"new independent lower tree");
    assert_eq!(
        (left.mutations_to(&id_a), right.mutations_to(&id_b)),
        old_mutations
    );
    assert_eq!(right.drive.bytes(&healthy_id), b"healthy after");
    assert_eq!(right.read_file(rel_b), b"old alias pair");
    assert!(right.drive.named(&right.root_object_id(), "Notebook").is_empty());
    assert!(contains_bytes(
        &bisync::versions_dir(&partial.state.as_ref().unwrap().pair_id),
        b"healthy before"
    ));
    let aliases = (left.aliases(&left.root), right.aliases(&right.root));
    let mutations = left.mutations() + right.mutations();
    let a = left.restart();
    let b = right.restart();
    let reopened = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_alias_protected(&reopened, &seed, rel_a, rel_b);
    assert_eq!(reopened.baseline, partial.baseline);
    assert_eq!(
        reopened.stats.a_to_b + reopened.stats.b_to_a + reopened.stats.deleted,
        0
    );
    assert_eq!(reopened.stats.bytes, 0);
    assert_eq!(left.mutations() + right.mutations(), mutations);
    assert_eq!(
        (left.aliases(&left.root), right.aliases(&right.root)),
        aliases
    );
    assert_eq!(left.drive.bytes(&id_a), b"old alias pair");
    assert_eq!(right.drive.bytes(&id_b), b"old alias pair");
    assert_eq!(left.drive.bytes(&new_id), b"new independent lower tree");
    assert_eq!(
        (left.mutations_to(&id_a), right.mutations_to(&id_b)),
        old_mutations
    );

    // An explicit ordinary rename chooses a new independent target address.
    // The job never guesses another counterpart for the colliding old title.
    let source = vfs::sync_child_path(a.as_ref(), &left.root, "notebook").unwrap();
    let target = vfs::sync_child_path(a.as_ref(), &left.root, "Independent").unwrap();
    assert_eq!(
        vfs::sync_stat(a.as_ref(), &source).unwrap().id.as_deref(),
        Some(new_folder.as_str())
    );
    a.rename(&source, &target).unwrap();
    assert_eq!(left.drive.object(&new_folder).unwrap()["name"], "Independent");
    assert_eq!(left.drive.bytes(&new_id), b"new independent lower tree");
    let a = left.restart();
    let b = right.restart();
    let recovered = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&recovered, "alias collision explicit rename recovery");
    assert_eq!(recovered.state, seed.state);
    assert_eq!(recovered.baseline.get(rel_a), seed.baseline.get(rel_a));
    assert!(!recovered.baseline.contains_key(rel_b));
    assert!(recovered.baseline.contains_key("Independent/note.md"));
    assert_eq!(recovered.stats.a_to_b, 1);
    assert_eq!(recovered.stats.b_to_a + recovered.stats.deleted, 0);
    assert_eq!(left.drive.bytes(&id_a), b"old alias pair");
    assert_eq!(right.drive.bytes(&id_b), b"old alias pair");
    assert_eq!(
        (left.mutations_to(&id_a), right.mutations_to(&id_b)),
        old_mutations
    );
    assert_eq!(
        left.read_file("Independent/note.md"),
        b"new independent lower tree"
    );
    assert_eq!(
        right.read_file("Independent/note.md"),
        b"new independent lower tree"
    );
    assert_eq!(right.drive.bytes(&healthy_id), b"healthy after");
    assert!(right.drive.named(&right.root_object_id(), "Notebook").is_empty());
    assert_persisted(&recovered);
    reopened_noop(
        &left,
        &right,
        &recovered,
        "alias collision recovered reopened no-op",
    );
}

fn assert_alias_protected(
    out: &bisync::Outcome,
    historical: &bisync::Outcome,
    rel_a: &str,
    rel_b: &str,
) {
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert!(out.conflicts.is_empty() && out.deferred.is_empty());
    assert!(out.blocked.is_none() && out.stopped.is_none() && !out.busy && !out.canceled);
    assert!(out.omissions.protects(rel_b), "{:?}", out.omissions);
    assert_eq!(out.state, historical.state);
    assert_eq!(out.baseline.get(rel_a), historical.baseline.get(rel_a));
    assert!(!out.baseline.contains_key(rel_b));
    assert_persisted(out);
}

fn legacy_case_roundtrip(rel_b: &str) {
    let left = DriveFixture::new("Job");
    let right = DriveFixture::new("Job");
    let rel_a = "OldTree/Note.md";
    let id_a = left.write_file(rel_a, b"historical paired bytes");
    let id_b = right.write_file(rel_b, b"historical paired bytes");
    let seed = run_backends(
        &left.backend,
        &left.root,
        &right.backend,
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&seed, "legacy actual seed Both");
    if rel_a != rel_b {
        let folder = vfs::sync_child_path(&right.backend, &right.root, "OldTree").unwrap();
        right.backend.remove_dir(&folder).unwrap();
        let folder = vfs::sync_child_path(&left.backend, &left.root, "oldtree").unwrap();
        left.backend.remove_dir(&folder).unwrap();
    }
    let historical = legacy_folded_state(&seed, rel_a, rel_b);
    left.restore_pre_registry_state();
    right.restore_pre_registry_state();
    let mutations = left.mutations() + right.mutations();
    let a = left.restart();
    let b = right.restart();
    let reopened = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&reopened, "legacy first reopened Both no-op");
    assert_eq!(reopened.state, seed.state);
    assert_eq!(reopened.baseline, historical);
    assert_eq!(state_baseline(seed.state.as_ref().unwrap()), historical);
    assert_eq!(reopened.stats.bytes, 0);
    assert_eq!(
        reopened.stats.a_to_b + reopened.stats.b_to_a + reopened.stats.deleted,
        0
    );
    assert_eq!(left.mutations() + right.mutations(), mutations);
    assert_eq!(right.write_file(rel_b, b"historical counterchange"), id_b);
    left.write_file("Notebook/note.md", b"new uppercase tree");
    left.write_file("notebook/note.md", b"new lowercase tree");
    let changed = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&changed, "legacy counterpart and independent case trees");
    assert_eq!(changed.state, seed.state);
    assert_eq!(left.drive.bytes(&id_a), b"historical counterchange");
    assert_eq!(right.drive.bytes(&id_b), b"historical counterchange");
    assert_eq!(left.drive.object(&id_a).unwrap()["name"], "Note.md");
    assert_eq!(
        right.drive.object(&id_b).unwrap()["name"],
        rel_b.rsplit_once('/').unwrap().1
    );
    assert_eq!(changed.stats.a_to_b, 2);
    assert_eq!(changed.stats.b_to_a, 1);
    assert_eq!(changed.stats.deleted, 0);
    assert!(changed.baseline[rel_a].0.is_some() && changed.baseline[rel_a].1.is_some());
    assert!(rel_a == rel_b || !changed.baseline.contains_key(rel_b));
    assert!(contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        b"historical paired bytes"
    ));
    for (rel, bytes) in [
        ("Notebook/note.md", b"new uppercase tree".as_slice()),
        ("notebook/note.md", b"new lowercase tree".as_slice()),
    ] {
        assert_eq!(left.read_file(rel), bytes);
        assert_eq!(right.read_file(rel), bytes);
        assert!(changed.baseline.contains_key(rel));
    }
    assert_persisted(&changed);
    let mutations = left.mutations() + right.mutations();
    let a = left.restart();
    let b = right.restart();
    let noop = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&noop, "legacy changed reopened Both no-op");
    assert_eq!(noop.state, changed.state);
    assert_eq!(noop.baseline, changed.baseline);
    assert_eq!(noop.stats.bytes, 0);
    assert_eq!(
        noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
        0
    );
    assert_eq!(left.mutations() + right.mutations(), mutations);
    assert_persisted(&noop);
    historical_side_b_restore(&left, &right, rel_a, rel_b, &id_a, &id_b);
}

fn historical_side_b_restore(
    left: &DriveFixture,
    right: &DriveFixture,
    rel_a: &str,
    rel_b: &str,
    id_a: &str,
    id_b: &str,
) {
    use crate::bisync::versions::{
        list_versions, restore_version, VersionReason, VersionSide, VersionStore,
    };
    use crate::bisync::{PairLock, PairSide};
    use std::sync::atomic::AtomicBool;

    assert_eq!(left.write_file(rel_a, b"historical overwrite"), id_a);
    let a = left.restart();
    let b = right.restart();
    let overwritten = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::AtoB,
    );
    assert_complete_at(&overwritten, "legacy overwrite actual B counterpart");
    assert_eq!(overwritten.stats.a_to_b, 1);
    assert_eq!(overwritten.stats.b_to_a + overwritten.stats.deleted, 0);
    assert_eq!(right.drive.bytes(id_b), b"historical overwrite");
    assert_persisted(&overwritten);
    reopened_noop(left, right, &overwritten, "legacy overwrite reopened no-op");
    let a = left.restart();
    let b = right.restart();
    let state = overwritten.state.as_ref().unwrap();
    let recorded = bisync::recorded_original_paths_for_key(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        state,
        rel_a,
    )
    .unwrap();
    assert_eq!(recorded.rel_a, rel_a);
    assert_eq!(recorded.rel_b, rel_b);
    assert_eq!(
        vfs::sync_stat(a.as_ref(), &recorded.path_a)
            .unwrap()
            .id
            .as_deref(),
        Some(id_a)
    );
    assert_eq!(
        vfs::sync_stat(b.as_ref(), &recorded.path_b)
            .unwrap()
            .id
            .as_deref(),
        Some(id_b)
    );
    let cancel = AtomicBool::new(false);
    let sides = [
        VersionSide {
            side: PairSide::A,
            backend: a.as_ref(),
            root: &left.root,
        },
        VersionSide {
            side: PairSide::B,
            backend: b.as_ref(),
            root: &right.root,
        },
    ];
    let versions = list_versions(&state.pair_id, &sides, &cancel).unwrap();
    let backups: Vec<_> = versions
        .iter()
        .filter(|entry| {
            entry.rel == rel_b
                && entry.side == Some(PairSide::B)
                && entry.reason == Some(VersionReason::Replaced)
        })
        .collect();
    assert_eq!(backups.len(), 1, "one backup at the actual B literal path");
    let backup = backups[0];
    assert_eq!(backup.store, VersionStore::AppData);
    assert_eq!(
        std::fs::read(&backup.stored_path).unwrap(),
        b"historical counterchange"
    );
    {
        let lock = PairLock::acquire(&state.lock_id).unwrap();
        restore_version(&lock, &state.pair_id, backup, &sides[1], &cancel).unwrap();
    }
    assert_eq!(right.drive.bytes(id_b), b"historical counterchange");
    assert_eq!(left.drive.bytes(id_a), b"historical overwrite");
    if rel_a != rel_b {
        assert!(right.drive.named(&right.root_object_id(), "OldTree").is_empty());
    }
    assert_persisted(&overwritten);
    let versions = list_versions(&state.pair_id, &sides, &cancel).unwrap();
    assert!(versions.iter().any(|entry| {
        entry.rel == rel_b
            && entry.side == Some(PairSide::B)
            && entry.reason == Some(VersionReason::Restored)
            && std::fs::read(&entry.stored_path).unwrap() == b"historical overwrite"
    }));
    let restored = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&restored, "legacy restored B counterpart convergence");
    assert_eq!(restored.state, overwritten.state);
    assert_eq!(restored.stats.b_to_a, 1);
    assert_eq!(restored.stats.a_to_b + restored.stats.deleted, 0);
    assert_eq!(left.drive.bytes(id_a), b"historical counterchange");
    assert_eq!(right.drive.bytes(id_b), b"historical counterchange");
    assert_persisted(&restored);
    reopened_noop(left, right, &restored, "legacy restore reopened no-op");
}

fn reopened_noop(
    left: &DriveFixture,
    right: &DriveFixture,
    expected: &bisync::Outcome,
    phase: &str,
) {
    let mutations = left.mutations() + right.mutations();
    let a = left.restart();
    let b = right.restart();
    let out = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&out, phase);
    assert_eq!(out.state, expected.state);
    assert_eq!(out.baseline, expected.baseline);
    assert_eq!(out.stats.a_to_b + out.stats.b_to_a + out.stats.deleted, 0);
    assert_eq!(out.stats.bytes, 0);
    assert_eq!(left.mutations() + right.mutations(), mutations);
    assert_persisted(&out);
}
