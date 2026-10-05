use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::{filter, MIME};
use super::sync_reliability_task_fixture::{assert_complete, options, DriveFixture};
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};
use serde_json::json;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

fn remote_run(left: &DriveFixture, right: &DriveFixture, direction: Direction) -> bisync::Outcome {
    run_backends(
        &left.backend,
        &left.root,
        &right.backend,
        &right.root,
        direction,
    )
}

fn run_backends(
    left: &dyn Backend,
    left_root: &str,
    right: &dyn Backend,
    right_root: &str,
    direction: Direction,
) -> bisync::Outcome {
    bisync::run(
        left,
        left_root,
        right,
        right_root,
        options(direction),
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    )
}

fn assert_complete_at(out: &bisync::Outcome, phase: &str) {
    eprintln!("C02 Drive roundtrip phase={phase}");
    assert_complete(out);
}

#[test]
fn sync_reliability_task_names_drive_roundtrip_keeps_trees_literals_and_marker_origins() {
    let left = DriveFixture::new("Job");
    let right = DriveFixture::new("Job");
    assert_ne!(
        left.backend.state_identity(),
        right.backend.state_identity()
    );
    let left_previous = vfs::previous_state_identities(&left.backend).unwrap();
    let right_previous = vfs::previous_state_identities(&right.backend).unwrap();
    assert_eq!(left_previous.len(), 1);
    assert_eq!(right_previous.len(), 1);
    assert_ne!(left_previous, right_previous);
    for (fixture, previous) in [(&left, &left_previous), (&right, &right_previous)] {
        assert_eq!(
            vfs::previous_state_identities(&fixture.fresh_backend(&fixture.root)).unwrap(),
            *previous
        );
    }
    let parent = left.root_object_id();
    for (id, title, bytes) in [
        ("sameprefixAA", "Notebook", b"first tree".as_slice()),
        ("sameprefixAB", "Notebook", b"second tree".as_slice()),
        ("lower-title", "notebook", b"literal case tree".as_slice()),
        (
            "marker-title",
            "Notebook [drive-id sameprefixAB]",
            b"literal marker tree".as_slice(),
        ),
        ("case-canonical", "CaseTree", b"case canonical".as_slice()),
        ("CASEID01", "CaseTree", b"uppercase ID".as_slice()),
        ("caseid01", "CaseTree", b"lowercase ID".as_slice()),
        ("mixed-folder", "Mixed", b"mixed folder".as_slice()),
    ] {
        left.drive.insert(id, title, &parent, FOLDER_MIME, b"");
        left.drive
            .insert(&format!("{id}-file"), "note.md", id, MIME, bytes);
    }
    left.drive.change(
        "case-canonical",
        json!({"modifiedTime":"2030-01-01T00:00:00Z"}),
    );
    for (index, title) in [
        "%3A",
        "literal [drive-id samepref]",
        "Ünicode😀",
        " leading",
        "tail ",
        "colon:name",
        "back\\slash",
    ]
    .iter()
    .enumerate()
    {
        left.drive.insert(
            &format!("literal-{index}"),
            title,
            &parent,
            MIME,
            title.as_bytes(),
        );
    }
    left.drive
        .insert("mixed-file", "Mixed", &parent, MIME, b"mixed file");
    left.drive
        .insert("same-a", "same.txt", &parent, MIME, b"common content");
    left.drive
        .insert("same-b", "same.txt", &parent, MIME, b"common content");
    right.write_file("same.txt", b"common content");
    let aliases = left.aliases(&left.root);
    assert_eq!(aliases.len(), 8);
    assert_eq!(aliases["sameprefixAA"], "Notebook");
    assert_ne!(aliases["sameprefixAB"], "Notebook [drive-id sameprefixAB]");
    assert_eq!(aliases["lower-title"], "notebook");
    assert_eq!(aliases["marker-title"], "Notebook [drive-id sameprefixAB]");
    let case_keys: HashSet<_> = ["case-canonical", "CASEID01", "caseid01"]
        .into_iter()
        .map(|id| aliases[id].to_uppercase())
        .collect();
    assert_eq!(
        case_keys.len(),
        3,
        "new markers avoid portable case collisions"
    );
    assert!(aliases["caseid01"].contains("6361736569643031"));
    assert_ne!(aliases["mixed-folder"], "Mixed");

    let seed = remote_run(&left, &right, Direction::AtoB);
    assert_complete_at(&seed, "seed AtoB");
    for (id, alias) in &aliases {
        let rel = format!("{alias}/note.md");
        assert_eq!(
            right.read_file(&rel),
            left.drive.bytes(&format!("{id}-file"))
        );
        assert!(seed.baseline.contains_key(&rel));
        let locator = vfs::sync_child_path(&right.backend, &right.root, alias).unwrap();
        assert_eq!(
            vfs::sync_stat(&right.backend, &locator).unwrap().name,
            *alias
        );
        if alias.contains("[drive-id") {
            assert!(
                locator.contains("%5Bdrive-id"),
                "other account stores the marker text literally"
            );
        }
    }
    for title in [
        "%3A",
        "literal [drive-id samepref]",
        "Ünicode😀",
        " leading",
        "tail ",
        "colon:name",
        "back\\slash",
    ] {
        assert_eq!(right.read_file(title), title.as_bytes());
        assert!(seed.baseline.contains_key(title));
    }
    assert_eq!(right.read_file("Mixed"), b"mixed file");
    assert_eq!(left.drive.named(&parent, "same.txt").len(), 1);
    let (left_posts, right_posts) = (left.folder_posts(), right.folder_posts());
    let remote_aliases = right.aliases(&right.root);
    let changed_rel = format!("{}/note.md", aliases["sameprefixAB"]);
    let changed_id = right.write_file(&changed_rel, b"changed from other account");
    let changed = remote_run(&left, &right, Direction::BtoA);
    assert_complete_at(&changed, "nested counterchange BtoA");
    assert_eq!(
        left.drive.bytes("sameprefixAB-file"),
        b"changed from other account"
    );
    assert_eq!(left.drive.bytes("sameprefixAA-file"), b"first tree");
    assert_eq!(
        right.drive.bytes(&changed_id),
        b"changed from other account"
    );
    assert!(super::sync_reliability_task_fixture::contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        b"second tree"
    ));
    assert_eq!(left.aliases(&left.root), aliases);
    assert_eq!(right.aliases(&right.root), remote_aliases);
    let mutations = left.mutations() + right.mutations();
    let noop = remote_run(&left, &right, Direction::Both);
    assert_complete_at(&noop, "initial Both no-op");
    assert_eq!(noop.stats.bytes, 0);
    assert_eq!(
        noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
        0
    );
    assert_eq!(noop.baseline, changed.baseline);
    assert_eq!(left.mutations() + right.mutations(), mutations);
    assert_eq!(
        (left.folder_posts(), right.folder_posts()),
        (left_posts, right_posts)
    );
    super::sync_reliability_task_fixture::assert_persisted(&noop);
    literal_overwrite_restore(&left, &right);
}

