//! Demand-flight and sibling-stat acceptance; no remote volume is required here.
use super::{LoadSlot, MetadataCache, MetadataLookup, MAX_CACHED_BYTES};
use super::vault_task_tests::file;
use crate::mount::{engine::MountEngine, optimization_fixture::OptimizationBackend,
    MountId, MountMetadataPolicy, MountMode, MountRuntimeConfig};
use crate::vfs::{Backend, BackendHandle, RootConfinement, Scheme, VfsMeta};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::{Duration, Instant};

struct CountingBackend {
    base: Arc<OptimizationBackend>,
    lists: AtomicUsize,
    deny_listing: AtomicBool,
    wide: AtomicUsize,
    over_retention: AtomicBool,
    listing: Mutex<Option<Vec<VfsMeta>>>,
}

impl CountingBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self { base: OptimizationBackend::new(), lists: AtomicUsize::new(0),
            deny_listing: AtomicBool::new(false), wide: AtomicUsize::new(0),
            over_retention: AtomicBool::new(false), listing: Mutex::new(None) })
    }
    fn lists(&self) -> usize { self.lists.load(Ordering::SeqCst) }
}

impl Backend for CountingBackend {
    fn scheme(&self) -> Scheme { self.base.scheme() }
    fn root_display(&self) -> String { self.base.root_display() }
    fn parallelism(&self) -> usize { 8 }
    fn case_sensitive_paths(&self, path: &str) -> bool { self.base.case_sensitive_paths(path) }
    fn root_confinement(&self, path: &str) -> RootConfinement { self.base.root_confinement(path) }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> { self.base.stat(path) }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        if self.deny_listing.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture listing denied"));
        }
        if let Some(entries) = self.listing.lock().unwrap().as_ref() {
            return Ok(entries.clone());
        }
        let count = self.wide.load(Ordering::SeqCst);
        let mut entries = if count > 0 {
            (0..count).map(|index| file(&format!("note-{index:05}.md"), 1)).collect()
        } else {
            self.base.list_dir(path)?
        };
        if self.over_retention.load(Ordering::SeqCst) {
            // A small valid value with a large retained allocation proves that
            // capacity, not String::len(), is charged. No 128 MiB string fill,
            // metadata cloning or large notification-pressure image is needed.
            let mut identity = String::new();
            identity.try_reserve_exact(MAX_CACHED_BYTES + 1)
                .map_err(|error| io::Error::other(error.to_string()))?;
            identity.push('x');
            entries[0].id = Some(identity);
        }
        Ok(entries)
    }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> { self.base.open_read(path) }
    fn open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> { self.base.open_write(path) }
    fn rename(&self, from: &str, to: &str) -> io::Result<()> { self.base.rename(from, to) }
    fn remove_file(&self, path: &str) -> io::Result<()> { self.base.remove_file(path) }
    fn remove_dir(&self, path: &str) -> io::Result<()> { self.base.remove_dir(path) }
    fn mkdir_all(&self, path: &str) -> io::Result<()> { self.base.mkdir_all(path) }
}

fn engine(backend: Arc<CountingBackend>, root: &std::path::Path) -> io::Result<MountEngine> {
    let backend: BackendHandle = backend;
    MountEngine::open_host_cache(MountRuntimeConfig::new(
        MountId::parse("vault-metadata-fixture")?, MountMode::ReadOnly)
        .with_metadata_policy(MountMetadataPolicy::new(0)?), backend, root)
}

fn wait_for_owners(slot: &Arc<LoadSlot>, owners: usize) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Arc::strong_count(slot) < owners {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut,
                "fixture callers did not acquire the same directory flight"));
        }
        std::thread::yield_now();
    }
    Ok(())
}

fn directory_wave(engine: &MountEngine, count: usize) -> io::Result<Vec<io::Result<Arc<[VfsMeta]>>>> {
    let slot = engine.metadata_cache.load_slot("/")?;
    std::thread::scope(|scope| {
        // Teardown drops this guard before scope auto-join, including on a
        // spawn/arrival error. Every caller owns the slot before it is released.
        let guard = slot.lock()?;
        let mut workers = Vec::new();
        for _ in 0..count {
            workers.push(std::thread::Builder::new().spawn_scoped(scope,
                || engine.cached_remote_directory("/", 0))?);
        }
        wait_for_owners(&slot, count + 1)?;
        drop(guard);
        let joined = workers.into_iter().map(|worker| worker.join()).collect::<Vec<_>>();
        joined.into_iter().map(|result| result
            .map_err(|_| io::Error::other("directory fixture worker panicked"))).collect()
    })
}

