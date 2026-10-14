use super::*;
use crate::vfs::{LocalBackend, Scheme, VfsResult};
use std::{
    cell::Cell,
    collections::BTreeMap,
    io::{Read, Write},
    sync::Mutex,
};

fn canonical(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// A non-local tree: path → (folder, link).
struct Remote {
    entries: BTreeMap<String, (bool, bool)>,
    listed: Mutex<Vec<String>>,
    removed: Mutex<Vec<String>>,
}

impl Remote {
    fn new(entries: &[(&str, bool, bool)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(path, dir, link)| (path.to_string(), (*dir, *link)))
                .collect(),
            listed: Mutex::default(),
            removed: Mutex::default(),
        }
    }
    fn meta(&self, path: &str) -> VfsResult<VfsMeta> {
        let (is_dir, is_symlink) =
            self.entries.get(path).copied().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("missing {path}"))
            })?;
        Ok(VfsMeta {
            name: path.rsplit('/').next().unwrap_or(path).into(),
            is_dir,
            is_symlink,
            ..Default::default()
        })
    }
}

impl Backend for Remote {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/r".into()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.listed.lock().unwrap().push(path.into());
        let prefix = format!("{path}/");
        self.entries
            .keys()
            .filter(|child| {
                child
                    .strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.contains('/'))
            })
            .map(|child| self.meta(child))
            .collect()
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.meta(path)
    }
    fn open_read(&self, _: &str) -> VfsResult<Box<dyn Read + Send>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn open_write(&self, _: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn rename(&self, _: &str, _: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.removed.lock().unwrap().push(path.into());
        Ok(())
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.removed.lock().unwrap().push(path.into());
        Ok(())
    }
    fn mkdir_all(&self, _: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[test]
fn delete_speed_task_local_tree_delete_counts_every_entry() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("target/sub/deeper")).unwrap();
    std::fs::write(root.path().join("target/a"), b"a").unwrap();
    std::fs::write(root.path().join("target/sub/b"), b"b").unwrap();
    std::fs::write(root.path().join("target/sub/deeper/c"), b"c").unwrap();
    let physical = canonical(root.path());
    let backend = LocalBackend::new(&physical);
    let removed = Cell::new(0u64);
    remove_tree_observed(&backend, &format!("{physical}/target"), &|| Ok(()), &|| {
        removed.set(removed.get() + 1)
    })
    .unwrap();
    // a, sub, b, deeper, c and the folder itself.
    assert_eq!(removed.get(), 6);
    assert!(!root.path().join("target").exists());
    assert!(root.path().exists());
}

#[cfg(unix)]
#[test]
fn delete_speed_task_local_tree_delete_removes_folder_link_as_entry() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("target")).unwrap();
    std::fs::create_dir_all(root.path().join("outside")).unwrap();
    std::fs::write(root.path().join("outside/kept"), b"kept").unwrap();
    std::os::unix::fs::symlink(root.path().join("outside"), root.path().join("target/link"))
        .unwrap();
    let physical = canonical(root.path());
    let backend = LocalBackend::new(&physical);
    let removed = Cell::new(0u64);
    remove_tree_observed(&backend, &format!("{physical}/target"), &|| Ok(()), &|| {
        removed.set(removed.get() + 1)
    })
    .unwrap();
    // The link and the folder; never the link target's content.
    assert_eq!(removed.get(), 2);
    assert!(!root.path().join("target").exists());
    assert_eq!(
        std::fs::read(root.path().join("outside/kept")).unwrap(),
        b"kept"
    );
}

#[test]
fn delete_speed_task_local_tree_delete_refuses_export_root() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("kept"), b"kept").unwrap();
    let physical = canonical(root.path());
    let backend = LocalBackend::new(&physical);
    let removed = Cell::new(0u64);
    let error = remove_tree_observed(&backend, &physical, &|| Ok(()), &|| {
        removed.set(removed.get() + 1)
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(removed.get(), 0);
    assert_eq!(std::fs::read(root.path().join("kept")).unwrap(), b"kept");
}

#[test]
fn delete_speed_task_remote_tree_delete_counts_entries_and_never_lists_links() {
    let backend = Remote::new(&[
        ("/r/t", true, false),
        ("/r/t/a", false, false),
        ("/r/t/l", true, true),
        ("/r/t/s", true, false),
        ("/r/t/s/b", false, false),
    ]);
    let removed = Cell::new(0u64);
    remove_tree_observed(&backend, "/r/t", &|| Ok(()), &|| {
        removed.set(removed.get() + 1)
    })
    .unwrap();
    assert_eq!(removed.get(), 5);
    let removed_paths = backend.removed.lock().unwrap().clone();
    assert_eq!(removed_paths.len(), 5);
    assert_eq!(removed_paths.last().map(String::as_str), Some("/r/t"));
    assert!(!backend
        .listed
        .lock()
        .unwrap()
        .contains(&"/r/t/l".to_string()));
}

#[test]
fn delete_speed_task_tree_delete_stops_at_the_next_entry_when_checked_out() {
    let backend = Remote::new(&[
        ("/r/t", true, false),
        ("/r/t/a", false, false),
        ("/r/t/b", false, false),
        ("/r/t/c", false, false),
    ]);
    let checks = Cell::new(0u32);
    let removed = Cell::new(0u64);
    // The first check opens the run, the second admits one entry.
    let error = remove_tree_observed(
        &backend,
        "/r/t",
        &|| {
            checks.set(checks.get() + 1);
            if checks.get() > 2 {
                Err(io::ErrorKind::Interrupted.into())
            } else {
                Ok(())
            }
        },
        &|| removed.set(removed.get() + 1),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(removed.get(), 1);
    assert_eq!(backend.removed.lock().unwrap().len(), 1);
}
