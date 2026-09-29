use super::sync_conflict_task_fixture::{filter, Fixture, FILE, MIME};
use crate::bisync::{self, BisyncOptions, CompareMode, ConflictMode, DeletePolicy};
use crate::vfs::{Backend, CachingBackend};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[test]
fn sync_conflict_task_sync_names_preserve_browsing_literals_and_folder_identity() {
    let f = Fixture::new(Some(b"local"), &[("a-file", b"one"), ("b-file", b"two")]);
    let literal = "literal [drive-id abc]";
    f.drive.insert("literal", literal, "root", MIME, b"literal");
    for id in ["folder-a", "folder-b"] {
        f.drive.insert(id, "Notebook", "root", super::api::FOLDER_MIME, b"");
    }
    let browse = f.remote.list_dir("/").unwrap();
    assert!(browse.iter().any(|m| m.name == format!("{FILE} [drive-id b-file]")));
    let sync = f.remote.list_dir_for_sync("/").unwrap();
    assert_eq!(sync.iter().filter(|m| m.name == FILE).count(), 2);
    assert_eq!(sync.iter().filter(|m| m.is_dir).map(|m| &m.name).collect::<Vec<_>>(),
        browse.iter().filter(|m| m.is_dir).map(|m| &m.name).collect::<Vec<_>>());
    assert!(sync.iter().any(|m| m.name == super::names::encode(literal)));
    let cached = CachingBackend::new(Arc::new(f.server.backend()));
    assert!(cached.has_duplicate_file_names());
    assert_eq!(cached.list_dir_for_sync("/").unwrap().iter().filter(|m| m.name == FILE).count(), 2);
    assert_eq!(f.mutation_count(), 0);
}

#[test]
fn sync_conflict_task_unique_common_content_converges_without_alias_copies() {
    let f = Fixture::new(Some(b"winner"), &[("a-old", b"loser!"), ("b-good", b"winner"), ("c-good", b"winner")]);
    let preview = f.preview();
    assert!(preview.error.is_none(), "{:?}", preview.error);
    assert!(preview.actions.is_empty());
    assert!(preview.conflicts.is_empty());
    assert_eq!(preview.duplicate_removals, 2);
    assert_eq!(f.mutation_count(), 0);
    let opts = BisyncOptions { compare: CompareMode::SizeOnly, ..Default::default() };
    let dry = f.run(BisyncOptions { dry_run: true, ..opts });
    assert!(dry.errors.is_empty(), "{:?}", dry.errors);
    assert_eq!(f.mutation_count(), 0);
    let out = f.run(opts);
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert!(out.conflicts.is_empty());
    assert_eq!(out.stats.deleted, 2);
    assert_eq!(f.remote_content(), b"winner");
    assert_eq!(f.drive.named("root", FILE)[0]["id"], "b-good");
    assert_eq!(std::fs::read_dir(f.directory.path()).unwrap().count(), 1);
    f.assert_backed_up(b"loser!");
    let again = f.preview();
    assert!(again.actions.is_empty() && again.conflicts.is_empty());
    assert_eq!(again.duplicate_removals, 0);
}

