use super::*;
use crate::{analytics::{Progress, ScanPhase, ScanStatus}, share::CopyPastePeerFixture};

#[test]
fn windows_remote_task_analysis_combines_export_roots_and_rejects_escape() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::with_labels(["Docs", "docs"])?;
    std::fs::create_dir_all(fixture.root_a.join("node_modules/ordinary"))?;
    std::fs::write(fixture.root_a.join("node_modules/ordinary/file"), b"abc")?;
    std::fs::write(fixture.root_b.join("file"), b"12345")?;
    let progress = Progress::default();
    let result = fixture.backend.scan_storage("/", &progress)?.unwrap();
    assert_eq!(result.status, ScanStatus::Complete);
    assert_eq!(progress.snapshot().files, 2);
    assert_eq!(progress.snapshot().bytes, 8);
    assert_eq!(progress.snapshot().dirs, 4); // two export roots and two nested dirs
    let tree = result.tree.unwrap();
    assert_eq!(tree.size, 8);
    assert_eq!(tree.children.iter().map(|node| &*node.name).collect::<Vec<_>>(), ["Docs", "docs"]);
    let escaped = fixture.backend.scan_storage("/Docs/../docs", &Progress::default());
    assert!(escaped.is_err(), "operation paths must retain their authorization boundary");
    Ok(())
}

#[test]
fn windows_remote_task_analysis_cancel_closes_queued_peer_request() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::new()?;
    let backend = fixture.backend.clone();
    let held = fixture.peer.node.block_on(async {
        let one = slots().acquire_owned().await.unwrap();
        let two = slots().acquire_owned().await.unwrap();
        (one, two)
    });
    let progress = Progress::default();
    let worker_progress = progress.clone();
    let worker = std::thread::spawn(move || backend.scan_storage("/A", &worker_progress));
    let deadline = Instant::now() + Duration::from_secs(5);
    while progress.snapshot().phase != ScanPhase::Queued && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(progress.snapshot().phase, ScanPhase::Queued);
    progress.cancel.store(true, Ordering::Relaxed);
    assert_eq!(worker.join().unwrap().err().unwrap().kind(), io::ErrorKind::Interrupted);
    drop(held);
    // A canceled queued request cannot strand the bounded worker pool.
    let result = fixture.backend.scan_storage("/A", &Progress::default())?.unwrap();
    assert_eq!(result.status, ScanStatus::Complete);
    assert_eq!(fixture.backend.list_dir("/")?.len(), 2);
    Ok(())
}
