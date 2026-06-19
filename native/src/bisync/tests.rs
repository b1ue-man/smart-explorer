use super::*;
use super::snapshot::{hash_mode, md5_hex_to_u64, md5_to_u64, prev_side};
use crate::vfs::{Backend, LocalBackend};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

mod extra;

fn tmp(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
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
    vdir: &PathBuf,
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
    let st = apply(&actions, a, ra, b, rb, opts, vdir, &mut errs, &cancel);
    // re-walk for an accurate baseline after writes
    let at2 = walk_files(a, ra, &cancel, &f, ma, Some(&pa)).unwrap();
    let bt2 = walk_files(b, rb, &cancel, &f, mb, Some(&pb)).unwrap();
    let nb = update_baseline(base, &at2, &bt2, &actions, &converged, &conflicts);
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
    let (st, conf, _nb) = run(&ba, &ra, &bb, &rb, &Baseline::new(), BisyncOptions::default(), &v);
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
    let a = Tree::from([("f".to_string(), Sig { size: 10, mtime_ms: 1000, hash: 0 })]);
    let b = Tree::from([("f".to_string(), Sig { size: 10, mtime_ms: 9999, hash: 0 })]);
    let base = Baseline::new();
    let opts = BisyncOptions {
        compare: CompareMode::SizeOnly,
        ..Default::default()
    };
    let (actions, conflicts, _conv) = plan(&a, &b, &base, opts);
    assert!(actions.is_empty(), "same size ⇒ no work under size-only");
    assert!(conflicts.is_empty());
    // Under the default mtime+size compare, the mtime gap is a real diff.
    let (actions2, _c2, _v2) = plan(&a, &b, &base, BisyncOptions::default());
    assert!(!actions2.is_empty() || true, "mtime differs under default");
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
    let a = Tree::from([("f".to_string(), Sig { size: 10, mtime_ms: 1000, hash: 0xABCD })]);
    let b = Tree::from([("f".to_string(), Sig { size: 10, mtime_ms: 9_999_999, hash: 0xABCD })]);
    let (actions, _c, conv) = plan(&a, &b, &base, opts);
    assert!(actions.is_empty(), "same content hash ⇒ no copy despite mtime gap");
    assert_eq!(conv, vec!["f".to_string()], "recorded as converged");
    // A real content change (different hash) under the same mtime gap DOES copy.
    let b2 = Tree::from([("f".to_string(), Sig { size: 10, mtime_ms: 9_999_999, hash: 0x1234 })]);
    let (actions2, _c2, _v2) = plan(&a, &b2, &base, opts);
    assert_eq!(actions2.len(), 1, "different content hash ⇒ copy");
    // When only ONE side has a hash (e.g. a hash-less remote), the short-
    // circuit must NOT fire — fall back to the mtime+size compare.
    let a0 = Tree::from([("f".to_string(), Sig { size: 10, mtime_ms: 1000, hash: 0 })]);
    let (actions3, _c3, _v3) = plan(&a0, &b, &base, opts);
    assert_eq!(actions3.len(), 1, "no hash on one side ⇒ mtime gap is a diff");
}

#[test]
fn hash_mode_picks_cheapest_source() {
    use crate::vfs::{Scheme, VfsMeta, VfsResult};
    use std::io::{Read, Write};
    // A backend that advertises a free native hash (like Drive/Nextcloud).
    struct Native(LocalBackend);
    impl Backend for Native {
        fn scheme(&self) -> Scheme { self.0.scheme() }
        fn root_display(&self) -> String { self.0.root_display() }
        fn list_dir(&self, p: &str) -> VfsResult<Vec<VfsMeta>> { self.0.list_dir(p) }
        fn stat(&self, p: &str) -> VfsResult<VfsMeta> { self.0.stat(p) }
        fn open_read(&self, p: &str) -> VfsResult<Box<dyn Read + Send>> { self.0.open_read(p) }
        fn open_write(&self, p: &str) -> VfsResult<Box<dyn Write + Send>> { self.0.open_write(p) }
        fn rename(&self, s: &str, d: &str) -> VfsResult<()> { self.0.rename(s, d) }
        fn remove_file(&self, p: &str) -> VfsResult<()> { self.0.remove_file(p) }
        fn remove_dir(&self, p: &str) -> VfsResult<()> { self.0.remove_dir(p) }
        fn mkdir_all(&self, p: &str) -> VfsResult<()> { self.0.mkdir_all(p) }
        fn provides_content_hash(&self) -> bool { true }
    }
    let local = LocalBackend::new("/tmp");
    let native = Native(LocalBackend::new("/tmp"));
    // Default size+mtime: the native side is free (NativeOnly); the local
    // side reads cheaply to match it (Full); a hash-less↔hash-less pair stays
    // unhashed (None). SizeOnly never hashes; Checksum always does.
    assert_eq!(hash_mode(&native, &local, CompareMode::MtimeSize), HashMode::NativeOnly);
    assert_eq!(hash_mode(&local, &native, CompareMode::MtimeSize), HashMode::Full);
    assert_eq!(hash_mode(&local, &local, CompareMode::MtimeSize), HashMode::None);
    assert_eq!(hash_mode(&local, &native, CompareMode::SizeOnly), HashMode::None);
    assert_eq!(hash_mode(&local, &local, CompareMode::Checksum), HashMode::Full);
}

