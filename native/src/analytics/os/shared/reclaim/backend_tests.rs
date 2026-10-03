use super::backend::{find_backend_duplicates, scan_reclaim_backend};
use super::backend_duplicates::{Candidate, Candidates};
use super::finder::{DuplicateReport, DuplicateSummary};
use super::types::{DuplicateEvidence, ReclaimItem, ReclaimOptions, ReclaimProgress};
use crate::vfs::{
    Backend, BackendExtensions, HashWalkEntry, HashWalkItem, HashWalkRequest, Scheme, VfsMeta,
    VfsResult,
};
use std::collections::HashMap;
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct MockBackend {
    scheme: Option<Scheme>,
    entries: Mutex<HashMap<String, Vec<VfsMeta>>>,
    contents: Mutex<HashMap<String, Vec<u8>>>,
    lists: AtomicUsize,
    open_reads: AtomicUsize,
    read_paths: Mutex<Vec<String>>,
    walk_hits: Mutex<Option<Vec<HashWalkItem>>>,
    walk_error: Mutex<Option<String>>,
    /// The walk sends nothing and waits for its cancel (one large file
    /// being hashed); `walk_canceled` records that the cancel arrived.
    walk_blocks: bool,
    walk_started: Option<crossbeam_channel::Sender<()>>,
    walk_canceled: AtomicBool,
    /// The storing host's own duplicate search.
    host_search: Option<DuplicateReport>,
    host_withdraws: bool,
    /// Reads may start mid-file (`open_read_at`).
    seekable: bool,
}

impl MockBackend {
    fn with_entries(entries: Vec<VfsMeta>) -> Arc<Self> {
        let be = Arc::new(Self::default());
        be.entries.lock().unwrap().insert("/".to_string(), entries);
        be
    }

    fn with_walk_hits(hits: Vec<HashWalkItem>) -> Arc<Self> {
        let be = Arc::new(Self::default());
        *be.walk_hits.lock().unwrap() = Some(hits);
        be
    }

    fn with_failing_walk(hits: Vec<HashWalkItem>, error: &str) -> Arc<Self> {
        let be = Self::with_walk_hits(hits);
        *be.walk_error.lock().unwrap() = Some(error.to_string());
        be
    }
}

impl Backend for MockBackend {
    fn scheme(&self) -> Scheme {
        self.scheme.unwrap_or(Scheme::GDrive)
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

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.open_reads.fetch_add(1, Ordering::Relaxed);
        self.read_paths.lock().unwrap().push(path.to_string());
        let bytes = self
            .contents
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_default();
        Ok(Box::new(Cursor::new(bytes)))
    }

    fn open_read_at(
        &self,
        path: &str,
        _id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        if !self.seekable {
            return Ok(None);
        }
        self.open_reads.fetch_add(1, Ordering::Relaxed);
        self.read_paths
            .lock()
            .unwrap()
            .push(format!("{path}@{offset}"));
        let bytes = self
            .contents
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_default();
        let mut cursor = Cursor::new(bytes);
        cursor.set_position(offset);
        Ok(Some(Box::new(cursor)))
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

    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
}

impl BackendExtensions for MockBackend {
    fn supports_duplicate_search(&self, _root: &str) -> VfsResult<bool> {
        Ok(self.host_search.is_some() || self.host_withdraws)
    }

    fn find_duplicates(
        &self,
        _root: &str,
        _min_bytes: u64,
        progress: &ReclaimProgress,
    ) -> VfsResult<Option<DuplicateReport>> {
        if self.host_withdraws {
            progress.files.store(100, Ordering::Relaxed);
        }
        Ok(self.host_search.as_ref().map(|report| DuplicateReport {
            groups: report.groups.clone(),
            summary: report.summary.clone(),
            root_error: report.root_error.clone(),
        }))
    }

    fn supports_hash_walk(&self, _root: &str) -> VfsResult<bool> {
        Ok(self.walk_blocks || self.walk_hits.lock().unwrap().is_some())
    }

