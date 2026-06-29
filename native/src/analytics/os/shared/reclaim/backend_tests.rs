use super::backend::scan_reclaim_backend;
use super::types::{DuplicateEvidence, ReclaimOptions, ReclaimProgress};
use crate::vfs::{Backend, HashHit, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MockBackend {
    entries: Mutex<HashMap<String, Vec<VfsMeta>>>,
    open_reads: AtomicUsize,
    walk_hits: Mutex<Option<Vec<HashHit>>>,
}

impl MockBackend {
    fn with_entries(entries: Vec<VfsMeta>) -> Arc<Self> {
        let be = Arc::new(Self::default());
        be.entries.lock().unwrap().insert("/".to_string(), entries);
        be
    }

    fn with_walk_hits(hits: Vec<HashHit>) -> Arc<Self> {
        let be = Arc::new(Self::default());
        *be.walk_hits.lock().unwrap() = Some(hits);
        be
    }
}

impl Backend for MockBackend {
    fn scheme(&self) -> Scheme {
        Scheme::GDrive
    }

    fn root_display(&self) -> String {
        "/".to_string()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.entries
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path))
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stat"))
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.open_reads.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "write"))
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "rename"))
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "remove"))
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "remove"))
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "mkdir"))
    }

    fn supports_walk_hashed(&self) -> bool {
        self.walk_hits.lock().unwrap().is_some()
    }

    fn walk_hashed(
        &self,
        _root: &str,
        _want_hash: bool,
        tx: crossbeam_channel::Sender<HashHit>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> bool {
        let Some(hits) = self.walk_hits.lock().unwrap().clone() else {
            return false;
        };
        for hit in hits {
            let _ = tx.send(hit);
        }
        true
    }
}

fn file(name: &str, size: u64, md5: Option<&str>) -> VfsMeta {
    VfsMeta {
        name: name.to_string(),
        is_dir: false,
        size,
        mtime_ms: 1,
        content_md5: md5.map(str::to_string),
        ..VfsMeta::default()
    }
}

#[test]
fn provider_md5_groups_without_open_read() {
    let md5 = "900150983cd24fb0d6963f7d28e17f72";
    let be = MockBackend::with_entries(vec![
        file("a.bin", 3, Some(md5)),
        file("b.bin", 3, Some(md5)),
    ]);
    let p = ReclaimProgress::default();
    let opts = ReclaimOptions {
        duplicate_min_bytes: 1,
        ..ReclaimOptions::default()
    };
    let report = scan_reclaim_backend(be.clone(), "/", &p, &opts);
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(
        report.duplicate_groups[0].evidence,
        DuplicateEvidence::ProviderMd5
    );
    assert_eq!(be.open_reads.load(Ordering::Relaxed), 0);
}

#[test]
fn hashless_remote_does_not_download_to_hash() {
    let be = MockBackend::with_entries(vec![file("a.bin", 3, None), file("b.bin", 3, None)]);
    let p = ReclaimProgress::default();
    let opts = ReclaimOptions {
        duplicate_min_bytes: 1,
        ..ReclaimOptions::default()
    };
    let report = scan_reclaim_backend(be.clone(), "/", &p, &opts);
    assert!(report.duplicate_groups.is_empty());
    assert_eq!(be.open_reads.load(Ordering::Relaxed), 0);
}

#[test]
fn agent_walk_hashed_is_preferred() {
    let md5 = "900150983cd24fb0d6963f7d28e17f72".to_string();
    let be = MockBackend::with_walk_hits(vec![
        HashHit {
            rel: "a.bin".into(),
            is_dir: false,
            size: 3,
            mtime_ms: 1,
            md5: Some(md5.clone()),
        },
        HashHit {
            rel: "b.bin".into(),
            is_dir: false,
            size: 3,
            mtime_ms: 2,
            md5: Some(md5),
        },
    ]);
    let p = ReclaimProgress::default();
    let opts = ReclaimOptions {
        duplicate_min_bytes: 1,
        ..ReclaimOptions::default()
    };
    let report = scan_reclaim_backend(be.clone(), "/", &p, &opts);
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(
        report.duplicate_groups[0].evidence,
        DuplicateEvidence::AgentMd5
    );
    assert_eq!(be.open_reads.load(Ordering::Relaxed), 0);
}
