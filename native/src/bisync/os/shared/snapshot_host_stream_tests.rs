//! The sync side read from a walk with listing attributes, checked folder by
//! folder while the walk streams: a folder is checked (and reported to the
//! early copies) before the walk ends, a protected subtree is dropped
//! without counting against the walk budget, a folder that could not be
//! opened stays a folder with its omission, and items outside their folder's
//! run fail the side.
use super::super::early_copy::FolderListed;
use super::super::omissions::{OmissionKind, SyncOmissions};
use super::super::snapshot::WalkFilter;
use super::super::snapshot_dir::{Listed, WalkContext};
use super::super::snapshot_hash::HashMode;
use super::super::snapshot_types::DirSet;
use super::super::sync_overload::Progress;
use super::super::types::{BisyncOptions, Tree};
use crate::vfs::{
    Backend, BackendExtensions, HashWalkEntry, HashWalkItem, HashWalkMeta, HashWalkRequest,
    OmissionReason, Scheme, VfsMeta, VfsOmission, VfsResult,
};
use crossbeam_channel::Sender;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A remote side whose walk next to the data sends fixed items: `before`,
/// then (after `wait_for` was reported listed, at most five seconds) `after`.
struct Scripted {
    before: Vec<HashWalkItem>,
    after: Vec<HashWalkItem>,
    wait_for: Option<String>,
    listed: Arc<Mutex<Vec<String>>>,
    /// `wait_for` was listed while the walk still had items to send.
    seen_before_end: AtomicBool,
}

impl Scripted {
    fn new(before: Vec<HashWalkItem>, after: Vec<HashWalkItem>, wait_for: Option<&str>) -> Self {
        Self {
            before,
            after,
            wait_for: wait_for.map(str::to_string),
            listed: Arc::new(Mutex::new(Vec::new())),
            seen_before_end: AtomicBool::new(false),
        }
    }
}

fn refused<T>() -> VfsResult<T> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

impl Backend for Scripted {
    fn scheme(&self) -> Scheme {
        Scheme::GDrive
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, _: &str) -> VfsResult<Vec<VfsMeta>> {
        refused()
    }
    fn stat(&self, _: &str) -> VfsResult<VfsMeta> {
        refused()
    }
    fn open_read(&self, _: &str) -> VfsResult<Box<dyn Read + Send>> {
        refused()
    }
    fn open_write(&self, _: &str) -> VfsResult<Box<dyn Write + Send>> {
        refused()
    }
    fn rename(&self, _: &str, _: &str) -> VfsResult<()> {
        refused()
    }
    fn remove_file(&self, _: &str) -> VfsResult<()> {
        refused()
    }
    fn remove_dir(&self, _: &str) -> VfsResult<()> {
        refused()
    }
    fn mkdir_all(&self, _: &str) -> VfsResult<()> {
        refused()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
}

impl BackendExtensions for Scripted {
    fn supports_hash_walk(&self, _: &str) -> VfsResult<bool> {
        Ok(true)
    }

