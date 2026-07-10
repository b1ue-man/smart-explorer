use super::*;
use crate::app::app_models::TransferMsg;
use crate::types::FilterDef;
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn temp_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!(
        "se_clip_test_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn txt_filter() -> FilterDef {
    let mut filter = FilterDef::new();
    filter.extensions = vec!["txt".to_string()];
    filter
}

fn done_from(
    rx: &crossbeam_channel::Receiver<TransferMsg>,
) -> (crate::app::app_models::TransferProgress, Vec<String>, bool) {
    let mut done = None;
    while let Ok(msg) = rx.try_recv() {
        if let TransferMsg::Done {
            progress,
            errors,
            canceled,
        } = msg
        {
            done = Some((progress, errors, canceled));
        }
    }
    done.expect("transfer should send Done")
}

fn numbered_name(index: usize) -> String {
    if index == 1 {
        "entry.txt".to_string()
    } else {
        format!("entry ({index}).txt")
    }
}

#[test]
fn remote_unique_name_checks_the_bound_and_never_reuses_it() {
    let root = temp_dir("remote_unique_bound");
    for index in 1..1000 {
        std::fs::write(root.join(numbered_name(index)), b"occupied").unwrap();
    }
    let backend = crate::vfs::LocalBackend::new(&fwd(&root));

    let last = find_remote_unique_name(&backend, &fwd(&root), numbered_name).unwrap();
    assert_eq!(last, numbered_name(1000));
    std::fs::write(root.join(&last), b"occupied").unwrap();
    assert!(find_remote_unique_name(&backend, &fwd(&root), numbered_name).is_err());

    assert!(ensure_remote_destination_free(&backend, &fwd(&root.join(&last))).is_err());
    assert!(ensure_remote_destination_free(&backend, &fwd(&root.join("free.txt"))).is_ok());

    let probe_error = find_remote_unique_name_with(
        |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked")),
        &fwd(&root),
        numbered_name,
    )
    .expect_err("a failed existence probe must not look like a free name");
    assert!(probe_error.contains("Ziel prüfen"));
    let _ = std::fs::remove_dir_all(root);
}

fn copy_tree_contents(src: &Path, dst: &Path) -> io::Result<u64> {
    std::fs::create_dir_all(dst)?;
    let mut files = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());
        if ft.is_dir() {
            files += copy_tree_contents(&child_src, &child_dst)?;
        } else if ft.is_file() {
            if let Some(parent) = child_dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&child_src, &child_dst)?;
            files += 1;
        }
    }
    Ok(files)
}

struct BulkLocalBackend {
    inner: crate::vfs::LocalBackend,
    get_calls: AtomicUsize,
    put_calls: AtomicUsize,
}

impl BulkLocalBackend {
    fn new(root: &str) -> Self {
        Self {
            inner: crate::vfs::LocalBackend::new(root),
            get_calls: AtomicUsize::new(0),
            put_calls: AtomicUsize::new(0),
        }
    }
}

impl Backend for BulkLocalBackend {
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
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
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

    fn supports_bulk_tree(&self) -> bool {
        true
    }

    fn get_tree(&self, root: &str, dst: &Path) -> VfsResult<u64> {
        self.get_calls.fetch_add(1, Ordering::Relaxed);
        copy_tree_contents(Path::new(root), dst)
    }

    fn put_tree(&self, src: &Path, root: &str) -> VfsResult<u64> {
        self.put_calls.fetch_add(1, Ordering::Relaxed);
        copy_tree_contents(src, Path::new(root))
    }
}

