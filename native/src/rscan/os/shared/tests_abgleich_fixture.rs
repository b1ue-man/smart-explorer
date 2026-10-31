//! Fixture of the remote recursive view tests: a storing side with a fixed
//! tree, listed folder by folder and optionally walked as a whole.
use crate::scanner::ScanMessage;
use crate::types::{FileEntry, ScanProgress};
use crate::vfs::{
    Backend, BackendExtensions, BackendHandle, HashWalkEntry, HashWalkItem, HashWalkMeta,
    HashWalkRequest, Scheme, VfsMeta, VfsResult,
};
use crossbeam_channel::{unbounded, Sender};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How the storing side answers a whole-tree request.
pub(super) enum Walk {
    /// Refuses before sending anything: no such capability.
    Unsupported,
    /// Sends the items, then completes.
    Items(Vec<HashWalkItem>),
    /// Sends the items, then fails.
    ItemsThenFail(Vec<HashWalkItem>),
    /// Fails before the first item.
    FailBefore,
}

/// A storing side with a fixed tree. Folders are listed one by one; the whole
/// tree can also be requested once.
pub(super) struct TreeBackend {
    listings: HashMap<String, Vec<VfsMeta>>,
    walk: Walk,
    width: usize,
    /// Folder listings received.
    pub(super) lists: AtomicUsize,
    /// Whole-tree requests received, and the last one.
    pub(super) walk_requests: AtomicUsize,
    pub(super) last_request: Mutex<Option<HashWalkRequest>>,
    /// Folders in the order their listings completed.
    pub(super) done_order: Mutex<Vec<String>>,
    /// One folder whose listing takes this long.
    slow: Option<(String, Duration)>,
}

impl TreeBackend {
    pub(super) fn new(
        listings: HashMap<String, Vec<VfsMeta>>,
        walk: Walk,
        width: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            listings,
            walk,
            width,
            lists: AtomicUsize::new(0),
            walk_requests: AtomicUsize::new(0),
            last_request: Mutex::new(None),
            done_order: Mutex::new(Vec::new()),
            slow: None,
        })
    }

    pub(super) fn with_slow_folder(
        listings: HashMap<String, Vec<VfsMeta>>,
        width: usize,
        folder: &str,
        delay: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            listings,
            walk: Walk::Unsupported,
            width,
            lists: AtomicUsize::new(0),
            walk_requests: AtomicUsize::new(0),
            last_request: Mutex::new(None),
            done_order: Mutex::new(Vec::new()),
            slow: Some((folder.to_string(), delay)),
        })
    }
}

impl Backend for TreeBackend {
    fn scheme(&self) -> Scheme {
        Scheme::GDrive
    }

    fn root_display(&self) -> String {
        "/r".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, AtomicOrdering::SeqCst);
        if let Some((folder, delay)) = &self.slow {
            if folder == path {
                std::thread::sleep(*delay);
            }
        }
        let listed = self.listings.get(path).cloned().unwrap_or_default();
        if let Ok(mut order) = self.done_order.lock() {
            order.push(path.to_string());
        }
        Ok(listed)
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Ok(VfsMeta {
            name: "r".into(),
            is_dir: true,
            mtime_ms: 1_000,
            btime_ms: 500,
            ..Default::default()
        })
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Ok(())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn parallelism(&self) -> usize {
        self.width
    }

    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
}

impl BackendExtensions for TreeBackend {
    fn supports_hash_walk(&self, _root: &str) -> VfsResult<bool> {
        Ok(!matches!(self.walk, Walk::Unsupported))
    }

    fn hash_walk(
        &self,
        _root: &str,
        request: HashWalkRequest,
        tx: Sender<HashWalkItem>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        self.walk_requests.fetch_add(1, AtomicOrdering::SeqCst);
        if let Ok(mut last) = self.last_request.lock() {
            *last = Some(request);
        }
        match &self.walk {
            Walk::Unsupported => Ok(false),
            Walk::FailBefore => Err(io::Error::other("walk refused")),
            Walk::Items(items) => {
                send_all(items, &tx, cancel);
                Ok(true)
            }
            Walk::ItemsThenFail(items) => {
                send_all(items, &tx, cancel);
                Err(io::Error::other("connection lost"))
            }
        }
    }
}

fn send_all(items: &[HashWalkItem], tx: &Sender<HashWalkItem>, cancel: &AtomicBool) {
    for item in items {
        if cancel.load(AtomicOrdering::Relaxed) || tx.send(item.clone()).is_err() {
            return;
        }
    }
}

/// A listed entry. Modification time and birth time follow the size, so the
/// listing and the walk of one entry carry the same values.
pub(super) fn vmeta(name: &str, is_dir: bool, size: u64) -> VfsMeta {
    VfsMeta {
        name: name.to_string(),
        is_dir,
        size,
        mtime_ms: 1_000 + size as i64,
        btime_ms: 500,
        hidden: name.starts_with('.'),
        ..Default::default()
    }
}

