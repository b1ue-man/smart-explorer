use super::metadata_cache::MetadataCache;
use super::{
    BackendRoot, DriveSelection, MountConfig, MountEngine, MountId, MountMetadataPolicy, MountMode,
    MountRuntimeConfig, MountSource, DEFAULT_METADATA_PRELOAD_DEPTH, MAX_METADATA_PRELOAD_DEPTH,
};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

struct MetadataBackend {
    metadata: Mutex<HashMap<String, VfsMeta>>,
    listings: Mutex<HashMap<String, Vec<VfsMeta>>>,
    stats: AtomicUsize,
    lists: AtomicUsize,
    list_paths: Mutex<Vec<String>>,
    list_delay: Duration,
}

impl MetadataBackend {
    fn tree(list_delay: Duration) -> Arc<Self> {
        let root = directory("/");
        let alpha = directory("Alpha");
        let nested = directory("Nested");
        let note = file("note.md", 12);
        Arc::new(Self {
            metadata: Mutex::new(HashMap::from([
                ("/".into(), root),
                ("/Alpha".into(), alpha.clone()),
                ("/Alpha/Nested".into(), nested.clone()),
                ("/Alpha/note.md".into(), note.clone()),
            ])),
            listings: Mutex::new(HashMap::from([
                (
                    "/".into(),
                    vec![
                        alpha,
                        file("CON", 1),
                        file("draft.se-mount-0123456789abcdef", 1),
                        file("old.se-mount-delete-fedcba9876543210", 1),
                        file("visible.se-mount-deadbeef", 1),
                        file(".se-mount-0123456789abcdef", 1),
                    ],
                ),
                ("/Alpha".into(), vec![nested, note]),
                ("/Alpha/Nested".into(), Vec::new()),
            ])),
            stats: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            list_paths: Mutex::new(Vec::new()),
            list_delay,
        })
    }

    fn wide_tree(children: usize) -> Arc<Self> {
        let mut metadata = HashMap::from([("/".into(), directory("/"))]);
        let mut listings = HashMap::new();
        let mut root = Vec::new();
        for index in 0..children {
            let name = format!("Directory{index:02}");
            root.push(directory(&name));
            metadata.insert(format!("/{name}"), directory(&name));
            listings.insert(format!("/{name}"), Vec::new());
        }
        if children > 0 {
            let nested = directory("Nested");
            metadata.insert("/Directory00/Nested".into(), nested.clone());
            listings.insert("/Directory00".into(), vec![nested]);
            listings.insert("/Directory00/Nested".into(), Vec::new());
        }
        listings.insert("/".into(), root);
        Arc::new(Self {
            metadata: Mutex::new(metadata),
            listings: Mutex::new(listings),
            stats: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            list_paths: Mutex::new(Vec::new()),
            list_delay: Duration::ZERO,
        })
    }
}

impl Backend for MetadataBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        self.list_paths
            .lock()
            .map_err(|_| io::Error::other("listing trace poisoned"))?
            .push(path.into());
        if !self.list_delay.is_zero() {
            std::thread::sleep(self.list_delay);
        }
        self.listings
            .lock()
            .map_err(|_| io::Error::other("listing state poisoned"))?
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "directory missing"))
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.stats.fetch_add(1, Ordering::SeqCst);
        self.metadata
            .lock()
            .map_err(|_| io::Error::other("metadata state poisoned"))?
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "entry missing"))
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        unsupported()
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        unsupported()
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        unsupported()
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        unsupported()
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        unsupported()
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        unsupported()
    }
}

