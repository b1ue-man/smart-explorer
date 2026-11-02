#![cfg(unix)]

use super::*;
use crate::vfs::{Backend, BackendHandle, LocalBackend, Scheme, VfsMeta, VfsResult};
use crossbeam_channel::unbounded;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn queued_source_directory_link_swap_is_rejected_before_listing() {
    let source = temp("queued_source");
    let destination = temp("queued_destination");
    let victim = temp("queued_victim");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    std::fs::write(source.join("sub/planned.txt"), b"planned").unwrap();
    std::fs::write(victim.join("outside.txt"), b"outside").unwrap();

    let source_root = forward(&source);
    let target = forward(&source.join("sub"));
    let source_backend = Arc::new(SwapQueuedDirectory {
        inner: LocalBackend::new(&source_root),
        target,
        victim: victim.clone(),
        swapped: AtomicBool::new(false),
        target_listings: AtomicUsize::new(0),
    });
    let source_handle: BackendHandle = source_backend.clone();
    let destination_handle: BackendHandle = Arc::new(LocalBackend::new(&forward(&destination)));
    let (sender, receiver) = unbounded();
    start_sync(
        source_handle,
        source_root,
        destination_handle,
        forward(&destination),
        SyncOptions {
            delete_extra: false,
            dry_run: false,
        },
        sender,
    );

    let result = wait(&receiver);
    assert!(!result.errors.is_empty());
    assert_eq!(source_backend.target_listings.load(Ordering::Relaxed), 0);
    assert!(!destination.join("sub/outside.txt").exists());
    assert_eq!(
        std::fs::read(victim.join("outside.txt")).unwrap(),
        b"outside"
    );

    std::fs::remove_file(source.join("sub")).ok();
    for path in [source, destination, victim] {
        std::fs::remove_dir_all(path).ok();
    }
}

struct SwapQueuedDirectory {
    inner: LocalBackend,
    target: String,
    victim: std::path::PathBuf,
    swapped: AtomicBool,
    target_listings: AtomicUsize,
}

impl Backend for SwapQueuedDirectory {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path == self.target {
            self.target_listings.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path == self.target && !self.swapped.swap(true, Ordering::Relaxed) {
            std::fs::remove_dir_all(path)?;
            std::os::unix::fs::symlink(&self.victim, path)?;
        }
        self.inner.stat(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
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

fn temp(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "sync-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn forward(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn wait(receiver: &crossbeam_channel::Receiver<SyncMsg>) -> SyncResult {
    loop {
        match receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("sync result")
        {
            SyncMsg::Done(result) => return result,
            SyncMsg::Progress(_) => {}
        }
    }
}