    fn hash_walk(
        &self,
        _: &str,
        request: HashWalkRequest,
        tx: Sender<HashWalkItem>,
        _: &AtomicBool,
    ) -> VfsResult<bool> {
        assert!(request.meta, "a side with attributes is asked for them");
        let send = |items: &[HashWalkItem]| {
            for item in items {
                if tx.send(item.clone()).is_err() {
                    return false;
                }
            }
            true
        };
        if !send(&self.before) {
            return Err(io::Error::other("receiver gone"));
        }
        if let Some(folder) = &self.wait_for {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(5) {
                if self.listed.lock().unwrap().contains(folder) {
                    self.seen_before_end.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if !send(&self.after) {
            return Err(io::Error::other("receiver gone"));
        }
        Ok(true)
    }
}

struct Seen(Arc<Mutex<Vec<String>>>);

impl FolderListed for Seen {
    fn listed(&self, dir_rel: &str, _: &Listed) {
        self.0.lock().unwrap().push(dir_rel.to_string());
    }
}

fn dir(rel: &str) -> HashWalkItem {
    HashWalkItem::Entry(HashWalkEntry {
        rel: rel.into(),
        is_dir: true,
        size: 0,
        mtime_ms: 1_000,
        digest: None,
        meta: Some(HashWalkMeta::default()),
    })
}

fn file(rel: &str) -> HashWalkItem {
    HashWalkItem::Entry(HashWalkEntry {
        rel: rel.into(),
        is_dir: false,
        size: 3,
        mtime_ms: 1_000,
        digest: None,
        meta: Some(HashWalkMeta::default()),
    })
}

fn unreadable(rel: &str) -> HashWalkItem {
    HashWalkItem::Omitted(VfsOmission {
        rel: rel.into(),
        reason: OmissionReason::Unreadable,
        detail: "Zugriff verweigert".into(),
    })
}

/// What one side's walk gave: its tree (or error) and what the context
/// collected.
struct Side {
    tree: io::Result<Option<Tree>>,
    dirs: Vec<String>,
    omissions: Vec<(String, OmissionKind)>,
}

fn read_side(backend: &Scripted, walk_entries: u64) -> Side {
    let cancel = AtomicBool::new(false);
    let globs = globset::GlobSet::empty();
    let filter = WalkFilter::basic(true, &globs);
    let omissions = Mutex::new(SyncOmissions::new(false));
    let filtered = Mutex::new(Tree::new());
    let dirs = Mutex::new(DirSet::new());
    let observer = Seen(backend.listed.clone());
    let mut limits = super::super::SyncLimits::for_memory(None);
    limits.walk_entries = walk_entries;
    let ctx = WalkContext {
        be: backend,
        root: "/",
        cancel: &cancel,
        filter: &filter,
        hash: HashMode::None,
        prev: None,
        allow_duplicate_files: false,
        omissions: Some(&omissions),
        duplicates: None,
        filtered: Some(&filtered),
        dirs: Some(&dirs),
        opts: BisyncOptions::default(),
        limits,
        nodes: AtomicU64::new(0),
        text_bytes: AtomicU64::new(0),
        reads: None,
        progress: Progress::default(),
        log: None,
        listed: Some(&observer),
        host_listed: true,
        local: None,
    };
    let tree = super::super::snapshot_agent::walk_snapshot_via_agent(&ctx);
    drop(ctx);
    Side {
        tree,
        dirs: dirs.into_inner().unwrap().into_iter().collect(),
        omissions: omissions
            .into_inner()
            .unwrap()
            .paths()
            .map(|(path, kind, _)| (path.to_string(), kind))
            .collect(),
    }
}

fn keys(tree: &Tree) -> Vec<String> {
    tree.keys().cloned().collect()
}

#[test]
fn abgleich_task_host_stream_checks_each_folder_before_the_walk_ends() {
    // The root's run ends with the first item of `a`; the walk then waits
    // until the root was checked and reported, before it sends the rest.
    let backend = Scripted::new(
        vec![dir("a"), file("x.txt"), file("a/y.txt")],
        vec![dir("a/b"), file("a/b/z.txt")],
        Some(""),
    );
    let side = read_side(&backend, 1_000);
    let tree = side
        .tree
        .unwrap()
        .expect("the walk next to the data served");
    assert!(
        backend.seen_before_end.load(Ordering::SeqCst),
        "the root folder was checked while the walk still ran"
    );
    assert_eq!(keys(&tree), ["a/b/z.txt", "a/y.txt", "x.txt"]);
    assert_eq!(side.dirs, ["a", "a/b"]);
    assert_eq!(*backend.listed.lock().unwrap(), ["", "a", "a/b"]);
}

#[test]
fn abgleich_task_host_stream_drops_a_protected_subtree_without_counting_it() {
    // `lost+found` at a volume root is protected: `scan_listing` never
    // enters it, so its 60 entries are dropped as they arrive, while the
    // budget of eight entries covers the four the walk over listings reads.
    let mut items = vec![dir("lost+found"), dir("keep")];
    items.push(dir("lost+found/deep"));
    for index in 0..30 {
        items.push(file(&format!("lost+found/f{index}")));
    }
    for index in 0..30 {
        items.push(file(&format!("lost+found/deep/g{index}")));
    }
    items.extend([file("keep/1"), file("keep/2")]);
    let backend = Scripted::new(items, Vec::new(), None);
    let side = read_side(&backend, 8);
    let tree = side
        .tree
        .unwrap()
        .expect("the walk next to the data served");
    assert_eq!(keys(&tree), ["keep/1", "keep/2"]);
    assert_eq!(side.dirs, ["keep"]);
    assert!(side
        .omissions
        .contains(&("lost+found".to_string(), OmissionKind::SystemFolder)));
    assert_eq!(*backend.listed.lock().unwrap(), ["", "keep"]);
}

#[test]
fn abgleich_task_host_stream_keeps_folders_that_could_not_be_read() {
    // `locked` could not be opened (its entry, then its omission, W6);
    // `broken` broke off while it was read: its partial content and the
    // subfolder it had found are left out, its failure is reported.
    let backend = Scripted::new(
        vec![
            dir("locked"),
            unreadable("locked"),
            dir("broken"),
            file("f.txt"),
            dir("broken/sub"),
            file("broken/1"),
            unreadable("broken"),
            file("broken/sub/2"),
        ],
        Vec::new(),
        None,
    );
    let side = read_side(&backend, 1_000);
    let tree = side
        .tree
        .unwrap()
        .expect("the walk next to the data served");
    assert_eq!(keys(&tree), ["f.txt"]);
    assert_eq!(side.dirs, ["broken", "locked"]);
    for folder in ["broken", "locked"] {
        assert!(
            side.omissions
                .contains(&(folder.to_string(), OmissionKind::Unreadable)),
            "{folder}: {:?}",
            side.omissions
        );
    }
}

#[test]
fn abgleich_task_host_stream_refuses_items_outside_their_folder_run() {
    let backend = Scripted::new(
        vec![dir("a"), dir("b"), file("a/1"), file("b/1"), file("a/2")],
        Vec::new(),
        None,
    );
    let error = read_side(&backend, 1_000).tree.unwrap_err();
    assert!(
        error.to_string().contains("outside their folder's run"),
        "{error}"
    );
}