#[test]
fn remote_drive_task_metadata_policy_is_backward_compatible_and_bounded() -> io::Result<()> {
    let config = MountConfig::new(
        MountId::parse("metadata-policy")?,
        MountSource::SavedRemote {
            account: "metadata-policy".into(),
            root: BackendRoot::parse("/")?,
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "Metadata policy",
    )?;
    let mut legacy = serde_json::to_value(&config).map_err(io::Error::other)?;
    legacy.as_object_mut().unwrap().remove("metadata");
    let decoded: MountConfig = serde_json::from_value(legacy).map_err(io::Error::other)?;
    assert_eq!(
        decoded.metadata.preload_depth(),
        DEFAULT_METADATA_PRELOAD_DEPTH
    );
    assert!(MountMetadataPolicy::new(MAX_METADATA_PRELOAD_DEPTH).is_ok());
    assert!(MountMetadataPolicy::new(MAX_METADATA_PRELOAD_DEPTH + 1).is_err());
    Ok(())
}

#[test]
fn remote_drive_task_preload_is_complete_depth_bounded_and_keeps_admission_live() -> io::Result<()>
{
    let temporary = tempfile::tempdir()?;
    let backend = MetadataBackend::tree(Duration::ZERO);
    let handle: BackendHandle = backend.clone();
    let policy = MountMetadataPolicy::new(2)?;
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("metadata-preload")?, MountMode::ReadOnly)
            .with_metadata_policy(policy),
        handle,
        temporary.path(),
    )?;

    engine.preload_metadata()?;
    assert_eq!(backend.lists.load(Ordering::SeqCst), 1);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(engine.preload_metadata_batch()?, 1);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(
        engine
            .list_dir(r"\")?
            .into_iter()
            .map(|metadata| metadata.name)
            .collect::<Vec<_>>(),
        vec![
            "Alpha",
            "visible.se-mount-deadbeef",
            ".se-mount-0123456789abcdef"
        ]
    );
    assert_eq!(engine.stat_cached(r"\Alpha\note.md")?.size, 12);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);

    assert_eq!(engine.stat(r"\Alpha\note.md")?.size, 12);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 2);
    let (directories, entries, bytes) = engine.metadata_cache_usage()?;
    assert_eq!(directories, 2);
    assert!(entries >= 4);
    assert!(bytes > 0 && bytes <= 16 * 1024 * 1024);
    Ok(())
}

#[test]
fn remote_drive_task_startup_walk_is_root_only_and_background_batches_are_bounded() -> io::Result<()>
{
    let temporary = tempfile::tempdir()?;
    let backend = MetadataBackend::wide_tree(12);
    let handle: BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(
            MountId::parse("metadata-bounded-preload")?,
            MountMode::ReadOnly,
        )
        .with_metadata_policy(MountMetadataPolicy::new(3)?),
        handle,
        temporary.path(),
    )?;

    engine.preload_metadata()?;
    assert_eq!(backend.lists.load(Ordering::SeqCst), 1);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(engine.preload_metadata_batch()?, 8);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 9);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(engine.preload_metadata_batch()?, 5);
    assert_eq!(engine.preload_metadata_batch()?, 0);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    let loaded = backend
        .list_paths
        .lock()
        .map_err(|_| io::Error::other("listing trace poisoned"))?;
    assert_eq!(
        loaded[9..14].iter().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "/Directory08",
            "/Directory09",
            "/Directory10",
            "/Directory11",
            "/Directory00/Nested"
        ]
    );
    Ok(())
}

#[test]
fn remote_drive_task_background_preload_honors_stop_between_targets() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = MetadataBackend::wide_tree(12);
    let handle: BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(
            MountId::parse("metadata-cancellable-preload")?,
            MountMode::ReadOnly,
        )
        .with_metadata_policy(MountMetadataPolicy::new(2)?),
        handle,
        temporary.path(),
    )?;

    engine.preload_metadata()?;
    let checks = AtomicUsize::new(0);
    assert_eq!(
        engine.preload_metadata_batch_while(|| checks.fetch_add(1, Ordering::SeqCst) > 0)?,
        1
    );
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn remote_drive_task_one_cached_directory_cannot_amplify_memory_without_bound() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let oversized_name = "x".repeat(8 * 1024 * 1024);
    assert!(!cache.install_directory(
        "/",
        directory("/"),
        vec![file(&oversized_name, 1)].into(),
        0,
    )?);
    assert_eq!(cache.usage()?, (0, 0, 0));
    Ok(())
}

#[test]
fn remote_drive_task_mutation_keeps_unrelated_snapshot_backoff() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    cache.cool_down_snapshot("/Alpha")?;
    cache.cool_down_snapshot("/Beta")?;
    cache.invalidate("/Alpha/note.md", false)?;
    assert_eq!(cache.cooldown_count()?, 1);
    Ok(())
}