    fn hash_walk(
        &self,
        _root: &str,
        _request: HashWalkRequest,
        tx: crossbeam_channel::Sender<HashWalkItem>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        if self.walk_blocks {
            drop(tx); // No entries; the backend call remains active.
            if let Some(started) = &self.walk_started {
                let _ = started.send(());
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if cancel.load(Ordering::Relaxed) {
                    self.walk_canceled.store(true, Ordering::Relaxed);
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "canceled"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            return Ok(true);
        }
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

fn hit(rel: &str, size: u64, md5: &str) -> HashWalkItem {
    HashWalkItem::Entry(HashWalkEntry {
        rel: rel.into(),
        is_dir: false,
        size,
        mtime_ms: 1,
        digest: Some(md5.into()),
    })
}

fn duplicates_from_one_byte() -> ReclaimOptions {
    ReclaimOptions {
        duplicate_min_bytes: 1,
        ..ReclaimOptions::default()
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
    let report = scan_reclaim_backend(be.clone(), "/", &p, &duplicates_from_one_byte());
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(
        report.duplicate_groups[0].evidence,
        DuplicateEvidence::ProviderMd5
    );
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
    let md5 = "900150983cd24fb0d6963f7d28e17f72";
    let be = MockBackend::with_walk_hits(vec![hit("a.bin", 3, md5), hit("b.bin", 3, md5)]);
    let p = ReclaimProgress::default();
    let report = scan_reclaim_backend(be.clone(), "/", &p, &duplicates_from_one_byte());
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(
        report.duplicate_groups[0].evidence,
        DuplicateEvidence::AgentMd5
    );
    assert_eq!(be.open_reads.load(Ordering::Relaxed), 0);
}

#[test]
fn review_task_agent_walk_failure_falls_back_to_listing() {
    let md5 = "900150983cd24fb0d6963f7d28e17f72";
    let be = MockBackend::with_failing_walk(
        vec![hit("partial.bin", 3, md5)],
        crate::agent_proto::HASH_WALK_LINK_BOUNDARY,
    );
    be.entries.lock().unwrap().insert(
        "/".to_string(),
        vec![file("a.bin", 3, Some(md5)), file("b.bin", 3, Some(md5))],
    );
    let report = scan_reclaim_backend(
        be.clone(),
        "/",
        &ReclaimProgress::default(),
        &duplicates_from_one_byte(),
    );
    // The listing walk starts over: nothing of the failed stream is counted.
    assert_eq!(report.files, 2);
    assert!(report.root_error.is_none(), "{:?}", report.root_error);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(be.lists.load(Ordering::Relaxed), 1);
}

#[test]
fn review_task_host_search_is_preferred_over_any_walk() {
    let group = super::types::DuplicateGroup {
        hash: super::types::ContentHash {
            algorithm: super::types::HashAlgorithm::Sha256,
            hex: "ab".into(),
        },
        evidence: DuplicateEvidence::LocalSha256,
        size: 3,
        reclaimable: 3,
        items: vec![
            ReclaimItem::new("/a.bin".into(), "a.bin".into(), 3, 1, false),
            ReclaimItem::new("/b.bin".into(), "b.bin".into(), 3, 1, false),
        ],
    };
    let be = Arc::new(MockBackend {
        scheme: Some(Scheme::Peer),
        host_search: Some(DuplicateReport {
            groups: vec![group],
            summary: DuplicateSummary {
                files: 9,
                candidates: 2,
                compared: 2,
                groups: 1,
                ..DuplicateSummary::default()
            },
            root_error: None,
        }),
        ..MockBackend::default()
    });
    *be.walk_hits.lock().unwrap() = Some(vec![hit("a.bin", 3, "900150983cd24fb0d6963f7d28e17f72")]);
    let report = find_backend_duplicates(be.clone(), "/", &ReclaimProgress::default(), 1);
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.summary.files, 9);
    assert_eq!(
        be.lists.load(Ordering::Relaxed),
        0,
        "the host searched, nothing was listed"
    );
    assert_eq!(be.open_reads.load(Ordering::Relaxed), 0);

    let desktop = duplicates_from_one_byte();
    be.entries.lock().unwrap().insert(
        "/".to_string(),
        vec![file("a.bin", 3, None), file("b.bin", 3, None)],
    );
    let reclaim = scan_reclaim_backend(be.clone(), "/", &ReclaimProgress::default(), &desktop);
    assert_eq!(reclaim.duplicate_groups.len(), 1);
    assert_eq!(reclaim.duplicate_candidates, 2);
    assert_eq!(
        be.open_reads.load(Ordering::Relaxed),
        0,
        "no content compared here"
    );
}

#[test]
fn review_task_withdrawn_host_search_keeps_the_sparse_fallback() {
    let be = Arc::new(MockBackend {
        host_withdraws: true,
        ..MockBackend::default()
    });
    be.entries.lock().unwrap().insert(
        "/".into(),
        vec![
            file("a", 3, None),
            file("b", 3, None),
            file("unique", 7, None),
        ],
    );
    be.contents.lock().unwrap().extend([
        ("/a".into(), b"abc".to_vec()),
        ("/b".into(), b"abc".to_vec()),
    ]);
    let report = find_backend_duplicates(be.clone(), "/", &ReclaimProgress::default(), 1);
    assert!(report.root_error.is_none());
    assert_eq!(report.summary.files, 3);
    assert_eq!(report.groups.len(), 1);
    assert!(!be
        .read_paths
        .lock()
        .unwrap()
        .iter()
        .any(|path| path == "/unique"));
    be.read_paths.lock().unwrap().clear();
    let report = scan_reclaim_backend(
        be.clone(),
        "/",
        &ReclaimProgress::default(),
        &duplicates_from_one_byte(),
    );
    assert_eq!(report.files, 3);
    assert_eq!(report.duplicate_groups.len(), 1);
    assert!(!be
        .read_paths
        .lock()
        .unwrap()
        .iter()
        .any(|path| path == "/unique"));
}

#[test]
fn review_task_agent_walk_cancel_arrives_without_entries() {
    let (walk_started_tx, walk_started_rx) = crossbeam_channel::bounded(1);
    let be = Arc::new(MockBackend {
        walk_blocks: true,
        walk_started: Some(walk_started_tx),
        ..MockBackend::default()
    });
    let progress = ReclaimProgress::default();
    let cancel = progress.cancel.clone();
    let started = Instant::now();
    let canceler = std::thread::spawn(move || {
        walk_started_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        cancel.store(true, Ordering::Relaxed);
    });
    let report = find_backend_duplicates(be.clone(), "/", &progress, 1);
    canceler.join().unwrap();
    assert!(be.walk_canceled.load(Ordering::Relaxed));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(report.root_error.is_none());
    assert_eq!(be.lists.load(Ordering::Relaxed), 0);
    assert_eq!(be.open_reads.load(Ordering::Relaxed), 0);
}

#[test]
fn backend_retains_candidates_by_memory_and_groups_by_the_display_cap() {
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
    // Candidates are bounded by their memory, not by the display cap.
    assert_eq!(report.duplicate_candidates, 8);
    assert_eq!(report.duplicate_candidates_retained, 8);
    assert_eq!(report.result_counts.duplicate_groups, 1);
    assert_eq!(report.duplicate_groups.len(), 1);
    assert!(report
        .large_files
        .iter()
        .all(|item| item.name.starts_with("largest-")));
}

#[test]
fn review_task_candidate_memory_keeps_the_largest_files() {
    let candidate = |name: &str, size| Candidate {
        item: ReclaimItem::new(format!("/{name}"), name.to_string(), size, 0, false),
        hash: None,
    };
    // Path and name of "x.bin" take 11 bytes; room for two of them.
    let mut kept = Candidates::new(22);
    for (name, size) in [("a.bin", 5), ("b.bin", 50), ("c.bin", 1), ("d.bin", 20)] {
        kept.offer(candidate(name, size));
    }
    assert_eq!((kept.seen(), kept.dropped(), kept.len()), (4, 2, 2));
    let mut sizes: Vec<u64> = kept.take().iter().map(|kept| kept.item.size).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, [20, 50]);
}

/// Content comparison of hashless candidates (`backend_compare.rs`).
#[path = "backend_compare_tests.rs"]
mod compare;
