//! Task suite `delete_speed_task_`: a folder goes to the backend's own tree
//! delete when offered, and the listed tree delete lists each folder once,
//! never stats an entry, never enters a link and removes folders last.
use super::{
    remove_entry_controlled, remove_tree_listed, Backend, BackendExtensions, DeleteTarget,
    RecursiveDeleteFailure, RecursiveDeletePhase, RecursiveDeleteProgress, RecursiveDeleteReport,
    RecursiveDeleteStatus, Scheme, VfsMeta, VfsResult,
};
use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Default)]
struct Calls {
    stat: AtomicUsize,
    list: Mutex<Vec<String>>,
    removed: Mutex<Vec<String>>,
    tree: Mutex<Vec<String>>,
}

/// An in-memory tree: path → (is_dir, is_symlink). Offers `remove_tree`
/// only when `tree_delete` is set.
struct MemoryTree {
    entries: BTreeMap<String, (bool, bool)>,
    tree_delete: bool,
    calls: Calls,
}

impl MemoryTree {
    fn new(tree_delete: bool) -> Self {
        let mut entries = BTreeMap::new();
        for (path, dir, link) in [
            ("/root", true, false),
            ("/root/a.txt", false, false),
            ("/root/link", true, true),
            ("/root/sub", true, false),
            ("/root/sub/b.txt", false, false),
            ("/root/sub/deep", true, false),
            ("/root/sub/deep/c.txt", false, false),
            ("/outside", true, false),
            ("/outside/keep.txt", false, false),
        ] {
            entries.insert(path.to_string(), (dir, link));
        }
        Self {
            entries,
            tree_delete,
            calls: Calls::default(),
        }
    }

    fn meta(&self, path: &str) -> VfsResult<VfsMeta> {
        let (is_dir, is_symlink) = *self
            .entries
            .get(path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Ok(VfsMeta {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            is_dir,
            is_symlink,
            ..VfsMeta::default()
        })
    }

    fn removed(&self) -> Vec<String> {
        self.calls.removed.lock().unwrap().clone()
    }
}

impl Backend for MemoryTree {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.calls.list.lock().unwrap().push(path.to_string());
        let prefix = format!("{path}/");
        self.entries
            .keys()
            .filter(|key| key.starts_with(&prefix) && !key[prefix.len()..].contains('/'))
            .map(|key| self.meta(key))
            .collect()
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.calls.stat.fetch_add(1, Ordering::Relaxed);
        self.meta(path)
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
        self.calls.removed.lock().unwrap().push(path.to_string());
        Ok(())
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let prefix = format!("{path}/");
        let removed = self.calls.removed.lock().unwrap();
        let open = self
            .entries
            .keys()
            .filter(|key| key.starts_with(&prefix) && !key[prefix.len()..].contains('/'))
            .any(|child| !removed.contains(child));
        drop(removed);
        if open {
            return Err(io::Error::other(format!("folder not empty: {path}")));
        }
        self.calls.removed.lock().unwrap().push(path.to_string());
        Ok(())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
}

impl BackendExtensions for MemoryTree {
    fn supports_remove_tree(&self, _path: &str) -> VfsResult<bool> {
        Ok(self.tree_delete)
    }

    fn remove_tree(
        &self,
        path: &str,
        _cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        self.calls.tree.lock().unwrap().push(path.to_string());
        progress(RecursiveDeleteProgress {
            phase: RecursiveDeletePhase::Applying,
            planned: 7,
            removed: 7,
            current: path.to_string(),
        });
        Ok(RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Complete,
            planned: 7,
            removed: 7,
        })
    }
}

fn folder(path: &str) -> DeleteTarget {
    DeleteTarget {
        path: path.into(),
        id: None,
        is_dir: true,
        is_symlink: false,
    }
}

#[test]
fn delete_speed_task_folder_goes_to_the_backend_tree_delete_in_one_call() {
    let backend = MemoryTree::new(true);
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let report = remove_entry_controlled(&backend, &folder("/root"), &cancel, |progress| {
        seen.push(progress.removed)
    })
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Complete);
    assert_eq!((report.planned, report.removed), (7, 7));
    assert_eq!(&*backend.calls.tree.lock().unwrap(), &["/root"]);
    assert_eq!(backend.calls.stat.load(Ordering::Relaxed), 0);
    assert!(backend.calls.list.lock().unwrap().is_empty());
    assert!(backend.removed().is_empty());
    assert_eq!(seen, vec![7]);
}

