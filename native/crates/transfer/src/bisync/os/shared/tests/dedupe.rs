use super::super::*;
use super::{fwd, tmp};
use crate::vfs::{Backend, DedupeCandidate, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct PlannedDedupe {
    inner: LocalBackend,
    candidates: usize,
    fail_plan: bool,
    /// Every path the destination was asked to write.
    written: Arc<Mutex<Vec<String>>>,
    removals: Arc<AtomicUsize>,
}

impl PlannedDedupe {
    fn record_write(&self, path: &str) {
        self.written
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(path.to_string());
    }
}

impl Backend for PlannedDedupe {
    // Guarded stages finish, flush and confirm their namespace through the
    // extensions of the local folder below (b9f9b2ae).
    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        Some(self)
    }
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
        self.record_write(path);
        self.inner.open_write(path)
    }
    // Sync stages are created exclusively (b9f9b2ae): a backend without
    // this writer is refused ("no atomic exclusive-create writer").
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.record_write(path);
        self.inner.open_write_new(path)
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
        root: &str,
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
                // Candidates outside the sync root are refused.
                path: format!("{}/duplicate-{index}", root.trim_end_matches('/')),
                // A local folder has no duplicate names: each name is one
                // object without a provider ID.
                id: None,
            })
            .collect())
    }
    fn apply_dedupe_plan(&self, plan: &[DedupeCandidate]) -> VfsResult<usize> {
        self.removals.fetch_add(plan.len(), Ordering::Relaxed);
        Ok(plan.len())
    }
}

impl crate::vfs::BackendExtensions for PlannedDedupe {
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        crate::vfs::open_read_regular(&self.inner, path, id)
    }
    fn finish_stage(
        &self,
        path: &str,
        finish: crate::vfs::StageFinish,
    ) -> VfsResult<crate::vfs::StageFinished> {
        crate::vfs::finish_stage(&self.inner, path, finish)
    }
    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        crate::vfs::sync_filesystem(&self.inner, root)
    }
    fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
        crate::vfs::confirm_namespace(&self.inner, parent)
    }
    fn target_limits(&self, root: &str) -> crate::vfs::TargetLimits {
        crate::vfs::target_limits(&self.inner, root)
    }
    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        crate::vfs::unix_mode(&self.inner, path)
    }
    fn volume_identity(&self, root: &str) -> VfsResult<Option<crate::vfs::VolumeIdentity>> {
        crate::vfs::volume_identity(&self.inner, root)
    }
}

/// One run against a destination holding `candidates` planted files that the
/// source lacks, and what it did to that destination.
struct GuardRun {
    outcome: Outcome,
    /// Every path the destination was asked to write.
    written: Vec<String>,
    removals: usize,
    /// Planted destination files still present with their content.
    intact: usize,
}

fn run_guard_case(candidates: usize, fail_plan: bool) -> GuardRun {
    let source_dir = tmp("dedupe_source");
    let destination_dir = tmp("dedupe_destination");
    std::fs::write(source_dir.join("new.txt"), b"new").unwrap();
    // Each planned candidate is an observed destination file without a
    // source counterpart; one outside the sync root is refused (b9f9b2ae).
    for index in 0..candidates {
        std::fs::write(
            destination_dir.join(format!("duplicate-{index}")),
            b"duplicate",
        )
        .unwrap();
    }
    let source = LocalBackend::new(&fwd(&source_dir));
    let written = Arc::new(Mutex::new(Vec::new()));
    let removals = Arc::new(AtomicUsize::new(0));
    let destination = PlannedDedupe {
        inner: LocalBackend::new(&fwd(&destination_dir)),
        candidates,
        fail_plan,
        written: written.clone(),
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
    let intact = (0..candidates)
        .filter(|index| {
            std::fs::read(destination_dir.join(format!("duplicate-{index}")))
                .is_ok_and(|content| content == b"duplicate")
        })
        .count();
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
    let written = written
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    GuardRun {
        outcome,
        written,
        removals: removals.load(Ordering::Relaxed),
        intact,
    }
}

/// What a run that stops for review, or lost its duplicate preflight, may
/// still create on the destination: the replica marker (written before any
/// guard) and the source's new files, which are copied while the scan runs
/// (b9585585, FS3 of 2026-10-28: only deletions and decisions that derive
/// from absence wait for the complete scan). Whether such a copy started
/// before the scan ended depends on timing, so the check is on the names, not
/// on their number. No planted destination name is ever written.
fn only_marker_and_new_files(written: &[String]) -> bool {
    written.iter().all(|path| {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        name.starts_with(crate::bisync::REPLICA_MARKER_NAME) || name.starts_with("new.txt.")
    })
}

#[test]
fn duplicate_count_above_guard_mutates_nothing() {
    let run = run_guard_case(2, false);
    // Above the guard the run stops for review before its first deletion
    // (`Outcome::blocked`, FS3, dd0dccfb).
    assert!(run.outcome.blocked.is_some(), "{:?}", run.outcome.errors);
    assert_eq!(run.removals, 0);
    assert_eq!(run.intact, 2, "a planted duplicate was touched");
    assert!(only_marker_and_new_files(&run.written), "{:?}", run.written);
}

#[test]
fn duplicate_preflight_error_mutates_nothing() {
    let run = run_guard_case(0, true);
    assert!(!run.outcome.errors.is_empty());
    assert_eq!(run.removals, 0);
    assert!(only_marker_and_new_files(&run.written), "{:?}", run.written);
}

#[test]
fn under_limit_plan_is_applied_before_copy() {
    let run = run_guard_case(1, false);
    assert!(run.outcome.errors.is_empty(), "{:?}", run.outcome.errors);
    assert!(!run.written.is_empty());
    assert_eq!(run.removals, 1);
    assert_eq!(run.outcome.stats.deleted, 1);
}
