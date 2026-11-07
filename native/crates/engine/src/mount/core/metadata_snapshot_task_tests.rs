use super::metadata_cache::{MetadataCache, MetadataLookup};
use super::metadata_point_cache::MetadataPointCache;
use super::{MountEngine, MountId, MountMetadataPolicy, MountMode, MountRuntimeConfig};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

#[test]
fn remote_drive_task_cached_enumerations_share_one_immutable_snapshot() -> io::Result<()> {
    type Row = (&'static str, fn() -> io::Result<()>);
    let cases: [Row; 4] = [
        (
            "remote_drive_task_cached_enumerations_share_one_immutable_snapshot",
            remote_drive_task_cached_enumerations_share_one_immutable_snapshot_case,
        ),
        (
            "remote_drive_task_global_snapshot_admission_uses_bytes_not_count_limits",
            remote_drive_task_global_snapshot_admission_uses_bytes_not_count_limits,
        ),
        (
            "remote_drive_task_one_cached_directory_cannot_amplify_memory_without_bound",
            remote_drive_task_one_cached_directory_cannot_amplify_memory_without_bound,
        ),
        (
            "remote_drive_task_refresh_prioritizes_recently_accessed_directory",
            remote_drive_task_refresh_prioritizes_recently_accessed_directory,
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check)
            .unwrap_or_else(|_| panic!("{case}"))
            .map_err(|error| io::Error::new(error.kind(), format!("{case}: {error}")))?;
    }
    Ok(())
}

fn remote_drive_task_cached_enumerations_share_one_immutable_snapshot_case() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let entries: Arc<[VfsMeta]> = vec![file("note.md", 12)].into();
    assert!(cache.install_directory("/", directory("/"), Arc::clone(&entries), 0)?);

    let first = cache.directory("/")?.expect("root snapshot");
    let second = cache.directory("/")?.expect("root snapshot");
    assert!(Arc::ptr_eq(&entries, &first));
    assert!(Arc::ptr_eq(&first, &second));
    Ok(())
}

#[test]
fn remote_drive_task_parent_snapshot_recursively_reconciles_point_metadata() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let points = MetadataPointCache::new(true);
    let mut linked = directory("Linked");
    linked.is_symlink = true;
    let initial: Arc<[VfsMeta]> = vec![
        directory("Kept"),
        directory("Removed"),
        directory("Changed"),
        directory("Linked"),
    ]
    .into();
    assert!(cache.install_directory("/", directory("/"), initial, 0)?);
    for directory in ["Kept", "Removed", "Changed", "Linked"] {
        points.install(&format!("/{directory}/deep.txt"), file("deep.txt", 1))?;
    }

    let refreshed: Arc<[VfsMeta]> = vec![directory("Kept"), file("Changed", 2), linked].into();
    assert!(cache.install_directory("/", directory("/"), Arc::clone(&refreshed), 0,)?);
    points.reconcile_directory("/", &refreshed)?;

    assert!(points.get("/Kept/deep.txt")?.is_some());
    for path in ["/Removed/deep.txt", "/Changed/deep.txt", "/Linked/deep.txt"] {
        assert!(points.get(path)?.is_none());
        assert!(matches!(cache.stat(path)?, MetadataLookup::KnownMissing));
    }
    assert!(matches!(
        cache.stat("/Kept/deep.txt")?,
        MetadataLookup::Uncached
    ));
    Ok(())
}

struct RacingDirectoryBackend {
    metadata: Mutex<HashMap<String, VfsMeta>>,
    root_entries: Mutex<Vec<VfsMeta>>,
    list_calls: AtomicUsize,
    stale_snapshot_captured: Barrier,
    release_stale_snapshot: Barrier,
}

impl RacingDirectoryBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            metadata: Mutex::new(HashMap::from([
                ("/".into(), directory("/")),
                ("/observed.txt".into(), file("observed.txt", 22)),
                ("/removed.txt".into(), file("removed.txt", 7)),
            ])),
            root_entries: Mutex::new(vec![file("observed.txt", 22), file("removed.txt", 7)]),
            list_calls: AtomicUsize::new(0),
            stale_snapshot_captured: Barrier::new(2),
            release_stale_snapshot: Barrier::new(2),
        })
    }

    fn remove_child(&self, path: &str, name: &str) -> io::Result<()> {
        self.metadata
            .lock()
            .map_err(|_| io::Error::other("racing metadata poisoned"))?
            .remove(path);
        self.root_entries
            .lock()
            .map_err(|_| io::Error::other("racing listing poisoned"))?
            .retain(|metadata| metadata.name != name);
        Ok(())
    }
}

