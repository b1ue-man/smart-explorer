use super::{Backend, BackendHandle, CachingBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct WideListingBackend {
    children: usize,
    name_bytes: usize,
    lists: AtomicUsize,
}

impl Backend for WideListingBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        let mut entries = vec![VfsMeta::default(); self.children];
        if let Some(first) = entries.first_mut() {
            first.name = "x".repeat(self.name_bytes);
        }
        Ok(entries)
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        unsupported()
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
fn remote_drive_task_shared_listing_cache_accounts_for_directory_and_invalidates() {
    let admitted = Arc::new(WideListingBackend {
        children: 49_999,
        name_bytes: 0,
        lists: AtomicUsize::new(0),
    });
    let admitted_cache = CachingBackend::new(admitted.clone() as BackendHandle);
    assert_eq!(admitted_cache.list_dir("/wide").unwrap().len(), 49_999);
    assert_eq!(admitted_cache.list_dir("/wide").unwrap().len(), 49_999);
    assert_eq!(admitted.lists.load(Ordering::SeqCst), 1);
    admitted_cache.invalidate_cache();
    assert_eq!(admitted_cache.list_dir("/wide").unwrap().len(), 49_999);
    assert_eq!(admitted.lists.load(Ordering::SeqCst), 2);

    let rejected = Arc::new(WideListingBackend {
        children: 50_000,
        name_bytes: 0,
        lists: AtomicUsize::new(0),
    });
    let rejected_cache = CachingBackend::new(rejected.clone() as BackendHandle);
    assert_eq!(rejected_cache.list_dir("/wide").unwrap().len(), 50_000);
    assert_eq!(rejected_cache.list_dir("/wide").unwrap().len(), 50_000);
    assert_eq!(rejected.lists.load(Ordering::SeqCst), 2);

    let byte_rejected = Arc::new(WideListingBackend {
        children: 1,
        name_bytes: 32 * 1024 * 1024 + 1,
        lists: AtomicUsize::new(0),
    });
    let byte_cache = CachingBackend::new(byte_rejected.clone() as BackendHandle);
    assert_eq!(byte_cache.list_dir("/wide").unwrap().len(), 1);
    assert_eq!(byte_cache.list_dir("/wide").unwrap().len(), 1);
    assert_eq!(byte_rejected.lists.load(Ordering::SeqCst), 2);
}

struct DropCommitBackend {
    size: Arc<AtomicUsize>,
    lists: AtomicUsize,
    publish_on_write: bool,
    fail_flush: bool,
    commit_on_drop: bool,
}

impl Backend for DropCommitBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        Ok(vec![VfsMeta {
            name: "note.md".into(),
            size: self.size.load(Ordering::SeqCst) as u64,
            ..VfsMeta::default()
        }])
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        unsupported()
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        unsupported()
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(DropCommitWriter {
            size: Arc::clone(&self.size),
            bytes: Mutex::new(Vec::new()),
            publish_on_write: self.publish_on_write,
            fail_flush: self.fail_flush,
            commit_on_drop: self.commit_on_drop,
        }))
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

struct DropCommitWriter {
    size: Arc<AtomicUsize>,
    bytes: Mutex<Vec<u8>>,
    publish_on_write: bool,
    fail_flush: bool,
    commit_on_drop: bool,
}

impl Write for DropCommitWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut bytes = self
            .bytes
            .lock()
            .map_err(|_| io::Error::other("test writer poisoned"))?;
        bytes.extend_from_slice(buffer);
        if self.publish_on_write {
            self.size.store(bytes.len(), Ordering::SeqCst);
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::Error::other("ambiguous test flush"))
        } else {
            Ok(())
        }
    }
}

impl Drop for DropCommitWriter {
    fn drop(&mut self) {
        if self.commit_on_drop {
            let Ok(bytes) = self.bytes.lock() else {
                return;
            };
            self.size.store(bytes.len(), Ordering::SeqCst);
        }
    }
}

