//! `vfs::content_digest`: local backends hash on this device and match
//! known SHA-256 values, the browsing cache forwards, and a backend without
//! a runtime next to its data answers `Ok(None)` instead of reading the file.
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use sha2::Digest;

use super::{
    content_digest, Backend, BackendExtensions, CachingBackend, ConfinedLocalBackend,
    ContentDigest, LocalBackend, Scheme, VfsMeta, VfsResult,
};

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn fixture() -> (tempfile::TempDir, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_string_lossy().replace('\\', "/");
    (directory, root)
}

fn idle() -> AtomicBool {
    AtomicBool::new(false)
}

/// A remote-like backend with only the core interface; every read counts.
#[derive(Default)]
struct Plain {
    reads: AtomicUsize,
    extensions: bool,
}

impl Backend for Plain {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        self.extensions.then_some(self as &dyn BackendExtensions)
    }
    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        Ok(Vec::new())
    }
    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Ok(VfsMeta {
            name: "file.bin".into(),
            size: 3,
            ..VfsMeta::default()
        })
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(io::Cursor::new(b"abc".to_vec())))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(io::sink()))
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Ok(())
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
}

/// Extensions with every default: no host-side digest.
impl BackendExtensions for Plain {}

#[test]
fn screen_metadata_runtime_task_local_file_digest_equals_known_sha256() {
    let (_directory, root) = fixture();
    std::fs::write(format!("{root}/abc.txt"), b"abc").unwrap();
    std::fs::write(format!("{root}/empty.txt"), b"").unwrap();
    let backend = LocalBackend::new(&root);
    assert_eq!(
        content_digest(&backend, &format!("{root}/abc.txt"), &idle()).unwrap(),
        Some(ContentDigest {
            sha256: ABC_SHA256.into(),
            size: 3,
        })
    );
    assert_eq!(
        content_digest(&backend, &format!("{root}/empty.txt"), &idle()).unwrap(),
        Some(ContentDigest {
            sha256: EMPTY_SHA256.into(),
            size: 0,
        })
    );
}

#[test]
fn screen_metadata_runtime_task_local_digest_spans_many_reads() {
    // Larger than three hashing portions and not a multiple of one; checked
    // against the independent `sha2` implementation.
    let (_directory, root) = fixture();
    let payload = (0..3 * 1024 * 1024 + 7)
        .map(|index: usize| (index % 251) as u8)
        .collect::<Vec<_>>();
    std::fs::write(format!("{root}/big.bin"), &payload).unwrap();
    let expected = format!("{:x}", sha2::Sha256::digest(&payload));
    let path = format!("{root}/big.bin");
    let local = content_digest(&LocalBackend::new(&root), &path, &idle())
        .unwrap()
        .unwrap();
    assert_eq!(local.sha256, expected);
    assert_eq!(local.size, payload.len() as u64);
    // Confinement needs a qualified filesystem; elsewhere it is not offered.
    match ConfinedLocalBackend::open(&root) {
        Ok(confined) => {
            let held = content_digest(&confined, &path, &idle()).unwrap().unwrap();
            assert_eq!(held, local, "the confined backend hashes the same file");
        }
        Err(error) => assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}"),
    }
    let cached = CachingBackend::new(Arc::new(LocalBackend::new(&root)));
    let forwarded = content_digest(&cached, &path, &idle()).unwrap().unwrap();
    assert_eq!(forwarded, local, "the browsing cache forwards the digest");
}

#[test]
fn screen_metadata_runtime_task_local_digest_refuses_folders_missing_and_cancel() {
    let (_directory, root) = fixture();
    std::fs::create_dir(format!("{root}/folder")).unwrap();
    std::fs::write(format!("{root}/abc.txt"), b"abc").unwrap();
    let backend = LocalBackend::new(&root);
    let folder = content_digest(&backend, &format!("{root}/folder"), &idle()).unwrap_err();
    assert_eq!(folder.kind(), io::ErrorKind::InvalidInput, "{folder}");
    let missing = content_digest(&backend, &format!("{root}/gone.txt"), &idle()).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound, "{missing}");
    let abc = format!("{root}/abc.txt");
    let canceled = content_digest(&backend, &abc, &AtomicBool::new(true)).unwrap_err();
    assert_eq!(canceled.kind(), io::ErrorKind::Interrupted, "{canceled}");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&abc, format!("{root}/link.txt")).unwrap();
        assert!(content_digest(&backend, &format!("{root}/link.txt"), &idle()).is_err());
    }
}

#[test]
fn screen_metadata_runtime_task_unsupported_backend_returns_none_without_reading() {
    let bare = Plain::default();
    assert_eq!(content_digest(&bare, "/file.bin", &idle()).unwrap(), None);
    let extended = Plain {
        extensions: true,
        ..Plain::default()
    };
    assert_eq!(
        content_digest(&extended, "/file.bin", &idle()).unwrap(),
        None
    );
    let cached = CachingBackend::new(Arc::new(Plain {
        extensions: true,
        ..Plain::default()
    }));
    assert_eq!(content_digest(&cached, "/file.bin", &idle()).unwrap(), None);
    let reads = bare.reads.load(Ordering::Relaxed) + extended.reads.load(Ordering::Relaxed);
    assert_eq!(reads, 0, "no content was read to obtain a digest");
}
