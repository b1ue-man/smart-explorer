use super::super::*;
use super::{fwd, tmp};
use crate::vfs::{Backend, DedupeCandidate, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

struct PlannedDedupe {
    inner: LocalBackend,
    candidates: usize,
    fail_plan: bool,
    writes: Arc<AtomicUsize>,
    removals: Arc<AtomicUsize>,
}

impl Backend for PlannedDedupe {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn state_identity(&self) -> String {
        self.inner.state_identity()
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
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.inner.open_write(path)
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.removals.fetch_add(1, Ordering::Relaxed);
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.removals.fetch_add(1, Ordering::Relaxed);
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn is_local(&self) -> bool {
        true
    }
    fn plan_dedupe_recursive(
        &self,
        _root: &str,
        _keep: &dyn Fn(&str) -> bool,
    ) -> VfsResult<Vec<DedupeCandidate>> {
        if self.fail_plan {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected duplicate preflight failure",
            ));
        }
        Ok((0..self.candidates)
            .map(|index| DedupeCandidate {
                path: format!("/duplicate-{index}"),
                id: Some(format!("id-{index}")),
            })
            .collect())
    }
    fn apply_dedupe_plan(&self, plan: &[DedupeCandidate]) -> VfsResult<usize> {
        self.removals.fetch_add(plan.len(), Ordering::Relaxed);
        Ok(plan.len())
    }
}

fn run_guard_case(candidates: usize, fail_plan: bool) -> (Outcome, usize, usize) {
    let source_dir = tmp("dedupe_source");
    let destination_dir = tmp("dedupe_destination");
    std::fs::write(source_dir.join("new.txt"), b"new").unwrap();
    let source = LocalBackend::new(&fwd(&source_dir));
    let writes = Arc::new(AtomicUsize::new(0));
    let removals = Arc::new(AtomicUsize::new(0));
    let destination = PlannedDedupe {
        inner: LocalBackend::new(&fwd(&destination_dir)),
        candidates,
        fail_plan,
        writes: writes.clone(),
        removals: removals.clone(),
    };
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let options = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        max_delete: 1,
        ..Default::default()
    };
    let outcome = super::super::run(
        &source,
        &fwd(&source_dir),
        &destination,
        &fwd(&destination_dir),
        options,
        &cancel,
        &filter,
    );
    let pair = pair_id_for(
        &source,
        &fwd(&source_dir),
        &destination,
        &fwd(&destination_dir),
    );
    std::fs::remove_file(baseline_path(&pair)).ok();
    std::fs::remove_dir_all(versions_dir(&pair)).ok();
    std::fs::remove_dir_all(source_dir).ok();
    std::fs::remove_dir_all(destination_dir).ok();
    (
        outcome,
        writes.load(Ordering::Relaxed),
        removals.load(Ordering::Relaxed),
    )
}

#[test]
fn duplicate_count_above_guard_mutates_nothing() {
    let (outcome, writes, removals) = run_guard_case(2, false);
    assert!(!outcome.errors.is_empty());
    assert_eq!((writes, removals), (0, 0));
}

#[test]
fn duplicate_preflight_error_mutates_nothing() {
    let (outcome, writes, removals) = run_guard_case(0, true);
    assert!(!outcome.errors.is_empty());
    assert_eq!((writes, removals), (0, 0));
}

#[test]
fn under_limit_plan_is_applied_before_copy() {
    let (outcome, writes, removals) = run_guard_case(1, false);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert!(writes > 0);
    assert_eq!(removals, 1);
    assert_eq!(outcome.stats.deleted, 1);
}
