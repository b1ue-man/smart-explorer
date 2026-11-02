//! Probes of drive paths take a folder's "not optical" answer for the paths
//! in it (`windows_plain.rs`). Test names start with `abgleich_task_` (the
//! remote task suite selects them). Windows only: the file is compiled
//! through `source_media/os/windows.rs`.
use super::{drive_parent, key, probe_known};
use crate::source_media::probe_cache;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn verbatim(path: &Path) -> PathBuf {
    crate::local_access::normalize_scan_root(path)
}

/// A resolver that counts its calls and answers `optical` for every path.
fn counted(calls: &AtomicUsize, optical: bool) -> impl Fn(&Path) -> io::Result<Option<()>> + '_ {
    move |_: &Path| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(optical.then_some(()))
    }
}

/// `link` as a directory junction to `target` (no privilege needed).
fn junction(target: &Path, link: &Path) {
    let created = std::process::Command::new("cmd.exe")
        .current_dir(link.parent().unwrap())
        .args(["/d", "/c", "mklink", "/J"])
        .arg(link.file_name().unwrap())
        .arg(target)
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "junction creation: {} {}",
        String::from_utf8_lossy(&created.stdout),
        String::from_utf8_lossy(&created.stderr)
    );
}

#[cfg(windows)]
#[test]
fn abgleich_task_paths_below_a_plain_folder_take_its_answer_without_resolving() {
    let dir = tempfile::tempdir().unwrap();
    let root = verbatim(dir.path());
    std::fs::create_dir_all(root.join("a").join("b").join("c")).unwrap();
    std::fs::write(root.join("a").join("y.txt"), b"y").unwrap();
    std::fs::write(root.join("a").join("b").join("c").join("x.txt"), b"x").unwrap();
    let calls = AtomicUsize::new(0);
    let resolve = counted(&calls, false);
    let paths = [
        root.clone(),
        root.join("a"),
        root.join("a").join("y.txt"),
        root.join("a").join("b"),
        root.join("a").join("b").join("c"),
        root.join("a").join("b").join("c").join("x.txt"),
    ];
    for path in &paths {
        assert!(probe_known(path, &resolve).unwrap().is_none(), "{path:?}");
    }
    // At most the root is resolved (its parent, the temp folder, may have a
    // standing answer of another test); before, each of the six was.
    assert!(calls.load(Ordering::SeqCst) <= 1, "{:?}", calls);
    // Folders keep the answer for the paths below them, files do not.
    assert!(probe_cache::plain_since(&key(&root.join("a").join("b"))).is_some());
    assert!(probe_cache::plain_since(&key(&root.join("a").join("y.txt"))).is_none());
}

#[cfg(windows)]
#[test]
fn abgleich_task_a_junction_and_an_optical_answer_are_resolved_every_time() {
    let dir = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    std::fs::create_dir(target.path().join("inner")).unwrap();
    let link = dir.path().join("j");
    junction(target.path(), &link);
    let link = verbatim(&link);
    let inner = link.join("inner");
    let calls = AtomicUsize::new(0);
    let optical = counted(&calls, true);
    for expected in 1..=2 {
        assert!(probe_known(&link, &optical).unwrap().is_some());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected,
            "a junction is resolved itself"
        );
    }
    for expected in 3..=4 {
        assert!(probe_known(&inner, &optical).unwrap().is_some());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected,
            "an optical answer is never kept"
        );
    }
    // A plain answer of the junction itself is kept for it and for the
    // folders reached through it, which lie on the volume it leads to.
    let plain = counted(&calls, false);
    assert!(probe_known(&link, &plain).unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    assert!(probe_known(&inner, &plain).unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    std::fs::remove_dir(&link).unwrap();
}

#[cfg(windows)]
#[test]
fn abgleich_task_only_drive_paths_take_an_answer_from_the_folder_above() {
    assert_eq!(
        drive_parent(Path::new(r"\\?\C:\a\b")),
        Some(Path::new(r"\\?\C:\a"))
    );
    assert_eq!(
        drive_parent(Path::new(r"\\?\C:\a")),
        Some(Path::new(r"\\?\C:\"))
    );
    assert_eq!(
        drive_parent(Path::new(r"\\?\C:\")),
        None,
        "a drive root has none"
    );
    for device in [
        r"\\?\GLOBALROOT\Device\CdRom0\a",
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\a",
        r"\\?\UNC\server\share\a",
    ] {
        assert_eq!(drive_parent(Path::new(device)), None, "{device}");
    }
}
