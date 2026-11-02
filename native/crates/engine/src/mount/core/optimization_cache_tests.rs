use super::cache_space::CACHE_RESERVE_BYTES;
use super::clean_cache::{IdleClean, MAX_CONTENT_AGE, MAX_IDLE_RECORDS};
use super::engine::{lock, write_lock};
use super::open_handle::OpenHandleKind;
use super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::{
    Baseline, CacheSpaceProbe, EntryCondition, FlushOutcome, HandleId, MountCachePolicy,
    MountEngine, MountId, MountMode, MountRuntimeConfig, OpenDisposition, OpenFileOptions,
    RenameOutcome,
};
use crate::vfs::Backend;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn config(mib: u32) -> MountRuntimeConfig {
    MountRuntimeConfig::new(MountId::parse("cache-task").unwrap(), MountMode::ReadWrite)
        .with_cache_policy(MountCachePolicy::new(mib).unwrap())
}

fn new_engine(directory: &FixtureDirectory, backend: &Arc<OptimizationBackend>, mib: u32) -> MountEngine {
    let engine = MountEngine::open_host_cache(config(mib), backend.clone(), directory.path()).unwrap();
    engine.prepare_host_remote().unwrap();
    engine
}

fn open(engine: &MountEngine, path: &str, writable: bool) -> HandleId {
    engine.open_file(path, OpenFileOptions { writable, disposition: OpenDisposition::OpenExisting }).unwrap()
}

fn contents(engine: &MountEngine, handle: HandleId) -> Vec<u8> {
    let mut output = vec![0; engine.len(handle).unwrap() as usize + 1];
    let read = engine.read(handle, 0, &mut output).unwrap();
    output.truncate(read);
    output
}

fn read_path(engine: &MountEngine, path: &str) -> Vec<u8> {
    let handle = open(engine, path, false);
    let bytes = contents(engine, handle);
    engine.close(handle).unwrap();
    bytes
}

fn open_error(engine: &MountEngine, path: &str) -> io::Error {
    engine.open_file(path, OpenFileOptions {
        writable: false, disposition: OpenDisposition::OpenExisting,
    }).err().expect("open must fail")
}

#[test]
fn mount_optimization_task_reopen_reuses_contents_but_revalidates() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"first");
    let engine = new_engine(&directory, &backend, 500);
    let lazy = engine.open_metadata_file("\\note", backend.stat("/note").unwrap(), false).unwrap();
    assert_eq!(backend.read_count(), 0, "metadata-only admission must not download");
    engine.close(lazy).unwrap();
    assert_eq!(read_path(&engine, "\\note"), b"first");
    let initial_stats = backend.stat_count();
    assert_eq!(read_path(&engine, "\\note"), b"first");
    assert_eq!(backend.read_count(), 1);
    assert_eq!(backend.stat_count(), initial_stats + 1, "idle reuse requires a fresh stat");
    eprintln!("optimization cache: cold+reopen downloads={} bytes={} reuse_stats={}",
        backend.read_count(), backend.read_byte_count(), backend.stat_count() - initial_stats);
    backend.put("/note", b"second revision");
    assert_eq!(read_path(&engine, "\\note"), b"second revision");
    assert_eq!(backend.read_count(), 2);
    backend.remove_file("/note").unwrap();
    assert_eq!(open_error(&engine, "\\note").kind(), io::ErrorKind::NotFound);
    backend.mkdir("/note");
    assert_eq!(open_error(&engine, "\\note").kind(), io::ErrorKind::InvalidInput);
    backend.make_link("/link");
    assert_eq!(open_error(&engine, "\\link").kind(), io::ErrorKind::InvalidInput);
    assert_eq!(backend.read_count(), 2, "missing/type/link failures must not transfer content");
}

#[test]
fn mount_optimization_task_short_and_overlong_streams_never_enter_cache() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"abc");
    let engine = new_engine(&directory, &backend, 500);
    for invalid in [b"a".to_vec(), b"abcd-and-more".to_vec()] {
        let expected_read = invalid.len().min(4);
        let before = backend.read_byte_count();
        backend.override_next_read(invalid);
        assert_eq!(open_error(&engine, "\\note").kind(), io::ErrorKind::InvalidData);
        assert_eq!(backend.read_byte_count() - before, expected_read,
            "transfer stops at the advertised length plus one detection byte");
        assert_eq!(engine.clean_cache.usage().unwrap(), (0, 0));
        assert!(lock(&engine.entries).unwrap().is_empty());
    }
    assert_eq!(read_path(&engine, "\\note"), b"abc");
    assert_eq!(backend.read_count(), 3);
}