impl Backend for RacingDirectoryBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path != "/" {
            return Err(io::Error::new(io::ErrorKind::NotFound, "directory missing"));
        }
        let snapshot = self
            .root_entries
            .lock()
            .map_err(|_| io::Error::other("racing listing poisoned"))?
            .clone();
        if self.list_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.stale_snapshot_captured.wait();
            self.release_stale_snapshot.wait();
        }
        Ok(snapshot)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.metadata
            .lock()
            .map_err(|_| io::Error::other("racing metadata poisoned"))?
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
fn remote_drive_task_older_parent_fetch_cannot_resurrect_removed_child() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = RacingDirectoryBackend::new();
    let handle: BackendHandle = backend.clone();
    let engine = Arc::new(MountEngine::open_host_cache(
        MountRuntimeConfig::new(
            MountId::parse("metadata-generation-race")?,
            MountMode::ReadOnly,
        )
        .with_metadata_policy(MountMetadataPolicy::new(0)?),
        handle,
        temporary.path(),
    )?);

    let listing_engine = Arc::clone(&engine);
    let stale_fetch = std::thread::spawn(move || listing_engine.list_dir(r"\"));
    backend.stale_snapshot_captured.wait();

    backend.remove_child("/removed.txt", "removed.txt")?;
    assert_eq!(engine.stat_cached(r"\observed.txt")?.size, 22);
    backend.release_stale_snapshot.wait();

    let stale_result = stale_fetch
        .join()
        .map_err(|_| io::Error::other("stale metadata worker panicked"))??;
    assert!(stale_result
        .iter()
        .any(|metadata| metadata.name == "removed.txt"));

    let current = engine.list_dir(r"\")?;
    assert!(!current
        .iter()
        .any(|metadata| metadata.name == "removed.txt"));
    assert_eq!(backend.list_calls.load(Ordering::SeqCst), 2);
    let error = engine.stat_cached(r"\removed.txt").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    Ok(())
}

fn remote_drive_task_global_snapshot_admission_uses_bytes_not_count_limits() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    assert!(cache.install_directory(
        "/",
        directory("/"),
        (0..4_200)
            .map(|index| directory(&format!("directory-{index:04}")))
            .collect::<Vec<_>>()
            .into(),
        0
    )?);
    let entries: Arc<[VfsMeta]> = (0..16)
        .map(|index| file(&format!("file-{index:02}.txt"), index))
        .collect::<Vec<_>>()
        .into();
    for index in 0..4_200 {
        assert!(cache.install_directory(
            &format!("/directory-{index:04}"),
            directory(&format!("directory-{index:04}")),
            Arc::clone(&entries),
            1,
        )?);
    }

    let (directories, entries, bytes) = cache.usage()?;
    assert_eq!(directories, 4_201);
    assert!(entries > 50_000);
    assert!(bytes <= 128 * 1024 * 1024);
    assert!(
        cache.directory("/")?.is_some(),
        "all these snapshots fit the byte budget"
    );
    Ok(())
}

fn remote_drive_task_one_cached_directory_cannot_amplify_memory_without_bound() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    // Retained capacity matters even when a string's visible length is small.
    let mut oversized_name = String::with_capacity(128 * 1024 * 1024 + 1);
    oversized_name.push('x');
    assert!(!cache.install_directory(
        "/",
        directory("/"),
        vec![VfsMeta {
            name: oversized_name,
            size: 1,
            ..VfsMeta::default()
        }]
        .into(),
        0,
    )?);
    assert_eq!(cache.usage()?, (0, 0, 0));
    Ok(())
}

fn remote_drive_task_refresh_prioritizes_recently_accessed_directory() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    assert!(cache.install_directory(
        "/",
        directory("/"),
        vec![directory("Alpha"), directory("Beta"), directory("Gamma")].into(),
        0,
    )?);
    assert!(cache.install_directory("/Alpha", directory("Alpha"), Vec::new().into(), 1)?);
    assert!(cache.install_directory("/Beta", directory("Beta"), Vec::new().into(), 1)?);
    assert!(cache.install_directory("/Gamma", directory("Gamma"), Vec::new().into(), 1)?);
    assert!(cache.directory("/Beta")?.is_some());
    assert_eq!(
        cache.refresh_targets(3, true)?,
        // Root + fairness reservation + most recently demanded snapshot.
        vec![("/".into(), 0), ("/Alpha".into(), 1), ("/Beta".into(), 1)]
    );
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
