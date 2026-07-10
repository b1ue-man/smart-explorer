use super::*;
use crate::app::app_models::TransferMsg;
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn temp_dir(tag: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "se_transfer_cancel_{tag}_{}_{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn terminal(rx: &crossbeam_channel::Receiver<TransferMsg>) -> (u64, bool, Vec<String>) {
    loop {
        match rx.recv().unwrap() {
            TransferMsg::Progress(_) => {}
            TransferMsg::Done {
                progress,
                errors,
                canceled,
            } => return (progress.files_done, canceled, errors),
        }
    }
}

struct CancelAfterRead {
    inner: Box<dyn Read + Send>,
    cancel: Arc<AtomicBool>,
}

impl Read for CancelAfterRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read > 0 {
            self.cancel.store(true, Ordering::Release);
        }
        Ok(read)
    }
}

struct CancelAfterWrite {
    inner: Box<dyn Write + Send>,
    cancel: Arc<AtomicBool>,
}

impl Write for CancelAfterWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        if written > 0 {
            self.cancel.store(true, Ordering::Release);
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct CancelIoBackend {
    inner: crate::vfs::LocalBackend,
    cancel: Arc<AtomicBool>,
    on_read: bool,
    on_write: bool,
}

impl Backend for CancelIoBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
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

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let reader = self.inner.open_read(path)?;
        if self.on_read {
            Ok(Box::new(CancelAfterRead {
                inner: reader,
                cancel: self.cancel.clone(),
            }))
        } else {
            Ok(reader)
        }
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let writer = self.inner.open_write(path)?;
        if self.on_write {
            Ok(Box::new(CancelAfterWrite {
                inner: writer,
                cancel: self.cancel.clone(),
            }))
        } else {
            Ok(writer)
        }
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.inner.copy_file(src, dst)
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

#[test]
fn pre_canceled_transfers_are_terminal_without_mutation() {
    let source = temp_dir("pre_source");
    let target = temp_dir("pre_target");
    let local = temp_dir("pre_local");
    std::fs::write(source.join("item.bin"), b"content").unwrap();
    let src = crate::vfs::LocalBackend::new(&fwd(&source));
    let tgt = crate::vfs::LocalBackend::new(&fwd(&target));
    let cancel = AtomicBool::new(true);

    let (tx, rx) = crossbeam_channel::unbounded();
    upload_paths_progress(
        &tgt,
        &[fwd(&source.join("item.bin"))],
        &fwd(&target),
        &tx,
        &cancel,
    );
    assert_eq!(terminal(&rx), (0, true, Vec::new()));

    let (tx, rx) = crossbeam_channel::unbounded();
    download_paths_progress(
        &src,
        &[fwd(&source.join("item.bin"))],
        &fwd(&local),
        None,
        &tx,
        &cancel,
    );
    assert_eq!(terminal(&rx), (0, true, Vec::new()));

    let (tx, rx) = crossbeam_channel::unbounded();
    copy_remote_paths_progress(
        &src,
        &[fwd(&source.join("item.bin"))],
        &tgt,
        &fwd(&target),
        false,
        None,
        &tx,
        &cancel,
    );
    assert_eq!(terminal(&rx), (0, true, Vec::new()));
    assert!(std::fs::read_dir(&target).unwrap().next().is_none());
    assert!(std::fs::read_dir(&local).unwrap().next().is_none());

    let _ = std::fs::remove_dir_all(source);
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(local);
}

#[test]
fn canceled_download_removes_partial_staging_file() {
    let remote = temp_dir("read_remote");
    let local = temp_dir("read_local");
    std::fs::write(remote.join("large.bin"), vec![7u8; 128 * 1024]).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let backend = CancelIoBackend {
        inner: crate::vfs::LocalBackend::new(&fwd(&remote)),
        cancel: cancel.clone(),
        on_read: true,
        on_write: false,
    };
    let (tx, rx) = crossbeam_channel::unbounded();

    download_paths_progress(
        &backend,
        &[fwd(&remote.join("large.bin"))],
        &fwd(&local),
        None,
        &tx,
        &cancel,
    );

    assert_eq!(terminal(&rx), (0, true, Vec::new()));
    assert!(std::fs::read_dir(&local).unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(remote);
    let _ = std::fs::remove_dir_all(local);
}

#[test]
fn canceled_upload_removes_remote_staging_file() {
    let local = temp_dir("write_local");
    let remote = temp_dir("write_remote");
    std::fs::write(local.join("large.bin"), vec![9u8; 128 * 1024]).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let backend = CancelIoBackend {
        inner: crate::vfs::LocalBackend::new(&fwd(&remote)),
        cancel: cancel.clone(),
        on_read: false,
        on_write: true,
    };
    let (tx, rx) = crossbeam_channel::unbounded();

    upload_paths_progress(
        &backend,
        &[fwd(&local.join("large.bin"))],
        &fwd(&remote),
        &tx,
        &cancel,
    );

    assert_eq!(terminal(&rx), (0, true, Vec::new()));
    assert!(std::fs::read_dir(&remote).unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(local);
    let _ = std::fs::remove_dir_all(remote);
}
