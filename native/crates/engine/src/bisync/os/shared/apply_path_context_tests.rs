//! The per-run path context keeps every leaf check fresh: a parent that
//! became a link after the context saw it is still refused, a remembered
//! stage folder is created again after it vanished, and the context ends
//! with its scope.
use super::*;
use crate::vfs::LocalBackend;

fn slash(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn speed_task_sync_context_is_scoped_and_restored() {
    let outer = PathContext::new();
    let inner = PathContext::new();
    assert!(current().is_none());
    scoped(&outer, || {
        assert!(Arc::ptr_eq(&current().unwrap(), &outer));
        scoped(&inner, || {
            assert!(Arc::ptr_eq(&current().unwrap(), &inner));
        });
        assert!(Arc::ptr_eq(&current().unwrap(), &outer));
    });
    assert!(current().is_none());
}

#[test]
fn speed_task_sync_stage_parent_is_created_again_after_it_vanished() {
    let root = tempfile::tempdir().unwrap();
    let backend = LocalBackend::new(&slash(root.path()));
    let folder = slash(&root.path().join("deep/er"));
    let context = PathContext::new();
    scoped(&context, || {
        ensure_parent(&backend, &folder).unwrap();
        assert!(root.path().join("deep/er").is_dir());
        std::fs::remove_dir_all(root.path().join("deep")).unwrap();
        // Remembered: no request, the folder stays missing ...
        ensure_parent(&backend, &folder).unwrap();
        assert!(!root.path().join("deep").exists());
        // ... until a stage below it failed and forgot it.
        forget_parent(&backend, &folder);
        ensure_parent(&backend, &folder).unwrap();
        assert!(root.path().join("deep/er").is_dir());
    });
}

#[test]
fn speed_task_sync_limits_are_computed_once_per_root_and_run() {
    let root = tempfile::tempdir().unwrap();
    let path = slash(root.path());
    let backend = LocalBackend::new(&path);
    let context = PathContext::new();
    scoped(&context, || {
        let first = target_limits(&backend, &path);
        let again = target_limits(&backend, &path);
        assert_eq!(first, again);
        assert_eq!(context.limits.lock().unwrap().len(), 1);
    });
    // Outside a run nothing is remembered.
    let _ = target_limits(&backend, &path);
    assert_eq!(context.limits.lock().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn speed_task_sync_parent_turned_link_is_refused_within_one_run() {
    use super::super::apply_boundary::{guard, omitted};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("dir")).unwrap();
    let path = slash(root.path());
    let backend = LocalBackend::new(&path);
    let context = PathContext::new();
    scoped(&context, || {
        guard(&backend, &path, "dir/file", false).unwrap();
        // The folder is replaced by a link to a place outside the root.
        std::fs::remove_dir(root.path().join("dir")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("dir")).unwrap();
        let refused = guard(&backend, &path, "dir/file", false).unwrap_err();
        assert_eq!(omitted(&refused), Some(OmissionKind::Link));
    });
}
