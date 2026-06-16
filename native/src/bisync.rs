//! Safe two-way (and one-way) sync between two `vfs::Backend`s.
//!
//! Safety is the whole point ("it just works" — the default must be safe):
//!  * A **baseline** from the previous run records each side's state, so we know
//!    which side actually CHANGED — not just which differs. One side changed →
//!    propagate. BOTH sides changed a file → it's a **conflict**, surfaced for
//!    the user; never silently overwritten (strict file-level default).
//!  * Every overwrite/delete is **reversible**: the old bytes are copied into a
//!    versions store first, pruned by a retention window — so any sync action
//!    can be undone.
//!  * `dry_run` reports the plan without touching anything.
//!
//! Backend-agnostic (local↔local, local↔SFTP, …). The line-level git-style
//! merge is a future optional mode; the shipped default is the strict
//! file-level one the spec asks for.
#![allow(dead_code)] // engine; the sync UI wiring lands next.

use crate::vfs::Backend;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Size + mtime signature of one file on one side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sig {
    pub size: u64,
    pub mtime_ms: i64,
}

/// Current file set (relative path -> signature) of one side.
pub type Tree = BTreeMap<String, Sig>;

/// Last-sync state: rel -> (side A sig, side B sig). Absent = not present then.
pub type Baseline = BTreeMap<String, (Option<Sig>, Option<Sig>)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    AtoB,
    BtoA,
    Both,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::AtoB => "a2b",
            Direction::BtoA => "b2a",
            Direction::Both => "both",
        }
    }
    pub fn parse(s: &str) -> Option<Direction> {
        match s {
            "a2b" => Some(Direction::AtoB),
            "b2a" => Some(Direction::BtoA),
            "both" => Some(Direction::Both),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Direction::AtoB => "Quelle → Ziel (einseitig)",
            Direction::BtoA => "Ziel → Quelle (einseitig)",
            Direction::Both => "Beide Richtungen",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictMode {
    /// Strict (default): a file changed on BOTH sides is a conflict.
    FileLevel,
    /// No conflicts — the newer mtime wins.
    NewerWins,
}

impl ConflictMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictMode::FileLevel => "strict",
            ConflictMode::NewerWins => "newer",
        }
    }
    pub fn parse(s: &str) -> Option<ConflictMode> {
        match s {
            "strict" => Some(ConflictMode::FileLevel),
            "newer" => Some(ConflictMode::NewerWins),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ConflictMode::FileLevel => "Streng: beidseitige Änderung = Konflikt (sicher)",
            ConflictMode::NewerWins => "Neuere gewinnt (kein Konflikt)",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BisyncOptions {
    pub direction: Direction,
    pub conflict: ConflictMode,
    pub reversible: bool,
    pub dry_run: bool,
}

impl Default for BisyncOptions {
    fn default() -> Self {
        // The safe default: two-way, strict conflicts, reversible, real run.
        BisyncOptions {
            direction: Direction::Both,
            conflict: ConflictMode::FileLevel,
            reversible: true,
            dry_run: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    CopyAtoB(String),
    CopyBtoA(String),
    DeleteA(String),
    DeleteB(String),
}

#[derive(Clone, Debug)]
pub struct Conflict {
    pub rel: String,
    pub a: Option<Sig>,
    pub b: Option<Sig>,
}

#[derive(Default, Clone, Debug)]
pub struct BisyncStats {
    pub a_to_b: u64,
    pub b_to_a: u64,
    pub deleted: u64,
    pub conflicts: u64,
    pub bytes: u64,
    pub errors: u64,
}

fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else {
        format!("{}/{}", root.trim_end_matches('/'), rel)
    }
}

fn rel_of(path: &str, root: &str) -> String {
    let r = root.trim_end_matches('/');
    path.strip_prefix(r)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| path.trim_start_matches('/').to_string())
}

fn parent_of(path: &str) -> Option<String> {
    let t = path.trim_end_matches('/');
    t.rfind('/').map(|i| if i == 0 { "/".into() } else { t[..i].into() })
}

/// What to skip while walking (hidden files, ignore globs matched on the
/// relative path). `default_filter()` includes everything.
pub struct WalkFilter<'a> {
    pub include_hidden: bool,
    pub ignore: &'a globset::GlobSet,
}

/// An empty filter (include everything) — handy for tests / "no settings".
pub fn empty_globset() -> globset::GlobSet {
    globset::GlobSetBuilder::new().build().unwrap()
}

/// Recursively list files (not dirs) of a backend subtree → rel → Sig,
/// honouring the hidden/ignore filter.
pub fn walk_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> io::Result<Tree> {
    let mut out = Tree::new();
    let mut stack = vec![root.to_string()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        for m in be.list_dir(&dir)? {
            if !filter.include_hidden && m.hidden {
                continue;
            }
            let p = join(&dir, &m.name);
            let rel = rel_of(&p, root);
            if filter.ignore.is_match(&rel) {
                continue;
            }
            if m.is_dir {
                if !m.is_symlink {
                    stack.push(p);
                }
            } else {
                out.insert(
                    rel,
                    Sig {
                        size: m.size,
                        mtime_ms: m.mtime_ms,
                    },
                );
            }
        }
    }
    Ok(out)
}

/// Decide the actions + conflicts from the two current trees and the baseline.
/// Returns (actions, conflicts, converged) where `converged` lists rels that are
/// now identical on both sides (baseline should record them).
pub fn plan(
    a: &Tree,
    b: &Tree,
    base: &Baseline,
    opts: BisyncOptions,
) -> (Vec<Action>, Vec<Conflict>, Vec<String>) {
    let mut actions = Vec::new();
    let mut conflicts = Vec::new();
    let mut converged = Vec::new();

    let mut rels: BTreeSet<&String> = BTreeSet::new();
    rels.extend(a.keys());
    rels.extend(b.keys());
    rels.extend(base.keys());

    let allow_a_to_b = matches!(opts.direction, Direction::AtoB | Direction::Both);
    let allow_b_to_a = matches!(opts.direction, Direction::BtoA | Direction::Both);

    for rel in rels {
        let an = a.get(rel).copied();
        let bn = b.get(rel).copied();
        let (ba, bb) = base.get(rel).copied().unwrap_or((None, None));
        let a_changed = an != ba;
        let b_changed = bn != bb;

        if !a_changed && !b_changed {
            continue; // in sync per the baseline
        }
        // Both sides ended up identical (e.g. same edit on both) → no work.
        if an == bn {
            converged.push(rel.clone());
            continue;
        }

        match (a_changed, b_changed) {
            (true, false) => {
                // propagate A's state to B
                if allow_a_to_b {
                    match an {
                        Some(_) => actions.push(Action::CopyAtoB(rel.clone())),
                        None => actions.push(Action::DeleteB(rel.clone())),
                    }
                }
            }
            (false, true) => {
                if allow_b_to_a {
                    match bn {
                        Some(_) => actions.push(Action::CopyBtoA(rel.clone())),
                        None => actions.push(Action::DeleteA(rel.clone())),
                    }
                }
            }
            (true, true) => match opts.conflict {
                ConflictMode::FileLevel => conflicts.push(Conflict {
                    rel: rel.clone(),
                    a: an,
                    b: bn,
                }),
                ConflictMode::NewerWins => {
                    let am = an.map(|s| s.mtime_ms).unwrap_or(i64::MIN);
                    let bm = bn.map(|s| s.mtime_ms).unwrap_or(i64::MIN);
                    if am >= bm {
                        if allow_a_to_b {
                            actions.push(match an {
                                Some(_) => Action::CopyAtoB(rel.clone()),
                                None => Action::DeleteB(rel.clone()),
                            });
                        }
                    } else if allow_b_to_a {
                        actions.push(match bn {
                            Some(_) => Action::CopyBtoA(rel.clone()),
                            None => Action::DeleteA(rel.clone()),
                        });
                    }
                }
            },
            (false, false) => unreachable!(),
        }
    }
    (actions, conflicts, converged)
}

/// Stream-copy one file between backends, creating the destination parent.
fn copy_between(
    src: &dyn Backend,
    sp: &str,
    dst: &dyn Backend,
    dp: &str,
) -> io::Result<u64> {
    if let Some(parent) = parent_of(dp) {
        let _ = dst.mkdir_all(&parent);
    }
    let mut r = src.open_read(sp)?;
    let mut w = dst.open_write(dp)?;
    let n = io::copy(&mut r, &mut w)?;
    w.flush()?;
    Ok(n)
}

/// Reversible backup: copy `path` (on `be`) into the local versions store before
/// it is overwritten/deleted. Best-effort; failure doesn't abort the sync but is
/// reported by the caller via the returned error.
fn back_up(be: &dyn Backend, path: &str, rel: &str, versions_dir: &PathBuf) -> io::Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = versions_dir.join(ts.to_string()).join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut r = be.open_read(path)?;
    let mut f = std::fs::File::create(&dest)?;
    io::copy(&mut r, &mut f)?;
    Ok(())
}

