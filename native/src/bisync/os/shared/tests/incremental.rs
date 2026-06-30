use super::super::*;
use super::{fwd, tmp};
use crate::vfs::{
    Backend, ChangeKind, LocalBackend, Scheme, VfsChange, VfsChangeBatch, VfsMeta, VfsResult,
};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Counting {
    inner: LocalBackend,
    lists: Arc<AtomicUsize>,
    stats: Arc<AtomicUsize>,
}

impl Backend for Counting {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.stats.fetch_add(1, Ordering::Relaxed);
        self.inner.stat(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }

    fn rename_overwrites(&self) -> bool {
        true
    }

    fn is_local(&self) -> bool {
        true
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}

struct ChangeSource {
    inner: LocalBackend,
    root: String,
    ids: Arc<Mutex<HashMap<String, String>>>,
    changes: Arc<Mutex<Vec<VfsChange>>>,
    cursor: Arc<Mutex<String>>,
}

impl ChangeSource {
    fn rel(&self, path: &str) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .trim_start_matches('/')
            .to_string()
    }
}

impl Backend for ChangeSource {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }

    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        if path.trim_end_matches('/') == self.root.trim_end_matches('/') {
            return Ok(Some("root-id".into()));
        }
        Ok(self.ids.lock().unwrap().get(&self.rel(path)).cloned())
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }

    fn rename_overwrites(&self) -> bool {
        true
    }

    fn is_local(&self) -> bool {
        true
    }

    fn supports_changes(&self) -> bool {
        true
    }

    fn change_root_id(&self, _root: &str) -> VfsResult<Option<String>> {
        Ok(Some("root-id".into()))
    }

    fn current_change_cursor(&self, _root: &str) -> VfsResult<Option<String>> {
        Ok(Some(self.cursor.lock().unwrap().clone()))
    }

    fn changes_since(&self, _root: &str, _cursor: &str) -> VfsResult<VfsChangeBatch> {
        Ok(VfsChangeBatch {
            changes: self.changes.lock().unwrap().clone(),
            new_cursor: Some(self.cursor.lock().unwrap().clone()),
            reset: false,
        })
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}

#[test]
fn incremental_mirror_skips_target_tree_listing() {
    let a = tmp("inc_a");
    let b = tmp("inc_b");
    let db = tmp("inc_db").join("state.sqlite");
    std::fs::write(a.join("f.txt"), b"v1").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let a_lists = Arc::new(AtomicUsize::new(0));
    let a_stats = Arc::new(AtomicUsize::new(0));
    let b_lists = Arc::new(AtomicUsize::new(0));
    let b_stats = Arc::new(AtomicUsize::new(0));
    let ca = Counting {
        inner: LocalBackend::new(&ra),
        lists: a_lists.clone(),
        stats: a_stats.clone(),
    };
    let cb = Counting {
        inner: LocalBackend::new(&rb),
        lists: b_lists.clone(),
        stats: b_stats.clone(),
    };
    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let filter = WalkFilter::basic(true, &gs);
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        ..Default::default()
    };

    let first = super::super::orchestration::run_with_store_path(
        &ca, &ra, &cb, &rb, opts, &cancel, &filter, &db,
    );
    assert!(first.errors.is_empty());
    assert!(b.join("f.txt").exists());

    a_lists.store(0, Ordering::Relaxed);
    a_stats.store(0, Ordering::Relaxed);
    b_lists.store(0, Ordering::Relaxed);
    b_stats.store(0, Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(a.join("f.txt"), b"v2-longer").unwrap();

    let second = super::super::orchestration::run_with_store_path(
        &ca, &ra, &cb, &rb, opts, &cancel, &filter, &db,
    );
    assert!(second.errors.is_empty());
    assert_eq!(second.stats.a_to_b, 1);
    assert_eq!(std::fs::read(b.join("f.txt")).unwrap(), b"v2-longer");
    assert!(
        a_lists.load(Ordering::Relaxed) > 0,
        "source is still walked"
    );
    assert_eq!(
        b_lists.load(Ordering::Relaxed),
        0,
        "target tree must not be enumerated on incremental mirror"
    );
    assert!(
        b_stats.load(Ordering::Relaxed) > 0,
        "target touched path is verified"
    );

    let pair = pair_id(&ra, &rb);
    let _ = std::fs::remove_file(baseline_path(&pair));
    let _ = std::fs::remove_dir_all(versions_dir(&pair));
    for d in [&a, &b] {
        std::fs::remove_dir_all(d).ok();
    }
    if let Some(parent) = db.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

#[test]
fn incremental_drive_like_rename_deletes_old_target_path() {
    let a = tmp("inc_ra");
    let b = tmp("inc_rb");
    let db = tmp("inc_rdb").join("state.sqlite");
    std::fs::write(a.join("old.txt"), b"v1").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let ids = Arc::new(Mutex::new(HashMap::from([(
        "old.txt".to_string(),
        "file-id-1".to_string(),
    )])));
    let changes = Arc::new(Mutex::new(Vec::new()));
    let cursor = Arc::new(Mutex::new("c1".to_string()));
    let source = ChangeSource {
        inner: LocalBackend::new(&ra),
        root: ra.clone(),
        ids: ids.clone(),
        changes: changes.clone(),
        cursor: cursor.clone(),
    };
    let b_lists = Arc::new(AtomicUsize::new(0));
    let b_stats = Arc::new(AtomicUsize::new(0));
    let target = Counting {
        inner: LocalBackend::new(&rb),
        lists: b_lists.clone(),
        stats: b_stats,
    };
    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let filter = WalkFilter::basic(true, &gs);
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        ..Default::default()
    };

    let first = super::super::orchestration::run_with_store_path(
        &source, &ra, &target, &rb, opts, &cancel, &filter, &db,
    );
    assert!(first.errors.is_empty());
    assert!(b.join("old.txt").exists());

    std::fs::rename(a.join("old.txt"), a.join("new.txt")).unwrap();
    ids.lock().unwrap().remove("old.txt");
    ids.lock()
        .unwrap()
        .insert("new.txt".into(), "file-id-1".into());
    *cursor.lock().unwrap() = "c2".into();
    let meta = source.stat(&format!("{}/new.txt", ra)).unwrap();
    *changes.lock().unwrap() = vec![VfsChange {
        kind: ChangeKind::Upsert,
        rel: None,
        id: Some("file-id-1".into()),
        parent_id: Some("root-id".into()),
        name: Some("new.txt".into()),
        meta: Some(meta),
    }];
    b_lists.store(0, Ordering::Relaxed);

    let second = super::super::orchestration::run_with_store_path(
        &source, &ra, &target, &rb, opts, &cancel, &filter, &db,
    );
    assert!(second.errors.is_empty());
    assert_eq!(second.stats.a_to_b, 1);
    assert_eq!(second.stats.deleted, 1);
    assert!(!b.join("old.txt").exists());
    assert_eq!(std::fs::read(b.join("new.txt")).unwrap(), b"v1");
    assert_eq!(b_lists.load(Ordering::Relaxed), 0);

    let pair = pair_id(&ra, &rb);
    let _ = std::fs::remove_file(baseline_path(&pair));
    let _ = std::fs::remove_dir_all(versions_dir(&pair));
    for d in [&a, &b] {
        std::fs::remove_dir_all(d).ok();
    }
    if let Some(parent) = db.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}
