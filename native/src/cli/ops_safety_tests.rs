use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::ops::{copy, move_path};
use super::target::Target;
use crate::vfs::{Backend, BackendHandle, LocalBackend, Scheme, VfsMeta, VfsResult};

#[test]
fn force_copy_and_move_reject_the_same_file_without_modifying_it() {
    let temp = TempRoot::new("same-target");
    let path = temp.file("same.txt", b"keep me");
    let alias = format!("{}/./same.txt", temp.path_string());
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));

    let err = copy(
        &target(&backend, &path, "local"),
        &target_with_keys(&backend, &alias, "local", "other-handle"),
        false,
        true,
    )
    .unwrap_err();
    assert!(err.contains("same path"));
    assert_eq!(fs::read(vfs_to_os(&path)).unwrap(), b"keep me");

    let err = move_path(
        &target(&backend, &path, "local"),
        &target_with_keys(&backend, &path, "local", "other-handle"),
        false,
        true,
    )
    .unwrap_err();
    assert!(err.contains("same path"));
    assert_eq!(fs::read(vfs_to_os(&path)).unwrap(), b"keep me");
}

#[test]
fn force_copy_and_move_reject_hard_link_aliases_without_modifying_them() {
    let temp = TempRoot::new("hard-link-target");
    let path = temp.file("source.txt", b"keep me");
    let alias = temp.path("alias.txt");
    fs::hard_link(vfs_to_os(&path), vfs_to_os(&alias)).unwrap();
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));

    let err = copy(
        &target(&backend, &path, "local"),
        &target(&backend, &alias, "local"),
        false,
        true,
    )
    .unwrap_err();
    assert!(err.contains("same path"));
    assert_eq!(fs::read(vfs_to_os(&path)).unwrap(), b"keep me");

    let err = move_path(
        &target(&backend, &path, "local"),
        &target(&backend, &alias, "local"),
        false,
        true,
    )
    .unwrap_err();
    assert!(err.contains("same path"));
    assert_eq!(fs::read(vfs_to_os(&path)).unwrap(), b"keep me");
    assert_eq!(fs::read(vfs_to_os(&alias)).unwrap(), b"keep me");
}

#[test]
fn recursive_copy_and_move_reject_directory_descendants() {
    let temp = TempRoot::new("dir-descendant");
    let src = temp.dir("src");
    fs::write(vfs_to_os(&format!("{src}/child.txt")), b"child").unwrap();
    let nested = format!("{src}/nested");
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));

    let err = copy(
        &target(&backend, &src, "local"),
        &target(&backend, &nested, "local"),
        true,
        false,
    )
    .unwrap_err();
    assert!(err.contains("own descendant"));
    assert!(!vfs_to_os(&nested).exists());

    let err = move_path(
        &target(&backend, &src, "local"),
        &target(&backend, &nested, "local"),
        true,
        false,
    )
    .unwrap_err();
    assert!(err.contains("own descendant"));
    assert!(vfs_to_os(&src).exists());
    assert!(!vfs_to_os(&nested).exists());
}

#[cfg(unix)]
#[test]
fn recursive_copy_rejects_a_descendant_reached_through_a_symlink() {
    let temp = TempRoot::new("symlink-descendant");
    let src = temp.dir("src");
    let alias = temp.path("src-alias");
    std::os::unix::fs::symlink(vfs_to_os(&src), vfs_to_os(&alias)).unwrap();
    let nested_alias = format!("{alias}/nested");
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));

    let err = copy(
        &target(&backend, &src, "local"),
        &target(&backend, &nested_alias, "local"),
        true,
        false,
    )
    .unwrap_err();

    assert!(err.contains("own descendant"));
    assert!(!vfs_to_os(&format!("{src}/nested")).exists());
}

