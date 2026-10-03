//! Executable engine-boundary assertions for the single remote task suite.
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

use crate::bisync as engine;
use crate::vfs::{Backend, ChangeKind, VfsChange};
use super::engine_change_feed::{FeedIndex, FeedResolution};
use super::incremental::SyncEndpoints;
use super::state_store::Side;
use super::types::{Action, Baseline, BisyncOptions, DeletePolicy, Direction};
use super::versions::{RunVersions, VersionsContext};

#[path = "engine_provider_fixture.rs"]
mod fixture;
#[path = "engine_identity_task_tests.rs"]
mod identity_tests;
use fixture::*;



#[test]
fn engine_provider_pending_protects_owners_orientation_and_single_actions() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let a = TestRemote::new(a_dir.path(), "pending-a");
    let b = TestRemote::new(b_dir.path(), "pending-b");
    let endpoints = SyncEndpoints::new(&a, REMOTE_ROOT, &b, REMOTE_ROOT);
    let identity = Identity::current(endpoints);
    let reversed = reverse(&identity);
    let mut files = StateFiles::new(&[identity.clone(), reversed.clone()]);
    let lock = super::PairLock::acquire(&identity.lock).unwrap();
    let current = key(&identity, "current");
    let other = key(&identity, "other-owner");
    merge(&other, "file", Some("file-sibling"));
    let mut reversed_key = key(&reversed, "reverse-owner");
    reversed_key.replica_a = current.replica_b.clone();
    reversed_key.replica_b = current.replica_a.clone();
    merge(&reversed_key, "reverse-file", None);
    let mut foreign = key(&identity, "foreign-replica");
    foreign.replica_a = super::ReplicaRef::Marker("different-device".into());
    merge(&foreign, "foreign-file", None);
    let legacy = super::StateKey::legacy(&identity.pair, &identity.lock);
    files.track(super::baseline_path(&identity.pair).with_extension(format!("merge-{}.json",
        super::version_manifest::token("legacy-file"))));
    merge(&legacy, "legacy-file", None);
    let pending = super::orchestration_plan::pending_paths(&lock, &current, endpoints).unwrap();
    assert_eq!(pending, vec!["file", "file-sibling", "legacy-file", "reverse-file"]);
    let sig = super::Sig { size: 8, mtime_ms: 7, hash: 19 };
    let names = ["file", "file-sibling", "reverse-file", "legacy-file", "untouched"];
    let baseline: Baseline = names.iter().map(|name| ((*name).into(), (Some(sig), Some(sig)))).collect();
    let basis_path = super::baseline_file(&current).unwrap();
    super::save_baseline(&basis_path, &baseline).unwrap();
    let mut snapshot = snapshot(&names, sig);
    let keys = super::orchestration_plan::keys(endpoints);
    super::orchestration_plan::protect_pending(&lock, &current, endpoints, keys, &mut snapshot).unwrap();
    assert_eq!(snapshot.a.tree.keys().cloned().collect::<Vec<_>>(), vec!["untouched"]);
    assert_eq!(snapshot.b.tree.keys().cloned().collect::<Vec<_>>(), vec!["untouched"]);
    let (actions, _, converged) = snapshot.plan(&baseline, BisyncOptions::default());
    assert!(actions.is_empty());
    assert!(converged.iter().all(|rel| !pending.contains(rel)));
    for rel in &pending {
        let result = super::single_recorded::apply_one(endpoints, &lock, &current,
            &Action::DeleteA(rel.clone()), (None, None), &Default::default(),
            BisyncOptions::default(), &AtomicBool::new(false));
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }
    assert_eq!(super::load_baseline(&basis_path).unwrap(), baseline);
    let settings = super::RunSettings::for_job("current");
    let ignore = super::empty_globset();
    let filter = super::WalkFilter::basic(true, &ignore);
    let versions = RunVersions::begin(VersionsContext::new(&identity.pair, current.owner.clone(),
        super::VersionsLocation::AppData, Default::default()));
    let cancel = AtomicBool::new(false);
    let db = state_dir.path().join("index.sqlite");
    let state = super::orchestration::RunState {
        endpoints, opts: BisyncOptions { direction: Direction::AtoB, delete: DeletePolicy::Mirror,
            ..Default::default() }, settings: &settings, cancel: &cancel, filter: &filter,
        observer: None, lock: &lock, key: &current, history: None, baseline: &baseline,
        dirs: None, versions: &versions, store_path: Some(&db),
    };
    assert!(super::incremental::try_incremental_run(&state).is_none());
    assert!(super::incremental::bootstrap_run(&state, &baseline, None).is_err());
    assert!(!db.exists());
    for token in ["job-", "job-../escape", "adhoc.extra", "job-bad/name"] {
        assert!(super::replica_state::owner_from_token(token).is_err());
    }
}