#[test]
fn mount_optimization_task_reuse_rejects_a_changed_local_spool_length() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"complete");
    let engine = new_engine(&directory, &backend, 500);
    assert_eq!(read_path(&engine, "\\note"), b"complete");
    let record = engine.clean_cache.claim("/note").unwrap().unwrap();
    engine.spool.open_file(&record.spool_name, true).unwrap().set_len(1).unwrap();
    engine.clean_cache.retain(record).unwrap();
    assert_eq!(read_path(&engine, "\\note"), b"complete");
    assert_eq!(backend.read_count(), 2, "a truncated retained spool is never reused");
}

#[test]
fn mount_optimization_task_pins_bridge_close_and_reopen_without_double_disposal() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"pinned bytes");
    let engine = new_engine(&directory, &backend, 500);
    let acquisition = engine.materialize_at("\\note", OpenDisposition::OpenExisting).unwrap();
    engine.maintain_cache().unwrap();
    assert!(!engine.evict_clean("\\note").unwrap(), "unbound acquisition is pinned");
    let first = open(&engine, "\\note", false);
    drop(acquisition);
    let operation = engine.handle(first).unwrap();
    engine.close(first).unwrap();
    assert!(!engine.evict_clean("\\note").unwrap(), "cloned in-flight operation is pinned");
    let second = open(&engine, "\\note", false);
    assert_eq!(contents(&engine, second), b"pinned bytes");
    let old_entry = match operation.kind {
        OpenHandleKind::Materialized(pin) => pin.release(),
        _ => panic!("expected materialized operation"),
    };
    engine.close(second).unwrap();
    let third = open(&engine, "\\note", false);
    {
        // Deterministically replay the delayed-close Arc after a new Entry has
        // adopted the same retained spool. This must be an idempotent no-op.
        let _namespace = write_lock(&engine.namespace).unwrap();
        engine.cleanup_committed_entry(&old_entry).unwrap();
    }
    assert_eq!(contents(&engine, third), b"pinned bytes");
    assert_eq!(backend.read_count(), 1);
    engine.close(third).unwrap();
}

#[test]
fn mount_optimization_task_post_transfer_stat_rejects_remote_change() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let engine = Arc::new(new_engine(&directory, &backend, 500));
    let gate = backend.gate_stat("/note", 0); // First baseline captured, transfer not started.
    let (done_tx, done) = mpsc::channel();
    let worker_engine = Arc::clone(&engine);
    let worker = std::thread::spawn(move || {
        done_tx.send(open_error(&worker_engine, "\\note").kind()).unwrap();
    });
    gate.wait();
    backend.put("/note", b"new"); // Same length and object ID, but a newer modification time.
    gate.release();
    let result = done.recv_timeout(Duration::from_secs(10)).expect("changed acquisition finished");
    worker.join().unwrap();
    assert_eq!(result, io::ErrorKind::WouldBlock);
    assert!(lock(&engine.entries).unwrap().is_empty());
    assert_eq!(engine.clean_cache.usage().unwrap(), (0, 0));
    assert_eq!(read_path(&engine, "\\note"), b"new");
}

#[test]
fn mount_optimization_task_namespace_change_after_verification_rejects_install() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"captured");
    let engine = Arc::new(new_engine(&directory, &backend, 500));
    let gate = backend.gate_stat("/note", 1); // Pause the post-download stat, after snapshotting it.
    let (done_tx, done) = mpsc::channel();
    let worker_engine = Arc::clone(&engine);
    let worker = std::thread::spawn(move || {
        let result = worker_engine.open_file("\\note", OpenFileOptions {
            writable: false, disposition: OpenDisposition::OpenExisting,
        });
        done_tx.send(result.map(|_| ()).map_err(|error| error.kind())).unwrap();
    });
    gate.wait();
    assert!(matches!(engine.rename("\\note", "\\renamed", false).unwrap(), RenameOutcome::Complete));
    gate.release();
    let result = done.recv_timeout(Duration::from_secs(10)).expect("content acquisition finished");
    worker.join().unwrap();
    assert_eq!(result, Err(io::ErrorKind::WouldBlock));
    assert!(!lock(&engine.entries).unwrap().contains_key("/note"));
    assert_eq!(backend.bytes("/renamed"), b"captured");
}

#[test]
fn mount_optimization_task_failed_upload_and_restart_preserve_dirty_bytes() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let mounted = new_engine(&directory, &backend, 0);
    let handle = open(&mounted, "\\note", true);
    mounted.write(handle, 0, b"new").unwrap();
    mounted.close(handle).unwrap();
    assert!(!mounted.evict_clean("\\note").unwrap());
    backend.fail_upload(true);
    assert_eq!(mounted.flush_path("\\note").unwrap_err().kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(backend.bytes("/note"), b"old");
    assert_eq!(mounted.dirty_entries().unwrap().len(), 1);
    drop(mounted);
    let recovered = new_engine(&directory, &backend, 0);
    assert_eq!(read_path(&recovered, "\\note"), b"new");
    assert!(!recovered.evict_clean("\\note").unwrap());
    assert!(recovered.retry_pending_changes().is_err());
    backend.fail_upload(false);
    recovered.retry_pending_changes().unwrap();
    assert_eq!(backend.bytes("/note"), b"new");
    assert!(recovered.dirty_entries().unwrap().is_empty());
    recovered.maintain_cache().unwrap();
    assert_eq!(recovered.clean_cache.usage().unwrap(), (0, 0));
}

