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
    lists: AtomicUsize,
    open_reads: AtomicUsize,
    walk_hits: Mutex<Option<Vec<HashHit>>>,
    walk_error: Mutex<Option<String>>,
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

    fn with_failing_walk(hits: Vec<HashHit>, error: &str) -> Arc<Self> {
        let be = Self::with_walk_hits(hits);
        *be.walk_error.lock().unwrap() = Some(error.to_string());
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
        self.lists.fetch_add(1, Ordering::Relaxed);
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
    ) -> VfsResult<bool> {
        let Some(hits) = self.walk_hits.lock().unwrap().clone() else {
            return Ok(false);
        };
        for hit in hits {
            let _ = tx.send(hit);
        }
        if let Some(error) = self.walk_error.lock().unwrap().clone() {
            return Err(io::Error::other(error));
        }
        Ok(true)
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
fn root_listing_failure_is_explicit() {
    let be = Arc::new(MockBackend::default());
    let report = scan_reclaim_backend(
        be,
        "/missing",
        &ReclaimProgress::default(),
        &ReclaimOptions::default(),
    );
    assert!(report.root_error.is_some());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.suppressed_errors, 0);
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

#[test]
fn partial_agent_walk_error_is_reported_without_listing_fallback() {
    let be = MockBackend::with_failing_walk(
        vec![HashHit {
            rel: "partial.bin".into(),
            is_dir: false,
            size: 3,
            mtime_ms: 1,
            md5: Some("900150983cd24fb0d6963f7d28e17f72".into()),
        }],
        "late hash failure",
    );
    let report = scan_reclaim_backend(
        be.clone(),
        "/",
        &ReclaimProgress::default(),
        &ReclaimOptions {
            duplicate_min_bytes: 1,
            ..ReclaimOptions::default()
        },
    );

    assert_eq!(report.files, 1);
    assert!(report
        .root_error
        .as_deref()
        .is_some_and(|error| error.contains("late hash failure")));
    assert_eq!(report.errors.len(), 1);
    assert_eq!(be.lists.load(Ordering::Relaxed), 0);
}

#[test]
fn backend_retains_only_bounded_top_candidates_and_reports_totals() {
    let largest_md5 = "900150983cd24fb0d6963f7d28e17f72";
    let other_md5 = "d41d8cd98f00b204e9800998ecf8427e";
    let mut entries = vec![
        file("largest-a.bin", 32, Some(largest_md5)),
        file("largest-b.bin", 32, Some(largest_md5)),
    ];
    for index in 1..=6 {
        entries.push(file(&format!("small-{index}.bin"), index, Some(other_md5)));
    }
    let backend = MockBackend::with_entries(entries);
    let report = scan_reclaim_backend(
        backend,
        "/",
        &ReclaimProgress::default(),
        &ReclaimOptions {
            large_min_bytes: 1,
            duplicate_min_bytes: 1,
            max_items: 2,
            ..ReclaimOptions::default()
        },
    );

    assert_eq!(report.result_counts.large_files, 8);
    assert_eq!(report.large_files.len(), 2);
    assert_eq!(report.duplicate_candidates, 8);
    assert_eq!(report.duplicate_candidates_retained, 2);
    assert_eq!(report.result_counts.duplicate_groups, 1);
    assert_eq!(report.duplicate_groups.len(), 1);
    assert!(report
        .large_files
        .iter()
        .all(|item| item.name.starts_with("largest-")));
}
