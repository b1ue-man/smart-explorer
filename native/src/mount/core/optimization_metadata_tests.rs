//! Focused M3 cases, selected only by the one remote mount task entrypoint.
use super::metadata_cache::{Admission, DirectoryObservation, MetadataCache, MetadataChange,
    MetadataLookup, MAX_CACHED_ENTRIES, run_metadata_batch};
use super::metadata_point_cache::MetadataPointCache;
use super::{engine::MountEngine, optimization_fixture::OptimizationBackend,
    types::{MountId, MountMode, MountRuntimeConfig}};
use crate::vfs::{BackendHandle, VfsMeta};
use std::{collections::HashSet, io, sync::{Arc, Condvar, Mutex, atomic::{AtomicUsize, Ordering}},
    time::{Duration, Instant}};

fn directory(name: &str) -> VfsMeta {
    VfsMeta { name: name.into(), is_dir: true, ..VfsMeta::default() }
}

fn file(name: &str, generation: i64) -> VfsMeta {
    VfsMeta { name: name.into(), size: 4, mtime_ms: generation,
        id: Some(format!("object-{name}")), ..VfsMeta::default() }
}

fn observe(entries: Vec<VfsMeta>, expires: Instant) -> DirectoryObservation {
    DirectoryObservation { metadata: directory(""), metadata_expires_at: expires,
        entries: entries.into(), listing_expires_at: expires }
}

fn rendezvous(wave: &(Mutex<usize>, Condvar), participants: usize) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut arrived = wave.0.lock().map_err(|_| io::Error::other("metadata rendezvous poisoned"))?;
    *arrived += 1;
    wave.1.notify_all();
    while *arrived < participants {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut,
                "independent metadata workers did not overlap"));
        }
        arrived = wave.1.wait_timeout(arrived, remaining)
            .map_err(|_| io::Error::other("metadata rendezvous poisoned"))?.0;
    }
    Ok(())
}

#[test]
fn mount_optimization_task_metadata_authority_and_revision() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let past = Instant::now() - Duration::from_secs(1);
    let future = Instant::now() + Duration::from_secs(20);
    assert!(cache.install_observation("/", observe(Vec::new(), past),
        0, None, Admission::Demand)?);
    assert!(matches!(cache.stat("/missing")?, MetadataLookup::Uncached));
    assert!(cache.directory("/")?.is_none());
    // A fresh child remains usable without inheriting expired parent absence.
    assert!(cache.install_observation("/folder", observe(vec![file("note.md", 1)], future),
        1, None, Admission::Demand)?);
    assert!(matches!(cache.stat("/folder/note.md")?, MetadataLookup::Found(_)));
    assert!(matches!(cache.stat("/folder/absent")?, MetadataLookup::KnownMissing));
    // Expired images are retained only for a concrete future change diff.
    assert!(cache.install_observation("/folder", observe(vec![file("note.md", 2)], past),
        1, None, Admission::Demand)?);
    assert!(matches!(cache.stat("/folder/note.md")?, MetadataLookup::Uncached));
    assert!(cache.drain_changes(20)?.iter().any(|change| matches!(change,
        MetadataChange::Modified { path } if path == "/folder/note.md")));

    let slot = cache.load_slot("/folder")?;
    let revision = slot.revision();
    assert!(cache.install_observation("/folder", observe(vec![file("new.md", 3)], future),
        1, Some((&slot, revision)), Admission::Demand)?);
    assert!(!cache.install_observation("/folder", observe(vec![file("stale.md", 1)], future),
        1, Some((&slot, revision)), Admission::Refresh)?);
    assert!(matches!(cache.stat("/folder/new.md")?, MetadataLookup::Found(_)));
    let changes = cache.drain_changes(20)?;
    assert!(!changes.iter().any(|change| change.path() == "/folder/stale.md"));
    cache.note_path_observation("/folder/new.md")?;
    assert!(cache.directory("/folder")?.is_none());
    assert!(cache.install_observation("/folder", observe(vec![file("new.md", 4)], future),
        1, None, Admission::Refresh)?);
    assert_eq!(cache.drain_changes(20)?, vec![MetadataChange::Modified {
        path: "/folder/new.md".into() }]);

    // Refreshing child entries must not renew an older directory-metadata hint.
    let hints = MetadataCache::new("/", true);
    let mut listing = observe(vec![file("leaf", 1)], future);
    listing.metadata_expires_at = past;
    assert!(hints.install_observation("/", listing, 0, None, Admission::Demand)?);
    assert!(hints.metadata_hint("/")?.is_none());
    assert!(hints.directory("/")?.is_some());
    assert!(matches!(hints.stat("/leaf")?, MetadataLookup::Found(_)));

    // A short missing point expires independently of positive point authority.
    let points = MetadataPointCache::new(true);
    points.install("/present", file("present", 1))?;
    points.install_missing("/missing")?;
    assert!(matches!(points.lookup("/missing")?, MetadataLookup::KnownMissing));
    std::thread::sleep(Duration::from_millis(1_100));
    assert!(matches!(points.lookup("/missing")?, MetadataLookup::Uncached));
    assert!(matches!(points.lookup("/present")?, MetadataLookup::Found(_)));
    points.invalidate("/present", false)?;
    assert!(matches!(points.lookup("/present")?, MetadataLookup::Uncached));
    Ok(())
}

