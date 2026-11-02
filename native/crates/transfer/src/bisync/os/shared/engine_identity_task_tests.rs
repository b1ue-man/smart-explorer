//! Recorded state retries and immutable-version migration at the adapter boundary.
use std::io;
use std::sync::atomic::AtomicBool;

use super::fixture::*;
use crate::bisync as engine;
use crate::vfs::Backend;
use engine::apply_guard::{capture, ExpectedFile};
use engine::checkpoint_journal::{Frame, Journal};
use engine::incremental::SyncEndpoints;
use engine::types::{Baseline, BisyncOptions, DeletePolicy, Direction};
use engine::versions::{RunVersions, VersionReason, VersionSide, VersionsContext};
use std::sync::atomic::Ordering;

#[test]
fn engine_provider_account_identity_preserves_state_locks_inputs_and_versions() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    std::fs::write(a_dir.path().join("file"), b"saved").unwrap();
    let old_a = TestRemote::new(a_dir.path(), "old-token");
    let b = TestRemote::new(b_dir.path(), "unchanged-other");
    let mut a = TestRemote::new(a_dir.path(), "stable-account");
    a.identity = format!("gdrive:path-v2:account-{}:/data", old_a.identity);
    a.previous = vec![old_a.identity.clone()];
    a.reject_archives = true;
    let old_endpoints = SyncEndpoints::new(&old_a, REMOTE_ROOT, &b, REMOTE_ROOT);
    let endpoints = SyncEndpoints::new(&a, REMOTE_ROOT, &b, REMOTE_ROOT);
    let old = Identity::current(old_endpoints);
    let current = Identity::current(endpoints);
    let _files = StateFiles::new(&[
        old.clone(),
        current.clone(),
        reverse(&old),
        reverse(&current),
    ]);
    assert_ne!(old.pair, current.pair);
    assert_eq!(current.lock, reverse(&current).lock);
    assert_ne!(current.pair, reverse(&current).pair);
    let old_key = key(&old, "migration-owner");
    let key = engine::StateKey {
        pair_id: current.pair.clone(),
        lock_id: current.lock.clone(),
        ..old_key.clone()
    };
    let sig = signature(&old_a, "/data/file");
    let baseline = engine::Baseline::from([("file".into(), (Some(sig), None))]);
    engine::save_baseline(&engine::baseline_file(&old_key).unwrap(), &baseline).unwrap();
    let history = engine::state_metadata::PairHistory {
        replica_a: old_key.replica_a.clone(),
        replica_b: old_key.replica_b.clone(),
        entries_a: 1,
        entries_b: 0,
        full_ms: 57,
    };
    engine::state_metadata::save_history(&old_key, &history).unwrap();
    let dirs = ["literal%2F-directory".to_string()].into_iter().collect();
    engine::state_metadata::save_dirs(&old_key, &dirs).unwrap();
    let mut names = engine::state_spellings::StateSpellings::default();
    names.files_a.insert("file".into(), "file".into());
    engine::state_spellings::save(&old_key, &names).unwrap();
    let old_spelling = std::fs::read(
        engine::baseline_file(&old_key)
            .unwrap()
            .with_extension("spellings.json"),
    )
    .unwrap();
    merge(&old_key, "file", Some("file-sibling"));
    let mut recovery = engine::merge_recovery::load(&old_key, "file")
        .unwrap()
        .unwrap();
    recovery.original_a = Some(format!("{:x}", md5::compute(b"saved")));
    engine::merge_recovery::save(&old_key, &recovery).unwrap();
    engine::merge_inputs::save(
        &old_key,
        &recovery,
        engine::OriginalContent {
            signature: Some(sig),
            bytes: Some(b"saved"),
        },
        engine::OriginalContent {
            signature: None,
            bytes: None,
        },
        b"merged",
    )
    .unwrap();
    let old_legacy = engine::StateKey::legacy(&old.pair, &old.lock);
    let (mut journal, _, _) = Journal::load(&old_legacy, Default::default()).unwrap();
    journal
        .append(&Frame {
            records: vec![("file".into(), (Some(sig), None))],
            ..Frame::default()
        })
        .unwrap();
    assert!(!engine::baseline_path(&old.pair).exists());
    let old_lock = engine::PairLock::acquire(&old.lock).unwrap();
    let versions = RunVersions::begin(VersionsContext::new(
        &old.pair,
        old_key.owner.clone(),
        engine::VersionsLocation::AppData,
        Default::default(),
    ));
    versions.bind_lock(old_lock.id()).unwrap();
    let old_side = VersionSide {
        side: engine::PairSide::A,
        backend: &old_a,
        root: REMOTE_ROOT,
    };
    let cancel = AtomicBool::new(false);
    let captured = capture(
        &old_a,
        "/data/file",
        ExpectedFile::Present(sig),
        "fixture version",
    )
    .unwrap();
    let saved = engine::version_save::save(
        &versions,
        &old_side,
        "/data/file",
        "file",
        &captured,
        ExpectedFile::Present(sig),
        VersionReason::Replaced,
        &cancel,
    )
    .unwrap();
    let manifest_path = std::path::Path::new(&saved.path)
        .parent()
        .unwrap()
        .join("entry.json");
    let immutable = std::fs::read(&manifest_path).unwrap();
    let entry = engine::versions::list_versions(&old.pair, &[old_side], &cancel)
        .unwrap()
        .remove(0);
    drop(old_lock);
    let lock = engine::PairLock::acquire(&current.lock).unwrap();
    let guards = engine::backend_identity_migration::migrate(&lock, endpoints, &cancel).unwrap();
    assert_eq!(
        engine::PairLock::acquire(&old.lock).err().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        engine::load_baseline(&engine::baseline_file(&key).unwrap()).unwrap(),
        baseline
    );
    let history = engine::state_metadata::load_history(&key).unwrap().unwrap();
    assert_eq!(
        (history.replica_a, history.replica_b),
        (old_key.replica_a, old_key.replica_b)
    );
    assert_eq!(
        engine::state_metadata::load_dirs(&key).unwrap().unwrap(),
        dirs
    );
    assert_eq!(
        std::fs::read(
            engine::baseline_file(&key)
                .unwrap()
                .with_extension("spellings.json")
        )
        .unwrap(),
        old_spelling
    );
    let legacy = engine::StateKey::legacy(&current.pair, &current.lock);
    let (_, records, _) = Journal::load(&legacy, Default::default()).unwrap();
    assert_eq!(records.baseline, baseline);
    let recovery = engine::merge_recovery::load(&key, "file").unwrap().unwrap();
    assert_eq!(recovery.lock, current.lock);
    let pending = engine::merge_inputs::load(&key, &recovery).unwrap();
    assert_eq!(pending.original_a.as_deref(), Some(b"saved".as_slice()));
    assert_eq!(pending.merged, b"merged");
    assert_eq!(
        engine::orchestration_plan::pending_paths(&lock, &key, endpoints).unwrap(),
        vec!["file", "file-sibling"]
    );
    engine::merge_recovery::remove(&key, "file").unwrap();
    engine::merge_inputs::remove(&key, "file").unwrap();
    let side = VersionSide {
        side: engine::PairSide::A,
        backend: &a,
        root: REMOTE_ROOT,
    };
    let listed =
        engine::versions::list_versions(&current.pair, std::slice::from_ref(&side), &cancel)
            .unwrap();
    assert!(listed.contains(&entry));
    let job_basis = engine::baseline_file(&key).unwrap();
    let legacy_basis = engine::baseline_file(&legacy).unwrap();
    let state_paths = [
        job_basis.clone(),
        job_basis.with_extension("journal"),
        legacy_basis.clone(),
        legacy_basis.with_extension("journal"),
    ];
    let state_bytes = || {
        state_paths
            .iter()
            .map(|path| optional_file_bytes(path))
            .collect::<Vec<_>>()
    };
    let unchanged = state_bytes();
    *a.intent_pair.lock().unwrap() = Some(current.pair.clone());
    std::fs::write(a_dir.path().join("file"), b"changed").unwrap();
    engine::versions::restore_version(&lock, &current.pair, &entry, &side, &cancel).unwrap();
    assert_eq!(std::fs::read(a_dir.path().join("file")).unwrap(), b"saved");
    assert_eq!(std::fs::read(&manifest_path).unwrap(), immutable);
    let prepared = {
        let observed = a.prepared.lock().unwrap();
        assert_eq!(observed.len(), 1);
        observed[0].clone()
    };
    assert!(!prepared.binding.checkpoint_allowed);
    assert_eq!(
        prepared.original.digest,
        format!("{:x}", md5::compute(b"changed"))
    );
    assert_publication_finished(&a, &prepared);
    assert_eq!(state_bytes(), unchanged);
    let mut restored_backups: Vec<_> =
        engine::versions::list_versions(&current.pair, std::slice::from_ref(&side), &cancel)
            .unwrap()
            .into_iter()
            .filter(|version| {
                version.reason == Some(VersionReason::Restored)
                    && std::fs::read(&version.stored_path).unwrap() == b"changed"
            })
            .collect();
    assert_eq!(restored_backups.len(), 1);
    a.lose_next_ack.store(true, Ordering::SeqCst);
    let error = engine::versions::restore_version(
        &lock,
        &current.pair,
        &restored_backups.remove(0),
        &side,
        &cancel,
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
    let pending = intent(&current.pair);
    assert!(!pending.binding.checkpoint_allowed);
    assert_eq!(
        std::fs::read(a_dir.path().join("file")).unwrap(),
        b"changed"
    );
    assert_eq!(state_bytes(), unchanged);
    let promotions = a.promotions.load(Ordering::SeqCst);
    engine::replacement_recovery::recover_locked(&lock, &key, endpoints, false, &cancel).unwrap();
    assert_eq!(a.promotions.load(Ordering::SeqCst), promotions);
    assert_publication_finished(&a, &pending);
    assert_eq!(
        std::fs::read(a_dir.path().join("file")).unwrap(),
        b"changed"
    );
    assert_eq!(state_bytes(), unchanged);
    engine::versions::restore_version(&lock, &current.pair, &entry, &side, &cancel).unwrap();
    let prepared = a.prepared.lock().unwrap().last().unwrap().clone();
    assert!(!prepared.binding.checkpoint_allowed);
    assert_publication_finished(&a, &prepared);
    assert_eq!(std::fs::read(a_dir.path().join("file")).unwrap(), b"saved");
    assert_eq!(std::fs::read(&manifest_path).unwrap(), immutable);
    assert_eq!(state_bytes(), unchanged);
    assert_eq!(
        engine::load_baseline(&engine::baseline_file(&key).unwrap()).unwrap(),
        baseline
    );
    drop(guards);
    let again = engine::backend_identity_migration::migrate(&lock, endpoints, &cancel).unwrap();
    assert_eq!(
        engine::load_baseline(&engine::baseline_file(&key).unwrap()).unwrap(),
        baseline
    );
    drop(again);
    assert!(engine::PairLock::acquire(&old.lock).is_ok());
}