fn literal_overwrite_restore(left: &DriveFixture, right: &DriveFixture) {
    use super::sync_reliability_task_fixture::{assert_persisted, read};
    use crate::bisync::versions::{
        list_versions, restore_version, VersionReason, VersionSide, VersionStore,
    };
    use crate::bisync::{PairLock, PairSide};

    let cases = [
        ("%3A", b"literal percent replacement".as_slice()),
        ("colon:name", b"literal colon replacement".as_slice()),
        ("back\\slash", b"literal slash replacement".as_slice()),
        (
            "literal [drive-id samepref]",
            b"literal marker replacement".as_slice(),
        ),
    ];
    let aliases = (left.aliases(&left.root), right.aliases(&right.root));
    let posts = (left.folder_posts(), right.folder_posts());
    for (rel, bytes) in cases {
        left.write_file(rel, bytes);
    }
    let overwritten = remote_run(left, right, Direction::AtoB);
    assert_complete_at(&overwritten, "literal overwrite AtoB");
    assert_eq!(overwritten.stats.a_to_b, cases.len() as u64);
    assert_eq!(overwritten.stats.b_to_a + overwritten.stats.deleted, 0);
    assert_eq!(
        overwritten.stats.bytes,
        cases
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .sum::<u64>()
    );
    assert_persisted(&overwritten);

    // Open new endpoint handles before loading manifests and the checkpoint.
    let a = left.restart();
    let b = right.restart();
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
    let state = overwritten.state.as_ref().unwrap();
    let versions = list_versions(&state.pair_id, &sides, &cancel).unwrap();
    let originals: Vec<_> = cases
        .iter()
        .map(|(rel, bytes)| {
            for (backend, root) in [(a.as_ref(), &left.root), (b.as_ref(), &right.root)] {
                let path = vfs::sync_path(backend, root, rel).unwrap();
                assert_eq!(vfs::sync_stat(backend, &path).unwrap().name, *rel);
                assert_eq!(read(backend, &path), *bytes);
            }
            let exact = right.drive.named(&right.root_object_id(), rel);
            assert_eq!(exact.len(), 1, "overwrite keeps one literal destination");
            assert_eq!(exact[0]["name"].as_str(), Some(*rel));
            assert!(overwritten.baseline.contains_key(*rel));
            let recorded = bisync::recorded_original_paths_for_key(
                a.as_ref(),
                &left.root,
                b.as_ref(),
                &right.root,
                state,
                rel,
            )
            .unwrap();
            assert_eq!(recorded.rel_a, *rel);
            assert_eq!(recorded.rel_b, *rel);
            assert_eq!(read(a.as_ref(), &recorded.path_a), *bytes);
            assert_eq!(read(b.as_ref(), &recorded.path_b), *bytes);
            let matches: Vec<_> = versions
                .iter()
                .filter(|entry| {
                    entry.rel == *rel
                        && entry.side == Some(PairSide::B)
                        && entry.reason == Some(VersionReason::Replaced)
                })
                .collect();
            assert_eq!(matches.len(), 1, "one exact backup for {rel}");
            let entry = matches[0];
            assert_eq!(entry.store, VersionStore::AppData);
            assert_eq!(std::fs::read(&entry.stored_path).unwrap(), rel.as_bytes());
            (*entry).clone()
        })
        .collect();
    let mutations = left.mutations() + right.mutations();
    let reopened = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&reopened, "literal reopened Both no-op");
    assert_eq!(reopened.state, overwritten.state);
    assert_eq!(reopened.baseline, overwritten.baseline);
    assert_eq!(reopened.stats.bytes, 0);
    assert_eq!(
        reopened.stats.a_to_b + reopened.stats.b_to_a + reopened.stats.deleted,
        0
    );
    assert_eq!(left.mutations() + right.mutations(), mutations);
    assert_persisted(&reopened);
    {
        let lock = PairLock::acquire(&state.lock_id).unwrap();
        for entry in &originals {
            eprintln!("C02 Drive roundtrip phase=literal restore rel={:?}", entry.rel);
            restore_version(&lock, &state.pair_id, entry, &sides[1], &cancel).unwrap();
            let path = vfs::sync_path(b.as_ref(), &right.root, &entry.rel).unwrap();
            assert_eq!(read(b.as_ref(), &path), entry.rel.as_bytes());
        }
    }
    assert_persisted(&overwritten);
    let restored_versions = list_versions(&state.pair_id, &sides, &cancel).unwrap();
    for (rel, bytes) in cases {
        assert!(
            restored_versions.iter().any(|entry| {
                entry.rel == rel
                    && entry.side == Some(PairSide::B)
                    && entry.reason == Some(VersionReason::Restored)
                    && std::fs::read(&entry.stored_path).unwrap() == bytes
            }),
            "restore preserves overwritten bytes of {rel}"
        );
    }
    let restored = run_backends(
        a.as_ref(),
        &left.root,
        b.as_ref(),
        &right.root,
        Direction::Both,
    );
    assert_complete_at(&restored, "literal restored Both convergence");
    assert_eq!(restored.stats.b_to_a, cases.len() as u64);
    assert_eq!(restored.stats.a_to_b + restored.stats.deleted, 0);
    assert_persisted(&restored);
    for (rel, _) in cases {
        assert_eq!(left.read_file(rel), rel.as_bytes());
        assert_eq!(right.read_file(rel), rel.as_bytes());
    }
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
    assert_complete_at(&noop, "literal restored reopened Both no-op");
    assert_eq!(noop.baseline, restored.baseline);
    assert_eq!(noop.stats.bytes, 0);
    assert_eq!(
        noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
        0
    );
    assert_eq!(left.mutations() + right.mutations(), mutations);
    assert_eq!(
        (left.aliases(&left.root), right.aliases(&right.root)),
        aliases
    );
    assert_eq!((left.folder_posts(), right.folder_posts()), posts);
    assert_persisted(&noop);
}
