use super::super::{incremental::SyncEndpoints, orchestration::run_with_store_path, *};
use super::fwd;
use crate::vfs::{Backend, LocalBackend};
use std::sync::atomic::AtomicBool;

const LINK: &str = "Notebook/Schule/Techniker arbeit/Code & Tools/pptx-build/node_modules";

#[test]
fn sync_links_task_nested_link_preserves_counterparts_baseline_and_incremental_recovery() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let store = store_dir.path().join("sync.sqlite");
    let link = a.path().join(LINK);
    let package = format!("{LINK}/package.json");
    std::fs::create_dir_all(&link).unwrap();
    std::fs::write(a.path().join(&package), b"original package").unwrap();
    std::fs::write(a.path().join("note.txt"), b"old note").unwrap();
    std::fs::write(outside.path().join("private.txt"), b"outside root").unwrap();
    let (ra, rb) = (fwd(a.path()), fwd(b.path()));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let endpoints = SyncEndpoints::new(&ba, &ra, &bb, &rb);
    let opts = BisyncOptions { direction: Direction::AtoB, delete: DeletePolicy::Mirror,
        compare: CompareMode::Checksum, ..Default::default() };
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let first = run_with_store_path(endpoints, opts, &cancel, &filter, &store);
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert!(first.omissions.is_empty());
    assert_eq!(std::fs::read(b.path().join(&package)).unwrap(), b"original package");
    let saved = first.baseline[&package];

    std::fs::remove_dir_all(&link).unwrap();
    link_fixture::directory(outside.path(), &link);
    assert!(ba.stat(&fwd(&link)).unwrap().is_symlink);
    assert!(ba.list_dir(&fwd(link.parent().unwrap())).unwrap().iter()
        .any(|entry| entry.name == "node_modules" && entry.is_symlink));
    std::fs::write(a.path().join("note.txt"), b"new independent note").unwrap();
    std::fs::write(b.path().join(&package), b"protected destination edit").unwrap();
    std::fs::write(b.path().join(LINK).join("target-only.txt"), b"must remain").unwrap();
    std::fs::write(b.path().join("extra.txt"), b"ordinary extra").unwrap();
    let preview = super::super::preview(&ba, &ra, &bb, &rb, opts, &cancel, &filter);
    assert!(preview.error.is_none(), "{:?}", preview.error);
    assert_eq!(preview.omissions.reported_paths().collect::<Vec<_>>(), [LINK]);
    assert_eq!(preview.actions, vec![Action::DeleteB("extra.txt".into()),
        Action::CopyAtoB("note.txt".into())]);

    let partial = run_with_store_path(endpoints, opts, &cancel, &filter, &store);
    assert!(partial.errors.is_empty(), "{:?}", partial.errors);
    assert!(partial.conflicts.is_empty());
    assert_eq!(partial.baseline[&package], saved);
    assert_eq!(std::fs::read(b.path().join(&package)).unwrap(), b"protected destination edit");
    assert!(b.path().join(LINK).join("target-only.txt").exists());
    assert!(!b.path().join(LINK).join("private.txt").exists());
    assert_eq!(std::fs::read(b.path().join("note.txt")).unwrap(), b"new independent note");
    assert!(!b.path().join("extra.txt").exists());
    assert!(partial.omissions.result_note("ok").starts_with("mit Auslassungen"));
    let pair = pair_id_for(&ba, &ra, &bb, &rb);
    let index = state_store::SyncStateStore::open_at(&store).unwrap();
    assert!(!index.load_pair(&pair).unwrap().unwrap().bootstrapped);
    assert_eq!(load_baseline(&baseline_path(&pair)).unwrap()[&package], saved);
    drop(index);

    // Two-way mode also preserves the previously synchronized subtree.
    let both = run_with_store_path(endpoints, BisyncOptions { direction: Direction::Both,
        ..opts }, &cancel, &filter, &store);
    assert!(both.errors.is_empty() && both.conflicts.is_empty());
    assert_eq!(both.baseline[&package], saved);

    link_fixture::remove_directory(&link);
    std::fs::create_dir(&link).unwrap();
    std::fs::write(a.path().join(&package), b"recovered real directory").unwrap();
    let recovered = run_with_store_path(endpoints, opts, &cancel, &filter, &store);
    assert!(recovered.errors.is_empty(), "{:?}", recovered.errors);
    assert!(recovered.omissions.is_empty());
    assert_eq!(std::fs::read(b.path().join(&package)).unwrap(), b"recovered real directory");
    assert!(!b.path().join(LINK).join("target-only.txt").exists());
    assert!(state_store::SyncStateStore::open_at(&store).unwrap()
        .load_pair(&pair).unwrap().unwrap().bootstrapped);
    std::fs::remove_file(baseline_path(&pair)).unwrap();
    let _ = std::fs::remove_dir_all(versions_dir(&pair));
}