#[test]
fn engine_provider_account_feed_proves_ancestry_and_preserves_literal_names() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("metadata"), b"x").unwrap();
    let remote = TestRemote::new(folder.path(), "feed");
    let record = record();
    let items = BTreeMap::from([
        ("parent".into(), item("parent", "parent-id", "root", true)),
        ("parent/nested".into(), item("parent/nested", "nested-id", "parent-id", true)),
    ]);
    let changes = [event(&remote, "file", Some("nested-id"), "literal%2F # name", false),
        event(&remote, "outside", None, "other-root", true),
        event(&remote, "foreign", Some("outside"), "foreign", false)];
    let index = FeedIndex::new(&record, Side::A, &items, &changes).unwrap();
    match index.resolve(&changes[0]).unwrap() {
        FeedResolution::Rooted(change, _) => assert_eq!(change.rel, "parent/nested/literal%2F # name"),
        FeedResolution::Outside => panic!("selected-root change was discarded"),
    }
    assert!(matches!(index.resolve(&changes[2]).unwrap(), FeedResolution::Outside));
    let unknown = event(&remote, "unknown", Some("unobserved-parent"), "file", false);
    assert!(index.resolve(&unknown).is_err());
    let removed = VfsChange { kind: ChangeKind::Remove, rel: None, id: Some("unmanaged".into()),
        parent_id: None, name: None, meta: None };
    assert!(matches!(index.resolve(&removed).unwrap(), FeedResolution::Outside));
}

#[test]
fn engine_provider_ambiguous_feed_ids_and_changed_parent_require_rebuild() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("metadata"), b"x").unwrap();
    let remote = TestRemote::new(folder.path(), "feed-ambiguity");
    let record = record();
    let mut items = BTreeMap::from([("parent".into(), item("parent", "dir", "root", true))]);
    let changes = [event(&remote, "dir", Some("root"), "parent", true),
        event(&remote, "file", Some("dir"), "file", false)];
    let index = FeedIndex::new(&record, Side::A, &items, &changes).unwrap();
    assert!(index.resolve(&changes[1]).is_err());
    items.insert("alias".into(), item("alias", "dir", "root", true));
    assert!(FeedIndex::new(&record, Side::A, &items, &[]).is_err());
    let repeated = [event(&remote, "repeated", Some("root"), "file", false),
        event(&remote, "repeated", Some("root"), "renamed", false)];
    assert!(FeedIndex::new(&record, Side::A, &BTreeMap::new(), &repeated).is_err());
}

#[test]
fn engine_provider_literal_walk_keeps_existing_encoded_root_and_connection() {
    let folder = tempfile::tempdir().unwrap();
    let name = "literal%3A # ü";
    let directory = folder.path().join("encoded%2F-root");
    std::fs::create_dir_all(directory.join("node_modules")).unwrap();
    std::fs::write(directory.join(name), b"literal").unwrap();
    std::fs::write(directory.join("node_modules").join(name), b"ordinary").unwrap();
    let mut remote = TestRemote::new(folder.path(), "literal");
    let root = "/data/encoded%2F-root";
    remote.literal_root = Some(root.into());
    let path = crate::vfs::sync_path(&remote, root, name).unwrap();
    assert_eq!(path, format!("{root}/literal%253A%20%23%20ü"));
    assert_eq!(signature(&remote, &path).size, 7);
    let opts = BisyncOptions::default();
    let ignore = super::empty_globset();
    let filter = super::WalkFilter::basic(true, &ignore);
    let snapshot = super::snapshot::walk_snapshot_with_options(&remote, root, &AtomicBool::new(false),
        &filter, super::snapshot::hash_mode(&remote, &remote, opts.compare), None, false, false, opts).unwrap();
    assert!(snapshot.tree.contains_key(name));
    assert!(snapshot.tree.contains_key(&format!("node_modules/{name}")));
    assert!(snapshot.omissions.is_empty());
    assert!(remote.requested.lock().unwrap().iter().any(|seen| seen == &path));
    let other = TestRemote::new(folder.path(), "other-connection");
    assert_ne!(super::pair_id_for(&remote, root, &remote, "/data/target"),
        super::pair_id_for(&other, root, &remote, "/data/target"));
}