#[test]
fn remote_drive_task_writer_drop_invalidates_a_listing_loaded_while_open() -> io::Result<()> {
    let size = Arc::new(AtomicUsize::new(1));
    let inner = Arc::new(DropCommitBackend {
        size: Arc::clone(&size),
        lists: AtomicUsize::new(0),
        publish_on_write: false,
        fail_flush: false,
        commit_on_drop: true,
    });
    let cache = CachingBackend::new(inner.clone() as BackendHandle);
    assert_eq!(cache.list_dir("/docs")?[0].size, 1);

    let mut writer = cache.open_write("/docs/note.md")?;
    writer.write_all(b"updated")?;
    assert_eq!(cache.list_dir("/docs")?[0].size, 1);
    drop(writer);

    assert_eq!(cache.list_dir("/docs")?[0].size, 7);
    assert_eq!(inner.lists.load(Ordering::SeqCst), 3);
    Ok(())
}

#[test]
fn remote_drive_task_ambiguous_flush_invalidates_while_writer_is_open() -> io::Result<()> {
    let size = Arc::new(AtomicUsize::new(1));
    let inner = Arc::new(DropCommitBackend {
        size: Arc::clone(&size),
        lists: AtomicUsize::new(0),
        publish_on_write: true,
        fail_flush: true,
        commit_on_drop: false,
    });
    let cache = CachingBackend::new(inner.clone() as BackendHandle);
    assert_eq!(cache.list_dir("/docs")?[0].size, 1);

    let mut writer = cache.open_write("/docs/note.md")?;
    assert_eq!(cache.list_dir("/docs")?[0].size, 1);
    writer.write_all(b"updated")?;
    assert!(writer.flush().is_err());

    assert_eq!(cache.list_dir("/docs")?[0].size, 7);
    assert_eq!(inner.lists.load(Ordering::SeqCst), 3);
    Ok(())
}

#[derive(Default)]
struct BlockingState {
    arrived: bool,
    released: bool,
}

struct BlockingListingBackend {
    entries: Mutex<Vec<VfsMeta>>,
    state: Mutex<BlockingState>,
    wake: Condvar,
    lists: AtomicUsize,
}

impl BlockingListingBackend {
    fn wait_until_arrived(&self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("test listing state poisoned"))?;
        while !state.arrived {
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "cache miss did not reach backend",
                ));
            }
            state = self
                .wake
                .wait_timeout(state, deadline - now)
                .map_err(|_| io::Error::other("test listing state poisoned"))?
                .0;
        }
        Ok(())
    }

    fn release(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("test listing state poisoned"))?;
        state.released = true;
        self.wake.notify_all();
        Ok(())
    }
}

impl Backend for BlockingListingBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| io::Error::other("test entries poisoned"))?
            .clone();
        if self.lists.fetch_add(1, Ordering::SeqCst) == 0 {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self
                .state
                .lock()
                .map_err(|_| io::Error::other("test listing state poisoned"))?;
            state.arrived = true;
            self.wake.notify_all();
            while !state.released {
                let now = Instant::now();
                if now >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "cache miss release timed out",
                    ));
                }
                state = self
                    .wake
                    .wait_timeout(state, deadline - now)
                    .map_err(|_| io::Error::other("test listing state poisoned"))?
                    .0;
            }
        }
        Ok(entries)
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        unsupported()
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
        self.entries
            .lock()
            .map_err(|_| io::Error::other("test entries poisoned"))?
            .clear();
        Ok(())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        unsupported()
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        unsupported()
    }
}

#[test]
fn remote_drive_task_mutation_prevents_racing_stale_snapshot_install() -> io::Result<()> {
    let inner = Arc::new(BlockingListingBackend {
        entries: Mutex::new(vec![VfsMeta {
            name: "note.md".into(),
            ..VfsMeta::default()
        }]),
        state: Mutex::new(BlockingState::default()),
        wake: Condvar::new(),
        lists: AtomicUsize::new(0),
    });
    let cache = Arc::new(CachingBackend::new(inner.clone() as BackendHandle));
    let racing_cache = Arc::clone(&cache);
    let racing = std::thread::spawn(move || racing_cache.list_dir("/docs"));
    inner.wait_until_arrived()?;

    cache.remove_file("/docs/note.md")?;
    inner.release()?;
    assert_eq!(
        racing
            .join()
            .map_err(|_| io::Error::other("racing listing panicked"))??
            .len(),
        1
    );
    assert!(cache.list_dir("/docs")?.is_empty());
    assert_eq!(inner.lists.load(Ordering::SeqCst), 2);
    Ok(())
}

fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
}