#[test]
fn engine_provider_recorded_lost_ack_preserves_old_baseline_until_retry() {
    let source_dir = tempfile::tempdir().unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    std::fs::write(source_dir.path().join("file"), b"replacement").unwrap();
    std::fs::write(target_dir.path().join("file"), b"original").unwrap();
    let source = FakeRemote::new(source_dir.path(), "recorded-source");
    let mut target = TestRemote::new(target_dir.path(), "recorded-target");
    target.fault = Some(Fault::AtomicAfter);
    target.reject_archives = true;
    let endpoints = SyncEndpoints::new(&source, REMOTE_ROOT, &target, REMOTE_ROOT);
    let identity = Identity::current(endpoints);
    *target.intent_pair.lock().unwrap() = Some(identity.pair.clone());
    let _files = StateFiles::new(&[identity.clone(), reverse(&identity)]);
    let settings = engine::RunSettings::default();
    let key = engine::replica::identify(endpoints, &settings, true)
        .unwrap()
        .key;
    let original = signature(&target, "/data/file");
    let baseline = Baseline::from([("file".into(), (Some(original), Some(original)))]);
    engine::save_baseline(&engine::baseline_file(&key).unwrap(), &baseline).unwrap();
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        verify: true,
        max_transfers: 1,
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let ignore = engine::empty_globset();
    let filter = engine::WalkFilter::basic(true, &ignore);
    let db = state_dir.path().join("index.sqlite");
    let outcome =
        engine::orchestration::run_with_store_path(endpoints, opts, &cancel, &filter, &db);
    assert!(
        !outcome.errors.is_empty() || outcome.stopped.is_some() || !outcome.deferred.is_empty(),
        "promotions={}, errors={:?}, blocked={:?}, stopped={:?}, deferred={:?}, busy={}, omissions={:?}, state={:?}",
        target.promotions.load(Ordering::SeqCst), outcome.errors, outcome.blocked, outcome.stopped,
        outcome.deferred, outcome.busy, outcome.omissions, outcome.state
    );
    assert_eq!(
        target.promotions.load(Ordering::SeqCst),
        1,
        "first run: errors={:?}, blocked={:?}, stopped={:?}, deferred={:?}, state={:?}",
        outcome.errors,
        outcome.blocked,
        outcome.stopped,
        outcome.deferred,
        outcome.state
    );
    let (_, records, _) =
        engine::checkpoint_journal::Journal::load(&key, Default::default()).unwrap();
    assert_eq!(records.baseline, baseline);
    assert_eq!(
        std::fs::read(target_dir.path().join("file")).unwrap(),
        b"replacement"
    );
    let pending = intent(&identity.pair);
    assert!(pending.path().unwrap().exists());
    let resumed =
        engine::orchestration::run_with_store_path(endpoints, opts, &cancel, &filter, &db);
    assert!(
        resumed.errors.is_empty() && resumed.blocked.is_none() && resumed.stopped.is_none(),
        "retry errors={:?}, blocked={:?}, stopped={:?}, deferred={:?}, omissions={:?}, state={:?}",
        resumed.errors,
        resumed.blocked,
        resumed.stopped,
        resumed.deferred,
        resumed.omissions,
        resumed.state
    );
    assert_eq!(target.promotions.load(Ordering::SeqCst), 1);
    assert_eq!(target.prepared.lock().unwrap().len(), 1);
    assert_publication_finished(&target, &pending);
    let (_, records, _) =
        engine::checkpoint_journal::Journal::load(&key, Default::default()).unwrap();
    assert_eq!(records.baseline["file"].1.unwrap().size, 11);
    assert!(engine::replacement_recovery::relatives(
        &engine::PairLock::acquire(&key.lock_id).unwrap(),
        &key,
        endpoints
    )
    .unwrap()
    .is_empty());
}

#[test]
fn engine_provider_publication_and_lost_ack_use_exactly_one_contract() {
    for fault in [
        Fault::AtomicBefore,
        Fault::AtomicAfter,
        Fault::NoReplaceMoved,
        Fault::NoReplacePublished,
        Fault::ForeignCreator,
        Fault::AtomicSuccess,
        Fault::NoReplaceSuccess,
    ] {
        super::publication_tests::publication_case(fault);
    }
}
