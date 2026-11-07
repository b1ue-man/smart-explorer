//! R-FS-3 (plan 2026-11-07): an error the Share host sends names its export
//! by the virtual path the peer asked for, never by the folder on the host.
use super::*;
use crate::share::{
    fs::{resolve, ShareExportConfig, SharedRoot},
    fs_host_policy_task_tests::text,
    ExportAccess,
};
use crate::vfs::Backend;
use std::sync::Arc;

fn sent(error: &io::Error) -> (Option<FsErrorKind>, String) {
    let FsResponse::Err { kind, msg, .. } = response(error) else {
        panic!("error response expected");
    };
    (kind, msg)
}

/// T-R-FS-3, the text: every spelling of a remembered root becomes the
/// export's name, the longest root wins, a longer name stays, and the root of
/// a whole volume is not remembered at all.
#[test]
fn review_task_error_text_names_the_virtual_export_root() {
    note_export_root("/home/u/Docs", "/Docs");
    let missing = io::Error::new(io::ErrorKind::NotFound, "Ordner /home/u/Docs/a/b fehlt");
    let (kind, msg) = sent(&missing);
    assert_eq!(msg, "Ordner /Docs/a/b fehlt");
    assert_eq!(kind, Some(FsErrorKind::NotFound));
    assert_eq!(virtual_text("/home/u/Docs"), "/Docs");
    assert_eq!(virtual_text("/home/u/Docs2/x"), "/home/u/Docs2/x");
    assert_eq!(virtual_text("/home/u/Docs-alt/x"), "/home/u/Docs-alt/x");

    note_export_root("C:/Users/u/Docs", "/Docs");
    assert_eq!(virtual_text(r"C:\Users\u\Docs\a"), r"/Docs\a");
    assert_eq!(virtual_text(r"\\?\C:\Users\u\Docs\a"), r"/Docs\a");
    assert_eq!(virtual_text("//?/C:/Users/u/Docs/a"), "/Docs/a");

    note_export_root("/home/u/Docs/tief", "/Tief");
    assert_eq!(virtual_text("/home/u/Docs/tief/x"), "/Tief/x");
    assert_eq!(virtual_text("/home/u/Docs/anders/x"), "/Docs/anders/x");

    for volume in ["/", "C:/", r"C:\"] {
        note_export_root(volume, "/Wurzel");
    }
    let plain = "Fehler in /a/b und C:/x und C:\\y";
    assert_eq!(virtual_text(plain), plain);
}

/// T-R-FS-3, end to end: `MkdirAll /Docs/datei.txt/neu` below a file fails
/// with a text that names the file on the host; the host sends it as
/// `/Docs/datei.txt`, with the same error kind.
#[test]
fn review_task_mkdir_under_a_file_is_sent_without_the_host_path() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().canonicalize().unwrap();
    std::fs::write(folder.join("datei.txt"), b"x").unwrap();
    let exports = Arc::new(Mutex::new(ShareExportConfig {
        roots: vec![SharedRoot::new("Docs", text(&folder)).with_access(ExportAccess::ReadWrite)],
        ..Default::default()
    }));
    let target = resolve("/Docs/datei.txt/neu", &exports).unwrap();
    let error = target.backend.mkdir_all(&target.path).unwrap_err();
    let host = text(&folder);
    let host = host.strip_prefix("//?/").unwrap_or(&host);
    assert!(
        error.to_string().replace('\\', "/").contains(host),
        "{error}"
    );

    let (kind, msg) = sent(&error);
    assert!(!msg.replace('\\', "/").contains(host), "{msg}");
    assert!(msg.replace('\\', "/").contains("/Docs/datei.txt"), "{msg}");
    assert_eq!(kind, kind_of(&error));
}