#[test]
fn mount_vault_task_50001_valid_children_enumerate_and_reuse_snapshot() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = CountingBackend::new();
    backend.wide.store(50_001, Ordering::SeqCst);
    let engine = engine(backend.clone(), temporary.path())?;
    let first = engine.list_dir_cached(r"\")?;
    assert_eq!(first.len(), 50_001);
    assert_eq!(first[50_000].name, "note-50000.md");
    let second = engine.list_dir_cached(r"\")?;
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(backend.lists(), 1);
    assert_eq!(engine.stat_cached(r"\note-50000.md")?.size, 4);
    assert_eq!(backend.base.stat_count(), 1, "only the root required exact stat");
    assert_eq!(backend.base.read_count(), 0);
    Ok(())
}

#[test]
fn mount_vault_task_listing_name_and_collision_safety() -> io::Result<()> {
    for (entries, collision) in [
        (vec![file("same", 1), file("same", 2)], true),
        (vec![file("safe.md", 1), file("../escape", 1), file("", 1), file("bad:name", 1)], false),
    ] {
        let temporary = tempfile::tempdir()?;
        let backend = CountingBackend::new();
        *backend.listing.lock().unwrap() = Some(entries);
        let engine = engine(backend.clone(), temporary.path())?;
        let result = engine.list_dir_cached(r"\");
        if collision {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        } else {
            let entries = result?;
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].name, "safe.md");
        }
        assert_eq!(backend.lists(), 1);
        assert_eq!(backend.base.read_count(), 0);
    }
    Ok(())
}

