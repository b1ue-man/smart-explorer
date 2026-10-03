//! Recorded state retries and immutable-version migration at the adapter boundary.
use std::io;
use std::sync::atomic::AtomicBool;

use super::fixture::*;
use crate::bisync as engine;
use crate::vfs::Backend;
use engine::apply_guard::{capture, ExpectedFile};
use engine::checkpoint_journal::{Frame, Journal};
use engine::incremental::SyncEndpoints;
use engine::types::{Baseline, BisyncOptions, DeletePolicy, Direction, PairSide};
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
    std::fs::write(a_dir.path().join("file"), b"changed").unwrap();
    engine::versions::restore_version(&lock, &current.pair, &entry, &side, &cancel).unwrap();
    assert_eq!(std::fs::read(a_dir.path().join("file")).unwrap(), b"saved");
    assert_eq!(std::fs::read(&manifest_path).unwrap(), immutable);
    let intent = intent(&current.pair);
    assert!(!intent.binding.checkpoint_allowed);
    assert_eq!(
        intent.original.digest,
        format!("{:x}", md5::compute(b"changed"))
    );
    engine::replacement_recovery::recover_locked(&lock, &key, endpoints, false, &cancel).unwrap();
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
        !outcome.errors.is_empty() || outcome.stopped.is_some() || !outcome.deferred.is_empty()
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
    assert!(intent(&identity.pair).path().unwrap().exists());
    let resumed =
        engine::orchestration::run_with_store_path(endpoints, opts, &cancel, &filter, &db);
    assert!(resumed.errors.is_empty() && resumed.blocked.is_none() && resumed.stopped.is_none());
    assert_eq!(target.promotions.load(Ordering::SeqCst), 1);
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
        publication_case(fault);
    }
}

fn publication_case(fault: Fault) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("file"), b"original").unwrap();
    let mut remote = TestRemote::new(folder.path(), "publication");
    remote.fault = Some(fault);
    let other_folder = tempfile::tempdir().unwrap();
    let other = FakeRemote::new(other_folder.path(), "other");
    let endpoints = SyncEndpoints::new(&remote, REMOTE_ROOT, &other, REMOTE_ROOT);
    let identity = Identity::current(endpoints);
    *remote.intent_pair.lock().unwrap() = Some(identity.pair.clone());
    let _files = StateFiles::new(&[identity.clone(), reverse(&identity)]);
    let lock = engine::PairLock::acquire(&identity.lock).unwrap();
    let legacy = engine::StateKey::legacy(&identity.pair, &identity.lock);
    let versions = RunVersions::begin(VersionsContext::new(
        &identity.pair,
        engine::StateOwner::AdHoc,
        engine::VersionsLocation::AppData,
        Default::default(),
    ));
    versions.bind_lock(lock.id()).unwrap();
    let side = VersionSide {
        side: PairSide::A,
        backend: &remote,
        root: REMOTE_ROOT,
    };
    let cancel = AtomicBool::new(false);
    let path = "/data/file";
    let original = signature(&remote, path);
    let current = capture(&remote, path, ExpectedFile::Present(original), "fixture").unwrap();
    let backup = engine::version_save::save(
        &versions,
        &side,
        path,
        "file",
        &current,
        ExpectedFile::Present(original),
        VersionReason::Replaced,
        &cancel,
    )
    .unwrap();
    assert!(!backup.moved);
    let mut stage = engine::apply_stage::stage_bytes(
        &remote,
        path,
        &current,
        b"replacement",
        original.mtime_ms,
        &cancel,
    )
    .unwrap();
    stage.bind(&versions, &side, "file", true).unwrap();
    stage.require_backup(backup.signature).unwrap();
    let stage_path = stage.path.clone();
    let result = stage.publish(path, &current, true, &cancel);
    let atomic = matches!(
        fault,
        Fault::AtomicBefore | Fault::AtomicAfter | Fault::AtomicSuccess
    );
    let succeeded = matches!(fault, Fault::AtomicSuccess | Fault::NoReplaceSuccess);
    assert_eq!(result.is_ok(), succeeded);
    if let Ok(outcome) = result {
        assert!(outcome.durable);
        assert_eq!(outcome.bytes, 11);
        assert_eq!(outcome.digest, md5::compute(b"replacement").0);
        assert_eq!(outcome.destination, signature(&remote, path));
    }
    assert_eq!(remote.hooks.load(Ordering::SeqCst), 1);
    assert_eq!(
        remote.promotions.load(Ordering::SeqCst),
        usize::from(atomic)
    );
    let intent = intent(&legacy.pair_id);
    assert_eq!(intent.stage, stage_path);
    assert_eq!(intent.binding.side, "a");
    assert_eq!(
        intent.original.digest,
        format!("{:x}", md5::compute(b"original"))
    );
    assert_eq!(
        engine::orchestration_plan::pending_paths(&lock, &legacy, endpoints).unwrap(),
        vec!["file"]
    );
    let other_key = key(&identity, "other-owner");
    let reversed_key = engine::StateKey {
        pair_id: reverse(&identity).pair,
        replica_a: other_key.replica_b.clone(),
        replica_b: other_key.replica_a.clone(),
        ..other_key.clone()
    };
    let reversed = SyncEndpoints::new(&other, REMOTE_ROOT, &remote, REMOTE_ROOT);
    for (owner, endpoints) in [(&other_key, endpoints), (&reversed_key, reversed)] {
        assert_eq!(
            engine::orchestration_plan::pending_paths(&lock, owner, endpoints).unwrap(),
            vec!["file"]
        );
        let action = engine::Action::DeleteA("file".into());
        let blocked = engine::single_recorded::apply_one(
            endpoints,
            &lock,
            owner,
            &action,
            (None, None),
            &Default::default(),
            Default::default(),
            &cancel,
        );
        assert_eq!(blocked.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }
    let baseline = engine::baseline_file(&legacy).unwrap();
    assert!(!baseline.exists() && !baseline.with_extension("journal").exists());
    assert_eq!(remote.try_exists(&intent.retained).unwrap(), !atomic);
    let published = matches!(
        fault,
        Fault::AtomicAfter
            | Fault::NoReplacePublished
            | Fault::AtomicSuccess
            | Fault::NoReplaceSuccess
    );
    assert_eq!(remote.try_exists(&stage_path).unwrap(), !published);
    let versions = engine::versions::list_versions(&identity.pair, &[side], &cancel).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(
        std::fs::read(&versions[0].stored_path).unwrap(),
        b"original"
    );
    let result =
        engine::replacement_recovery::recover_locked(&lock, &legacy, endpoints, false, &cancel);
    if matches!(fault, Fault::ForeignCreator) {
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(folder.path().join("file")).unwrap(),
            b"foreign"
        );
        assert!(intent.path().unwrap().exists() && remote.try_exists(&intent.retained).unwrap());
        assert!(remote.try_exists(&stage_path).unwrap());
    } else {
        result.unwrap();
        let expected: &[u8] = if published {
            b"replacement"
        } else {
            b"original"
        };
        assert_eq!(std::fs::read(folder.path().join("file")).unwrap(), expected);
        assert!(!intent.path().unwrap().exists() && !remote.try_exists(&stage_path).unwrap());
        assert!(!remote.try_exists(&intent.retained).unwrap());
    }
    assert!(!baseline.exists() && !baseline.with_extension("journal").exists());
    assert_eq!(
        std::fs::read(&versions[0].stored_path).unwrap(),
        b"original"
    );
}
