use super::*;

#[test]
fn search_recursive_access_task_local_budget_keeps_buffered_results() {
    let fixture = tempfile::tempdir().unwrap();
    for number in 0..8 {
        std::fs::write(fixture.path().join(format!("{number}.blend")), b"x").unwrap();
    }
    let (tx, rx) = crossbeam_channel::unbounded();
    let truncated = Arc::new(AtomicBool::new(false));
    let scanner = Arc::new(Scanner {
        opts: ScanOpts::everything(None),
        tx,
        cancel: Arc::new(AtomicBool::new(false)),
        scanned: Arc::new(AtomicU64::new(0)),
        bytes: Arc::new(AtomicU64::new(0)),
        errors: Arc::new(AtomicU64::new(0)),
        permission_denied: AtomicU64::new(0),
        start: Instant::now(),
        sample_path: Arc::new(Mutex::new(String::new())),
        failed_paths: Arc::new(Mutex::new(Vec::new())),
        budget: ScanBudget::with_limits(3, 8192, 512),
        budget_exhausted: AtomicBool::new(false),
        truncated: truncated.clone(),
        visited_directories: Mutex::new(HashSet::new()),
    });
    walk_parallel(
        &scanner,
        vec![PendingDir {
            path: fixture.path().to_path_buf(),
            lineage: None,
        }],
        1,
    );
    let retained: usize = rx
        .try_iter()
        .filter_map(|message| match message {
            ScanMessage::Entries(entries) => Some(entries.len()),
            _ => None,
        })
        .sum();
    assert_eq!(retained, 3);
    assert!(truncated.load(Ordering::Relaxed));
    assert!(!scanner.failed_paths.lock().unwrap().is_empty());
}