#[test]
fn mount_vault_task_over_retention_demand_succeeds_and_shares_completed_flight() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = CountingBackend::new();
    backend.base.put("/note.md", b"body");
    backend.over_retention.store(true, Ordering::SeqCst);
    let engine = engine(backend.clone(), temporary.path())?;
    let results = directory_wave(&engine, 8)?;
    let mut images = Vec::new();
    for result in results { images.push(result?); }
    assert_eq!(backend.lists(), 1, "waiters share success even without persistent admission");
    assert!(images.iter().all(|image| image.len() == 1 && image[0].name == "note.md"));
    assert!(images.iter().all(|image| Arc::ptr_eq(image, &images[0])));
    assert!(images[0][0].id.as_ref().unwrap().capacity() > MAX_CACHED_BYTES);
    assert!(engine.metadata_cache.directory("/")?.is_none());
    assert_eq!(engine.metadata_cache.usage()?.0, 0);
    drop(images);
    // No slot owner survives the preceding wave. A new demand must fetch anew,
    // not turn a completed-flight exception into unbounded persistent storage.
    backend.over_retention.store(false, Ordering::SeqCst);
    assert_eq!(engine.list_dir_cached(r"\")?.len(), 1);
    assert_eq!(backend.lists(), 2);
    Ok(())
}

#[test]
fn mount_vault_task_pressure_sharing_does_not_publish_unnotifiable_image() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = CountingBackend::new();
    backend.base.put("/note.md", b"old");
    let engine = engine(backend.clone(), temporary.path())?;
    let old = engine.list_dir_cached(r"\")?;
    let baseline = engine.metadata_cache.revision("/")?;
    engine.metadata_cache.test_change_budget(Some(0))?;
    backend.base.put("/note.md", b"new-body");
    engine.metadata_cache.test_expire_directory("/")?;
    let before = backend.lists();
    for result in directory_wave(&engine, 8)? { assert_eq!(result?[0].size, 8); }
    assert_eq!(backend.lists(), before + 1);
    assert_eq!(engine.metadata_cache.revision("/")?, baseline);
    assert!(engine.metadata_cache.directory("/")?.is_none(), "old image is no longer authoritative");
    assert_eq!(old[0].size, 3);
    assert!(engine.drain_metadata_changes(20)?.is_empty());
    engine.metadata_cache.test_change_budget(None)?;
    assert_eq!(engine.list_dir_cached(r"\")?[0].size, 8);
    assert_eq!(backend.lists(), before + 2);
    assert_eq!(engine.drain_metadata_changes(20)?.len(), 1);
    Ok(())
}

#[test]
fn mount_vault_task_completed_flights_expire_and_reject_invalidated_revisions() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let slot = cache.load_slot("/")?;
    let entries: Arc<[VfsMeta]> = vec![file("note.md", 1)].into();
    let revision = slot.revision();
    slot.complete_directory(revision, Instant::now() - Duration::from_secs(1), entries.clone())?;
    assert!(slot.completed_directory()?.is_none(), "already-expired fetch is never shared");
    slot.complete_directory(revision, Instant::now() + Duration::from_secs(20), entries.clone())?;
    assert!(Arc::ptr_eq(&entries, &slot.completed_directory()?.unwrap()));
    cache.invalidate("/", true)?;
    assert!(slot.completed_directory()?.is_none());
    slot.complete_directory(revision, Instant::now() + Duration::from_secs(20), entries)?;
    assert!(slot.completed_directory()?.is_none(), "an old fetch cannot borrow a new revision");
    let error = io::Error::new(io::ErrorKind::PermissionDenied, "fixture denied");
    slot.complete_directory_failure(slot.revision(), &error)?;
    for _ in 0..2 {
        let shared = slot.completed_directory().unwrap_err();
        assert_eq!(shared.kind(), error.kind());
        assert_eq!(shared.to_string(), error.to_string());
    }
    std::thread::sleep(Duration::from_millis(1_100));
    assert!(slot.completed_directory()?.is_none(), "failure has an absolute one-second lifetime");
    let weak = Arc::downgrade(&slot);
    drop(slot);
    assert!(weak.upgrade().is_none(), "weak registry does not retain completed results");
    Ok(())
}

#[test]
fn mount_vault_task_same_flight_listing_failures_share_without_persistent_error_cache() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = CountingBackend::new();
    backend.deny_listing.store(true, Ordering::SeqCst);
    let engine = engine(backend.clone(), temporary.path())?;
    for result in directory_wave(&engine, 8)? {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
    assert_eq!(backend.lists(), 1);
    backend.deny_listing.store(false, Ordering::SeqCst);
    assert!(engine.list_dir_cached(r"\")?.is_empty());
    assert_eq!(backend.lists(), 2, "a new owner flight immediately retries non-NotFound errors");
    Ok(())
}

#[test]
fn mount_vault_task_expired_parent_stats_share_listing_and_preserve_point_precedence() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = CountingBackend::new();
    for index in 0..8 { backend.base.put(&format!("/f{index}"), b"old"); }
    let engine = engine(backend.clone(), temporary.path())?;
    engine.list_dir_cached(r"\")?;
    backend.base.put("/f0", b"new-body");
    engine.metadata_cache.test_expire_directory("/")?;
    let lists = backend.lists();
    let stats = backend.base.stat_count();
    let slot = engine.metadata_cache.load_slot("/")?;
    std::thread::scope(|scope| -> io::Result<()> {
        let guard = slot.lock()?;
        let mut workers = Vec::new();
        for index in 0..8 {
            let engine = &engine;
            workers.push(std::thread::Builder::new().spawn_scoped(scope,
                move || engine.cached_remote_stat(&format!("/f{index}")))?);
        }
        wait_for_owners(&slot, 9)?;
        drop(guard);
        let joined = workers.into_iter().map(|worker| worker.join()).collect::<Vec<_>>();
        for (index, result) in joined.into_iter().enumerate() {
            let metadata = result.map_err(|_| io::Error::other("stat fixture worker panicked"))??;
            assert_eq!(metadata.size, if index == 0 { 8 } else { 3 });
        }
        Ok(())
    })?;
    assert_eq!(backend.lists(), lists + 1);
    assert_eq!(backend.base.stat_count(), stats + 1, "one root stat, not one exact stat per child");
    drop(slot);
    engine.metadata_cache.test_expire_directory("/")?;
    engine.metadata_points.install("/f0", file("f0", 999))?;
    let lists = backend.lists();
    let stats = backend.base.stat_count();
    assert_eq!(engine.cached_remote_stat("/f0")?.mtime_ms, 999);
    assert_eq!(backend.lists(), lists, "fresh points precede expired parent refresh");
    assert_eq!(backend.base.stat_count(), stats);
    Ok(())
}

#[test]
fn mount_vault_task_expired_parent_listing_denial_falls_back_to_exact_stat() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = CountingBackend::new();
    backend.base.put("/note.md", b"old");
    let engine = engine(backend.clone(), temporary.path())?;
    engine.list_dir_cached(r"\")?;
    backend.base.put("/note.md", b"new-body");
    backend.deny_listing.store(true, Ordering::SeqCst);
    engine.metadata_cache.test_expire_directory("/")?;
    let lists = backend.lists();
    let stats = backend.base.stat_count();
    assert_eq!(engine.cached_remote_stat("/note.md")?.size, 8);
    assert_eq!(backend.lists(), lists + 1);
    assert_eq!(backend.base.stat_count(), stats + 2, "root refresh stat plus exact file fallback");
    assert_eq!(engine.cached_remote_stat("/note.md")?.size, 8);
    assert_eq!(backend.base.stat_count(), stats + 2, "successful fallback point survives its own revision bump");
    assert!(matches!(engine.metadata_points.lookup("/note.md")?, MetadataLookup::Found(_)));
    // An unobserved parent remains an exact-stat-only path, even if listing is denied.
    backend.base.put("/cold/adjacent.md", b"body");
    assert_eq!(engine.cached_remote_stat("/cold/adjacent.md")?.size, 4);
    assert_eq!(backend.lists(), lists + 1);
    Ok(())
}
