//! The proof of the daylight-saving rule: digests formed where each file is
//! stored decide; a side that cannot form one (or only reports an uploader's
//! claim, WebDAV) gives no proof and moves no content.
use super::*;
use crate::vfs::{BackendExtensions, LocalBackend, VfsMeta, VfsResult};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A remote side over this machine's disk: `scheme` as given, digests formed
/// by "the device" only when `hashes` is set.
struct Device {
    inner: LocalBackend,
    scheme: Scheme,
    hashes: bool,
    digests: AtomicUsize,
    reads: AtomicUsize,
}

impl Device {
    fn new(scheme: Scheme, hashes: bool) -> Self {
        Self {
            inner: LocalBackend::new("/"),
            scheme,
            hashes,
            digests: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
        }
    }
}

impl Backend for Device {
    fn scheme(&self) -> Scheme {
        self.scheme
    }
    fn root_display(&self) -> String {
        "device".into()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
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

impl BackendExtensions for Device {
    fn content_digest(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<crate::vfs::ContentDigest>> {
        if !self.hashes {
            return Ok(None);
        }
        self.digests.fetch_add(1, Ordering::SeqCst);
        crate::vfs::digest_by_local_read(&self.inner, path, cancel)
    }
}

fn written(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write");
    path.to_str().expect("UTF-8 path").to_string()
}

#[test]
fn abgleich_task_hour_proof_compares_digests_formed_where_the_files_are() {
    let dir = tempfile::tempdir().expect("temp dir");
    let same_a = written(dir.path(), "a", b"same bytes");
    let same_b = written(dir.path(), "b", b"same bytes");
    let other = written(dir.path(), "c", b"other byte");
    let local = LocalBackend::new("/");
    let device = Device::new(Scheme::Sftp, true);
    let cancel = AtomicBool::new(false);
    for (remote_path, expected) in [(&same_b, Some(true)), (&other, Some(false))] {
        let found = same_content_where_stored(
            Stored {
                backend: &local,
                path: &same_a,
            },
            Stored {
                backend: &device,
                path: remote_path,
            },
            &cancel,
        );
        assert_eq!(found, expected, "{remote_path}");
    }
    assert_eq!(device.reads.load(Ordering::SeqCst), 0, "no content moved");
    assert_eq!(device.digests.load(Ordering::SeqCst), 2);
}

#[test]
fn abgleich_task_hour_proof_without_a_device_digest_gives_none_and_reads_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let a = written(dir.path(), "a", b"same bytes");
    let b = written(dir.path(), "b", b"same bytes");
    let cancel = AtomicBool::new(false);
    // No hashing on the device, and a WebDAV checksum claim: no proof.
    for device in [
        Device::new(Scheme::Sftp, false),
        Device::new(Scheme::Webdav, true),
    ] {
        let local = Device::new(Scheme::Local, true);
        let found = same_content_where_stored(
            Stored {
                backend: &local,
                path: &a,
            },
            Stored {
                backend: &device,
                path: &b,
            },
            &cancel,
        );
        assert_eq!(found, None);
        assert_eq!(device.reads.load(Ordering::SeqCst), 0, "nothing downloaded");
        assert_eq!(device.digests.load(Ordering::SeqCst), 0);
    }
    // A file that vanished is no proof either.
    let gone = dir.path().join("gone");
    let gone = gone.to_str().unwrap();
    let device = Device::new(Scheme::Sftp, true);
    assert_eq!(
        same_content_where_stored(
            Stored {
                backend: &LocalBackend::new("/"),
                path: &a,
            },
            Stored {
                backend: &device,
                path: gone,
            },
            &cancel,
        ),
        None
    );
}