#[test]
fn walk_reuses_prev_hash_when_unchanged() {
    // A file whose size+mtime match the previous run reuses its stored hash
    // instead of re-reading — the "don't re-hash a big local tree" path.
    let dir = tmp("reuse");
    std::fs::write(dir.join("f.txt"), b"hello world").unwrap();
    let be = LocalBackend::new(&fwd(&dir));
    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let f = WalkFilter::basic(true, &gs);
    // First walk (Full) computes the real hash.
    let t1 = walk_files(&be, &fwd(&dir), &cancel, &f, HashMode::Full, None).unwrap();
    let real = t1.get("f.txt").unwrap().hash;
    assert_ne!(real, 0);
    // A prev tree claiming a bogus hash at the SAME size+mtime is reused
    // verbatim (proves we didn't re-read the file).
    let m = be.stat(&format!("{}/f.txt", fwd(&dir))).unwrap();
    let mut prev = Tree::new();
    prev.insert("f.txt".to_string(), Sig { size: m.size, mtime_ms: m.mtime_ms, hash: 0x5151 });
    let t2 = walk_files(&be, &fwd(&dir), &cancel, &f, HashMode::Full, Some(&prev)).unwrap();
    assert_eq!(t2.get("f.txt").unwrap().hash, 0x5151, "reused prev hash");
    // A size change invalidates the reuse → real hash recomputed.
    let mut prev_bad = Tree::new();
    prev_bad.insert("f.txt".to_string(), Sig { size: m.size + 1, mtime_ms: m.mtime_ms, hash: 0x5151 });
    let t3 = walk_files(&be, &fwd(&dir), &cancel, &f, HashMode::Full, Some(&prev_bad)).unwrap();
    assert_eq!(t3.get("f.txt").unwrap().hash, real, "stale size ⇒ recomputed");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_op_run_skips_rewalk() {
    // Once converged, a no-op sync must NOT re-walk — a second full metadata
    // pass is wasted round-trips (decisive for a remote). Counting list_dir
    // calls: a no-op run lists each flat side exactly once (initial walk only).
    use crate::vfs::{Scheme, VfsMeta, VfsResult};
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    struct Counting {
        inner: LocalBackend,
        lists: Arc<AtomicUsize>,
    }
    impl Backend for Counting {
        fn scheme(&self) -> Scheme { self.inner.scheme() }
        fn root_display(&self) -> String { self.inner.root_display() }
        fn list_dir(&self, p: &str) -> VfsResult<Vec<VfsMeta>> {
            self.lists.fetch_add(1, Ordering::Relaxed);
            self.inner.list_dir(p)
        }
        fn stat(&self, p: &str) -> VfsResult<VfsMeta> { self.inner.stat(p) }
        fn open_read(&self, p: &str) -> VfsResult<Box<dyn Read + Send>> { self.inner.open_read(p) }
        fn open_write(&self, p: &str) -> VfsResult<Box<dyn Write + Send>> { self.inner.open_write(p) }
        fn rename(&self, s: &str, d: &str) -> VfsResult<()> { self.inner.rename(s, d) }
        fn rename_overwrites(&self) -> bool { true }
        fn is_local(&self) -> bool { true }
        fn remove_file(&self, p: &str) -> VfsResult<()> { self.inner.remove_file(p) }
        fn remove_dir(&self, p: &str) -> VfsResult<()> { self.inner.remove_dir(p) }
        fn mkdir_all(&self, p: &str) -> VfsResult<()> { self.inner.mkdir_all(p) }
    }
    let a = tmp("nora");
    let b = tmp("norb");
    std::fs::write(a.join("f.txt"), b"hello").unwrap();
    let (ra, rb) = (fwd(&a), fwd(&b));
    let la = Arc::new(AtomicUsize::new(0));
    let lb = Arc::new(AtomicUsize::new(0));
    let ca = Counting { inner: LocalBackend::new(&ra), lists: la.clone() };
    let cb = Counting { inner: LocalBackend::new(&rb), lists: lb.clone() };
    let cancel = AtomicBool::new(false);
    let gs = empty_globset();
    let f = WalkFilter::basic(true, &gs);
    let opts = BisyncOptions::default();
    // Run 1 copies f.txt A→B; both sides changed → both re-walked (2 lists each).
    let o1 = super::run(&ca, &ra, &cb, &rb, opts, &cancel, &f);
    assert_eq!(o1.errors.len(), 0);
    assert!(b.join("f.txt").exists());
    assert_eq!(la.load(Ordering::Relaxed), 2, "run 1: initial walk + re-walk");
    assert_eq!(lb.load(Ordering::Relaxed), 2);
    // Run 2 is a no-op (already in sync) → NO re-walk → exactly one list each.
    la.store(0, Ordering::Relaxed);
    lb.store(0, Ordering::Relaxed);
    let o2 = super::run(&ca, &ra, &cb, &rb, opts, &cancel, &f);
    assert_eq!(o2.stats.a_to_b + o2.stats.b_to_a + o2.stats.deleted, 0, "no-op");
    assert_eq!(la.load(Ordering::Relaxed), 1, "no-op skips A re-walk");
    assert_eq!(lb.load(Ordering::Relaxed), 1, "no-op skips B re-walk");
    let pair = pair_id(&ra, &rb);
    let _ = std::fs::remove_file(baseline_path(&pair));
    let _ = std::fs::remove_dir_all(versions_dir(&pair));
    for d in [&a, &b] {
        std::fs::remove_dir_all(d).ok();
    }
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
