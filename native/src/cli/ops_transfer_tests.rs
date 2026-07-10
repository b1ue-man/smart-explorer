use super::ops::{copy, move_path};
use super::target::Target;
use crate::vfs::{Backend, BackendHandle, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn short_source_read_never_publishes_or_deletes_during_move() {
    let temp = TempRoot::new("short-read-move");
    let destination = temp.file("destination.txt", b"old destination");
    let source_backend = Arc::new(SyntheticSource::new("source.txt", 4, b"abc"));
    let source_handle: BackendHandle = source_backend.clone();
    let destination_handle: BackendHandle = Arc::new(LocalBackend::new("/"));

    let error = move_path(
        &target(&source_handle, "/source", "source"),
        &target(&destination_handle, &destination, "destination"),
        false,
        true,
    )
    .unwrap_err();

    assert!(error.contains("ended early"));
    assert_eq!(source_backend.removes.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read(&destination).unwrap(), b"old destination");
}

#[test]
fn unsafe_source_metadata_name_is_rejected_before_destination_mutation() {
    let temp = TempRoot::new("unsafe-source-name");
    let destination_dir = temp.dir("destination");
    let source_backend = Arc::new(SyntheticSource::new("../escape", 3, b"abc"));
    let source_handle: BackendHandle = source_backend.clone();
    let destination_handle: BackendHandle = Arc::new(LocalBackend::new("/"));

    let error = copy(
        &target(&source_handle, "/source", "source"),
        &target(&destination_handle, &destination_dir, "destination"),
        false,
        false,
    )
    .unwrap_err();

    assert!(error.contains("child name"));
    assert_eq!(source_backend.reads.load(Ordering::Relaxed), 0);
    assert!(!temp.path.join("escape").exists());
}

fn target(backend: &BackendHandle, path: &str, key: &str) -> Target {
    Target::with_backend_key(backend.clone(), path.to_string(), key)
}

struct TempRoot {
    path: std::path::PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "smart-explorer-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.path.join(name);
        fs::write(&path, bytes).unwrap();
        path.to_string_lossy().replace('\\', "/")
    }

    fn dir(&self, name: &str) -> String {
        let path = self.path.join(name);
        fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().replace('\\', "/")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct SyntheticSource {
    name: String,
    advertised_size: u64,
    bytes: Vec<u8>,
    reads: AtomicUsize,
    removes: AtomicUsize,
}

impl SyntheticSource {
    fn new(name: &str, advertised_size: u64, bytes: &[u8]) -> Self {
        Self {
            name: name.to_string(),
            advertised_size,
            bytes: bytes.to_vec(),
            reads: AtomicUsize::new(0),
            removes: AtomicUsize::new(0),
        }
    }
}

impl Backend for SyntheticSource {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".to_string()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path != "/source" {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        Ok(VfsMeta {
            name: self.name.clone(),
            size: self.advertised_size,
            mtime_ms: 7,
            id: Some("stable-source".to_string()),
            ..VfsMeta::default()
        })
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn std::io::Read + Send>> {
        if path != "/source" {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(std::io::Cursor::new(self.bytes.clone())))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn std::io::Write + Send>> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(std::io::ErrorKind::Unsupported.into())
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
        Err(std::io::ErrorKind::Unsupported.into())
    }
}
