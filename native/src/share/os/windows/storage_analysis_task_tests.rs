use crate::{analytics::{Progress, ScanStatus}, share::CopyPastePeerFixture};
use std::io;

#[test]
fn windows_remote_task_analysis_preserves_junction_boundary_and_plain_directories() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::new()?;
    let outside = tempfile::tempdir()?;
    std::fs::write(outside.path().join("must-not-read"), b"outside")?;
    std::fs::create_dir(fixture.root_a.join("node_modules"))?;
    std::fs::write(fixture.root_a.join("node_modules/kept"), b"kept")?;
    let link = fixture.root_a.join("junction");
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"]).arg(&link).arg(outside.path()).output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result = (|| {
        let progress = Progress::default();
        let remote = fixture.backend.scan_storage("/A", &progress)?.unwrap();
        assert_eq!(remote.status, ScanStatus::Complete);
        assert_eq!(progress.snapshot().files, 1);
        assert_eq!(progress.snapshot().bytes, 4);
        let tree = remote.tree.unwrap();
        assert!(tree.children.iter().any(|node| &*node.name == "node_modules"));
        assert!(!tree.children.iter().any(|node| &*node.name == "junction"));
        assert!(fixture.backend.scan_storage("/A/junction", &Progress::default()).is_err());
        Ok(())
    })();
    std::fs::remove_dir(&link)?;
    result
}
