use super::*;
use std::{collections::HashSet, io::{Read, Write}, sync::{atomic::{AtomicBool, Ordering}, Mutex}};
use crate::vfs::{Scheme, VfsResult};

struct Tree { depth: usize, removed: Mutex<HashSet<String>>, listed: Mutex<Vec<String>> }
impl Tree {
    fn level(path: &str) -> usize { path.split('/').filter(|part| *part == "d").count() }
}
impl Backend for Tree {
    fn scheme(&self) -> Scheme { Scheme::Sftp }
    fn root_display(&self) -> String { "/root".into() }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.listed.lock().unwrap().push(path.into());
        if path.ends_with("/escape") { panic!("link target was traversed"); }
        if Self::level(path) < self.depth {
            Ok(vec![VfsMeta { name: "d".into(), is_dir: true, ..Default::default() }])
        } else { Ok(vec![VfsMeta { name: "escape".into(), is_dir: true, is_symlink: true, ..Default::default() }]) }
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        Ok(VfsMeta { name: path.rsplit('/').next().unwrap().into(), is_dir: true,
            is_symlink: path.ends_with("/escape"), ..Default::default() })
    }
    fn open_read(&self, _: &str) -> VfsResult<Box<dyn Read + Send>> { Err(io::ErrorKind::Unsupported.into()) }
    fn open_write(&self, _: &str) -> VfsResult<Box<dyn Write + Send>> { Err(io::ErrorKind::Unsupported.into()) }
    fn rename(&self, _: &str, _: &str) -> VfsResult<()> { Err(io::ErrorKind::Unsupported.into()) }
    fn remove_file(&self, path: &str) -> VfsResult<()> { self.remove_dir(path) }
    fn remove_dir(&self, path: &str) -> VfsResult<()> { self.removed.lock().unwrap().insert(path.into()); Ok(()) }
    fn mkdir_all(&self, _: &str) -> VfsResult<()> { Err(io::ErrorKind::Unsupported.into()) }
}

#[test]
fn review_task_host_delete_is_iterative_and_does_not_descend_into_child_links() {
    let backend = Tree { depth: 1024, removed: Mutex::default(), listed: Mutex::default() };
    remove_tree(&backend, "/root").unwrap();
    assert_eq!(backend.listed.lock().unwrap().len(), 1025);
    assert_eq!(backend.removed.lock().unwrap().len(), 1026);
    assert!(backend.removed.lock().unwrap().contains("/root"));
}

#[test]
fn review_task_host_delete_rechecks_write_authority_before_each_mutation() {
    let backend = Tree { depth: 1, removed: Mutex::default(), listed: Mutex::default() };
    let live = AtomicBool::new(true);
    let check = || {
        if live.swap(false, Ordering::AcqRel) { Ok(()) }
        else { Err(io::Error::new(io::ErrorKind::PermissionDenied, "revoked")) }
    };
    assert!(remove_tree_checked(&backend, "/root", &check).is_err());
    assert!(backend.removed.lock().unwrap().is_empty());
}

#[test]
fn review_task_host_local_delete_uses_parent_handles_and_preserves_export_root() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("target/sub")).unwrap();
    std::fs::write(root.path().join("target/sub/file"), b"ordinary").unwrap();
    let physical = std::fs::canonicalize(root.path()).unwrap().to_string_lossy().replace('\\', "/");
    let backend = crate::vfs::LocalBackend::new(&physical);
    remove_tree(&backend, &format!("{physical}/target")).unwrap();
    assert!(!root.path().join("target").exists());
    assert!(remove_tree(&backend, &physical).is_err());
    assert!(root.path().exists());
}
