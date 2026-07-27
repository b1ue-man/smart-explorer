use super::metadata_cache::{MetadataCache, MAX_CACHED_ENTRIES};
use super::{
    FlushOutcome, MountEngine, MountId, MountMetadataPolicy, MountMode, MountRuntimeConfig,
};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct BoundedRendezvous {
    arrivals: Mutex<usize>,
    wake: Condvar,
}

impl BoundedRendezvous {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            arrivals: Mutex::new(0),
            wake: Condvar::new(),
        })
    }

    fn arrive_and_wait(&self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut arrivals = self
            .arrivals
            .lock()
            .map_err(|_| io::Error::other("test rendezvous poisoned"))?;
        *arrivals += 1;
        self.wake.notify_all();
        while *arrivals < 2 {
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "parallel directory load did not rendezvous",
                ));
            }
            let (next, outcome) = self
                .wake
                .wait_timeout(arrivals, deadline - now)
                .map_err(|_| io::Error::other("test rendezvous poisoned"))?;
            arrivals = next;
            if outcome.timed_out() && *arrivals < 2 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "parallel directory load did not rendezvous",
                ));
            }
        }
        Ok(())
    }
}

struct NavigationBackend {
    metadata: HashMap<String, VfsMeta>,
    listings: HashMap<String, Vec<VfsMeta>>,
    stats: AtomicUsize,
    lists: AtomicUsize,
    open_reads: AtomicUsize,
    racing_child_lists: AtomicUsize,
    child_rendezvous: Option<Arc<BoundedRendezvous>>,
}

impl NavigationBackend {
    fn new(race_children: bool) -> Arc<Self> {
        let alpha = directory("Alpha");
        let beta = directory("Beta");
        let note = file("note.md", 12);
        let alpha_note = file("alpha.txt", 5);
        let beta_note = file("beta.txt", 4);
        Arc::new(Self {
            metadata: HashMap::from([
                ("/".into(), directory("/")),
                ("/Alpha".into(), alpha.clone()),
                ("/Beta".into(), beta.clone()),
                ("/note.md".into(), note.clone()),
                ("/Alpha/alpha.txt".into(), alpha_note.clone()),
                ("/Beta/beta.txt".into(), beta_note.clone()),
            ]),
            listings: HashMap::from([
                ("/".into(), vec![alpha, beta, note]),
                ("/Alpha".into(), vec![alpha_note]),
                ("/Beta".into(), vec![beta_note]),
            ]),
            stats: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            open_reads: AtomicUsize::new(0),
            racing_child_lists: AtomicUsize::new(0),
            child_rendezvous: race_children.then(BoundedRendezvous::new),
        })
    }

    fn with_root_listing(entries: Vec<VfsMeta>) -> Arc<Self> {
        Arc::new(Self {
            metadata: HashMap::from([("/".into(), directory("/"))]),
            listings: HashMap::from([("/".into(), entries)]),
            stats: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            open_reads: AtomicUsize::new(0),
            racing_child_lists: AtomicUsize::new(0),
            child_rendezvous: None,
        })
    }
}

impl Backend for NavigationBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        if matches!(path, "/Alpha" | "/Beta")
            && self.racing_child_lists.fetch_add(1, Ordering::SeqCst) < 2
        {
            if let Some(rendezvous) = self.child_rendezvous.as_ref() {
                rendezvous.arrive_and_wait()?;
            }
        }
        self.listings
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "directory missing"))
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.stats.fetch_add(1, Ordering::SeqCst);
        self.metadata
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "entry missing"))
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.open_reads.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Cursor::new(b"remote contents".to_vec())))
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

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        true
    }
}

