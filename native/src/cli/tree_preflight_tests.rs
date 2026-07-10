use super::ops::{copy, move_path};
use super::target::Target;
use crate::vfs::{Backend, BackendHandle, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::fs;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn late_unsafe_child_leaves_destination_untouched() {
    let temp = TempRoot::new("late-unsafe-child");
    let destination = temp.path("destination");
    let source = Arc::new(TreeSource::new(Fault::UnsafeName));

    let error = copy_between(&source, &destination, false).unwrap_err();

    assert!(error.contains("unsafe child name"), "{error}");
    assert!(!fs::exists(&destination).unwrap());
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn late_source_stat_failure_leaves_destination_untouched() {
    let temp = TempRoot::new("late-stat-failure");
    let destination = temp.path("destination");
    let source = Arc::new(TreeSource::new(Fault::LateStat));

    let error = copy_between(&source, &destination, false).unwrap_err();

    assert!(error.contains("synthetic late stat failure"), "{error}");
    assert!(!fs::exists(&destination).unwrap());
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn duplicate_child_names_fail_before_destination_mutation() {
    let temp = TempRoot::new("duplicate-source-child");
    let destination = temp.path("destination");
    let source = Arc::new(TreeSource::new(Fault::DuplicateName));

    let error = copy_between(&source, &destination, false).unwrap_err();

    assert!(error.contains("duplicate child name"), "{error}");
    assert!(!fs::exists(&destination).unwrap());
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn over_depth_tree_fails_before_destination_mutation() {
    let temp = TempRoot::new("over-depth-source-tree");
    let destination = temp.path("destination");
    let source = Arc::new(TreeSource::new(Fault::TooDeep));

    let error = copy_between(&source, &destination, false).unwrap_err();

    assert!(error.contains("exceeds 512 levels"), "{error}");
    assert!(!fs::exists(&destination).unwrap());
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn late_source_read_failure_is_spooled_before_destination_mutation() {
    let temp = TempRoot::new("late-read-failure");
    let destination = temp.path("destination");
    let source = Arc::new(TreeSource::new(Fault::LateRead));
    let source_handle: BackendHandle = source.clone();
    let destination_handle: BackendHandle = Arc::new(LocalBackend::new("/"));

    let error = move_path(
        &target(&source_handle, "/source", "synthetic-source"),
        &target(&destination_handle, &destination, "local-destination"),
        true,
        false,
    )
    .unwrap_err();

    assert!(error.contains("synthetic late read failure"), "{error}");
    assert!(source.reads.load(Ordering::Relaxed) >= 2);
    assert_eq!(source.removes.load(Ordering::Relaxed), 0);
    assert!(!fs::exists(&destination).unwrap());
}

#[test]
fn source_drift_during_spooling_leaves_destination_untouched() {
    let temp = TempRoot::new("source-drift");
    let destination = temp.path("destination");
    let source = Arc::new(TreeSource::new(Fault::DriftDuringRead));

    let error = copy_between(&source, &destination, false).unwrap_err();

    assert!(error.contains("type, identity, or size changed"), "{error}");
    assert!(!fs::exists(&destination).unwrap());
}

#[test]
fn recursive_local_move_applies_then_removes_the_exact_source_tree() {
    let temp = TempRoot::new("recursive-local-move");
    let source = temp.dir("source");
    temp.file("source/child.txt", b"contents");
    let destination = temp.path("destination");
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));

    move_path(
        &target(&backend, &source, "local"),
        &target(&backend, &destination, "local"),
        true,
        false,
    )
    .unwrap();

    assert!(!fs::exists(&source).unwrap());
    assert_eq!(
        fs::read(std::path::Path::new(&destination).join("child.txt")).unwrap(),
        b"contents"
    );
}

#[test]
fn late_destination_collision_is_found_before_any_source_read() {
    let temp = TempRoot::new("late-destination-collision");
    let container = temp.dir("container");
    let merge_root = temp.dir("container/source");
    let late = temp.file("container/source/late.txt", b"existing");
    let source = Arc::new(TreeSource::new(Fault::None));
    let source_handle: BackendHandle = source.clone();
    let destination_handle: BackendHandle = Arc::new(LocalBackend::new("/"));

    let error = copy(
        &target(&source_handle, "/source", "synthetic-source"),
        &target(&destination_handle, &container, "local-destination"),
        true,
        false,
    )
    .unwrap_err();

    assert!(error.contains("pass --force"), "{error}");
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(&late).unwrap(), b"existing");
    assert!(!std::path::Path::new(&merge_root).join("good.txt").exists());
}

#[cfg(unix)]
#[test]
fn link_like_destination_ancestor_is_rejected_before_apply() {
    use std::os::unix::fs::symlink;

    let temp = TempRoot::new("unsafe-destination-ancestor");
    let outside = temp.dir("outside");
    let link = temp.path("link");
    symlink(&outside, &link).unwrap();
    let destination = format!("{link}/new-tree");
    let source = Arc::new(TreeSource::new(Fault::None));

    let error = copy_between(&source, &destination, false).unwrap_err();

    assert!(error.contains("link-like and unsafe"), "{error}");
    assert!(!std::path::Path::new(&outside).join("new-tree").exists());
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
}

fn copy_between(source: &Arc<TreeSource>, destination: &str, force: bool) -> Result<(), String> {
    let source_handle: BackendHandle = source.clone();
    let destination_handle: BackendHandle = Arc::new(LocalBackend::new("/"));
    copy(
        &target(&source_handle, "/source", "synthetic-source"),
        &target(&destination_handle, destination, "local-destination"),
        true,
        force,
    )
}

fn target(backend: &BackendHandle, path: &str, key: &str) -> Target {
    Target::with_backend_key(backend.clone(), path.to_string(), key)
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    UnsafeName,
    DuplicateName,
    TooDeep,
    LateStat,
    LateRead,
    DriftDuringRead,
}

struct TreeSource {
    fault: Fault,
    drifted: Arc<AtomicBool>,
    reads: AtomicUsize,
    removes: AtomicUsize,
}

impl TreeSource {
    fn new(fault: Fault) -> Self {
        Self {
            fault,
            drifted: Arc::new(AtomicBool::new(false)),
            reads: AtomicUsize::new(0),
            removes: AtomicUsize::new(0),
        }
    }
}

impl Backend for TreeSource {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".to_string()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if matches!(self.fault, Fault::TooDeep) {
            let depth = deep_path_depth(path).ok_or(io::ErrorKind::NotFound)?;
            return Ok((depth <= 512)
                .then(|| directory_meta("d", depth + 1))
                .into_iter()
                .collect());
        }
        if path != "/source" {
            return Err(io::ErrorKind::NotFound.into());
        }
        let mut entries = vec![file_meta("good.txt")];
        entries.push(match self.fault {
            Fault::UnsafeName => file_meta("../escape"),
            Fault::DuplicateName => file_meta("good.txt"),
            _ => file_meta("late.txt"),
        });
        Ok(entries)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if matches!(self.fault, Fault::TooDeep) {
            let depth = deep_path_depth(path).ok_or(io::ErrorKind::NotFound)?;
            return Ok(directory_meta(
                if depth == 0 { "source" } else { "d" },
                depth,
            ));
        }
        match path {
            "/source" => Ok(VfsMeta {
                name: "source".to_string(),
                is_dir: true,
                id: Some("source-root".to_string()),
                ..VfsMeta::default()
            }),
            "/source/good.txt" => Ok(file_meta("good.txt")),
            "/source/late.txt" if matches!(self.fault, Fault::LateStat) => {
                Err(io::Error::other("synthetic late stat failure"))
            }
            "/source/late.txt" => {
                let mut metadata = file_meta("late.txt");
                if self.drifted.load(Ordering::Relaxed) {
                    metadata.size = 5;
                    metadata.mtime_ms = 8;
                }
                Ok(metadata)
            }
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        match path {
            "/source/good.txt" => Ok(Box::new(io::Cursor::new(b"good".to_vec()))),
            "/source/late.txt" if matches!(self.fault, Fault::LateRead) => {
                Err(io::Error::other("synthetic late read failure"))
            }
            "/source/late.txt" if matches!(self.fault, Fault::DriftDuringRead) => {
                Ok(Box::new(DriftReader {
                    inner: io::Cursor::new(b"late".to_vec()),
                    drifted: self.drifted.clone(),
                }))
            }
            "/source/late.txt" => Ok(Box::new(io::Cursor::new(b"late".to_vec()))),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn std::io::Write + Send>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        self.removes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        self.removes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn file_meta(name: &str) -> VfsMeta {
    VfsMeta {
        name: name.to_string(),
        size: 4,
        mtime_ms: 7,
        id: Some(format!("id-{name}")),
        ..VfsMeta::default()
    }
}

fn directory_meta(name: &str, depth: usize) -> VfsMeta {
    VfsMeta {
        name: name.to_string(),
        is_dir: true,
        id: Some(format!("directory-{depth}")),
        ..VfsMeta::default()
    }
}

fn deep_path_depth(path: &str) -> Option<usize> {
    let suffix = path.strip_prefix("/source")?;
    if suffix.is_empty() {
        return Some(0);
    }
    let components: Vec<&str> = suffix.strip_prefix('/')?.split('/').collect();
    components
        .iter()
        .all(|component| *component == "d")
        .then_some(components.len())
}

struct DriftReader {
    inner: io::Cursor<Vec<u8>>,
    drifted: Arc<AtomicBool>,
}

impl Read for DriftReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        if read == 0 {
            self.drifted.store(true, Ordering::Relaxed);
        }
        Ok(read)
    }
}

struct TempRoot {
    directory: tempfile::TempDir,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        Self {
            directory: tempfile::Builder::new().prefix(name).tempdir().unwrap(),
        }
    }

    fn path(&self, relative: &str) -> String {
        self.directory
            .path()
            .join(relative)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn dir(&self, relative: &str) -> String {
        let path = self.path(relative);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn file(&self, relative: &str, contents: &[u8]) -> String {
        let path = self.path(relative);
        if let Some(parent) = std::path::Path::new(&path).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
        path
    }
}