#[test]
fn remote_clipboard_downloads_folder_tree() {
    let remote = temp_dir("remote");
    std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
    std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
    let be = crate::vfs::LocalBackend::new(&fwd(&remote));
    let item = (format!("{}/Gate", fwd(&remote)), "Gate".to_string(), true);

    let local = download_remote_clipboard_items(&be, &[item], None).unwrap();

    assert_eq!(local.len(), 1);
    let local_dir = PathBuf::from(&local[0]);
    assert!(local_dir.is_dir());
    assert_eq!(std::fs::read(local_dir.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(local_dir.join("sub/b.txt")).unwrap(), b"beta");

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(local_dir);
}

#[test]
fn remote_clipboard_filters_folder_tree() {
    let remote = temp_dir("remote_filter_clip");
    std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
    std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(remote.join("Gate/drop.bin"), b"drop").unwrap();
    std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
    std::fs::write(remote.join("Gate/sub/drop.md"), b"drop").unwrap();
    let root = fwd(&remote);
    let be = crate::vfs::LocalBackend::new(&root);
    let item = (format!("{root}/Gate"), "Gate".to_string(), true);

    let local = download_remote_clipboard_items(&be, &[item], Some((txt_filter(), root))).unwrap();

    assert_eq!(local.len(), 1);
    let local_dir = PathBuf::from(&local[0]);
    assert_eq!(std::fs::read(local_dir.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(local_dir.join("sub/b.txt")).unwrap(), b"beta");
    assert!(!local_dir.join("drop.bin").exists());
    assert!(!local_dir.join("sub/drop.md").exists());

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(local_dir);
}

#[test]
fn remote_upload_copies_folder_tree_without_bulk() {
    let local = temp_dir("upload_plain_local");
    let remote = temp_dir("upload_plain_remote");
    std::fs::create_dir_all(local.join("Gate/sub")).unwrap();
    std::fs::write(local.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(local.join("Gate/sub/b.txt"), b"beta").unwrap();
    let be = crate::vfs::LocalBackend::new(&fwd(&remote));
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);

    upload_paths_progress(
        &be,
        &[fwd(&local.join("Gate"))],
        &fwd(&remote),
        &tx,
        &cancel,
    );

    let (progress, errors, canceled) = done_from(&rx);
    assert!(!canceled);
    assert_eq!(progress.files_total, 2);
    assert_eq!(progress.files_done, 2);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(std::fs::read(remote.join("Gate/a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(remote.join("Gate/sub/b.txt")).unwrap(),
        b"beta"
    );

    let _ = std::fs::remove_dir_all(&local);
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn remote_upload_uses_bulk_tree_for_folder_backend() {
    let local = temp_dir("upload_bulk_local");
    let remote = temp_dir("upload_bulk_remote");
    std::fs::create_dir_all(local.join("Gate/sub")).unwrap();
    std::fs::write(local.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(local.join("Gate/sub/b.txt"), b"beta").unwrap();
    let be = BulkLocalBackend::new(&fwd(&remote));
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);

    upload_paths_progress(
        &be,
        &[fwd(&local.join("Gate"))],
        &fwd(&remote),
        &tx,
        &cancel,
    );

    let (progress, errors, canceled) = done_from(&rx);
    assert!(!canceled);
    assert_eq!(be.put_calls.load(Ordering::Relaxed), 1);
    assert_eq!(progress.files_total, 2);
    assert_eq!(progress.files_done, 2);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(std::fs::read(remote.join("Gate/a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(remote.join("Gate/sub/b.txt")).unwrap(),
        b"beta"
    );

    let _ = std::fs::remove_dir_all(&local);
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn remote_download_uses_bulk_tree_for_folder_backend() {
    let remote = temp_dir("download_bulk_remote");
    let dest = temp_dir("download_bulk_dest");
    std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
    std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
    let be = BulkLocalBackend::new(&fwd(&remote));
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);

    download_paths_progress(
        &be,
        &[fwd(&remote.join("Gate"))],
        &fwd(&dest),
        None,
        &tx,
        &cancel,
    );

    let (progress, errors, canceled) = done_from(&rx);
    assert!(!canceled);
    assert_eq!(be.get_calls.load(Ordering::Relaxed), 1);
    assert_eq!(progress.files_total, 2);
    assert_eq!(progress.files_done, 2);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(std::fs::read(dest.join("Gate/a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(dest.join("Gate/sub/b.txt")).unwrap(), b"beta");

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn remote_download_filters_selected_folder() {
    let remote = temp_dir("remote_filter_download");
    let dest = temp_dir("remote_filter_dest");
    std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
    std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(remote.join("Gate/drop.bin"), b"drop").unwrap();
    std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
    std::fs::write(remote.join("Gate/sub/drop.md"), b"drop").unwrap();
    let root = fwd(&remote);
    let be = crate::vfs::LocalBackend::new(&root);
    let src = format!("{root}/Gate");
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);

    download_paths_progress(
        &be,
        &[src],
        &fwd(&dest),
        Some((txt_filter(), root)),
        &tx,
        &cancel,
    );

    let (progress, errors, canceled) = done_from(&rx);
    assert!(!canceled);
    assert_eq!(progress.files_total, 2);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(std::fs::read(dest.join("Gate/a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(dest.join("Gate/sub/b.txt")).unwrap(), b"beta");
    assert!(!dest.join("Gate/drop.bin").exists());
    assert!(!dest.join("Gate/sub/drop.md").exists());

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(&dest);
}