#[test]
fn mount_optimization_task_conflict_and_delete_pending_are_not_disposable() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    backend.put("/delete", b"keep until commit");
    let engine = new_engine(&directory, &backend, 0);
    let handle = open(&engine, "\\note", true);
    engine.write(handle, 0, b"new").unwrap();
    backend.put("/note", b"external");
    assert!(matches!(engine.flush(handle).unwrap(), FlushOutcome::Conflict(_)));
    engine.close(handle).unwrap();
    assert!(!engine.evict_clean("\\note").unwrap());
    assert_eq!(read_path(&engine, "\\note"), b"new");
    assert!(engine.dirty_entries().unwrap().iter()
        .any(|(_, condition)| matches!(condition, EntryCondition::Conflict(_))));
    let pinned = engine.materialize_at("\\delete", OpenDisposition::OpenExisting).unwrap();
    let token = engine.begin_delete("\\delete", false).unwrap();
    drop(pinned);
    engine.maintain_cache().unwrap();
    assert!(!engine.evict_clean("\\delete").unwrap());
    engine.cancel_delete(token).unwrap();
    assert_eq!(backend.bytes("/delete"), b"keep until commit");
}

struct DiskBudget { files: PathBuf, capacity: AtomicU64 }
impl CacheSpaceProbe for DiskBudget {
    fn available_bytes(&self) -> io::Result<u64> {
        let mut used = 0u64;
        for entry in std::fs::read_dir(&self.files)? {
            used += entry?.metadata()?.len();
        }
        Ok(self.capacity.load(Ordering::SeqCst).saturating_sub(used))
    }
}

#[test]
fn mount_optimization_task_space_reclaims_clean_but_never_unsaved_data() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/idle", &vec![1; 700]);
    backend.put("/active", &vec![2; 200]);
    let mounted = new_engine(&directory, &backend, 500);
    let probe = Arc::new(DiskBudget {
        files: directory.path().join("cache-task/files"),
        capacity: AtomicU64::new(CACHE_RESERVE_BYTES + 1000),
    });
    let engine = mounted.with_cache_space_probe(probe.clone());
    assert_eq!(read_path(&engine, "\\idle").len(), 700);
    probe.capacity.store(CACHE_RESERVE_BYTES + 400, Ordering::SeqCst);
    let active = open(&engine, "\\active", true);
    assert_eq!(engine.clean_cache.usage().unwrap(), (0, 0), "growth reclaimed idle bytes first");
    probe.capacity.store(CACHE_RESERVE_BYTES + 200, Ordering::SeqCst);
    assert_eq!(engine.write(active, 200, b"x").unwrap_err().kind(), io::ErrorKind::StorageFull);
    assert_eq!(contents(&engine, active), vec![2; 200]);
    probe.capacity.store(0, Ordering::SeqCst);
    engine.write(active, 0, b"XYZ").unwrap();
    engine.truncate(active, 3).unwrap();
    assert_eq!(contents(&engine, active), b"XYZ");
    assert!(!engine.evict_clean("\\active").unwrap());
    engine.close(active).unwrap();
    assert!(!engine.evict_clean("\\active").unwrap());
    assert_eq!(backend.bytes("/active"), vec![2; 200]);
}

#[test]
fn mount_optimization_task_pending_growth_is_reserved_between_callers() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    let mounted = new_engine(&directory, &backend, 500);
    let probe = Arc::new(DiskBudget {
        files: directory.path().join("cache-task/files"),
        capacity: AtomicU64::new(CACHE_RESERVE_BYTES + 8),
    });
    let engine = mounted.with_cache_space_probe(probe);
    let first = engine.reserve_growth(6).unwrap();
    let rejected = engine.reserve_growth(3).err().expect("concurrent growth must count pending bytes");
    assert_eq!(rejected.kind(), io::ErrorKind::StorageFull);
    drop(first);
    let accepted = engine.reserve_growth(3).unwrap();
    drop(accepted);
}