pub(super) fn plain(hidden: bool) -> HashWalkMeta {
    HashWalkMeta {
        hidden,
        btime_ms: 500,
        ..Default::default()
    }
}

pub(super) fn walk_entry(rel: &str, is_dir: bool, size: u64, meta: HashWalkMeta) -> HashWalkItem {
    HashWalkItem::Entry(HashWalkEntry {
        rel: rel.to_string(),
        is_dir,
        size,
        mtime_ms: 1_000 + size as i64,
        digest: None,
        meta: Some(meta),
    })
}

/// The tree of the parity tests: a file, a hidden file, a folder with a file
/// and a sub-folder, a link (with content that must stay hidden) and a special
/// file.
pub(super) fn parity_listings() -> HashMap<String, Vec<VfsMeta>> {
    let mut listings = HashMap::new();
    listings.insert(
        "/r".to_string(),
        vec![
            vmeta("a.txt", false, 3),
            vmeta(".hidden", false, 1),
            vmeta("docs", true, 0),
            VfsMeta {
                is_symlink: true,
                ..vmeta("link", true, 0)
            },
            VfsMeta {
                special: true,
                ..vmeta("pipe", false, 0)
            },
        ],
    );
    listings.insert(
        "/r/docs".to_string(),
        vec![vmeta("c.md", false, 7), vmeta("deep", true, 0)],
    );
    listings.insert("/r/docs/deep".to_string(), vec![vmeta("b.bin", false, 5)]);
    listings.insert("/r/link".to_string(), vec![vmeta("inside", false, 9)]);
    listings
}

/// The same tree as whole-tree items, folders before their content.
pub(super) fn parity_items() -> Vec<HashWalkItem> {
    vec![
        walk_entry("a.txt", false, 3, plain(false)),
        walk_entry(".hidden", false, 1, plain(true)),
        walk_entry("docs", true, 0, plain(false)),
        walk_entry("docs/c.md", false, 7, plain(false)),
        walk_entry("docs/deep", true, 0, plain(false)),
        walk_entry("docs/deep/b.bin", false, 5, plain(false)),
        walk_entry(
            "link",
            true,
            0,
            HashWalkMeta {
                is_symlink: true,
                ..plain(false)
            },
        ),
        walk_entry(
            "pipe",
            false,
            0,
            HashWalkMeta {
                special: true,
                ..plain(false)
            },
        ),
    ]
}

/// Everything the view shows per row, for comparing two walks.
pub(super) type RowKey = (
    String,
    String,
    String,
    String,
    u64,
    i64,
    i64,
    bool,
    bool,
    bool,
    bool,
    u32,
);

pub(super) fn row_keys(rows: &[FileEntry]) -> Vec<RowKey> {
    let mut keys: Vec<RowKey> = rows
        .iter()
        .map(|row| {
            (
                row.path.to_string(),
                row.parent.to_string(),
                row.name.to_string(),
                row.ext.to_string(),
                row.size,
                row.mtime_ms,
                row.btime_ms,
                row.is_dir,
                row.is_symlink,
                row.hidden,
                row.system,
                row.depth,
            )
        })
        .collect();
    keys.sort();
    keys
}

/// What one scan delivered.
pub(super) struct Scanned {
    pub(super) rows: Vec<FileEntry>,
    pub(super) failed: Vec<(String, String)>,
    pub(super) errors: Vec<String>,
    pub(super) done: Option<ScanProgress>,
    pub(super) truncated: bool,
}

pub(super) fn scan(
    backend: BackendHandle,
    root: &str,
    max_depth: Option<u32>,
    retention: Option<crate::scanner::RetentionHandle>,
) -> Scanned {
    let (tx, rx) = unbounded();
    let handle = super::start_scan_backend(backend, root.to_string(), max_depth, retention, tx);
    let mut scanned = Scanned {
        rows: Vec::new(),
        failed: Vec::new(),
        errors: Vec::new(),
        done: None,
        truncated: false,
    };
    loop {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(ScanMessage::Entries(entries)) => scanned.rows.extend(entries),
            Ok(ScanMessage::FailedPaths(paths)) => scanned.failed.extend(paths),
            Ok(ScanMessage::Error(error)) => scanned.errors.push(error),
            Ok(ScanMessage::Done(progress)) => {
                scanned.done = Some(progress);
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    scanned.truncated = handle.truncated.load(AtomicOrdering::Relaxed);
    scanned
}

pub(super) fn count(value: &AtomicUsize) -> usize {
    value.load(AtomicOrdering::SeqCst)
}
