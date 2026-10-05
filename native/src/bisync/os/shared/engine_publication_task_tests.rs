//! Seven real publication outcomes retain bytes, ownership and checkpoint evidence.
use super::fixture::*;
use crate::bisync as engine;
use crate::vfs::Backend;
use engine::apply_guard::{capture, ExpectedFile};
use engine::incremental::SyncEndpoints;
use engine::types::PairSide;
use engine::versions::{RunVersions, VersionReason, VersionSide, VersionsContext};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn publication_case(fault: Fault) {
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
    assert_eq!(result.is_ok(), succeeded, "fault={fault:?}");
    match result {
        Ok(outcome) => {
            assert!(outcome.durable);
            assert_eq!(outcome.bytes, 11);
            assert_eq!(outcome.digest, md5::compute(b"replacement").0);
            assert_eq!(outcome.destination, signature(&remote, path));
        }
        Err(error) => assert_eq!(error.kind(), io::ErrorKind::ConnectionReset),
    }
    let promotions = usize::from(atomic);
    assert_eq!(remote.hooks.load(Ordering::SeqCst), 1);
    assert_eq!(remote.promotions.load(Ordering::SeqCst), promotions);
    let prepared = {
        let observed = remote.prepared.lock().unwrap();
        assert_eq!(observed.len(), 1);
        observed[0].clone()
    };
    assert_eq!(prepared.stage, stage_path);
    assert_eq!(prepared.binding.side, "a");
    assert!(prepared.binding.checkpoint_allowed);
    assert_eq!(
        prepared.original.digest,
        format!("{:x}", md5::compute(b"original"))
    );
    let other_key = key(&identity, "other-owner");
    let reversed_key = engine::StateKey {
        pair_id: reverse(&identity).pair,
        replica_a: other_key.replica_b.clone(),
        replica_b: other_key.replica_a.clone(),
        ..other_key.clone()
    };
    let reversed = SyncEndpoints::new(&other, REMOTE_ROOT, &remote, REMOTE_ROOT);
    if succeeded {
        assert_publication_finished(&remote, &prepared);
    } else {
        let pending = intent(&legacy.pair_id);
        assert_eq!(pending.path().unwrap(), prepared.path().unwrap());
        assert_eq!(pending.stage, prepared.stage);
        assert_eq!(pending.retained, prepared.retained);
    }
    for (owner, endpoints) in [
        (&legacy, endpoints),
        (&other_key, endpoints),
        (&reversed_key, reversed),
    ] {
        let expected = if succeeded {
            Vec::new()
        } else {
            vec!["file".to_string()]
        };
        assert_eq!(
            engine::orchestration_plan::pending_paths(&lock, owner, endpoints).unwrap(),
            expected
        );
        if !succeeded {
            let blocked = engine::single_recorded::apply_one(
                endpoints,
                &lock,
                owner,
                &engine::Action::DeleteA("file".into()),
                (None, None),
                &Default::default(),
                Default::default(),
                &cancel,
            );
            assert_eq!(blocked.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        }
    }
    let baseline = engine::baseline_file(&legacy).unwrap();
    assert!(!baseline.exists() && !baseline.with_extension("journal").exists());
    assert_eq!(
        remote.try_exists(&prepared.retained).unwrap(),
        !atomic && !succeeded
    );
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
    for _ in 0..2 {
        let result =
            engine::replacement_recovery::recover_locked(&lock, &legacy, endpoints, false, &cancel);
        if matches!(fault, Fault::ForeignCreator) {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
            assert_eq!(
                std::fs::read(folder.path().join("file")).unwrap(),
                b"foreign"
            );
            assert!(prepared.path().unwrap().exists());
            assert!(remote.try_exists(&prepared.retained).unwrap());
            assert!(remote.try_exists(&stage_path).unwrap());
            assert_eq!(
                signature(&remote, &prepared.retained).hash,
                backup.signature.hash
            );
            assert_eq!(
                signature(&remote, &stage_path).hash,
                prepared.staged.signature.hash
            );
        } else {
            result.unwrap();
            let expected: &[u8] = if published {
                b"replacement"
            } else {
                b"original"
            };
            assert_eq!(std::fs::read(folder.path().join("file")).unwrap(), expected);
            assert_publication_finished(&remote, &prepared);
            assert!(
                engine::orchestration_plan::pending_paths(&lock, &legacy, endpoints)
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(!baseline.exists() && !baseline.with_extension("journal").exists());
        assert_eq!(
            std::fs::read(&versions[0].stored_path).unwrap(),
            b"original"
        );
        assert_eq!(remote.hooks.load(Ordering::SeqCst), 1);
        assert_eq!(remote.promotions.load(Ordering::SeqCst), promotions);
    }
}