#[test]
fn mount_optimization_task_idle_byte_limits_lru_zero_and_generation_age() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/a", &vec![1; 600 * 1024]);
    backend.put("/b", &vec![2; 600 * 1024]);
    let engine = new_engine(&directory, &backend, 1);
    read_path(&engine, "\\a");
    read_path(&engine, "\\b");
    assert_eq!(engine.clean_cache.usage().unwrap(), (1, 600 * 1024));
    read_path(&engine, "\\b");
    assert_eq!(backend.read_count(), 2, "most recently used file remains retained");
    read_path(&engine, "\\a");
    assert_eq!(backend.read_count(), 3, "least-recently used file was evicted");
    let mut old = engine.clean_cache.claim("/a").unwrap().unwrap();
    old.created = Instant::now().checked_sub(MAX_CONTENT_AGE + Duration::from_secs(1)).unwrap();
    engine.clean_cache.retain(old).unwrap();
    read_path(&engine, "\\a");
    assert_eq!(backend.read_count(), 4, "unchanged weak metadata cannot renew expired content forever");
    drop(engine);
    let disabled = new_engine(&directory, &backend, 0);
    read_path(&disabled, "\\a");
    read_path(&disabled, "\\a");
    assert_eq!(backend.read_count(), 6);
    assert_eq!(disabled.clean_cache.usage().unwrap(), (0, 0));
}

#[test]
fn mount_optimization_task_idle_record_cap_and_failed_disposal_accounting() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    let engine = new_engine(&directory, &backend, 500);
    // Model only the cache-index boundary: absent spool names are safe to reap.
    // This checks the record bound without creating ten thousand disk files.
    for index in 0..=MAX_IDLE_RECORDS {
        let path = format!("/record-{index}");
        engine.clean_cache.retain(IdleClean::new(path.clone(), path,
            format!("{index:032x}.spool"), Baseline::Missing, 0, Instant::now())).unwrap();
    }
    engine.clean_cache.trim(&engine.spool, 500 * 1024 * 1024).unwrap();
    assert_eq!(engine.clean_cache.usage().unwrap(), (MAX_IDLE_RECORDS, 0));
    engine.clean_cache.trim(&engine.spool, 0).unwrap();
    backend.put("/note", b"keep");
    read_path(&engine, "\\note");
    let record = engine.clean_cache.claim("/note").unwrap().unwrap();
    let path = directory.path().join("cache-task/files").join(&record.spool_name);
    engine.clean_cache.retain(record).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap(); // Portable non-file rejection, independent of admin permissions.
    assert_eq!(engine.clean_cache.trim(&engine.spool, 0).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(engine.clean_cache.usage().unwrap(), (1, 4));
    std::fs::remove_dir(&path).unwrap();
    engine.clean_cache.trim(&engine.spool, 0).unwrap();
    assert_eq!(engine.clean_cache.usage().unwrap(), (0, 0));
}

#[test]
fn mount_optimization_task_lazy_destination_survives_atomic_replace() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/destination", b"old object");
    backend.put("/source", b"replacement");
    let engine = new_engine(&directory, &backend, 500);
    let old = engine.open_metadata_file("\\destination", backend.stat("/destination").unwrap(), true).unwrap();
    assert_eq!(backend.read_count(), 0);
    assert!(matches!(engine.rename_with_shared_destination("\\source", "\\destination", true, true).unwrap(),
        RenameOutcome::Complete));
    assert_eq!(contents(&engine, old), b"old object");
    assert_eq!(read_path(&engine, "\\destination"), b"replacement");
    engine.write(old, 0, b"OLD").unwrap();
    assert!(matches!(engine.flush(old).unwrap(), FlushOutcome::NoChanges));
    assert_eq!(backend.bytes("/destination"), b"replacement");
    engine.close(old).unwrap();
}

#[test]
fn mount_optimization_task_other_directory_listing_does_not_wait_for_upload() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/a/note", b"old");
    backend.put("/b/other", b"other");
    let engine = Arc::new(new_engine(&directory, &backend, 500));
    let handle = open(&engine, "\\a\\note", true);
    engine.write(handle, 0, b"new").unwrap();
    let gate = backend.gate_stat("/a/note", 0);
    let (flush_tx, flush_rx) = mpsc::channel();
    let flushing = Arc::clone(&engine);
    let flusher = std::thread::spawn(move || { flush_tx.send(flushing.flush(handle)).unwrap(); });
    gate.wait();
    let (list_tx, list_rx) = mpsc::channel();
    let listing = Arc::clone(&engine);
    let lister = std::thread::spawn(move || { list_tx.send(listing.list_dir("\\b")).unwrap(); });
    let listed = list_rx.recv_timeout(Duration::from_secs(5));
    gate.release();
    let flushed = flush_rx.recv_timeout(Duration::from_secs(10)).expect("upload completed after release");
    flusher.join().unwrap();
    lister.join().unwrap();
    assert!(matches!(flushed.unwrap(), FlushOutcome::Committed));
    let listed = listed.expect("unrelated listing completed while upload remained paused").unwrap();
    assert_eq!(listed.iter().map(|meta| meta.name.as_str()).collect::<Vec<_>>(), ["other"]);
    engine.close(handle).unwrap();
}
