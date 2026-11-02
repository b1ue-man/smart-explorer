//! The existing incremental safety adapter, with complete local stage/ID forwarding.
use crate::bisync::incremental_collect::ResolvedChange;
use crate::bisync::state_store::{ItemRecord, PairRecord, Side};
use crate::bisync::types::Sig;
use crate::vfs::{
    Backend, ChangeKind, LocalBackend, Scheme, VfsChange, VfsChangeBatch, VfsMeta, VfsResult,
};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub(super) struct FeedBackend {
    inner: LocalBackend,
    root: String,
    pub(super) batch: Mutex<VfsChangeBatch>,
    pub(super) calls: AtomicUsize,
}

impl Backend for FeedBackend {
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

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }

    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }

    fn is_local(&self) -> bool {
        true
    }

    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        self.inner.extensions()
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }

    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.inner.discard_copy_stage(stage)
    }

    fn case_sensitive_paths(&self, root: &str) -> bool {
        self.inner.case_sensitive_paths(root)
    }

    fn change_root_id(&self, root: &str) -> VfsResult<Option<String>> {
        self.inner.stat(root)?;
        Ok(Some("feed-root".into()))
    }

    fn current_change_cursor(&self, _root: &str) -> VfsResult<Option<String>> {
        Ok(Some("cursor-1".into()))
    }

    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        self.inner.stat(path)?;
        if path == self.root {
            return Ok(Some("feed-root".into()));
        }
        let prefix = format!("{}/", self.root.trim_end_matches('/'));
        let rel = path.strip_prefix(&prefix).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "fixture ID escaped its root",
            )
        })?;
        if let Some(id) = self
            .batch
            .lock()
            .unwrap()
            .changes
            .iter()
            .find(|change| change.rel.as_deref() == Some(rel))
            .and_then(|change| change.id.clone())
        {
            return Ok(Some(id));
        }
        Ok(Some(match rel {
            "a.txt" => "id-a".into(),
            "b.txt" => "id-b".into(),
            _ => format!("fixture:{rel}"),
        }))
    }

    fn supports_changes(&self) -> bool {
        true
    }

    fn changes_since(&self, _root: &str, _cursor: &str) -> VfsResult<VfsChangeBatch> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.batch.lock().unwrap().clone())
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

pub(super) fn temp_path(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("bisync_{tag}_{}_{}", std::process::id(), nanos));
    path
}

pub(super) fn record() -> PairRecord {
    PairRecord {
        pair: "pair".into(),
        root_a: "a".into(),
        root_b: "b".into(),
        mode: "mirror".into(),
        source_side: Side::A,
        source_cursor: Some("cursor-1".into()),
        root_a_id: None,
        root_b_id: None,
        bootstrapped: true,
        target_managed: true,
    }
}

pub(super) fn feed(root: &str, changes: Vec<VfsChange>) -> FeedBackend {
    FeedBackend {
        inner: LocalBackend::new(root),
        root: root.to_string(),
        batch: Mutex::new(VfsChangeBatch {
            changes,
            new_cursor: Some("cursor-2".into()),
            reset: false,
        }),
        calls: AtomicUsize::new(0),
    }
}

pub(super) fn change(kind: ChangeKind, rel: &str) -> VfsChange {
    VfsChange {
        kind: kind.clone(),
        rel: Some(rel.into()),
        id: None,
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        meta: (kind == ChangeKind::Upsert).then(|| VfsMeta {
            name: rel.rsplit('/').next().unwrap().into(),
            size: 4,
            mtime_ms: 10,
            ..Default::default()
        }),
    }
}

pub(super) fn active_item(rel: &str) -> ItemRecord {
    ItemRecord {
        side: Side::A,
        rel: rel.into(),
        id: None,
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        sig: Some(Sig {
            size: 4,
            mtime_ms: 10,
            hash: 0,
        }),
        is_dir: false,
        deleted: false,
    }
}

pub(super) fn resolved(rel: &str, old_rel: Option<&str>) -> ResolvedChange {
    ResolvedChange {
        rel: rel.into(),
        old_rel: old_rel.map(str::to_owned),
        kind: ChangeKind::Upsert,
        id: None,
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        source_sig: Some(Sig {
            size: 1,
            mtime_ms: 1,
            hash: 0,
        }),
        managed: true,
        old_managed: old_rel.is_some(),
    }
}