#[test]
fn mount_optimization_task_change_queue_preserves_concrete_events() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let future = Instant::now() + Duration::from_secs(20);
    // A name replacement drains as delete then create, even across calls.
    assert!(cache.install_observation("/", observe(vec![file("Old.md", 1)], future),
        0, None, Admission::Demand)?);
    assert!(cache.install_observation("/", observe(vec![file("New.md", 2)], future),
        0, None, Admission::Refresh)?);
    assert_eq!(cache.drain_changes(1)?, vec![MetadataChange::Deleted {
        path: "/Old.md".into(), is_directory: false }]);
    assert!(cache.install_observation("/", observe(vec![file("Newest.md", 3)], future),
        0, None, Admission::Refresh)?);
    assert_eq!(cache.drain_changes(20)?, vec![
        MetadataChange::Created { path: "/New.md".into(), is_directory: false },
        MetadataChange::Deleted { path: "/New.md".into(), is_directory: false },
        MetadataChange::Created { path: "/Newest.md".into(), is_directory: false },
    ]);

    // Queue backpressure cannot silently publish a snapshot whose notification
    // is lost. The old image must survive until there is room for its diff.
    // Use an independent, truthful root image: /q* cannot be admitted beneath
    // the preceding fresh root that listed only /Newest.md.
    let cache = MetadataCache::new("/", true);
    assert!(cache.install_observation("/", observe((0..65)
        .map(|number| directory(&format!("q{number}"))).collect(), future),
        0, None, Admission::Demand)?);
    assert!(cache.drain_changes(20)?.is_empty());
    for number in 0..65 {
        let path = format!("/q{number}");
        assert!(cache.install_observation(&path, observe(vec![file("note.md", 1)], future),
            1, None, Admission::Demand)?);
        let accepted = cache.install_observation(&path,
            observe(vec![file("note.md", 2)], future), 1, None, Admission::Refresh)?;
        assert_eq!(accepted, number < 64);
    }
    assert!(cache.directory("/q64")?.is_some());
    let MetadataLookup::Found(old) = cache.stat("/q64/note.md")? else {
        panic!("queue pressure discarded its previous comparison image");
    };
    assert_eq!(old.mtime_ms, 1);
    let drained = cache.drain_changes(usize::MAX)?;
    assert_eq!(drained.len(), 64);
    assert!(drained.iter().all(|change| matches!(change, MetadataChange::Modified { .. })));
    assert!(cache.install_observation("/q64", observe(vec![file("note.md", 2)], future),
        1, None, Admission::Refresh)?);
    assert_eq!(cache.drain_changes(20)?, vec![MetadataChange::Modified {
        path: "/q64/note.md".into() }]);
    Ok(())
}

