use super::super::*;
use super::{fwd, tmp};
use crate::vfs::{Backend, BackendExtensions, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct FailOnceRemove {
    inner: LocalBackend,
    failures: AtomicUsize,
}

struct CountWrites {
    inner: LocalBackend,
    writes: AtomicUsize,
}

macro_rules! delegate_read_side {
    ($ty:ty, $field:ident) => {
        fn scheme(&self) -> Scheme {
            self.$field.scheme()
        }
        fn root_display(&self) -> String {
            self.$field.root_display()
        }
        fn state_identity(&self) -> String {
            self.$field.state_identity()
        }
        fn namespace_identity(&self) -> String {
            self.$field.namespace_identity()
        }
        fn extensions(&self) -> Option<&dyn BackendExtensions> {
            Some(self)
        }
        fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
            self.$field.list_dir(path)
        }
        fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
            self.$field.stat(path)
        }
        fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
            self.$field.open_read(path)
        }
        fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
            self.$field.rename(source, destination)
        }
        fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
            self.$field.rename_no_replace(source, destination)
        }
        fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
            self.$field.promote_staged(staged, destination)
        }
        fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
            self.$field.promote_staged_no_replace(staged, destination)
        }
        fn discard_copy_stage(&self, staged: &str) -> VfsResult<()> {
            self.$field.discard_copy_stage(staged)
        }
        fn remove_dir(&self, path: &str) -> VfsResult<()> {
            self.$field.remove_dir(path)
        }
        fn mkdir_all(&self, path: &str) -> VfsResult<()> {
            self.$field.mkdir_all(path)
        }
        fn rename_overwrites(&self) -> bool {
            self.$field.rename_overwrites()
        }
        fn case_sensitive_paths(&self, root: &str) -> bool {
            self.$field.case_sensitive_paths(root)
        }
        fn is_local(&self) -> bool {
            true
        }
    };
}

impl Backend for FailOnceRemove {
    delegate_read_side!(FailOnceRemove, inner);

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_copy_stage_sized(path, size)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        if self
            .failures
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected move-source removal failure",
            ))
        } else {
            self.inner.remove_file(path)
        }
    }
}

impl Backend for CountWrites {
    delegate_read_side!(CountWrites, inner);

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.inner.open_write(path)
    }

    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.inner.open_write_copy_stage_sized(path, size)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
}

// Keep the real local stage/namespace guarantees while observing the wrapper's
// sized stage writer. Returning inner.extensions() would bypass CountWrites.
macro_rules! local_extensions {
    ($ty:ty) => {
        impl BackendExtensions for $ty {
            fn open_read_regular(
                &self,
                path: &str,
                id: Option<&str>,
            ) -> VfsResult<Box<dyn Read + Send>> {
                crate::vfs::open_read_regular(&self.inner, path, id)
            }
            fn finish_stage(
                &self,
                path: &str,
                finish: crate::vfs::StageFinish,
            ) -> VfsResult<crate::vfs::StageFinished> {
                crate::vfs::finish_stage(&self.inner, path, finish)
            }
            fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
                crate::vfs::confirm_namespace(&self.inner, parent)
            }
            fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
                crate::vfs::unix_mode(&self.inner, path)
            }
        }
    };
}
local_extensions!(FailOnceRemove);
local_extensions!(CountWrites);

fn snapshot(
    backend: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter<'_>,
) -> Tree {
    walk_files(backend, root, cancel, filter, HashMode::None, None).unwrap()
}