#[test]
fn sync_links_task_target_link_reverse_mirror_and_exclusions_are_protected() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let (ra, rb) = (fwd(a.path()), fwd(b.path()));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    link_fixture::directory(outside.path(), &a.path().join("node_modules"));
    std::fs::create_dir(b.path().join("node_modules")).unwrap();
    std::fs::write(b.path().join("node_modules/keep.txt"), b"keep").unwrap();
    std::fs::write(b.path().join("normal.txt"), b"copy reverse").unwrap();
    let mut builder = globset::GlobSetBuilder::new();
    builder.add(globset::Glob::new("**/node_modules/**").unwrap());
    let globs = builder.build().unwrap();
    let filter = WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    let opts = BisyncOptions { direction: Direction::BtoA, delete: DeletePolicy::Mirror,
        ..Default::default() };
    let result = run_with_store_path(SyncEndpoints::new(&ba, &ra, &bb, &rb), opts,
        &cancel, &filter, &store.path().join("state.sqlite"));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(!result.omissions.is_empty());
    assert!(result.omissions.summary().is_none(), "explicitly excluded links need no warning");
    assert_eq!(std::fs::read(a.path().join("normal.txt")).unwrap(), b"copy reverse");
    assert!(!outside.path().join("keep.txt").exists());
    assert_eq!(std::fs::read(b.path().join("node_modules/keep.txt")).unwrap(), b"keep");
    let pair = pair_id_for(&ba, &ra, &bb, &rb);
    std::fs::remove_file(baseline_path(&pair)).unwrap();
    link_fixture::remove_directory(&a.path().join("node_modules"));
}

#[test]
fn sync_links_task_protection_keeps_ancestors_case_aliases_and_component_boundaries() {
    let mut omitted = SyncOmissions::new(true);
    omitted.record("Project/node_modules", true);
    for path in ["project", "PROJECT/NODE_MODULES", "Project/node_modules/file"] {
        assert!(omitted.protects(path), "{path}");
    }
    assert!(!omitted.contains("project"), "independent siblings remain reachable");
    assert!(!omitted.protects("Project/node_modules-two/file"));
    assert!(!omitted.protects("Project/normal.txt"));
    let mut exact = SyncOmissions::new(false);
    exact.record("Upper", true);
    assert!(!exact.protects("upper"));
    assert!(omitted.result_note("Fehler").starts_with("Fehler; "));
}

#[test]
fn sync_links_task_incremental_target_junction_returns_to_full_protected_scan() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let store = state.path().join("index.sqlite");
    std::fs::create_dir(a.path().join("folder")).unwrap();
    std::fs::write(a.path().join("folder/entry.txt"), b"old").unwrap();
    let (ra, rb) = (fwd(a.path()), fwd(b.path()));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let endpoints = SyncEndpoints::new(&ba, &ra, &bb, &rb);
    let options = BisyncOptions { direction: Direction::AtoB, delete: DeletePolicy::Mirror,
        compare: CompareMode::Checksum, ..Default::default() };
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let first = run_with_store_path(endpoints, options, &cancel, &filter, &store);
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    std::fs::remove_dir_all(b.path().join("folder")).unwrap();
    std::fs::write(outside.path().join("entry.txt"), b"old").unwrap();
    link_fixture::directory(outside.path(), &b.path().join("folder"));
    std::fs::write(a.path().join("folder/entry.txt"), b"must not cross junction").unwrap();
    std::fs::write(a.path().join("independent.txt"), b"keep syncing").unwrap();
    let next = run_with_store_path(endpoints, options, &cancel, &filter, &store);
    assert!(next.errors.is_empty(), "{:?}", next.errors);
    assert_eq!(next.omissions.reported_paths().collect::<Vec<_>>(), ["folder"]);
    assert_eq!(next.baseline["folder/entry.txt"], first.baseline["folder/entry.txt"]);
    assert_eq!(std::fs::read(outside.path().join("entry.txt")).unwrap(), b"old");
    assert_eq!(std::fs::read(b.path().join("independent.txt")).unwrap(), b"keep syncing");
    link_fixture::remove_directory(&b.path().join("folder"));
    let pair = pair_id_for(&ba, &ra, &bb, &rb);
    std::fs::remove_file(baseline_path(&pair)).unwrap();
}

#[test]
fn sync_links_task_cycles_and_dangling_links_do_not_abort_regular_files() {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    link_fixture::directory(root.path(), &root.path().join("cycle"));
    link_fixture::directory(target.path(), &root.path().join("dangling"));
    target.close().unwrap();
    std::fs::write(root.path().join("normal.txt"), b"independent").unwrap();
    let path = fwd(root.path());
    let backend = LocalBackend::new(&path);
    let globs = empty_globset();
    let snapshot = snapshot::walk_snapshot(&backend, &path, &AtomicBool::new(false),
        &WalkFilter::basic(true, &globs), HashMode::FullFresh, None, false, true).unwrap();
    assert_eq!(snapshot.tree.keys().map(String::as_str).collect::<Vec<_>>(), ["normal.txt"]);
    assert_eq!(snapshot.omissions.reported_paths().collect::<Vec<_>>(), ["cycle", "dangling"]);
    link_fixture::remove_directory(&root.path().join("cycle"));
    link_fixture::remove_directory(&root.path().join("dangling"));
}