fn snapshot(names: &[&str], signature: engine::Sig) -> engine::snapshot_pair::PairSnapshot {
    let side = || engine::SideSnapshot {
        tree: names.iter().map(|name| ((*name).to_string(), signature)).collect(),
        filtered: BTreeMap::new(), dirs: Default::default(), omissions: engine::SyncOmissions::new(false),
    };
    engine::snapshot_pair::PairSnapshot {
        a: side(), b: side(), omissions: engine::SyncOmissions::new(false), repairs: Vec::new(), conflicts: Vec::new(),
    }
}

fn record() -> engine::state_store::PairRecord {
    engine::state_store::PairRecord {
        pair: "fixture".into(), root_a: "/selected".into(), root_b: "/target".into(),
        mode: "mirror-rv2-ancestry".into(), source_side: engine::state_store::Side::A,
        source_cursor: None, root_a_id: Some("root".into()), root_b_id: None,
        bootstrapped: true, target_managed: true,
    }
}

fn item(rel: &str, id: &str, parent: &str, is_dir: bool) -> engine::state_store::ItemRecord {
    engine::state_store::ItemRecord {
        side: engine::state_store::Side::A, rel: rel.into(), id: Some(id.into()),
        parent_id: Some(parent.into()), name: rel.rsplit('/').next().map(str::to_string),
        sig: None, is_dir, deleted: false,
    }
}

fn event(remote: &TestRemote, id: &str, parent: Option<&str>, name: &str, is_dir: bool) -> VfsChange {
    let mut meta = remote.stat("/data/metadata").unwrap();
    meta.name = name.into();
    meta.id = Some(id.into());
    meta.is_dir = is_dir;
    VfsChange {
        kind: ChangeKind::Upsert, rel: None, id: Some(id.into()), parent_id: parent.map(str::to_string),
        name: Some(name.into()), meta: Some(meta),
    }
}

#[test]
fn engine_provider_identity_bounds_and_foreign_account_fail_closed() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let mut a = TestRemote::new(a_dir.path(), "account");
    let b = TestRemote::new(b_dir.path(), "other");
    let mut foreign = TestRemote::new(a_dir.path(), "foreign-account");
    foreign.identity = format!("gdrive:path-v2:foreign-{}:/data", a.identity);
    let current = Identity::current(SyncEndpoints::new(&a, REMOTE_ROOT, &b, REMOTE_ROOT));
    let unrelated = Identity::current(SyncEndpoints::new(&foreign, REMOTE_ROOT, &b, REMOTE_ROOT));
    let _files = StateFiles::new(&[current.clone(), reverse(&current), unrelated.clone(), reverse(&unrelated)]);
    std::fs::write(a_dir.path().join("file"), b"foreign-account").unwrap();
    let basis = engine::Baseline::from([("file".into(), (Some(signature(&foreign, "/data/file")), None))]);
    engine::save_baseline(&engine::baseline_path(&unrelated.pair), &basis).unwrap();
    a.previous = (0..9).map(|number| format!("old-token-{number}-{}", a.identity)).collect();
    assert!(crate::vfs::previous_state_identities(&a).is_err());
    let lock = engine::PairLock::acquire(&current.lock).unwrap();
    let cancel = AtomicBool::new(false);
    let endpoints = SyncEndpoints::new(&a, REMOTE_ROOT, &b, REMOTE_ROOT);
    assert!(engine::backend_identity_migration::migrate(&lock, endpoints, &cancel).is_err());
    assert!(engine::backend_identity_state::read(&current.pair).unwrap().is_none());
    a.previous.pop();
    assert_eq!(crate::vfs::previous_state_identities(&a).unwrap().len(), 8);
    a.previous.clear();
    let endpoints = SyncEndpoints::new(&a, REMOTE_ROOT, &b, REMOTE_ROOT);
    assert!(engine::backend_identity_migration::migrate(&lock, endpoints, &cancel).unwrap().is_empty());
    assert!(!engine::baseline_path(&current.pair).exists());
    assert_eq!(engine::load_baseline(&engine::baseline_path(&unrelated.pair)).unwrap(), basis);
    assert!(!engine::backend_identity_state::version_matches(&current.pair, &unrelated.pair,
        &foreign.identity, REMOTE_ROOT, 0, &a.identity).unwrap());
    let mut refreshed = TestRemote::new(a_dir.path(), "rotated-token");
    refreshed.identity = a.identity.clone();
    refreshed.previous = vec!["fresh-token-alias".into()];
    assert_eq!(Identity::current(SyncEndpoints::new(&refreshed, REMOTE_ROOT, &b, REMOTE_ROOT)).pair, current.pair);
}