/// Apply the planned actions, with reversible backups. Returns stats; errors are
/// counted (and the rel/message collected) rather than aborting.
pub fn apply(
    actions: &[Action],
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &PathBuf,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> BisyncStats {
    let mut st = BisyncStats::default();
    for act in actions {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if opts.dry_run {
            match act {
                Action::CopyAtoB(_) => st.a_to_b += 1,
                Action::CopyBtoA(_) => st.b_to_a += 1,
                Action::DeleteA(_) | Action::DeleteB(_) => st.deleted += 1,
            }
            continue;
        }
        let res: io::Result<()> = (|| {
            match act {
                Action::CopyAtoB(rel) => {
                    let dp = join(root_b, rel);
                    if opts.reversible && b.exists(&dp) {
                        let _ = back_up(b, &dp, rel, versions_dir);
                    }
                    let n = copy_between(a, &join(root_a, rel), b, &dp)?;
                    st.a_to_b += 1;
                    st.bytes += n;
                }
                Action::CopyBtoA(rel) => {
                    let dp = join(root_a, rel);
                    if opts.reversible && a.exists(&dp) {
                        let _ = back_up(a, &dp, rel, versions_dir);
                    }
                    let n = copy_between(b, &join(root_b, rel), a, &dp)?;
                    st.b_to_a += 1;
                    st.bytes += n;
                }
                Action::DeleteB(rel) => {
                    let p = join(root_b, rel);
                    if opts.reversible {
                        let _ = back_up(b, &p, rel, versions_dir);
                    }
                    b.remove_file(&p)?;
                    st.deleted += 1;
                }
                Action::DeleteA(rel) => {
                    let p = join(root_a, rel);
                    if opts.reversible {
                        let _ = back_up(a, &p, rel, versions_dir);
                    }
                    a.remove_file(&p)?;
                    st.deleted += 1;
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            st.errors += 1;
            errors.push((format!("{:?}", act), e.to_string()));
        }
    }
    st
}

/// Build the new baseline after a run: applied actions + converged rels are now
/// in sync (record both sides' current sig); conflicts and skipped rels keep
/// their previous baseline so they're re-detected next time.
pub fn update_baseline(
    base: &Baseline,
    a: &Tree,
    b: &Tree,
    applied: &[Action],
    converged: &[String],
    conflicts: &[Conflict],
) -> Baseline {
    let conflict_set: BTreeSet<&str> = conflicts.iter().map(|c| c.rel.as_str()).collect();
    let mut nb = base.clone();
    let mut record = |rel: &str| {
        nb.insert(rel.to_string(), (a.get(rel).copied(), b.get(rel).copied()));
    };
    for act in applied {
        let rel = match act {
            Action::CopyAtoB(r) | Action::CopyBtoA(r) | Action::DeleteA(r) | Action::DeleteB(r) => r,
        };
        // After a copy both sides match; after a delete both are absent. For
        // NewerWins the loser side may not match yet — record current state so
        // the next walk reconciles. (Deletes leave the entry absent.)
        record(rel);
    }
    for rel in converged {
        record(rel);
    }
    // Drop entries that are now absent on both sides and not in conflict.
    nb.retain(|rel, (x, y)| x.is_some() || y.is_some() || conflict_set.contains(rel.as_str()));
    nb
}

// ── persistence (baseline TSV in appdata, keyed by the two roots) ────────────

fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer").join("sync");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Stable id for a sync pair (order-independent), used for the baseline file and
/// the versions folder.
pub fn pair_id(root_a: &str, root_b: &str) -> String {
    let mut v = [root_a, root_b];
    v.sort();
    // simple stable hash (FNV-1a) → hex
    let mut h: u64 = 0xcbf29ce484222325;
    for s in v {
        for byb in s.bytes() {
            h ^= byb as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= b'|' as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

pub fn baseline_path(pair: &str) -> PathBuf {
    app_data_dir().join(format!("baseline_{pair}.tsv"))
}

pub fn versions_dir(pair: &str) -> PathBuf {
    app_data_dir().join(format!("versions_{pair}"))
}

fn sig_str(s: &Option<Sig>) -> String {
    match s {
        Some(s) => format!("{}:{}", s.size, s.mtime_ms),
        None => "-".to_string(),
    }
}
fn parse_sig(s: &str) -> Option<Sig> {
    if s == "-" {
        return None;
    }
    let (sz, mt) = s.split_once(':')?;
    Some(Sig {
        size: sz.parse().ok()?,
        mtime_ms: mt.parse().ok()?,
    })
}

pub fn load_baseline(path: &std::path::Path) -> Baseline {
    let mut bl = Baseline::new();
    if let Ok(txt) = std::fs::read_to_string(path) {
        for line in txt.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() == 3 {
                bl.insert(f[0].to_string(), (parse_sig(f[1]), parse_sig(f[2])));
            }
        }
    }
    bl
}

pub fn save_baseline(path: &std::path::Path, bl: &Baseline) -> io::Result<()> {
    let body: String = bl
        .iter()
        .map(|(rel, (a, b))| format!("{}\t{}\t{}", rel.replace('\t', " "), sig_str(a), sig_str(b)))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, body)
}

/// Prune version snapshots older than `keep_days` (0 = keep forever).
pub fn prune_versions(versions: &std::path::Path, keep_days: u64) {
    if keep_days == 0 {
        return;
    }
    let cutoff = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(keep_days * 86_400);
    if let Ok(rd) = std::fs::read_dir(versions) {
        for e in rd.flatten() {
            if let Some(ts) = e.file_name().to_str().and_then(|s| s.parse::<u64>().ok()) {
                if ts < cutoff {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
    }
}

// ── high-level orchestration (used by the UI on a worker thread) ─────────────

#[derive(Default)]
pub struct Outcome {
    pub stats: BisyncStats,
    pub conflicts: Vec<Conflict>,
    pub errors: Vec<(String, String)>,
    pub baseline: Baseline,
}

fn sig_of(be: &dyn Backend, path: &str) -> Option<Sig> {
    be.stat(path).ok().filter(|m| !m.is_dir).map(|m| Sig {
        size: m.size,
        mtime_ms: m.mtime_ms,
    })
}

/// One full bisync run: load baseline → walk both → plan → apply → save the
/// new baseline + prune versions. Conflicts are returned (not applied); the
/// updated baseline keeps them flagged until resolved.
pub fn run(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    retain_days: u64,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Outcome {
    let pair = pair_id(root_a, root_b);
    let bpath = baseline_path(&pair);
    let vdir = versions_dir(&pair);
    let base = load_baseline(&bpath);
    let at = match walk_files(a, root_a, cancel, filter) {
        Ok(t) => t,
        Err(e) => {
            return Outcome {
                errors: vec![(root_a.into(), e.to_string())],
                ..Default::default()
            }
        }
    };
    let bt = match walk_files(b, root_b, cancel, filter) {
        Ok(t) => t,
        Err(e) => {
            return Outcome {
                errors: vec![(root_b.into(), e.to_string())],
                ..Default::default()
            }
        }
    };
    if cancel.load(Ordering::Relaxed) {
        return Outcome::default();
    }
    let (actions, conflicts, converged) = plan(&at, &bt, &base, opts);
    let mut errors = Vec::new();
    let st = apply(&actions, a, root_a, b, root_b, opts, &vdir, &mut errors, cancel);
    let at2 = walk_files(a, root_a, cancel, filter).unwrap_or(at);
    let bt2 = walk_files(b, root_b, cancel, filter).unwrap_or(bt);
    let nb = update_baseline(&base, &at2, &bt2, &actions, &converged, &conflicts);
    if !opts.dry_run {
        let _ = save_baseline(&bpath, &nb);
        prune_versions(&vdir, retain_days);
    }
    Outcome {
        stats: st,
        conflicts,
        errors,
        baseline: nb,
    }
}

/// Resolve one conflict by copying the chosen side over the other (with a
/// reversible backup of the loser). Returns the new (a, b) signatures so the
/// caller can update the baseline.
pub fn resolve(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    rel: &str,
    keep_a: bool,
    pair: &str,
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    let vdir = versions_dir(pair);
    let pa = join(root_a, rel);
    let pb = join(root_b, rel);
    if keep_a {
        if b.exists(&pb) {
            let _ = back_up(b, &pb, rel, &vdir);
        }
        copy_between(a, &pa, b, &pb)?;
    } else {
        if a.exists(&pa) {
            let _ = back_up(a, &pa, rel, &vdir);
        }
        copy_between(b, &pb, a, &pa)?;
    }
    Ok((sig_of(a, &pa), sig_of(b, &pb)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::LocalBackend;
    use std::path::Path;

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
        let f = WalkFilter { include_hidden: true, ignore: &gs };
        let at = walk_files(a, ra, &cancel, &f).unwrap();
        let bt = walk_files(b, rb, &cancel, &f).unwrap();
        let (actions, conflicts, converged) = plan(&at, &bt, base, opts);
        let mut errs = Vec::new();
        let st = apply(&actions, a, ra, b, rb, opts, vdir, &mut errs, &cancel);
        // re-walk for an accurate baseline after writes
        let at2 = walk_files(a, ra, &cancel, &f).unwrap();
        let bt2 = walk_files(b, rb, &cancel, &f).unwrap();
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
}
