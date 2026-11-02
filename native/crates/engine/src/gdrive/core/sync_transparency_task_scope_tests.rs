//! Drive feed scope: only changes that can affect the sync root start a run.
use super::*;
use crate::vfs::{ChangeKind, VfsChange, VfsMeta};
use std::collections::HashMap as Map;

fn upsert(id: &str, parent: &str, is_dir: bool) -> VfsChange {
    VfsChange {
        kind: ChangeKind::Upsert,
        rel: None,
        id: Some(id.into()),
        parent_id: Some(parent.into()),
        name: Some(format!("{id}.name")),
        meta: Some(VfsMeta {
            name: format!("{id}.name"),
            is_dir,
            ..Default::default()
        }),
    }
}

fn removed(id: &str) -> VfsChange {
    VfsChange {
        kind: ChangeKind::Remove,
        rel: None,
        id: Some(id.into()),
        parent_id: None,
        name: None,
        meta: None,
    }
}

/// drive-root ─┬─ vault (root) ─ sub
///             └─ other ─ deep
fn tree() -> Map<&'static str, Option<&'static str>> {
    Map::from([
        ("vault", Some("drive-root")),
        ("sub", Some("vault")),
        ("other", Some("drive-root")),
        ("deep", Some("other")),
        ("drive-root", None),
    ])
}

fn lookup_in<'a>(
    tree: &'a Map<&'static str, Option<&'static str>>,
) -> impl FnMut(&str) -> io::Result<Option<String>> + 'a {
    move |id: &str| {
        tree.get(id)
            .map(|parent| parent.map(str::to_string))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown"))
    }
}

#[test]
fn sync_transparency_task_drive_feed_ignores_changes_outside_the_root() {
    let mut scope = ChangeScope::for_test("Vaults/vault", "vault");
    let known = HashSet::from(["vault".to_string(), "sub".to_string()]);
    let tree = tree();
    let mut lookup = lookup_in(&tree);
    assert!(!scope.relevant_with(&[upsert("x", "deep", false)], &known, &mut lookup));
    // The learned ancestry answers the next change below the same folder.
    let mut failing =
        |_: &str| -> io::Result<Option<String>> { panic!("cached ancestry must answer") };
    assert!(!scope.relevant_with(&[upsert("y", "deep", false)], &known, &mut failing));
}

#[test]
fn sync_transparency_task_drive_feed_reports_changes_below_the_root() {
    let mut scope = ChangeScope::for_test("Vaults/vault", "vault");
    let known = HashSet::from(["vault".to_string(), "sub".to_string(), "file".to_string()]);
    let mut never =
        |_: &str| -> io::Result<Option<String>> { panic!("known parent needs no lookup") };
    // New file in a known folder, edit of a known file, the root itself.
    assert!(scope.relevant_with(&[upsert("new", "sub", false)], &known, &mut never));
    assert!(scope.relevant_with(&[upsert("file", "sub", false)], &known, &mut never));
    assert!(scope.relevant_with(&[upsert("vault", "drive-root", true)], &known, &mut never));
    // An object that arrived below the root is known for its later removal.
    assert!(scope.relevant_with(&[removed("new")], &HashSet::new(), &mut never));
}

#[test]
fn sync_transparency_task_drive_feed_reports_moves_out_and_removals_of_known_objects() {
    let mut scope = ChangeScope::for_test("Vaults/vault", "vault");
    let known = HashSet::from(["vault".to_string(), "file".to_string()]);
    let tree = tree();
    let mut lookup = lookup_in(&tree);
    // Moved out: new parent outside, but the object was below the root.
    assert!(scope.relevant_with(&[upsert("file", "other", false)], &known, &mut lookup));
    // Removed (no metadata): known object counts, unknown one does not.
    assert!(scope.relevant_with(&[removed("file")], &known, &mut lookup));
    assert!(!scope.relevant_with(&[removed("stranger")], &known, &mut lookup));
}

#[test]
fn sync_transparency_task_drive_feed_counts_unresolvable_parents_as_relevant() {
    let mut scope = ChangeScope::for_test("Vaults/vault", "vault");
    let known = HashSet::from(["vault".to_string()]);
    let mut failing = |_: &str| -> io::Result<Option<String>> { Err(io::Error::other("network")) };
    assert!(scope.relevant_with(&[upsert("x", "unknown", false)], &known, &mut failing));
}

#[test]
fn sync_transparency_task_drive_feed_relearns_ancestry_after_a_folder_move() {
    let mut scope = ChangeScope::for_test("Vaults/vault", "vault");
    let known = HashSet::from(["vault".to_string()]);
    let tree = tree();
    let mut lookup = lookup_in(&tree);
    assert!(!scope.relevant_with(&[upsert("x", "deep", false)], &known, &mut lookup));
    // "other" moves below the root: its folder change clears the learned
    // ancestry, so the next file under "deep" is looked up again.
    let moved = Map::from([("deep", Some("other")), ("other", Some("vault"))]);
    let mut relearn = lookup_in(&moved);
    assert!(scope.relevant_with(
        &[upsert("other", "vault", true), upsert("z", "deep", false)],
        &known,
        &mut relearn
    ));
}
