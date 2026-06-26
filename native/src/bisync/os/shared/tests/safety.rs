use super::super::snapshot::{hash_mode, prev_side};
use super::super::*;
use super::{fwd, has_file_containing, tmp};
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

struct WriteFail<'a> {
    inner: &'a LocalBackend,
    needle: &'a str,
}

impl Backend for WriteFail<'_> {
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
        if path.contains(self.needle) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "write blocked",
            ))
        } else {
            self.inner.open_write(path)
        }
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }

    fn is_local(&self) -> bool {
        self.inner.is_local()
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

fn run_with_errors(
    a: &dyn Backend,
    ra: &str,
    b: &dyn Backend,
    rb: &str,
    base: &Baseline,
    opts: BisyncOptions,
    vdir: &Path,
) -> (BisyncStats, Vec<Conflict>, Baseline, Vec<(String, String)>) {
    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let f = WalkFilter::basic(true, &gs);
    let (ma, mb) = (hash_mode(a, b, opts.compare), hash_mode(b, a, opts.compare));
    let (pa, pb) = (prev_side(base, true), prev_side(base, false));
    let at = walk_files(a, ra, &cancel, &f, ma, Some(&pa)).unwrap();
    let bt = walk_files(b, rb, &cancel, &f, mb, Some(&pb)).unwrap();
    let (actions, conflicts, converged) = plan(&at, &bt, base, opts);
    let mut errs = Vec::new();
    let report = super::super::apply::apply_with_results(
        &actions, a, ra, b, rb, opts, vdir, &mut errs, &cancel,
    );
    let at2 = walk_files(a, ra, &cancel, &f, ma, Some(&pa)).unwrap();
    let bt2 = walk_files(b, rb, &cancel, &f, mb, Some(&pb)).unwrap();
    let nb = update_baseline(base, &at2, &bt2, &report.completed, &converged, &conflicts);
    (report.stats, conflicts, nb, errs)
}

#[test]
fn failed_apply_paths_stay_out_of_new_baseline_and_retry() {
    let a = tmp("sfa");
    let b = tmp("sfb");
    let v = tmp("sfv");
    std::fs::write(a.join("ok.txt"), b"ok").unwrap();
    std::fs::write(a.join("fail.txt"), b"retry").unwrap();

    let (ra, rb) = (fwd(&a), fwd(&b));
    let ba = LocalBackend::new(&ra);
    let bb = LocalBackend::new(&rb);
    let blocked_b = WriteFail {
        inner: &bb,
        needle: "fail.txt",
    };
    let opts = BisyncOptions {
        max_transfers: 1,
        ..Default::default()
    };

    let (st, conflicts, nb, errs) =
        run_with_errors(&ba, &ra, &blocked_b, &rb, &Baseline::new(), opts, &v);
    assert!(conflicts.is_empty());
    assert_eq!(st.a_to_b, 1);
    assert_eq!(st.errors, 1);
    assert_eq!(errs.len(), 1);
    assert!(b.join("ok.txt").exists());
    assert!(!b.join("fail.txt").exists());
    assert!(nb.contains_key("ok.txt"));
    assert!(!nb.contains_key("fail.txt"));

    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let f = WalkFilter::basic(true, &gs);
    let (ma, mb) = (
        hash_mode(&ba, &bb, opts.compare),
        hash_mode(&bb, &ba, opts.compare),
    );
    let (pa, pb) = (prev_side(&nb, true), prev_side(&nb, false));
    let at = walk_files(&ba, &ra, &cancel, &f, ma, Some(&pa)).unwrap();
    let bt = walk_files(&bb, &rb, &cancel, &f, mb, Some(&pb)).unwrap();
    let (retry, retry_conflicts, _) = plan(&at, &bt, &nb, opts);
    assert!(retry_conflicts.is_empty());
    assert!(retry
        .iter()
        .any(|a| matches!(a, Action::CopyAtoB(rel) if rel == "fail.txt")));
    assert!(!retry
        .iter()
        .any(|a| matches!(a, Action::CopyAtoB(rel) if rel == "ok.txt")));

    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn backup_failure_blocks_overwrite_and_delete() {
    let a = tmp("bfa");
    let b = tmp("bfb");
    let block = tmp("bfblock");
    let vfile = block.join("versions-file");
    std::fs::write(&vfile, b"not a directory").unwrap();
    std::fs::write(a.join("overwrite.txt"), b"new").unwrap();
    std::fs::write(b.join("overwrite.txt"), b"old").unwrap();
    std::fs::write(b.join("delete.txt"), b"keep").unwrap();

    let (ra, rb) = (fwd(&a), fwd(&b));
    let ba = LocalBackend::new(&ra);
    let bb = LocalBackend::new(&rb);
    let opts = BisyncOptions {
        max_transfers: 1,
        ..Default::default()
    };
    let actions = vec![
        Action::CopyAtoB("overwrite.txt".to_string()),
        Action::DeleteB("delete.txt".to_string()),
    ];
    let mut errs = Vec::new();
    let cancel = AtomicBool::new(false);
    let report = super::super::apply::apply_with_results(
        &actions, &ba, &ra, &bb, &rb, opts, &vfile, &mut errs, &cancel,
    );

    assert_eq!(report.stats.a_to_b, 0);
    assert_eq!(report.stats.deleted, 0);
    assert_eq!(report.stats.errors, 2);
    assert!(report.completed.is_empty());
    assert_eq!(errs.len(), 2);
    assert_eq!(std::fs::read(b.join("overwrite.txt")).unwrap(), b"old");
    assert!(b.join("delete.txt").exists());

    for d in [&a, &b, &block] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn keep_both_copy_failure_blocks_resolution_and_recovers() {
    let a = tmp("kba");
    let b = tmp("kbb");
    let v = tmp("kbv");
    std::fs::write(a.join("f.txt"), b"orig").unwrap();

    let (ra, rb) = (fwd(&a), fwd(&b));
    let ba = LocalBackend::new(&ra);
    let bb = LocalBackend::new(&rb);
    let opts = BisyncOptions {
        conflict: ConflictMode::KeepBoth,
        max_transfers: 1,
        ..Default::default()
    };
    let (_s1, _c1, base1) = super::run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);

    std::fs::write(b.join("f.txt"), b"B-edit").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(a.join("f.txt"), b"A-edit-newer").unwrap();

    let blocked_b = WriteFail {
        inner: &bb,
        needle: "Konflikt",
    };
    let (st_fail, conf_fail, base_fail, errs_fail) =
        run_with_errors(&ba, &ra, &blocked_b, &rb, &base1, opts, &v);
    assert!(conf_fail.is_empty());
    assert_eq!(st_fail.a_to_b, 0);
    assert_eq!(st_fail.errors, 1);
    assert_eq!(errs_fail.len(), 1);
    assert_eq!(std::fs::read(b.join("f.txt")).unwrap(), b"B-edit");
    assert!(!has_file_containing(&b, "Konflikt"));

    let (st_recover, conf_recover, _base_recover, errs_recover) =
        run_with_errors(&ba, &ra, &bb, &rb, &base_fail, opts, &v);
    assert!(conf_recover.is_empty());
    assert!(errs_recover.is_empty());
    assert_eq!(st_recover.a_to_b, 1);
    assert_eq!(std::fs::read(b.join("f.txt")).unwrap(), b"A-edit-newer");
    assert!(has_file_containing(&b, "Konflikt"));

    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}
