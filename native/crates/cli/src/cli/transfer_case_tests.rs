//! `se mv` of a name to another case in its folder (U10) and the local
//! cross-volume rename fallback (U12).
use super::try_case_rename;
use crate::cli::target::Target;
use crate::vfs::{BackendHandle, LocalBackend};
use std::io;
use std::sync::Arc;

fn local(path: &std::path::Path) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    Target::with_backend_key(backend, path.to_string_lossy().replace('\\', "/"), "local")
}

fn names(folder: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn wechselmedium_task_case_rename_changes_only_the_spelling() {
    let scratch = tempfile::tempdir().expect("scratch");
    let folder = scratch.path();
    std::fs::write(folder.join("foto.jpg"), b"picture").expect("file");
    let renamed = try_case_rename(
        &local(&folder.join("foto.jpg")),
        &local(&folder.join("Foto.jpg")),
        true,
    )
    .expect("no error");
    // A folding filesystem (Windows NTFS) renames in two steps; a
    // case-sensitive one has no such entry and leaves it to `se mv`.
    let expected = if renamed { "Foto.jpg" } else { "foto.jpg" };
    assert_eq!(names(folder), [expected], "no intermediate name is left");
    assert_eq!(
        std::fs::read(folder.join(expected)).expect("content"),
        b"picture"
    );
}

#[test]
fn wechselmedium_task_case_rename_never_touches_another_entry() {
    let scratch = tempfile::tempdir().expect("scratch");
    let folder = scratch.path();
    std::fs::write(folder.join("a.txt"), b"a").expect("file");
    // Another name, another folder or the very same name: not this case.
    for destination in ["b.txt", "sub/A.txt", "a.txt"] {
        if let Some(parent) = folder.join(destination).parent() {
            std::fs::create_dir_all(parent).expect("folder");
        }
        let renamed = try_case_rename(
            &local(&folder.join("a.txt")),
            &local(&folder.join(destination)),
            true,
        )
        .expect("no error");
        assert!(!renamed, "{destination}");
    }
    assert!(folder.join("a.txt").exists());
    #[cfg(target_os = "linux")]
    {
        // Two different files whose names differ in case only.
        std::fs::write(folder.join("A.txt"), b"other").expect("second file");
        let renamed = try_case_rename(
            &local(&folder.join("a.txt")),
            &local(&folder.join("A.txt")),
            true,
        )
        .expect("no error");
        assert!(!renamed);
        assert_eq!(std::fs::read(folder.join("A.txt")).expect("kept"), b"other");
        assert_eq!(std::fs::read(folder.join("a.txt")).expect("kept"), b"a");
    }
}

#[test]
fn wechselmedium_task_cross_volume_rename_code_is_the_platform_one() {
    assert!(crate::cli::os::crosses_devices(&io::Error::from(
        io::ErrorKind::CrossesDevices
    )));
    #[cfg(windows)]
    assert!(crate::cli::os::crosses_devices(
        &io::Error::from_raw_os_error(17)
    ));
    #[cfg(not(windows))]
    {
        assert!(crate::cli::os::crosses_devices(
            &io::Error::from_raw_os_error(libc::EXDEV)
        ));
        assert!(
            !crate::cli::os::crosses_devices(&io::Error::from_raw_os_error(libc::EEXIST)),
            "17 is EEXIST here, never a cross-volume rename"
        );
    }
}
