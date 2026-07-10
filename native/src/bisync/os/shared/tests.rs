use super::snapshot::{hash_mode, md5_hex_to_u64, md5_to_u64, prev_side};
use super::*;
use crate::vfs::{Backend, LocalBackend};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "tests/dedupe.rs"]
mod dedupe;
#[path = "tests/extra.rs"]
mod extra;
#[path = "tests/hash_walk.rs"]
mod hash_walk;
#[path = "tests/incremental.rs"]
mod incremental;
#[path = "tests/move_retry.rs"]
mod move_retry;
#[path = "tests/safety.rs"]
mod safety;

fn tmp(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!("bisync_{}_{}_{}", tag, std::process::id(), nanos));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Full run helper: walk, plan, apply, update+save baseline.
fn run(
    a: &LocalBackend,
    ra: &str,
    b: &LocalBackend,
    rb: &str,
    base: &Baseline,
    opts: BisyncOptions,
    vdir: &Path,
) -> (BisyncStats, Vec<Conflict>, Baseline) {
    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let f = WalkFilter::basic(true, &gs);
    let (ma, mb) = (hash_mode(a, b, opts.compare), hash_mode(b, a, opts.compare));
    let (pa, pb) = (prev_side(base, true), prev_side(base, false));
    let at = walk_files(a, ra, &cancel, &f, ma, Some(&pa)).unwrap();
    let bt = walk_files(b, rb, &cancel, &f, mb, Some(&pb)).unwrap();
    let (actions, conflicts, converged) = plan(&at, &bt, base, opts);
    let mut errs = Vec::new();
    let report = super::apply::apply_planned_with_results(
        &actions, &at, &bt, a, ra, b, rb, opts, vdir, &mut errs, &cancel,
    );
    let st = report.stats;
    // re-walk for an accurate baseline after writes
    let at2 = walk_files(a, ra, &cancel, &f, ma, Some(&pa)).unwrap();
    let bt2 = walk_files(b, rb, &cancel, &f, mb, Some(&pb)).unwrap();
    let nb = update_baseline(base, &at2, &bt2, &report.completed, &converged, &conflicts);
    (st, conflicts, nb)
}

#[test]
fn first_run_mirrors_both_ways() {
    let a = tmp("a");
    let b = tmp("b");
    std::fs::write(a.join("only_a.txt"), b"a").unwrap();
    std::fs::create_dir_all(b.join("sub")).unwrap();
    std::fs::write(b.join("sub/only_b.txt"), b"bb").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("v");
    let (st, conf, _nb) = run(
        &ba,
        &ra,
        &bb,
        &rb,
        &Baseline::new(),
        BisyncOptions::default(),
        &v,
    );
    assert_eq!(conf.len(), 0);
    assert!(a.join("sub/only_b.txt").exists(), "B's file copied to A");
    assert!(b.join("only_a.txt").exists(), "A's file copied to B");
    assert_eq!(st.a_to_b + st.b_to_a, 2);
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn one_side_change_propagates_then_stable() {
    let a = tmp("a2");
    let b = tmp("b2");
    std::fs::write(a.join("f.txt"), b"v1").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("v2");
    let opts = BisyncOptions::default();
    let (_s1, _c1, base1) = run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);
    assert!(b.join("f.txt").exists());
    // change A only
    std::thread::sleep(std::time::Duration::from_millis(15));
    std::fs::write(a.join("f.txt"), b"v2-longer").unwrap();
    let (s2, c2, base2) = run(&ba, &ra, &bb, &rb, &base1, opts, &v);
    assert_eq!(c2.len(), 0);
    assert_eq!(s2.a_to_b, 1);
    assert_eq!(std::fs::read(b.join("f.txt")).unwrap(), b"v2-longer");
    // a reversible backup of B's old "v1" must exist
    let any_backup = walkdir_count(&v) > 0;
    assert!(any_backup, "old version backed up");
    // third run: nothing to do
    let (s3, c3, _b3) = run(&ba, &ra, &bb, &rb, &base2, opts, &v);
    assert_eq!(s3.a_to_b + s3.b_to_a + s3.deleted, 0);
    assert_eq!(c3.len(), 0);
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn both_sides_change_is_a_conflict_not_overwrite() {
    let a = tmp("a3");
    let b = tmp("b3");
    std::fs::write(a.join("f.txt"), b"base").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("v3");
    let opts = BisyncOptions::default();
    let (_s, _c, base1) = run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);
    // change BOTH sides differently
    std::thread::sleep(std::time::Duration::from_millis(15));
    std::fs::write(a.join("f.txt"), b"edit-A").unwrap();
    std::fs::write(b.join("f.txt"), b"edit-B-different").unwrap();
    let (s2, c2, _b2) = run(&ba, &ra, &bb, &rb, &base1, opts, &v);
    assert_eq!(c2.len(), 1, "both-changed must be a conflict");
    assert_eq!(c2[0].rel, "f.txt");
    assert_eq!(s2.a_to_b + s2.b_to_a, 0, "nothing overwritten");
    // neither side was clobbered
    assert_eq!(std::fs::read(a.join("f.txt")).unwrap(), b"edit-A");
    assert_eq!(std::fs::read(b.join("f.txt")).unwrap(), b"edit-B-different");
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn newer_wins_mode_resolves_without_conflict() {
    let a = tmp("a4");
    let b = tmp("b4");
    std::fs::write(a.join("f.txt"), b"base").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("v4");
    let opts = BisyncOptions {
        conflict: ConflictMode::NewerWins,
        ..BisyncOptions::default()
    };
    let (_s, _c, base1) = run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);
    std::fs::write(a.join("f.txt"), b"older").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(b.join("f.txt"), b"newer-wins").unwrap();
    let (_s2, c2, _b2) = run(&ba, &ra, &bb, &rb, &base1, opts, &v);
    assert_eq!(c2.len(), 0);
    assert_eq!(std::fs::read(a.join("f.txt")).unwrap(), b"newer-wins");
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn one_way_direction_ignores_other_side() {
    let a = tmp("a5");
    let b = tmp("b5");
    std::fs::write(b.join("only_b.txt"), b"x").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("v5");
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        ..BisyncOptions::default()
    };
    let (_s, _c, _base) = run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);
    // A→B only: B's file is NOT pulled into A.
    assert!(!a.join("only_b.txt").exists());
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

