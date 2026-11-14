//! R-FS-3 (TODO `SEC-FS-FREIGABE-FEHLER`): an error about a path under a
//! share reaches the peer with the share's virtual root, never with the
//! host path that `resolve` found for it.
use super::*;
use crate::share::fs_host_policy_task_tests::text;
use crate::share::wire::FsResponse;

#[test]
fn fs_fehler_task_error_under_a_share_names_the_virtual_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("fehlerprobe-freigabe");
    std::fs::create_dir(&root).unwrap();
    let exports = Arc::new(Mutex::new(ShareExportConfig {
        roots: vec![
            SharedRoot::new("Fehlerprobe", text(&root)).with_access(ExportAccess::ReadWrite)
        ],
        ..Default::default()
    }));

    // The resolution itself names the host path of the missing file.
    let target = resolve("/Fehlerprobe/unter/fehlt.txt", &exports).expect("resolved");
    assert!(
        target.path.contains("fehlerprobe-freigabe"),
        "{}",
        target.path
    );
    let error = io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} nicht gefunden", target.path),
    );
    let FsResponse::Err { msg, .. } = crate::share::fs_error::response(&error) else {
        panic!("error response expected");
    };
    assert_eq!(
        msg.replace('\\', "/"),
        "/Fehlerprobe/unter/fehlt.txt nicht gefunden"
    );
    assert!(!msg.contains("fehlerprobe-freigabe"), "{msg}");

    // The same with the spelling the host OS uses for the root.
    let spelled = text(&root).replace('/', "\\");
    let error = io::Error::other(format!("{spelled}\\unter gesperrt"));
    let FsResponse::Err { msg, .. } = crate::share::fs_error::response(&error) else {
        panic!("error response expected");
    };
    assert!(!msg.contains("fehlerprobe-freigabe"), "{msg}");
    assert!(msg.starts_with("/Fehlerprobe"), "{msg}");
}
