use super::{
    remove_entry, remove_entry_controlled, Backend, DeleteTarget, RecursiveDeletePhase,
    RecursiveDeleteStatus, Scheme, VfsMeta, VfsResult,
};
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct LinkBoundaryBackend {
    list_calls: AtomicUsize,
    removed: std::sync::Mutex<Vec<String>>,
}

impl Backend for LinkBoundaryBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.list_calls.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        Ok(VfsMeta {
            name: path.to_string(),
            is_symlink: true,
            ..VfsMeta::default()
        })
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Ok(())
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.removed.lock().unwrap().push(path.to_string());
        Ok(())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("must not remove a link as a directory"))
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
}

#[test]
fn recursive_delete_stops_at_link_like_root() {
    let backend = LinkBoundaryBackend {
        list_calls: AtomicUsize::new(0),
        removed: std::sync::Mutex::new(Vec::new()),
    };
    remove_entry(
        &backend,
        &DeleteTarget {
            path: "/link".into(),
            id: None,
            is_dir: true,
            is_symlink: true,
        },
    )
    .unwrap();
    assert_eq!(backend.list_calls.load(Ordering::Relaxed), 0);
    assert_eq!(&*backend.removed.lock().unwrap(), &["/link"]);
}

struct TreeBackend {
    removed: std::sync::Mutex<Vec<String>>,
    fail_at: Option<String>,
}

impl TreeBackend {
    fn new(fail_at: Option<&str>) -> Self {
        Self {
            removed: std::sync::Mutex::new(Vec::new()),
            fail_at: fail_at.map(str::to_string),
        }
    }

    fn metadata(path: &str, is_dir: bool) -> VfsMeta {
        VfsMeta {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            is_dir,
            id: Some(format!("id:{path}")),
            ..VfsMeta::default()
        }
    }

    fn record_remove(&self, path: &str) -> VfsResult<()> {
        if self.fail_at.as_deref() == Some(path) {
            return Err(io::Error::other("injected delete failure"));
        }
        self.removed.lock().unwrap().push(path.to_string());
        Ok(())
    }
}

impl Backend for TreeBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path == "/root" {
            Ok(vec![Self::metadata("a", false), Self::metadata("b", false)])
        } else {
            Ok(Vec::new())
        }
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match path {
            "/root" => Ok(Self::metadata(path, true)),
            "/root/a" | "/root/b" => Ok(Self::metadata(path, false)),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "missing")),
        }
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Ok(())
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.record_remove(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.record_remove(path)
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
}

fn root_target() -> DeleteTarget {
    DeleteTarget {
        path: "/root".into(),
        id: Some("id:/root".into()),
        is_dir: true,
        is_symlink: false,
    }
}

#[test]
fn cancellation_during_planning_does_not_mutate() {
    let backend = TreeBackend::new(None);
    let cancel = AtomicBool::new(false);
    let report = remove_entry_controlled(&backend, &root_target(), &cancel, |progress| {
        if progress.phase == RecursiveDeletePhase::Planning && progress.planned >= 2 {
            cancel.store(true, Ordering::Release);
        }
    })
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Canceled);
    assert_eq!(report.removed, 0);
    assert!(backend.removed.lock().unwrap().is_empty());
}

#[test]
fn cancellation_during_apply_reports_confirmed_mutations() {
    let backend = TreeBackend::new(None);
    let cancel = AtomicBool::new(false);
    let report = remove_entry_controlled(&backend, &root_target(), &cancel, |progress| {
        if progress.phase == RecursiveDeletePhase::Applying && progress.removed == 1 {
            cancel.store(true, Ordering::Release);
        }
    })
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Canceled);
    assert_eq!(report.planned, 3);
    assert_eq!(report.removed, 1);
    assert_eq!(&*backend.removed.lock().unwrap(), &["/root/a"]);
}

#[test]
fn apply_failure_reports_already_removed_entries() {
    let backend = TreeBackend::new(Some("/root/b"));
    let cancel = AtomicBool::new(false);
    let failure = remove_entry_controlled(&backend, &root_target(), &cancel, |_| {}).unwrap_err();
    assert_eq!(failure.planned, 3);
    assert_eq!(failure.removed, 1);
    assert_eq!(&*backend.removed.lock().unwrap(), &["/root/a"]);
}

#[cfg(unix)]
#[test]
fn local_recursive_delete_unlinks_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    let link = temp.path().join("link");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
    symlink(&outside, &link).unwrap();
    let display = link.to_string_lossy().replace('\\', "/");
    remove_entry(
        &super::LocalBackend::new("/"),
        &DeleteTarget {
            path: display,
            id: None,
            is_dir: false,
            is_symlink: true,
        },
    )
    .unwrap();
    assert!(!link.exists());
    assert!(outside.join("keep.txt").exists());
}
