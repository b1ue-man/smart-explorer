use super::super::*;
use super::{fwd, tmp};
use crate::vfs::LocalBackend;
use std::sync::atomic::AtomicBool;

#[test]
fn prune_count_keeps_newest_n() {
    let v = tmp("pv");
    for ts in [100u64, 200, 300, 400] {
        let d = v.join(ts.to_string());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("x"), b"x").unwrap();
    }
    prune_versions(
        &v,
        &Versioning {
            scheme: VersioningScheme::Count,
            days: 0,
            count: 2,
        },
    );
    assert!(v.join("400").exists() && v.join("300").exists());
    assert!(!v.join("200").exists() && !v.join("100").exists());
    std::fs::remove_dir_all(&v).ok();
}

#[test]
fn max_delete_guard_aborts_mass_deletion() {
    let a = tmp("gda");
    let b = tmp("gdb");
    for n in ["1", "2", "3"] {
        std::fs::write(a.join(format!("f{n}.txt")), b"x").unwrap();
    }
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let f = WalkFilter::basic(true, &gs);
    // First run copies the 3 files A→B and records the baseline.
    let o1 = super::super::run(&ba, &ra, &bb, &rb, BisyncOptions::default(), &cancel, &f);
    assert_eq!(o1.errors.len(), 0);
    assert!(b.join("f1.txt").exists());
    // Delete all on A; a sync with max_delete=1 must abort and touch nothing.
    for n in ["1", "2", "3"] {
        std::fs::remove_file(a.join(format!("f{n}.txt"))).unwrap();
    }
    let opts = BisyncOptions {
        max_delete: 1,
        ..Default::default()
    };
    let o2 = super::super::run(&ba, &ra, &bb, &rb, opts, &cancel, &f);
    assert!(!o2.errors.is_empty(), "guard reports an abort");
    assert!(b.join("f1.txt").exists(), "nothing deleted when aborted");
    let pair = pair_id(&ra, &rb);
    let _ = std::fs::remove_file(baseline_path(&pair));
    let _ = std::fs::remove_dir_all(versions_dir(&pair));
    for d in [&a, &b] {
        std::fs::remove_dir_all(d).ok();
    }
}