#[test]
fn mount_optimization_task_metadata_parallelism_and_ancestor_waves() -> io::Result<()> {
    let active = AtomicUsize::new(0);
    let maximum = AtomicUsize::new(0);
    let wave = (Mutex::new(0), Condvar::new());
    let parent_finished = AtomicUsize::new(0);
    let log = Mutex::new(Vec::new());
    let targets = vec![("/a/child".into(), 2), ("/d".into(), 1),
        ("/c".into(), 1), ("/a".into(), 1), ("/b".into(), 1)];
    let count = run_metadata_batch(targets, 4, &|| false, &|path, depth| {
        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
        maximum.fetch_max(current, Ordering::SeqCst);
        if depth == 1 {
            rendezvous(&wave, 4)?;
            parent_finished.fetch_add(1, Ordering::SeqCst);
        } else {
            assert_eq!(parent_finished.load(Ordering::SeqCst), 4);
        }
        log.lock().unwrap().push(path.to_string());
        active.fetch_sub(1, Ordering::SeqCst);
        Ok(true)
    })?;
    assert_eq!(count, 5);
    assert_eq!(maximum.load(Ordering::SeqCst), 4);
    assert_eq!(log.lock().unwrap().last().map(String::as_str), Some("/a/child"));
    let calls = AtomicUsize::new(0);
    assert_eq!(run_metadata_batch(vec![("/cancelled".into(), 0)], 4, &|| true, &|_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    })?, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // Worker failure is returned after every started sibling has been joined.
    let joined = Arc::new(AtomicUsize::new(0));
    assert!(run_metadata_batch(vec![("/a".into(), 1), ("/b".into(), 1)], 2,
        &|| false, &|path, _| {
            joined.fetch_add(1, Ordering::SeqCst);
            if path == "/a" { Err(io::Error::other("injected metadata error")) } else { Ok(true) }
        }).is_err());
    assert_eq!(joined.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn mount_optimization_task_speculative_admission_preserves_demand() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let future = Instant::now() + Duration::from_secs(20);
    assert!(cache.install_observation("/",
        observe(vec![directory("demand"), directory("speculative")], future),
        0, None, Admission::Demand)?);
    // Three root records plus this directory's own record leave exactly this
    // many file records. Valid short names hit the entry cap, not a byte/type
    // validation failure, without depending on platform struct sizes.
    let files = MAX_CACHED_ENTRIES - 4;
    assert!(cache.install_observation("/demand", observe((0..files)
        .map(|number| file(&format!("f{number}"), 1)).collect(), future),
        1, None, Admission::Demand)?);
    let before = cache.usage()?;
    assert_eq!(before.1, MAX_CACHED_ENTRIES);
    let demand_revision = cache.revision("/demand")?;
    let descendant = cache.load_slot("/speculative/note.md")?;
    let descendant_revision = descendant.revision();
    assert!(!cache.install_observation("/speculative", observe(vec![file("note.md", 1)], future),
        1, None, Admission::Speculative)?);
    assert_eq!(cache.usage()?, before);
    assert_eq!(cache.revision("/demand")?, demand_revision);
    assert_eq!(descendant.revision(), descendant_revision);
    assert_eq!(cache.directory("/demand")?.map(|entries| entries.len()), Some(files));
    assert!(cache.directory("/speculative")?.is_none());
    assert!(cache.preload_targets(2, 8)?.is_empty());
    assert!(cache.drain_changes(20)?.is_empty());
    Ok(())
}

#[test]
fn mount_optimization_task_failed_refresh_attempts_remain_fair() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let future = Instant::now() + Duration::from_secs(20);
    let names = ["a", "b", "c", "hot-a", "hot-b"];
    assert!(cache.install_observation("/", observe(names.iter()
        .map(|name| directory(name)).collect(), future), 0, None, Admission::Demand)?);
    for name in names {
        assert!(cache.install_observation(&format!("/{name}"),
            observe(vec![file("note.md", 1)], future), 1, None, Admission::Demand)?);
    }
    let attempted = Mutex::new(HashSet::new());
    for _ in 0..3 {
        cache.mark_directory_access("/hot-a")?;
        cache.mark_directory_access("/hot-b")?;
        let targets = cache.refresh_targets(3, true)?;
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0], ("/".into(), 0));
        // Every selected fetch fails; no install advances successful freshness.
        // Nevertheless, the reserved oldest-attempt position must rotate.
        assert!(run_metadata_batch(targets, 2, &|| false, &|path, _| {
            attempted.lock().unwrap().insert(path.to_string());
            Err(io::Error::other("injected refresh failure"))
        }).is_err());
    }
    let attempted = attempted.lock().unwrap();
    for path in ["/a", "/b", "/c"] {
        assert!(attempted.contains(path), "failed refreshes starved {path}");
        let MetadataLookup::Found(meta) = cache.stat(&format!("{path}/note.md"))? else {
            panic!("failed refresh discarded its prior snapshot");
        };
        assert_eq!(meta.mtime_ms, 1);
    }
    assert!(cache.drain_changes(20)?.is_empty());
    Ok(())
}