fn walkdir_count(p: &Path) -> usize {
    let mut n = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    n += 1;
                }
            }
        }
    }
    n
}

#[test]
fn mirror_makes_dest_exact_and_deletes_orphans() {
    let a = tmp("ma");
    let b = tmp("mb");
    std::fs::write(a.join("keep.txt"), b"new").unwrap();
    std::fs::write(b.join("orphan.txt"), b"old").unwrap(); // only on B
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("mv");
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        ..Default::default()
    };
    let (st, conf, _nb) = run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);
    assert_eq!(conf.len(), 0);
    assert!(b.join("keep.txt").exists(), "A's file mirrored to B");
    assert!(!b.join("orphan.txt").exists(), "B orphan deleted by mirror");
    assert_eq!(st.a_to_b, 1);
    assert_eq!(st.deleted, 1);
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn nodelete_never_removes_dest_files() {
    let a = tmp("na");
    let b = tmp("nb");
    std::fs::write(a.join("f.txt"), b"v1").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("nv");
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::NoDelete,
        ..Default::default()
    };
    // First run copies f.txt to B and records a baseline.
    let (_s, _c, base1) = run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);
    assert!(b.join("f.txt").exists());
    // Delete on A, sync again: B must keep its copy (no-delete).
    std::fs::remove_file(a.join("f.txt")).unwrap();
    let (st, _c2, _b2) = run(&ba, &ra, &bb, &rb, &base1, opts, &v);
    assert!(b.join("f.txt").exists(), "no-delete kept B's file");
    assert_eq!(st.deleted, 0);
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

fn has_file_containing(p: &Path, needle: &str) -> bool {
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.contains(needle))
                    .unwrap_or(false)
                {
                    return true;
                }
            }
        }
    }
    false
}

#[test]
fn keep_both_preserves_loser_as_conflict_copy() {
    let a = tmp("ka");
    let b = tmp("kb");
    std::fs::write(a.join("f.txt"), b"orig").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let (ba, bb) = (LocalBackend::new(&ra), LocalBackend::new(&rb));
    let v = tmp("kv");
    let opts = BisyncOptions {
        conflict: ConflictMode::KeepBoth,
        ..Default::default()
    };
    // First run establishes the baseline (copies f.txt to B).
    let (_s, _c, base1) = run(&ba, &ra, &bb, &rb, &Baseline::new(), opts, &v);
    // Change both sides differently; make A clearly newer.
    std::fs::write(b.join("f.txt"), b"B-edit").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(a.join("f.txt"), b"A-edit-newer").unwrap();
    let (_st, conf, _b2) = run(&ba, &ra, &bb, &rb, &base1, opts, &v);
    assert_eq!(conf.len(), 0, "keep-both surfaces no conflict");
    assert_eq!(
        std::fs::read(b.join("f.txt")).unwrap(),
        b"A-edit-newer",
        "winner (newer) keeps the original name on B"
    );
    assert!(
        has_file_containing(&b, "Konflikt"),
        "loser preserved as a (Konflikt …) copy on B"
    );
    for d in [&a, &b, &v] {
        std::fs::remove_dir_all(d).ok();
    }
}

