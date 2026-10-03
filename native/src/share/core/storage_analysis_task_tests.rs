use super::*;
use crate::{analytics::{Progress, ScanStatus}, share::CopyPastePeerFixture};

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
fn windows_remote_task_analysis_precancel_preserves_browsing() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::new()?;
    let progress = Progress::default();
    progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(fixture.backend.scan_storage("/A", &progress).err().unwrap().kind(), io::ErrorKind::Interrupted);
    // A canceled queued request cannot strand the bounded worker pool.
    let result = fixture.backend.scan_storage("/A", &Progress::default())?.unwrap();
    assert_eq!(result.status, ScanStatus::Complete);
    assert_eq!(fixture.backend.list_dir("/")?.len(), 2);
    Ok(())
}