#[test]
fn remote_drive_task_refresh_prioritizes_recently_accessed_directory() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    assert!(cache.install_directory(
        "/",
        directory("/"),
        vec![directory("Alpha"), directory("Beta")].into(),
        0,
    )?);
    assert!(cache.install_directory("/Alpha", directory("Alpha"), Vec::new().into(), 1)?);
    assert!(cache.install_directory("/Beta", directory("Beta"), Vec::new().into(), 1)?);
    assert!(cache.directory("/Beta")?.is_some());
    assert_eq!(
        cache.refresh_targets(2, true)?,
        vec![("/".into(), 0), ("/Beta".into(), 1)]
    );
    Ok(())
}

#[test]
fn remote_drive_task_failed_background_preload_reports_no_progress() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = MetadataBackend::tree(Duration::ZERO);
    let handle: BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(
            MountId::parse("metadata-preload-backoff")?,
            MountMode::ReadOnly,
        )
        .with_metadata_policy(MountMetadataPolicy::new(2)?),
        handle,
        temporary.path(),
    )?;

    engine.preload_metadata()?;
    backend
        .listings
        .lock()
        .map_err(|_| io::Error::other("listing state poisoned"))?
        .remove("/Alpha");
    assert_eq!(engine.preload_metadata_batch()?, 0);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(engine.preload_metadata_batch()?, 0);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn remote_drive_task_failed_refresh_keeps_last_complete_snapshot() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = MetadataBackend::tree(Duration::ZERO);
    let handle: BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("metadata-stale-good")?, MountMode::ReadOnly)
            .with_metadata_policy(MountMetadataPolicy::new(1)?),
        handle,
        temporary.path(),
    )?;

    engine.preload_metadata()?;
    let expected = engine
        .list_dir(r"\")?
        .into_iter()
        .map(|metadata| metadata.name)
        .collect::<Vec<_>>();
    backend
        .listings
        .lock()
        .map_err(|_| io::Error::other("listing state poisoned"))?
        .remove("/");
    assert!(engine.refresh_metadata().is_err());
    assert_eq!(
        engine
            .list_dir(r"\")?
            .into_iter()
            .map(|metadata| metadata.name)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn remote_drive_task_cold_stat_avoids_listing_and_snapshot_supersedes_point_cache() -> io::Result<()>
{
    let temporary = tempfile::tempdir()?;
    let backend = MetadataBackend::tree(Duration::ZERO);
    let handle: BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("metadata-stat-parent")?, MountMode::ReadOnly)
            .with_metadata_policy(MountMetadataPolicy::new(0)?),
        handle,
        temporary.path(),
    )?;

    assert_eq!(engine.stat_cached(r"\Alpha\note.md")?.size, 12);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 0);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    if let Some(note) = backend
        .listings
        .lock()
        .map_err(|_| io::Error::other("listing state poisoned"))?
        .get_mut("/Alpha")
        .and_then(|entries| entries.iter_mut().find(|entry| entry.name == "note.md"))
    {
        note.size = 99;
    }
    backend
        .metadata
        .lock()
        .map_err(|_| io::Error::other("metadata state poisoned"))?
        .get_mut("/Alpha/note.md")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "note metadata missing"))?
        .size = 99;

    assert_eq!(engine.stat_cached(r"\Alpha\note.md")?.size, 12);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 0);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(engine.list_dir(r"\Alpha")?.len(), 2);
    assert_eq!(engine.stat_cached(r"\Alpha\note.md")?.size, 99);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 1);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn remote_drive_task_concurrent_directory_misses_share_one_remote_load() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = MetadataBackend::tree(Duration::from_millis(40));
    let handle: BackendHandle = backend.clone();
    let engine = Arc::new(MountEngine::open_host_cache(
        MountRuntimeConfig::new(
            MountId::parse("metadata-single-flight")?,
            MountMode::ReadOnly,
        )
        .with_metadata_policy(MountMetadataPolicy::new(0)?),
        handle,
        temporary.path(),
    )?);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            engine.list_dir(r"\").map(|entries| entries.len())
        }));
    }
    barrier.wait();
    for worker in workers {
        assert_eq!(worker.join().unwrap()?, 3);
    }
    assert_eq!(backend.lists.load(Ordering::SeqCst), 1);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    Ok(())
}

fn directory(name: &str) -> VfsMeta {
    VfsMeta {
        name: name.into(),
        is_dir: true,
        ..VfsMeta::default()
    }
}

fn file(name: &str, size: u64) -> VfsMeta {
    VfsMeta {
        name: name.into(),
        size,
        ..VfsMeta::default()
    }
}

fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
}
