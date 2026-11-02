//! Acceptance for B22: suppress only the successful, scoped written content.
use super::*;

#[test]
fn review_task_own_write_suppression_requires_success_scope_and_matching_content() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file.txt");
    std::fs::write(&file, b"abcd").unwrap();
    let endpoint = directory.path().to_str().unwrap().to_string();
    let (backend, root) = crate::connect::resolve_endpoint(&endpoint).unwrap();
    let path = format!("{}/file.txt", root.trim_end_matches('/'));
    let cancel = AtomicBool::new(false);
    let meta = backend.stat(&path).unwrap();
    let sig = Sig {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        hash: crate::bisync::current_content_signature(&*backend, &path, &cancel).unwrap(),
    };
    let generation = 42;
    let observer = Observer {
        job_id: endpoint.clone(),
        a: backend.clone(),
        b: backend.clone(),
        root_a: root.clone(),
        root_b: root,
        source: endpoint.clone(),
        target: endpoint.clone(),
        generation,
        progress: std::sync::Arc::new(AtomicI64::new(0)),
    };
    observer.completed(CompletedAction {
        rel: "file.txt".into(),
        kind: CompletedKind::Copied { from: PairSide::A },
        src_sig: Some(sig),
        dst_sig: Some(sig),
        durable: true,
    });
    assert!(candidate(
        &endpoint,
        PairSide::B,
        "file.txt",
        &endpoint,
        Some(generation)
    ));
    // A cancelled/failed attempt has no successful completion proof.
    assert!(!matches(
        &endpoint,
        PairSide::B,
        "file.txt",
        &endpoint,
        Some(generation),
        &cancel
    ));
    succeeded(&endpoint, generation);
    assert!(!matches(
        &endpoint,
        PairSide::A,
        "file.txt",
        &endpoint,
        Some(generation),
        &cancel
    ));
    assert!(!matches(
        &endpoint,
        PairSide::B,
        "file.txt",
        "different-location",
        Some(generation),
        &cancel
    ));
    assert!(!matches(
        &endpoint,
        PairSide::B,
        "file.txt",
        &endpoint,
        Some(generation + 1),
        &cancel
    ));
    assert!(matches(
        &endpoint,
        PairSide::B,
        "file.txt",
        &endpoint,
        Some(generation),
        &cancel
    ));

    let original_time = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::fs::write(&file, b"wxyz").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(original_time))
        .unwrap();
    let changed = backend.stat(&path).unwrap();
    assert_eq!((changed.size, changed.mtime_ms), (sig.size, sig.mtime_ms));
    assert!(!matches(
        &endpoint,
        PairSide::B,
        "file.txt",
        &endpoint,
        Some(generation),
        &cancel
    ));

    let hashless = Sig { hash: 0, ..sig };
    observer.completed(CompletedAction {
        rel: "file.txt".into(),
        kind: CompletedKind::Copied { from: PairSide::A },
        src_sig: Some(hashless),
        dst_sig: Some(hashless),
        durable: true,
    });
    succeeded(&endpoint, generation);
    assert!(!matches(
        &endpoint,
        PairSide::B,
        "file.txt",
        &endpoint,
        Some(generation),
        &cancel
    ));
    forget(&endpoint);
}