#[test]
fn remote_drive_task_snapshot_missing_and_child_navigation_avoid_remote_reprobes() -> io::Result<()>
{
    let temporary = tempfile::tempdir()?;
    let backend = NavigationBackend::new(false);
    let engine = engine("navigation-hints", backend.clone(), temporary.path())?;

    assert_eq!(engine.list_dir(r"\")?.len(), 3);
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 1);

    for _ in 0..2 {
        let missing = engine.stat_cached(r"\desktop.ini").unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);

    assert_eq!(engine.list_dir(r"\Alpha")?[0].name, "alpha.txt");
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    assert_eq!(engine.list_dir(r"\Alpha")?.len(), 1);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn remote_drive_task_unrelated_parallel_directory_loads_both_remain_cached() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = NavigationBackend::new(true);
    let engine = Arc::new(engine(
        "navigation-parallel",
        backend.clone(),
        temporary.path(),
    )?);
    assert_eq!(engine.list_dir(r"\")?.len(), 3);

    let alpha_engine = Arc::clone(&engine);
    let alpha = std::thread::spawn(move || alpha_engine.list_dir(r"\Alpha"));
    let beta_engine = Arc::clone(&engine);
    let beta = std::thread::spawn(move || beta_engine.list_dir(r"\Beta"));
    let alpha_result = alpha.join();
    let beta_result = beta.join();
    let alpha_result =
        alpha_result.map_err(|_| io::Error::other("alpha directory load panicked"))?;
    let beta_result = beta_result.map_err(|_| io::Error::other("beta directory load panicked"))?;
    assert_eq!(alpha_result?[0].name, "alpha.txt");
    assert_eq!(beta_result?[0].name, "beta.txt");
    assert_eq!(backend.stats.load(Ordering::SeqCst), 1);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 3);

    assert_eq!(engine.list_dir(r"\Alpha")?.len(), 1);
    assert_eq!(engine.list_dir(r"\Beta")?.len(), 1);
    assert_eq!(backend.lists.load(Ordering::SeqCst), 3);
    Ok(())
}

#[test]
fn remote_drive_task_engine_listing_rejects_entry_and_byte_overflow() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    {
        let backend =
            NavigationBackend::with_root_listing(vec![VfsMeta::default(); MAX_CACHED_ENTRIES + 1]);
        let engine = engine("listing-entry-overflow", backend.clone(), temporary.path())?;
        assert_eq!(
            engine.list_dir(r"\").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(backend.lists.load(Ordering::SeqCst), 1);
    }
    {
        let backend =
            NavigationBackend::with_root_listing(vec![file(&"x".repeat(16 * 1024 * 1024 + 1), 1)]);
        let engine = engine("listing-byte-overflow", backend.clone(), temporary.path())?;
        assert_eq!(
            engine.list_dir(r"\").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(backend.lists.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[test]
fn remote_drive_task_directory_snapshot_rejects_entry_and_byte_overflow() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let at_entry_limit = vec![VfsMeta::default(); MAX_CACHED_ENTRIES];
    cache.validate_listing(&at_entry_limit)?;
    drop(at_entry_limit);

    let over_entry_limit = vec![VfsMeta::default(); MAX_CACHED_ENTRIES + 1];
    assert_eq!(
        cache
            .validate_listing(&over_entry_limit)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    drop(over_entry_limit);

    let over_byte_limit = vec![file(&"x".repeat(16 * 1024 * 1024), 1)];
    assert_eq!(
        cache.validate_listing(&over_byte_limit).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    Ok(())
}

#[test]
fn remote_drive_task_metadata_handle_downloads_only_on_first_data_access() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let backend = NavigationBackend::new(false);
    let mount_id = "metadata-only-handle";
    let engine = engine(mount_id, backend.clone(), temporary.path())?;
    let metadata = engine.stat_cached(r"\note.md")?;
    let files = temporary.path().join(mount_id).join("files");
    assert_eq!(spool_file_count(&files)?, 0);

    let handle = engine.open_metadata_file(r"\note.md", metadata.clone(), false)?;
    // Pure metadata traffic never transfers the file or allocates a spool.
    assert_eq!(backend.open_reads.load(Ordering::SeqCst), 0);
    assert_eq!(engine.stat_handle(handle)?.name, metadata.name);
    assert_eq!(engine.stat_handle(handle)?.size, 12);
    assert_eq!(engine.len(handle)?, 12);
    assert_eq!(engine.flush(handle)?, FlushOutcome::NoChanges);
    assert_eq!(backend.open_reads.load(Ordering::SeqCst), 0);
    assert_eq!(spool_file_count(&files)?, 0);

    // The first data access upgrades the handle with exactly one transfer;
    // later reads are served from the spool.
    let mut buffer = [0u8; 6];
    assert_eq!(engine.read(handle, 0, &mut buffer)?, 6);
    assert_eq!(&buffer, b"remote");
    assert_eq!(backend.open_reads.load(Ordering::SeqCst), 1);
    assert_eq!(engine.read(handle, 7, &mut buffer)?, 6);
    assert_eq!(&buffer, b"conten");
    assert_eq!(backend.open_reads.load(Ordering::SeqCst), 1);
    engine.close(handle)?;
    Ok(())
}

fn engine(
    id: &str,
    backend: Arc<NavigationBackend>,
    spool: &std::path::Path,
) -> io::Result<MountEngine> {
    let handle: BackendHandle = backend;
    MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse(id)?, MountMode::ReadOnly)
            .with_metadata_policy(MountMetadataPolicy::new(0)?),
        handle,
        spool,
    )
}

fn spool_file_count(path: &std::path::Path) -> io::Result<usize> {
    Ok(std::fs::read_dir(path)?.count())
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
