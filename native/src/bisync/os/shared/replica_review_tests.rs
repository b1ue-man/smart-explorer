use super::*;
use super::checkpoint_review_tests::Fixture;
use super::state_metadata::PairHistory;
use crate::vfs::{Backend, Scheme, VfsMeta};
use std::io::{self, Read, Write};

fn history(key: &StateKey) -> PairHistory {
    PairHistory { replica_a: key.replica_a.clone(), replica_b: key.replica_b.clone(),
        entries_a: 10, entries_b: 10, full_ms: 1 }
}

#[test]
fn review_task_missing_previous_marker_blocks_creation_on_both_sides() {
    let fixture = Fixture::new();
    let settings = RunSettings::default();
    let first = replica::identify(fixture.endpoints(), &settings, true).unwrap();
    assert!(matches!(first.key.replica_a, ReplicaRef::Marker(_)));
    assert!(matches!(first.key.replica_b, ReplicaRef::Marker(_)));
    state_metadata::save_history(&first.key, &history(&first.key)).unwrap();
    let a = fixture.root.join("a").join(REPLICA_MARKER_NAME);
    let b = fixture.root.join("b").join(REPLICA_MARKER_NAME);
    std::fs::remove_file(&a).unwrap();
    std::fs::remove_file(&b).unwrap();
    let missing = replica::identify(fixture.endpoints(), &settings, true).unwrap();
    assert_eq!(missing.blocked, Some(RunBlock::ReplicaMissing { side: PairSide::A }));
    assert!(!a.exists() && !b.exists(), "both guards precede either marker creation");
}

#[test]
fn review_task_marker_upgrade_carries_basis_but_rotation_uses_another_state() {
    let fixture = Fixture::new();
    let settings = RunSettings::default();
    let initial = replica::identify(fixture.endpoints(), &settings, false).unwrap();
    state_metadata::save_history(&initial.key, &history(&initial.key)).unwrap();
    save_baseline(&baseline_file(&initial.key).unwrap(), &Baseline::from([
        ("kept".into(), (Some(Sig { size: 1, mtime_ms: 1, hash: 1 }), None))])).unwrap();
    let upgrade = replica::identify(fixture.endpoints(), &settings, true).unwrap();
    assert_eq!(upgrade.upgraded_from.as_ref(), Some(&initial.key));
    assert!(load_baseline(&baseline_file(upgrade.upgraded_from.as_ref().unwrap()).unwrap())
        .unwrap().contains_key("kept"));
    state_metadata::save_history(&upgrade.key, &history(&upgrade.key)).unwrap();
    let replacement = match &upgrade.key.replica_a {
        ReplicaRef::Marker(id) if id != &"f".repeat(32) => "f".repeat(32),
        _ => "e".repeat(32),
    };
    std::fs::write(fixture.root.join("a").join(REPLICA_MARKER_NAME),
        serde_json::to_vec(&serde_json::json!({ "replica_id": replacement,
            "created_ms": 2, "pair_hint": fixture.key.pair_id })).unwrap()).unwrap();
    let rotated = replica::identify(fixture.endpoints(), &settings, true).unwrap();
    assert!(rotated.changed && rotated.upgraded_from.is_none());
    assert_ne!(baseline_file(&rotated.key).unwrap(), baseline_file(&upgrade.key).unwrap());
    assert!(load_baseline(&baseline_file(&rotated.key).unwrap()).unwrap().is_empty());
}

struct Unavailable { identity: String }
fn denied<T>() -> io::Result<T> { Err(io::Error::from(io::ErrorKind::PermissionDenied)) }
impl Backend for Unavailable {
    fn scheme(&self) -> Scheme { crate::vfs::LocalBackend::new("/").scheme() }
    fn root_display(&self) -> String { self.identity.clone() }
    fn list_dir(&self, _: &str) -> io::Result<Vec<VfsMeta>> { denied() }
    fn stat(&self, _: &str) -> io::Result<VfsMeta> { denied() }
    fn open_read(&self, _: &str) -> io::Result<Box<dyn Read + Send>> { denied() }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> { denied() }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> { denied() }
    fn remove_file(&self, _: &str) -> io::Result<()> { denied() }
    fn remove_dir(&self, _: &str) -> io::Result<()> { denied() }
    fn mkdir_all(&self, _: &str) -> io::Result<()> { denied() }
}

#[test]
fn review_task_unavailable_identity_preserves_known_replicas_without_rotation() {
    let mut fixture = Fixture::new();
    let backend = Unavailable { identity: fixture.a.clone() };
    fixture.key = StateKey::legacy(&pair_id_for(&backend, &fixture.a, &backend, &fixture.b),
        &pair_lock_id(&backend, &fixture.a, &backend, &fixture.b));
    fixture.key.replica_a = ReplicaRef::Marker("1".repeat(32));
    fixture.key.replica_b = ReplicaRef::Marker("2".repeat(32));
    state_metadata::save_history(&fixture.key, &history(&fixture.key)).unwrap();
    let endpoints = incremental::SyncEndpoints::new(&backend, &fixture.a, &backend, &fixture.b);
    let unavailable = replica::identify(endpoints, &RunSettings::default(), true).unwrap();
    assert_eq!(unavailable.key, fixture.key);
    assert!(!unavailable.changed && unavailable.blocked.is_none());
}