#[test]
fn mount_optimization_task_point_stat_coalescence_and_error_policy() -> io::Result<()> {
    let spool = tempfile::tempdir()?;
    let backend = OptimizationBackend::new();
    let backend_handle: BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(MountRuntimeConfig::new(
        MountId::parse("metadata-stat-cache")?, MountMode::ReadOnly),
        backend_handle, spool.path())?;
    let before = backend.stat_count();
    let wave = (Mutex::new(0), Condvar::new());
    std::thread::scope(|scope| -> io::Result<()> {
        // Keep the release guard inside the scope: a spawn failure or assertion
        // must release the backend before scope teardown joins existing workers.
        let gate = backend.gate_stat("/missing", 0);
        let mut workers = Vec::new();
        for _ in 0..2 {
            workers.push(std::thread::Builder::new().spawn_scoped(scope, || {
                rendezvous(&wave, 2)?;
                engine.cached_remote_stat("/missing")
            })?);
        }
        gate.wait();
        gate.release();
        let results = workers.into_iter().map(|worker| worker.join()).collect::<Vec<_>>();
        for result in results {
            let result = result.map_err(|_| io::Error::other("point stat worker panicked"))?;
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
        }
        Ok(())
    })?;
    assert_eq!(backend.stat_count(), before + 1);
    assert_eq!(engine.cached_remote_stat("/missing").unwrap_err().kind(),
        io::ErrorKind::NotFound);
    assert_eq!(backend.stat_count(), before + 1);

    // Explicit local invalidation clears the negative point immediately; the
    // next positive result is itself reused without an additional backend stat.
    backend.put("/missing", b"body");
    engine.invalidate_metadata("/missing", false);
    assert_eq!(engine.cached_remote_stat("/missing")?.size, 4);
    assert_eq!(engine.cached_remote_stat("/missing")?.size, 4);
    assert_eq!(backend.stat_count(), before + 2);

    // Exercise this backend-facing cache directly. The in-memory fixture
    // rejects the invalid path lexically; no host filesystem path is traversed.
    for attempt in 1..=2 {
        assert_eq!(engine.cached_remote_stat("/bad/../path").unwrap_err().kind(),
            io::ErrorKind::InvalidInput);
        assert_eq!(backend.stat_count(), before + 2 + attempt);
    }
    assert!(engine.drain_metadata_changes(20)?.is_empty());
    Ok(())
}