#[test]
fn sync_conflict_task_a_replaces_exact_destination_then_removes_other_objects() {
    let f = Fixture::new(Some(b"chosen A"), &[("a-remote", b"B first"), ("b-remote", b"B second")]);
    let conflict = f.conflict();
    let group = conflict.duplicates.as_ref().unwrap();
    assert!(!group.needs_variant_choice(true));
    assert!(group.needs_variant_choice(false));
    let result = f.resolve(&conflict, true, None, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(result.0.is_some() && result.1.is_some());
    assert_eq!(f.local_content(), b"chosen A");
    assert_eq!(f.remote_content(), b"chosen A");
    assert_eq!(f.drive.named("root", FILE)[0]["id"], "a-remote");
    f.assert_backed_up(b"B first");
    f.assert_backed_up(b"B second");
    let requests = f.server.requests();
    assert_eq!(requests.iter().filter(|r| r.method == "PATCH" && r.path() == "/upload/drive/v3/files/a-remote").count(), 1);
    assert!(f.drive.object("b-remote").unwrap()["trashed"].as_bool().unwrap());
    assert!(f.preview().actions.is_empty());
}

#[test]
fn sync_conflict_task_b_requires_a_variant_and_preserves_the_selected_id() {
    let f = Fixture::new(Some(b"A"), &[("a-remote", b"B first"), ("b-remote", b"B chosen")]);
    let conflict = f.conflict();
    let error = f.resolve(&conflict, false, None, &AtomicBool::new(false), |_| {}).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("konkrete Version"));
    assert_eq!(f.mutation_count(), 0);
    f.resolve(&conflict, false, Some("b-remote"), &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(f.remote_content(), b"B chosen");
    assert_eq!(f.local_content(), b"B chosen");
    assert_eq!(f.drive.named("root", FILE)[0]["id"], "b-remote");
    f.assert_backed_up(b"A");
    f.assert_backed_up(b"B first");
}

#[test]
fn sync_conflict_task_multiple_common_versions_and_remote_pairs_need_selection() {
    let f = Fixture::new(None, &[]);
    for (id, name) in [("left", "Left"), ("right", "Right")] {
        f.drive.insert(id, name, "root", super::api::FOLDER_MIME, b"");
    }
    for (id, parent, bytes) in [("left-a", "left", b"red"), ("left-b", "left", b"tan"),
        ("right-a", "right", b"red"), ("right-b", "right", b"tan")] {
        f.drive.insert(id, FILE, parent, MIME, bytes);
    }
    let opts = BisyncOptions { conflict: ConflictMode::SourceWins, ..Default::default() };
    let preview = bisync::preview(&f.remote, "/Left", &f.remote, "/Right", opts,
        &AtomicBool::new(false), &filter(&bisync::empty_globset()));
    assert!(preview.error.is_none(), "{:?}", preview.error);
    assert!(preview.actions.is_empty());
    assert_eq!(preview.conflicts.len(), 1);
    assert_eq!(preview.duplicate_removals, 0);
    bisync::resolve_variant_checked(&f.remote, "/Left", &f.remote, "/Right", &preview.conflicts[0],
        true, Some("left-b"), &f.pair, &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(f.drive.named("left", FILE).len(), 1);
    assert_eq!(f.drive.named("right", FILE).len(), 1);
    assert_eq!(f.drive.named("left", FILE)[0]["id"], "left-b");
    assert_eq!(f.drive.named("right", FILE)[0]["id"], "right-b");
}

#[test]
fn sync_conflict_task_delete_policy_guard_and_filters_protect_variants() {
    let f = Fixture::new(Some(b"common"), &[("a", b"common"), ("b", b"old"), ("c", b"older")]);
    let out = f.run(BisyncOptions { max_delete: 1, ..Default::default() });
    assert!(out.errors.iter().any(|(_, e)| e.contains("Sicherheitsstopp")));
    assert_eq!(f.mutation_count(), 0);
    let out = f.run(BisyncOptions { delete: DeletePolicy::NoDelete, ..Default::default() });
    assert_eq!(out.conflicts.len(), 1);
    assert_eq!(f.mutation_count(), 0);
    let ignore = bisync::empty_globset();
    let limited = bisync::WalkFilter { min_size: 4, ..filter(&ignore) };
    let preview = f.preview_with(BisyncOptions::default(), &limited);
    assert!(preview.error.is_none(), "{:?}", preview.error);
    assert!(preview.actions.is_empty() && preview.conflicts.is_empty());
    assert_eq!(preview.duplicate_removals, 0);
    assert!(!preview.omissions.is_empty());
    assert_eq!(f.drive.named("root", FILE).len(), 3);
}
