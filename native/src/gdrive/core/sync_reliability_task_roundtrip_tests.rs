use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::{filter, MIME};
use super::sync_reliability_task_fixture::{assert_complete, options, DriveFixture};
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};
use serde_json::json;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

fn remote_run(left: &DriveFixture, right: &DriveFixture, direction: Direction) -> bisync::Outcome {
    bisync::run(
        &left.backend,
        &left.root,
        &right.backend,
        &right.root,
        options(direction),
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    )
}

#[test]
fn sync_reliability_task_names_drive_roundtrip_keeps_trees_literals_and_marker_origins() {
    let left = DriveFixture::new("Job");
    let right = DriveFixture::new("Job");
    assert_ne!(left.backend.state_identity(), right.backend.state_identity());
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
    assert_complete(&seed);
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
    assert_complete(&changed);
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
    assert_complete(&noop);
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
}
