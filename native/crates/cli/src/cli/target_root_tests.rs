//! Roots `se rm` protects until `--no-preserve-root`: Windows verbatim and
//! device roots by their spelling (U15) and local mount points such as
//! `/media/<user>/<STICK>` (U5, like `rm --preserve-root=all`).
use super::is_local_namespace_root;

#[test]
fn wechselmedium_task_verbatim_and_device_roots_are_namespace_roots() {
    for root in [
        "//?/E:",
        "//?/E:/",
        "\\\\?\\E:\\",
        "//./E:",
        "//?/Volume{0b3e8f27-1c2d-4e5f-8a9b-0c1d2e3f4a5b}/",
        "//?/UNC/server/share",
        "//?/GLOBALROOT/Device/HarddiskVolume3",
    ] {
        assert!(is_local_namespace_root(root), "expected root: {root}");
    }
    for child in [
        "//?/E:/DCIM",
        "//?/Volume{0b3e8f27-1c2d-4e5f-8a9b-0c1d2e3f4a5b}/DCIM",
        "//?/UNC/server/share/dir",
    ] {
        assert!(!is_local_namespace_root(child), "expected child: {child}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn wechselmedium_task_mount_points_are_preserved_roots() {
    use super::Target;
    use crate::vfs::{BackendHandle, LocalBackend};
    let local = |path: &str| {
        let backend: BackendHandle = std::sync::Arc::new(LocalBackend::new("/"));
        Target::with_backend_key(backend, path.to_string(), "local")
    };
    // `/proc` is a mount point on every Linux host the suites run on.
    assert!(local("/proc").is_preserved_root());
    let scratch = tempfile::tempdir().expect("scratch");
    let folder = scratch.path().join("folder");
    std::fs::create_dir(&folder).expect("folder");
    assert!(!local(folder.to_string_lossy().as_ref()).is_preserved_root());
    // A link to a mount point is removed as a link: never protected.
    let link = scratch.path().join("proc-link");
    std::os::unix::fs::symlink("/proc", &link).expect("link");
    assert!(!local(link.to_string_lossy().as_ref()).is_preserved_root());
    let error = super::super::ops::remove(&local("/proc"), true, true, false)
        .expect_err("a mount point needs --no-preserve-root");
    assert!(error.contains("--no-preserve-root"), "{error}");
}