#[test]
fn size_only_ignores_mtime_differences() {
    let a = Tree::from([(
        "f".to_string(),
        Sig {
            size: 10,
            mtime_ms: 1000,
            hash: 0,
        },
    )]);
    let b = Tree::from([(
        "f".to_string(),
        Sig {
            size: 10,
            mtime_ms: 9999,
            hash: 0,
        },
    )]);
    let base = Baseline::new();
    let opts = BisyncOptions {
        compare: CompareMode::SizeOnly,
        ..Default::default()
    };
    let (actions, conflicts, _conv) = plan(&a, &b, &base, opts);
    assert!(actions.is_empty(), "same size ⇒ no work under size-only");
    assert!(conflicts.is_empty());
    // Under the default mtime+size compare, the mtime gap is a real diff.
    let (_actions2, c2, _v2) = plan(&a, &b, &base, BisyncOptions::default());
    assert_eq!(c2.len(), 1, "mtime differs under default");
}

#[test]
fn content_hash_skips_mtime_only_difference() {
    // The local↔Drive case: Drive's modifiedTime never equals the local
    // mtime, so under the DEFAULT size+mtime compare every file looked
    // "changed" and got re-transferred. With a content hash on both sides,
    // equal size+hash means identical content → NO action, regardless of
    // mtime. Tested through Mirror (stateless) — exactly what re-uploaded
    // everything before.
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        ..Default::default()
    };
    let base = Baseline::new();
    let a = Tree::from([(
        "f".to_string(),
        Sig {
            size: 10,
            mtime_ms: 1000,
            hash: 0xABCD,
        },
    )]);
    let b = Tree::from([(
        "f".to_string(),
        Sig {
            size: 10,
            mtime_ms: 9_999_999,
            hash: 0xABCD,
        },
    )]);
    let (actions, _c, conv) = plan(&a, &b, &base, opts);
    assert!(
        actions.is_empty(),
        "same content hash ⇒ no copy despite mtime gap"
    );
    assert_eq!(conv, vec!["f".to_string()], "recorded as converged");
    // A real content change (different hash) under the same mtime gap DOES copy.
    let b2 = Tree::from([(
        "f".to_string(),
        Sig {
            size: 10,
            mtime_ms: 9_999_999,
            hash: 0x1234,
        },
    )]);
    let (actions2, _c2, _v2) = plan(&a, &b2, &base, opts);
    assert_eq!(actions2.len(), 1, "different content hash ⇒ copy");
    // When only ONE side has a hash (e.g. a hash-less remote), the short-
    // circuit must NOT fire — fall back to the mtime+size compare.
    let a0 = Tree::from([(
        "f".to_string(),
        Sig {
            size: 10,
            mtime_ms: 1000,
            hash: 0,
        },
    )]);
    let (actions3, _c3, _v3) = plan(&a0, &b, &base, opts);
    assert_eq!(
        actions3.len(),
        1,
        "no hash on one side ⇒ mtime gap is a diff"
    );
}

#[test]
fn walk_filter_size_age_bounds() {
    let gs = empty_globset();
    let mut f = WalkFilter::basic(true, &gs);
    f.min_size = 100;
    f.max_size = 1000;
    assert!(!f.size_age_ok(50, 0), "below min");
    assert!(f.size_age_ok(500, 0), "in range");
    assert!(!f.size_age_ok(2000, 0), "above max");
    let mut g = WalkFilter::basic(true, &gs);
    g.after_mtime_ms = 5_000;
    g.before_mtime_ms = 10_000;
    assert!(!g.size_age_ok(1, 4_000), "too old");
    assert!(g.size_age_ok(1, 7_000), "in window");
    assert!(!g.size_age_ok(1, 12_000), "too new");
}

#[test]
fn native_md5_matches_streamed_md5() {
    // A remote's native MD5 (e.g. Drive md5Checksum hex) must yield the SAME
    // Sig key as locally streaming the same bytes — so checksum compare works
    // without downloading the remote. MD5("abc") = 900150983cd24fb0d6963f7d28e17f72.
    let mut ctx = md5::Context::new();
    ctx.consume(b"abc");
    let streamed = md5_to_u64(&ctx.compute().0);
    let native = md5_hex_to_u64("900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(streamed, native);
    assert_ne!(streamed, 0);
    assert_eq!(md5_hex_to_u64("not-hex"), 0);
}