#[cfg(unix)]
#[test]
fn symlink_source_move_fails_closed_without_modifying_either_path() {
    let temp = TempRoot::new("rename-symlink-guard");
    let original = temp.file("original.txt", b"contents");
    let src = temp.path("source-link.txt");
    let dst = temp.path("dst.txt");
    std::os::unix::fs::symlink(vfs_to_os(&original), vfs_to_os(&src)).unwrap();
    let backend = Arc::new(CountingLocal::new());
    let handle: BackendHandle = backend.clone();

    let error = move_path(
        &target(&handle, &src, "local"),
        &target(&handle, &dst, "local"),
        false,
        false,
    )
    .unwrap_err();

    assert!(error.contains("link-like source"));
    assert_eq!(backend.renames.load(Ordering::Relaxed), 0);
    assert!(fs::symlink_metadata(vfs_to_os(&src))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(!vfs_to_os(&dst).exists());
    assert_eq!(fs::read(vfs_to_os(&original)).unwrap(), b"contents");
}

#[test]
fn same_backend_move_uses_rename_for_a_new_destination() {
    let temp = TempRoot::new("rename-fast-path");
    let src = temp.file("src.txt", b"moved");
    let dst = temp.path("dst.txt");
    let backend = Arc::new(CountingLocal::new());
    let handle: BackendHandle = backend.clone();

    move_path(
        &target(&handle, &src, "local"),
        &target(&handle, &dst, "local"),
        false,
        false,
    )
    .unwrap();

    assert_eq!(backend.renames.load(Ordering::Relaxed), 1);
    assert!(!vfs_to_os(&src).exists());
    assert_eq!(fs::read(vfs_to_os(&dst)).unwrap(), b"moved");
}

#[test]
fn existing_file_without_force_never_takes_the_rename_fast_path() {
    let temp = TempRoot::new("rename-force-guard");
    let src = temp.file("src.txt", b"source");
    let dst = temp.file("dst.txt", b"destination");
    let backend = Arc::new(CountingLocal::new());
    let handle: BackendHandle = backend.clone();

    let err = move_path(
        &target(&handle, &src, "local"),
        &target(&handle, &dst, "local"),
        false,
        false,
    )
    .unwrap_err();

    assert!(err.contains("--force"));
    assert_eq!(backend.renames.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(vfs_to_os(&src)).unwrap(), b"source");
    assert_eq!(fs::read(vfs_to_os(&dst)).unwrap(), b"destination");
}

#[test]
fn force_uses_replace_rename_when_the_backend_promises_it() {
    let temp = TempRoot::new("rename-force-replace");
    let src = temp.file("src.txt", b"source");
    let dst = temp.file("dst.txt", b"destination");
    let backend = Arc::new(CountingLocal::new());
    let handle: BackendHandle = backend.clone();

    move_path(
        &target(&handle, &src, "local"),
        &target(&handle, &dst, "local"),
        false,
        true,
    )
    .unwrap();

    assert_eq!(backend.renames.load(Ordering::Relaxed), 1);
    assert!(!vfs_to_os(&src).exists());
    assert_eq!(fs::read(vfs_to_os(&dst)).unwrap(), b"source");
}

#[cfg(unix)]
#[test]
fn existing_symlink_destination_fails_closed_without_touching_its_target() {
    let temp = TempRoot::new("rename-destination-symlink-guard");
    let src = temp.file("src.txt", b"source");
    let target_path = temp.file("target.txt", b"destination");
    let dst = temp.path("dst-link.txt");
    std::os::unix::fs::symlink(vfs_to_os(&target_path), vfs_to_os(&dst)).unwrap();
    let backend = Arc::new(CountingLocal::new());
    let handle: BackendHandle = backend.clone();

    let error = move_path(
        &target(&handle, &src, "local"),
        &target(&handle, &dst, "local"),
        false,
        true,
    )
    .unwrap_err();

    assert!(error.contains("not a regular file"));
    assert_eq!(backend.renames.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(vfs_to_os(&src)).unwrap(), b"source");
    assert!(fs::symlink_metadata(vfs_to_os(&dst))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(vfs_to_os(&target_path)).unwrap(), b"destination");
}

#[test]
fn rename_fast_path_never_crosses_logical_backends() {
    let temp = TempRoot::new("rename-backend-guard");
    let src = temp.file("src.txt", b"copied");
    let dst = temp.path("dst.txt");
    let backend = Arc::new(CountingLocal::new());
    let handle: BackendHandle = backend.clone();

    move_path(
        &target(&handle, &src, "source-backend"),
        &target(&handle, &dst, "destination-backend"),
        false,
        false,
    )
    .unwrap();

    let calls = backend.rename_calls();
    assert_eq!(
        calls.len(),
        1,
        "copy fallback should promote one staged file"
    );
    assert_ne!(calls[0].0, src, "source fast rename crossed backend keys");
    assert_eq!(calls[0].1, dst);
    assert!(!vfs_to_os(&src).exists());
    assert_eq!(fs::read(vfs_to_os(&dst)).unwrap(), b"copied");
}

#[test]
fn rename_fast_path_requires_compatible_handles_within_one_namespace() {
    let temp = TempRoot::new("rename-handle-guard");
    let src = temp.file("src.txt", b"copied");
    let dst = temp.path("dst.txt");
    let backend = Arc::new(CountingLocal::new());
    let handle: BackendHandle = backend.clone();

    move_path(
        &target_with_keys(&handle, &src, "shared-namespace", "source-handle"),
        &target_with_keys(&handle, &dst, "shared-namespace", "destination-handle"),
        false,
        false,
    )
    .unwrap();

    let calls = backend.rename_calls();
    assert_eq!(
        calls.len(),
        1,
        "copy fallback should promote one staged file"
    );
    assert_ne!(
        calls[0].0, src,
        "source fast rename crossed incompatible handle keys"
    );
    assert_eq!(calls[0].1, dst);
    assert!(!vfs_to_os(&src).exists());
    assert_eq!(fs::read(vfs_to_os(&dst)).unwrap(), b"copied");
}

struct TempRoot {
    path: std::path::PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let unique = format!(
            "smart-explorer-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path_string(&self) -> String {
        self.path.to_string_lossy().replace('\\', "/")
    }

    fn path(&self, rel: &str) -> String {
        self.path.join(rel).to_string_lossy().replace('\\', "/")
    }

    fn file(&self, rel: &str, content: &[u8]) -> String {
        let path = self.path(rel);
        fs::write(vfs_to_os(&path), content).unwrap();
        path
    }

    fn dir(&self, rel: &str) -> String {
        let path = self.path(rel);
        fs::create_dir_all(vfs_to_os(&path)).unwrap();
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn vfs_to_os(path: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

fn target(backend: &BackendHandle, path: &str, key: &str) -> Target {
    Target::with_backend_key(backend.clone(), path.to_string(), key)
}

fn target_with_keys(
    backend: &BackendHandle,
    path: &str,
    namespace_key: &str,
    rename_key: &str,
) -> Target {
    Target::with_backend_keys(backend.clone(), path.to_string(), namespace_key, rename_key)
}

struct CountingLocal {
    inner: LocalBackend,
    renames: AtomicUsize,
    rename_calls: Mutex<Vec<(String, String)>>,
}

impl CountingLocal {
    fn new() -> Self {
        Self {
            inner: LocalBackend::new("/"),
            renames: AtomicUsize::new(0),
            rename_calls: Mutex::new(Vec::new()),
        }
    }

    fn rename_calls(&self) -> Vec<(String, String)> {
        self.rename_calls.lock().unwrap().clone()
    }
}

impl Backend for CountingLocal {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn std::io::Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn std::io::Write + Send>> {
        self.inner.open_write(path)
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.inner.copy_file(src, dst)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.renames.fetch_add(1, Ordering::Relaxed);
        self.rename_calls
            .lock()
            .unwrap()
            .push((src.to_string(), dst.to_string()));
        self.inner.rename(src, dst)
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.renames.fetch_add(1, Ordering::Relaxed);
        self.rename_calls
            .lock()
            .unwrap()
            .push((src.to_string(), dst.to_string()));
        self.inner.rename_no_replace(src, dst)
    }

    fn rename_overwrites(&self) -> bool {
        true
    }

    fn is_local(&self) -> bool {
        true
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}