#[test]
fn delete_speed_task_without_tree_delete_keeps_the_walk() {
    let backend = MemoryTree::new(false);
    let cancel = AtomicBool::new(false);
    let report = remove_entry_controlled(&backend, &folder("/root/sub"), &cancel, |_| {}).unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Complete);
    assert!(backend.calls.tree.lock().unwrap().is_empty());
    assert_eq!(
        backend.removed(),
        [
            "/root/sub/b.txt",
            "/root/sub/deep/c.txt",
            "/root/sub/deep",
            "/root/sub"
        ]
    );
}

#[test]
fn delete_speed_task_listed_tree_lists_once_and_never_enters_links() {
    let backend = MemoryTree::new(false);
    let cancel = AtomicBool::new(false);
    let portions = Mutex::new(Vec::new());
    let report = remove_tree_listed(&backend, "/root", &cancel, &mut |_| {}, |leaves| {
        portions.lock().unwrap().push(leaves.len());
        leaves
            .iter()
            .map(|leaf| {
                assert!(!leaf.is_dir);
                backend.remove_file(&leaf.path)
            })
            .collect()
    })
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Complete);
    assert_eq!((report.planned, report.removed), (7, 7));
    // One stat for the root only; one listing per plain folder, none of the link.
    assert_eq!(backend.calls.stat.load(Ordering::Relaxed), 1);
    assert_eq!(
        &*backend.calls.list.lock().unwrap(),
        &["/root", "/root/sub", "/root/sub/deep"]
    );
    let removed = backend.removed();
    let position = |path: &str| removed.iter().position(|item| item == path).unwrap();
    assert!(position("/root/link") < position("/root"));
    assert!(position("/root/sub/deep/c.txt") < position("/root/sub/deep"));
    assert!(position("/root/sub/deep") < position("/root/sub"));
    assert!(position("/root/sub") < position("/root"));
    assert!(!removed.iter().any(|path| path.starts_with("/outside")));
    assert_eq!(&*portions.lock().unwrap(), &[1, 1, 2]);
}

#[test]
fn delete_speed_task_listed_tree_counts_a_partial_portion_and_stops() {
    let backend = MemoryTree::new(false);
    let cancel = AtomicBool::new(false);
    let failure = remove_tree_listed(&backend, "/root", &cancel, &mut |_| {}, |leaves| {
        leaves
            .iter()
            .map(|leaf| {
                if leaf.path == "/root/link" {
                    Err(io::Error::other("injected leaf failure"))
                } else {
                    backend.remove_file(&leaf.path)
                }
            })
            .collect()
    })
    .unwrap_err();
    assert_eq!(failure.error.to_string(), "injected leaf failure");
    assert_eq!(failure.planned, 7);
    // deep/c.txt, deep, sub/b.txt, sub and root/a.txt went before the failure.
    assert_eq!(failure.removed, 5);
    assert!(!backend.removed().contains(&"/root".to_string()));
}

#[test]
fn delete_speed_task_listed_tree_refuses_a_link_root_and_honours_cancel() {
    let backend = MemoryTree::new(false);
    let cancel = AtomicBool::new(false);
    let refused = remove_tree_listed(&backend, "/root/link", &cancel, &mut |_| {}, |_| {
        panic!("a link root must not be entered")
    })
    .unwrap_err();
    assert_eq!(refused.error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(refused.removed, 0);

    cancel.store(true, Ordering::Release);
    let canceled = remove_tree_listed(&backend, "/root", &cancel, &mut |_| {}, |_| {
        panic!("a canceled run must not remove anything")
    })
    .unwrap();
    assert_eq!(canceled.status, RecursiveDeleteStatus::Canceled);
    assert_eq!(canceled.removed, 0);
    assert!(backend.removed().is_empty());
}

#[test]
fn delete_speed_task_listed_tree_ends_canceled_when_a_slow_remover_stops() {
    let backend = MemoryTree::new(false);
    let cancel = AtomicBool::new(false);
    let report = remove_tree_listed(&backend, "/root", &cancel, &mut |_| {}, |leaves| {
        leaves
            .iter()
            .map(|leaf| {
                if cancel.load(Ordering::Acquire) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                // The user cancels while the first leaf is being removed.
                cancel.store(true, Ordering::Release);
                backend.remove_file(&leaf.path)
            })
            .collect()
    })
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Canceled);
    assert_eq!(report.removed, 1);
    assert_eq!(backend.removed(), ["/root/sub/deep/c.txt"]);
}