#[test]
fn failed_source_delete_retries_as_verified_finalize_without_recopy() {
    let a = tmp("move_a");
    let b = tmp("move_b");
    let versions = tmp("move_versions");
    std::fs::write(a.join("file.txt"), b"move me safely").unwrap();
    let (root_a, root_b) = (fwd(&a), fwd(&b));
    let source = FailOnceRemove {
        inner: LocalBackend::new(&root_a),
        failures: AtomicUsize::new(1),
    };
    let destination = CountWrites {
        inner: LocalBackend::new(&root_b),
        writes: AtomicUsize::new(0),
    };
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let options = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::NoDelete,
        move_files: true,
        reversible: false,
        // This test exercises byte-for-byte finalization, not filesystem
        // timestamp preservation. A streamed destination can legitimately
        // land in the next millisecond; size-only planning is deterministic,
        // while `verify_and_delete_source` still compares all content before
        // it removes the source.
        compare: CompareMode::SizeOnly,
        retries: 3,
        ..Default::default()
    };
    let baseline = Baseline::new();
    let (tree_a, tree_b) = (
        snapshot(&source, &root_a, &cancel, &filter),
        snapshot(&destination, &root_b, &cancel, &filter),
    );
    let (actions, _, _) = plan(&tree_a, &tree_b, &baseline, options);
    assert!(matches!(actions.as_slice(), [Action::CopyAtoB(path)] if path == "file.txt"));
    let mut first_errors = Vec::new();
    let first = super::super::apply::apply_planned_with_results(
        &actions,
        &tree_a,
        &tree_b,
        &source,
        &root_a,
        &destination,
        &root_b,
        options,
        &versions,
        &mut first_errors,
        &cancel,
    );
    assert_eq!(first.completed.len(), 0);
    assert_eq!(first_errors.len(), 1);
    assert!(
        first_errors[0]
            .1
            .contains("injected move-source removal failure"),
        "{first_errors:?}"
    );
    assert!(a.join("file.txt").exists());
    assert_eq!(
        std::fs::read(b.join("file.txt")).unwrap(),
        b"move me safely"
    );
    assert_eq!(destination.writes.load(Ordering::Relaxed), 1);

    let (after_a, after_b) = (
        snapshot(&source, &root_a, &cancel, &filter),
        snapshot(&destination, &root_b, &cancel, &filter),
    );
    let (retry_actions, _, converged) = plan(&after_a, &after_b, &baseline, options);
    assert!(matches!(
        retry_actions.as_slice(),
        [Action::FinalizeMoveAtoB(path)] if path == "file.txt"
    ));
    cancel.store(true, Ordering::Relaxed);
    let mut canceled_errors = Vec::new();
    let canceled = super::super::apply::apply_planned_with_results(
        &retry_actions,
        &after_a,
        &after_b,
        &source,
        &root_a,
        &destination,
        &root_b,
        options,
        &versions,
        &mut canceled_errors,
        &cancel,
    );
    assert!(canceled.completed.is_empty());
    assert!(a.join("file.txt").exists());
    cancel.store(false, Ordering::Relaxed);
    let mut retry_errors = Vec::new();
    let retry = super::super::apply::apply_planned_with_results(
        &retry_actions,
        &after_a,
        &after_b,
        &source,
        &root_a,
        &destination,
        &root_b,
        options,
        &versions,
        &mut retry_errors,
        &cancel,
    );
    assert!(retry_errors.is_empty());
    assert_eq!(retry.completed, retry_actions);
    assert_eq!(destination.writes.load(Ordering::Relaxed), 1);
    assert!(!a.join("file.txt").exists());
    let final_a = snapshot(&source, &root_a, &cancel, &filter);
    let final_b = snapshot(&destination, &root_b, &cancel, &filter);
    let updated = update_baseline(
        &baseline,
        &final_a,
        &final_b,
        &retry.completed,
        &converged,
        &[],
    );
    assert_eq!(updated["file.txt"].0, None);
    assert!(updated["file.txt"].1.is_some());
    assert_eq!(
        std::fs::read(b.join("file.txt")).unwrap(),
        b"move me safely"
    );
    let (noop, conflicts, _) = plan(&final_a, &final_b, &updated, options);
    assert!(noop.is_empty() && conflicts.is_empty());
    for directory in [a, b, versions] {
        std::fs::remove_dir_all(directory).ok();
    }
}
