use super::engine::lock;
use super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::*;
use std::{fs, io, sync::Arc};

fn config() -> MountRuntimeConfig {
    MountRuntimeConfig::new(MountId::parse("recovery-cache-task").unwrap(), MountMode::ReadWrite)
        .with_cache_policy(MountCachePolicy::new(0).unwrap())
}

fn engine(dir: &FixtureDirectory, backend: &Arc<OptimizationBackend>) -> MountEngine {
    MountEngine::open_host_cache(config(), backend.clone(), dir.path()).unwrap()
}

fn open(engine: &MountEngine, path: &str, writable: bool) -> HandleId {
    engine.open_file(path, OpenFileOptions {
        writable, disposition: OpenDisposition::OpenExisting,
    }).unwrap()
}

#[test]
fn mount_recovery_cache_task_missing_payload_keeps_siblings_and_restored_retry() {
    let dir = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/lost", b"old");
    backend.put("/intact", b"old");
    let mounted = engine(&dir, &backend);
    for path in ["\\lost", "\\intact"] {
        let handle = open(&mounted, path, true);
        mounted.write(handle, 0, b"new").unwrap();
        mounted.close(handle).unwrap();
    }
    let lost_name = lock(&mounted.entry_for_path("/lost").unwrap().unwrap().state)
        .unwrap().spool_name.clone();
    drop(mounted);
    let root = dir.path().join(config().id.as_str());
    let lost = root.join("files").join(&lost_name);
    let payload = fs::read(&lost).unwrap();
    let original_journal = fs::read(root.join("journal.jsonl")).unwrap();
    fs::remove_file(&lost).unwrap();
    assert_eq!(spool::audit_recovery(dir.path(), &config().id).unwrap(), MountRecovery::Required);
    assert_eq!(fs::read(root.join("journal.jsonl")).unwrap(), original_journal);
    let recovered = engine(&dir, &backend);
    assert_eq!(recovered.list_dir("\\").unwrap().len(), 2);
    let dirty = recovered.dirty_entries().unwrap();
    let (_, EntryCondition::Conflict(conflict)) = dirty.iter().find(|(p, _)| p == "/lost").unwrap() else {
        panic!("missing local changes must be a visible conflict");
    };
    assert!(conflict.detail.contains(&lost_name));
    assert!(conflict.detail.contains("/lost"));
    let handle = open(&recovered, "\\lost", false);
    assert_eq!(recovered.read(handle, 0, &mut [0; 3]).unwrap_err().kind(), io::ErrorKind::NotFound);
    recovered.close(handle).unwrap();
    recovered.retry_pending_changes().unwrap();
    assert_eq!(backend.bytes("/intact"), b"new");
    assert_eq!(backend.bytes("/lost"), b"old", "missing local bytes cannot be uploaded");
    assert_eq!(recovered.dirty_entries().unwrap().len(), 1);
    drop(recovered);
    fs::write(&lost, payload).unwrap();
    let restored = engine(&dir, &backend);
    restored.retry_pending_changes().unwrap();
    assert_eq!(backend.bytes("/lost"), b"new");
    assert!(restored.dirty_entries().unwrap().is_empty());
}

#[test]
fn mount_recovery_cache_task_absent_or_invalid_journal_preserves_payloads() {
    for journal in [None, Some(b"not-json\n".as_slice())] {
        let dir = FixtureDirectory::new();
        let root = dir.path().join(config().id.as_str());
        let files = root.join("files");
        fs::create_dir_all(&files).unwrap();
        let payload = files.join(format!("{:032x}.spool", 1));
        fs::write(&payload, b"unclassified recovery data").unwrap();
        if let Some(bytes) = journal { fs::write(root.join("journal.jsonl"), bytes).unwrap(); }
        assert!(spool::audit_recovery(dir.path(), &config().id).is_err());
        assert_eq!(fs::read(&payload).unwrap(), b"unclassified recovery data");
        if journal.is_none() { assert!(!root.join("journal.jsonl").exists()); }
    }
    let dir = FixtureDirectory::new();
    assert_eq!(spool::audit_recovery(dir.path(), &config().id).unwrap(), MountRecovery::Clean);
}

#[test]
fn mount_recovery_cache_task_incomplete_audit_cannot_clean_from_drop() {
    let dir = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let mounted = engine(&dir, &backend);
    let handle = open(&mounted, "\\note", true);
    mounted.write(handle, 0, b"new").unwrap();
    let name = lock(&mounted.entry_for_path("/note").unwrap().unwrap().state).unwrap().spool_name.clone();
    mounted.close(handle).unwrap();
    drop(mounted);
    let files = dir.path().join(config().id.as_str()).join("files");
    let orphan = files.join(format!("{:032x}.spool", 9));
    fs::write(&orphan, b"preserve until complete audit").unwrap();
    fs::remove_file(files.join(&name)).unwrap();
    fs::create_dir(files.join(&name)).unwrap();
    assert!(spool::audit_recovery(dir.path(), &config().id).is_err());
    assert_eq!(fs::read(orphan).unwrap(), b"preserve until complete audit");
}

#[test]
fn mount_recovery_cache_task_failed_eviction_does_not_starve_other_clean_files() {
    let dir = FixtureDirectory::new();
    let (spool, _) = spool::WholeFileSpool::open(dir.path(), &config().id).unwrap();
    let cache = clean_cache::CleanCache::default();
    let mut names = Vec::new();
    for index in 0..3 {
        let allocated = spool.allocate().unwrap();
        drop(allocated.file);
        let path = format!("/note-{index}");
        cache.retain(clean_cache::IdleClean::new(path.clone(), path,
            allocated.name.clone(), Baseline::Missing, 0, std::time::Instant::now())).unwrap();
        names.push(allocated.name);
    }
    let files = dir.path().join(config().id.as_str()).join("files");
    let blocked = files.join(&names[0]);
    fs::remove_file(&blocked).unwrap();
    fs::create_dir(&blocked).unwrap();
    assert_eq!(cache.trim(&spool, 0).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(cache.usage().unwrap(), (1, 0));
    assert!(blocked.is_dir());
    assert!(!files.join(&names[1]).exists());
    assert!(!files.join(&names[2]).exists());
    fs::remove_dir(&blocked).unwrap();
    cache.trim(&spool, 0).unwrap();
    assert_eq!(cache.usage().unwrap(), (0, 0));
}
