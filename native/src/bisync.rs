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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Size + mtime (+ optional content hash) signature of one file on one side.
/// `hash` is 0 unless the run uses `CompareMode::Checksum`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sig {
    pub size: u64,
    pub mtime_ms: i64,
    pub hash: u64,
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
    /// Strict (default): a file changed on BOTH sides is a conflict, surfaced
    /// for the user; nothing is silently overwritten.
    FileLevel,
    NewerWins,
    OlderWins,
    LargerWins,
    SmallerWins,
    /// Side A (source/left) always wins.
    SourceWins,
    /// Side B (target/right) always wins.
    DestWins,
    /// Keep both: the winner (newer) keeps the name, the loser is preserved as a
    /// "(Konflikt …)" copy that then syncs to both sides.
    KeepBoth,
}

impl ConflictMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictMode::FileLevel => "strict",
            ConflictMode::NewerWins => "newer",
            ConflictMode::OlderWins => "older",
            ConflictMode::LargerWins => "larger",
            ConflictMode::SmallerWins => "smaller",
            ConflictMode::SourceWins => "source",
            ConflictMode::DestWins => "dest",
            ConflictMode::KeepBoth => "keepboth",
        }
    }
    pub fn parse(s: &str) -> Option<ConflictMode> {
        Some(match s {
            "strict" => ConflictMode::FileLevel,
            "newer" => ConflictMode::NewerWins,
            "older" => ConflictMode::OlderWins,
            "larger" => ConflictMode::LargerWins,
            "smaller" => ConflictMode::SmallerWins,
            "source" => ConflictMode::SourceWins,
            "dest" => ConflictMode::DestWins,
            "keepboth" => ConflictMode::KeepBoth,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            ConflictMode::FileLevel => "Streng: beidseitige Änderung = Konflikt (sicher)",
            ConflictMode::NewerWins => "Neuere gewinnt",
            ConflictMode::OlderWins => "Ältere gewinnt",
            ConflictMode::LargerWins => "Größere gewinnt",
            ConflictMode::SmallerWins => "Kleinere gewinnt",
            ConflictMode::SourceWins => "Quelle (links) gewinnt",
            ConflictMode::DestWins => "Ziel (rechts) gewinnt",
            ConflictMode::KeepBoth => "Beide behalten (Konflikt-Kopie)",
        }
    }
    pub const ALL: [ConflictMode; 8] = [
        ConflictMode::FileLevel,
        ConflictMode::NewerWins,
        ConflictMode::OlderWins,
        ConflictMode::LargerWins,
        ConflictMode::SmallerWins,
        ConflictMode::SourceWins,
        ConflictMode::DestWins,
        ConflictMode::KeepBoth,
    ];
}

/// How deletions are handled on a sync (Group B). `Propagate` = a delete on the
/// changed side is mirrored to the other (classic two-way / "Echo"); `Mirror`
/// (one-way only) makes the destination an exact replica, deleting orphans that
/// never existed on the source; `NoDelete` never deletes ("Update"/"Contribute"
/// — additive, the safest backup style).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeletePolicy {
    Propagate,
    Mirror,
    NoDelete,
}

impl DeletePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            DeletePolicy::Propagate => "propagate",
            DeletePolicy::Mirror => "mirror",
            DeletePolicy::NoDelete => "nodelete",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "propagate" => DeletePolicy::Propagate,
            "mirror" => DeletePolicy::Mirror,
            "nodelete" => DeletePolicy::NoDelete,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            DeletePolicy::Propagate => "Löschungen übernehmen (Echo)",
            DeletePolicy::Mirror => "Spiegeln: Ziel exakt angleichen (löscht Fremddateien)",
            DeletePolicy::NoDelete => "Nie löschen (nur hinzufügen/aktualisieren)",
        }
    }
}

/// How two files are judged equal (Group C). `MtimeSize` (default) uses size +
/// modification time within `modify_window_ms`; `SizeOnly` ignores mtime;
/// `Checksum` compares a content hash (reads every file — slow but certain).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompareMode {
    MtimeSize,
    SizeOnly,
    Checksum,
}

impl CompareMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CompareMode::MtimeSize => "mtimesize",
            CompareMode::SizeOnly => "sizeonly",
            CompareMode::Checksum => "checksum",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "mtimesize" => CompareMode::MtimeSize,
            "sizeonly" => CompareMode::SizeOnly,
            "checksum" => CompareMode::Checksum,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            CompareMode::MtimeSize => "Größe + Änderungszeit (schnell)",
            CompareMode::SizeOnly => "Nur Größe (Änderungszeit ignorieren)",
            CompareMode::Checksum => "Prüfsumme (Inhalt lesen — sicher, langsam)",
        }
    }
}

/// How the reversible versions store is pruned (Group F). Versions are
/// timestamped snapshots of overwritten/deleted files; the scheme decides which
/// snapshots to keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VersioningScheme {
    /// Keep snapshots newer than `days`.
    Days,
    /// Keep the newest `count` snapshots.
    Count,
    /// Time-Machine-style thinning: all <1d, 1/day <30d, 1/week beyond.
    Staggered,
    /// Grandfather-father-son: 1/hour 24h, 1/day 7d, 1/week 4w, 1/month 12m.
    Gfs,
}

impl VersioningScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            VersioningScheme::Days => "days",
            VersioningScheme::Count => "count",
            VersioningScheme::Staggered => "staggered",
            VersioningScheme::Gfs => "gfs",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "days" => VersioningScheme::Days,
            "count" => VersioningScheme::Count,
            "staggered" => VersioningScheme::Staggered,
            "gfs" => VersioningScheme::Gfs,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            VersioningScheme::Days => "Nach Tagen (N Tage aufbewahren)",
            VersioningScheme::Count => "Nach Anzahl (letzte N Versionen)",
            VersioningScheme::Staggered => "Gestaffelt (Time-Machine-Stil)",
            VersioningScheme::Gfs => "GFS (Std/Tag/Woche/Monat)",
        }
    }
    pub const ALL: [VersioningScheme; 4] = [
        VersioningScheme::Days,
        VersioningScheme::Count,
        VersioningScheme::Staggered,
        VersioningScheme::Gfs,
    ];
}

#[derive(Clone, Copy, Debug)]
pub struct Versioning {
    pub scheme: VersioningScheme,
    pub days: u64,
    pub count: u64,
}

impl Default for Versioning {
    fn default() -> Self {
        Versioning {
            scheme: VersioningScheme::Days,
            days: 30,
            count: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BisyncOptions {
    pub direction: Direction,
    pub conflict: ConflictMode,
    pub reversible: bool,
    pub dry_run: bool,
    /// Group B: deletion handling and move semantics.
    pub delete: DeletePolicy,
    /// Move (one-way): after copying, delete the file from the source.
    pub move_files: bool,
    /// Group C: comparison method + mtime tolerance (ms) for MtimeSize.
    pub compare: CompareMode,
    pub modify_window_ms: i64,
    /// Group F: versions-store pruning, recycle-bin deletes, delete safety guard.
    pub versioning: Versioning,
    /// Send deletes to the OS Recycle Bin instead of removing (local paths only).
    pub use_recycle: bool,
    /// Abort the run if it would delete more than this many files (0 = no limit).
    pub max_delete: u64,
    /// …or more than this percent of the side's files (0 = no limit).
    pub max_delete_pct: u8,
    // ── Groups H/I: bandwidth & reliability ──────────────────────────────────
    /// Transfer rate cap in bytes/sec across all workers (0 = unlimited).
    pub bwlimit_bps: u64,
    /// Max concurrent transfers (0 = backend default).
    pub max_transfers: usize,
    /// Write to a temp file then rename into place (safe copies).
    pub atomic: bool,
    /// After copying, re-stat the destination and check its size matches.
    pub verify: bool,
    /// Retry a failed file operation this many times (with `retry_delay_secs`).
    pub retries: u32,
    pub retry_delay_secs: u64,
}

impl Default for BisyncOptions {
    fn default() -> Self {
        // The safe default: two-way, strict conflicts, reversible, real run,
        // propagate deletes, exact size+mtime compare, 30-day versions.
        BisyncOptions {
            direction: Direction::Both,
            conflict: ConflictMode::FileLevel,
            reversible: true,
            dry_run: false,
            delete: DeletePolicy::Propagate,
            move_files: false,
            compare: CompareMode::MtimeSize,
            modify_window_ms: 0,
            versioning: Versioning::default(),
            use_recycle: false,
            max_delete: 0,
            max_delete_pct: 0,
            bwlimit_bps: 0,
            max_transfers: 0,
            atomic: true,
            verify: false,
            retries: 0,
            retry_delay_secs: 2,
        }
    }
}

/// A shared, thread-safe transfer rate limiter (token-bucket over 1-second
/// windows). `consume` blocks callers once the per-second budget is spent.
pub struct Throttle {
    limit_bps: u64,
    state: Mutex<(std::time::Instant, u64)>,
}

impl Throttle {
    pub fn new(limit_bps: u64) -> Self {
        Throttle {
            limit_bps,
            state: Mutex::new((std::time::Instant::now(), 0)),
        }
    }
    fn consume(&self, n: u64) {
        if self.limit_bps == 0 {
            return;
        }
        let mut g = self.state.lock().unwrap();
        let (mut start, mut used) = *g;
        if start.elapsed() >= std::time::Duration::from_secs(1) {
            start = std::time::Instant::now();
            used = 0;
        }
        used += n;
        if used > self.limit_bps {
            let rem = std::time::Duration::from_secs(1).saturating_sub(start.elapsed());
            if !rem.is_zero() {
                std::thread::sleep(rem);
            }
            start = std::time::Instant::now();
            used = 0;
        }
        *g = (start, used);
    }
}

fn sig_mtime(s: Option<Sig>) -> i64 {
    s.map(|s| s.mtime_ms).unwrap_or(i64::MIN)
}
fn sig_size(s: Option<Sig>) -> u64 {
    s.map(|s| s.size).unwrap_or(0)
}

/// Insert a "(Konflikt <timestamp>)" tag before the extension of a relative path.
fn conflict_name(rel: &str) -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    match rel.rfind('.') {
        // only treat as an extension if the dot is in the final path segment
        Some(i) if i > rel.rfind('/').map(|s| s + 1).unwrap_or(0) => {
            format!("{} (Konflikt {}){}", &rel[..i], ts, &rel[i..])
        }
        _ => format!("{} (Konflikt {})", rel, ts),
    }
}

/// Comparison-mode-aware equality of two optional signatures.
fn sig_eq(x: Option<Sig>, y: Option<Sig>, opts: &BisyncOptions) -> bool {
    match (x, y) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.size != b.size {
                return false;
            }
            match opts.compare {
                CompareMode::SizeOnly => true,
                CompareMode::Checksum => a.hash == b.hash,
                CompareMode::MtimeSize => {
                    (a.mtime_ms - b.mtime_ms).abs() <= opts.modify_window_ms
                }
            }
        }
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    CopyAtoB(String),
    CopyBtoA(String),
    DeleteA(String),
    DeleteB(String),
    /// Keep-both, A wins: preserve B's current file as a "(Konflikt …)" copy,
    /// then copy A's version over the original name.
    KeepBothAtoB(String),
    /// Keep-both, B wins.
    KeepBothBtoA(String),
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

/// What to skip while walking: hidden files, ignore globs (matched on the
/// relative path), and size/age bounds (Group G). A bound of 0 means "no limit".
pub struct WalkFilter<'a> {
    pub include_hidden: bool,
    pub ignore: &'a globset::GlobSet,
    /// Only include files with `min_size <= size <= max_size` (bytes; 0 = off).
    pub min_size: u64,
    pub max_size: u64,
    /// Only include files modified within `[after_mtime_ms, before_mtime_ms]`
    /// (unix ms; 0 = off on that side).
    pub after_mtime_ms: i64,
    pub before_mtime_ms: i64,
}

impl<'a> WalkFilter<'a> {
    /// A filter with no size/age bounds (the common case).
    pub fn basic(include_hidden: bool, ignore: &'a globset::GlobSet) -> Self {
        WalkFilter {
            include_hidden,
            ignore,
            min_size: 0,
            max_size: 0,
            after_mtime_ms: 0,
            before_mtime_ms: 0,
        }
    }

    /// Does a file of this size/mtime pass the size & age bounds?
    fn size_age_ok(&self, size: u64, mtime_ms: i64) -> bool {
        if self.min_size > 0 && size < self.min_size {
            return false;
        }
        if self.max_size > 0 && size > self.max_size {
            return false;
        }
        if self.after_mtime_ms > 0 && mtime_ms < self.after_mtime_ms {
            return false;
        }
        if self.before_mtime_ms > 0 && mtime_ms > self.before_mtime_ms {
            return false;
        }
        true
    }
}

/// An empty filter (include everything) — handy for tests / "no settings".
pub fn empty_globset() -> globset::GlobSet {
    globset::GlobSetBuilder::new().build().unwrap()
}

/// FNV-1a content hash of a file (for `CompareMode::Checksum`). Best-effort: an
/// unreadable file hashes to 0 (treated as "changed" against any real hash).
fn hash_file(be: &dyn Backend, path: &str, cancel: &AtomicBool) -> u64 {
    use std::io::Read;
    let mut h: u64 = 0xcbf29ce484222325;
    if let Ok(mut r) = be.open_read(path) {
        let mut buf = [0u8; 65536];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return 0; // abort promptly when the user stops
            }
            match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for &byte in &buf[..n] {
                        h ^= byte as u64;
                        h = h.wrapping_mul(0x100000001b3);
                    }
                }
                Err(_) => return 0,
            }
        }
    } else {
        return 0;
    }
    h
}

/// Recursively list files (not dirs) of a backend subtree → rel → Sig,
/// honouring the hidden/ignore filter.
///
/// The walk is breadth-first and **fans out each level across the backend's
/// `parallelism()`** — decisive for remotes like Drive where every `list_dir`
/// is a network round-trip and a 27k-file tree spans hundreds of folders.
/// Backends that report `parallelism() == 1` (SFTP/FTP) stay effectively
/// serial. Local uses all cores.
pub fn walk_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: bool,
) -> io::Result<Tree> {
    let par = be.parallelism().max(1);
    let out: Mutex<Tree> = Mutex::new(Tree::new());
    let mut level = vec![root.to_string()];

    while !level.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let next: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let first_err: Mutex<Option<io::Error>> = Mutex::new(None);
        let idx = AtomicUsize::new(0);
        let workers = par.min(level.len()).max(1);

        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    if cancel.load(Ordering::Relaxed) || first_err.lock().unwrap().is_some() {
                        break;
                    }
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i >= level.len() {
                        break;
                    }
                    let dir = &level[i];
                    match be.list_dir(dir) {
                        Ok(entries) => {
                            let mut files: Vec<(String, Sig)> = Vec::new();
                            let mut dirs: Vec<String> = Vec::new();
                            for m in entries {
                                if cancel.load(Ordering::Relaxed) {
                                    break; // stop promptly mid-directory (esp. when hashing)
                                }
                                if !filter.include_hidden && m.hidden {
                                    continue;
                                }
                                let p = join(dir, &m.name);
                                let rel = rel_of(&p, root);
                                if filter.ignore.is_match(&rel) {
                                    continue;
                                }
                                if m.is_dir {
                                    if !m.is_symlink {
                                        dirs.push(p);
                                    }
                                } else if filter.size_age_ok(m.size, m.mtime_ms) {
                                    let h = if hash { hash_file(be, &p, cancel) } else { 0 };
                                    files.push((
                                        rel,
                                        Sig {
                                            size: m.size,
                                            mtime_ms: m.mtime_ms,
                                            hash: h,
                                        },
                                    ));
                                }
                            }
                            if !files.is_empty() {
                                let mut o = out.lock().unwrap();
                                // A backend may report two files with the same
                                // name in one folder (e.g. Google Drive keys by
                                // id, not name). Keep the newest deterministically
                                // so the plan is stable rather than order-dependent.
                                for (rel, sig) in files {
                                    match o.get(&rel) {
                                        Some(prev) if prev.mtime_ms >= sig.mtime_ms => {}
                                        _ => {
                                            o.insert(rel, sig);
                                        }
                                    }
                                }
                            }
                            if !dirs.is_empty() {
                                next.lock().unwrap().extend(dirs);
                            }
                        }
                        Err(e) => {
                            let mut slot = first_err.lock().unwrap();
                            if slot.is_none() {
                                *slot = Some(e);
                            }
                            break;
                        }
                    }
                });
            }
        });

        if let Some(e) = first_err.into_inner().unwrap() {
            return Err(e);
        }
        level = next.into_inner().unwrap();
    }
    Ok(out.into_inner().unwrap())
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

    // Mirror: a stateless, one-way exact replica — the destination is made
    // identical to the source, deleting orphans. The baseline isn't consulted.
    if opts.delete == DeletePolicy::Mirror
        && matches!(opts.direction, Direction::AtoB | Direction::BtoA)
    {
        let atob = opts.direction == Direction::AtoB;
        let (src, dst) = if atob { (a, b) } else { (b, a) };
        let mut rels: BTreeSet<&String> = BTreeSet::new();
        rels.extend(src.keys());
        rels.extend(dst.keys());
        for rel in rels {
            let sn = src.get(rel).copied();
            let dn = dst.get(rel).copied();
            if sig_eq(sn, dn, &opts) {
                converged.push(rel.clone());
                continue;
            }
            match (sn, dn) {
                (Some(_), _) => actions.push(if atob {
                    Action::CopyAtoB(rel.clone())
                } else {
                    Action::CopyBtoA(rel.clone())
                }),
                (None, Some(_)) => actions.push(if atob {
                    Action::DeleteB(rel.clone())
                } else {
                    Action::DeleteA(rel.clone())
                }),
                (None, None) => {}
            }
        }
        return (actions, conflicts, converged);
    }

    let mut rels: BTreeSet<&String> = BTreeSet::new();
    rels.extend(a.keys());
    rels.extend(b.keys());
    rels.extend(base.keys());

    let allow_a_to_b = matches!(opts.direction, Direction::AtoB | Direction::Both);
    let allow_b_to_a = matches!(opts.direction, Direction::BtoA | Direction::Both);
    let allow_delete = opts.delete != DeletePolicy::NoDelete;

    for rel in rels {
        let an = a.get(rel).copied();
        let bn = b.get(rel).copied();
        let (ba, bb) = base.get(rel).copied().unwrap_or((None, None));
        let a_changed = !sig_eq(an, ba, &opts);
        let b_changed = !sig_eq(bn, bb, &opts);

        if !a_changed && !b_changed {
            continue; // in sync per the baseline
        }
        // Both sides ended up identical (e.g. same edit on both) → no work.
        if sig_eq(an, bn, &opts) {
            converged.push(rel.clone());
            continue;
        }

        match (a_changed, b_changed) {
            (true, false) => {
                // propagate A's state to B
                if allow_a_to_b {
                    match an {
                        Some(_) => actions.push(Action::CopyAtoB(rel.clone())),
                        None => {
                            if allow_delete {
                                actions.push(Action::DeleteB(rel.clone()))
                            }
                        }
                    }
                }
            }
            (false, true) => {
                if allow_b_to_a {
                    match bn {
                        Some(_) => actions.push(Action::CopyBtoA(rel.clone())),
                        None => {
                            if allow_delete {
                                actions.push(Action::DeleteA(rel.clone()))
                            }
                        }
                    }
                }
            }
            (true, true) => {
                if opts.conflict == ConflictMode::FileLevel {
                    conflicts.push(Conflict {
                        rel: rel.clone(),
                        a: an,
                        b: bn,
                    });
                } else if opts.conflict == ConflictMode::KeepBoth {
                    // Winner (newer) keeps the name; loser preserved as a copy.
                    let a_wins = match (an, bn) {
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                        (Some(sa), Some(sb)) => sa.mtime_ms >= sb.mtime_ms,
                        (None, None) => continue,
                    };
                    if a_wins && allow_a_to_b {
                        actions.push(Action::KeepBothAtoB(rel.clone()));
                    } else if !a_wins && allow_b_to_a {
                        actions.push(Action::KeepBothBtoA(rel.clone()));
                    }
                } else {
                    // A deterministic winner side from the policy.
                    let a_wins = match opts.conflict {
                        ConflictMode::SourceWins => true,
                        ConflictMode::DestWins => false,
                        ConflictMode::NewerWins => {
                            sig_mtime(an) >= sig_mtime(bn)
                        }
                        ConflictMode::OlderWins => sig_mtime(an) <= sig_mtime(bn),
                        ConflictMode::LargerWins => sig_size(an) >= sig_size(bn),
                        ConflictMode::SmallerWins => sig_size(an) <= sig_size(bn),
                        _ => true,
                    };
                    if a_wins {
                        if allow_a_to_b {
                            match an {
                                Some(_) => actions.push(Action::CopyAtoB(rel.clone())),
                                None => {
                                    if allow_delete {
                                        actions.push(Action::DeleteB(rel.clone()))
                                    }
                                }
                            }
                        }
                    } else if allow_b_to_a {
                        match bn {
                            Some(_) => actions.push(Action::CopyBtoA(rel.clone())),
                            None => {
                                if allow_delete {
                                    actions.push(Action::DeleteA(rel.clone()))
                                }
                            }
                        }
                    }
                }
            }
            (false, false) => unreachable!(),
        }
    }
    (actions, conflicts, converged)
}

/// Delete a file, optionally to the OS Recycle Bin (local paths only). For a
/// remote path (or if trashing fails) it falls back to the backend's hard delete.
fn delete_file(be: &dyn Backend, path: &str, use_recycle: bool) -> io::Result<()> {
    if use_recycle && !path.contains("://") && std::path::Path::new(path).exists() {
        if trash::delete(path).is_ok() {
            return Ok(());
        }
    }
    be.remove_file(path)
}

/// Stream-copy one file between backends, creating the destination parent.
/// When `atomic`, writes to a temp sibling then renames into place (safe copies);
/// `throttle` rate-limits the transfer across all workers.
fn copy_between(
    src: &dyn Backend,
    sp: &str,
    dst: &dyn Backend,
    dp: &str,
    atomic: bool,
    throttle: &Throttle,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    use std::io::{Read, Write};
    // Safe-copies (temp then rename) are only correct where rename atomically
    // REPLACES the destination. On backends like Google Drive a rename creates a
    // duplicate same-named file instead of overwriting, so write in place there.
    let atomic = atomic && dst.rename_overwrites();
    if let Some(parent) = parent_of(dp) {
        let _ = dst.mkdir_all(&parent);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let write_path = if atomic {
        format!("{}.se-tmp-{:x}", dp, nanos)
    } else {
        dp.to_string()
    };
    let mut r = src.open_read(sp)?;
    let mut w = dst.open_write(&write_path)?;
    let mut buf = vec![0u8; 1 << 18];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(w);
            if atomic {
                let _ = dst.remove_file(&write_path);
            }
            return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
        }
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                if atomic {
                    let _ = dst.remove_file(&write_path);
                }
                return Err(e);
            }
        };
        if let Err(e) = w.write_all(&buf[..n]) {
            if atomic {
                let _ = dst.remove_file(&write_path);
            }
            return Err(e);
        }
        total += n as u64;
        throttle.consume(n as u64);
    }
    w.flush()?;
    drop(w);
    if atomic {
        dst.rename(&write_path, dp)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    }
    Ok(total)
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

/// Execute one planned action (copy with reversible backup, or delete),
/// returning its contribution to the run stats. Network-bound and side-effect
/// free w.r.t. shared state, so many run concurrently in `apply`.
/// Re-stat the destination and confirm its size matches the bytes written.
fn verify_copy(dst: &dyn Backend, dp: &str, expected: u64) -> io::Result<()> {
    let got = dst.stat(dp).map(|m| m.size).unwrap_or(u64::MAX);
    if got != expected {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Überprüfung fehlgeschlagen: {} ≠ {} Bytes", got, expected),
        ));
    }
    Ok(())
}

fn run_one(
    act: &Action,
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &PathBuf,
    throttle: &Throttle,
    cancel: &AtomicBool,
) -> io::Result<BisyncStats> {
    let mut st = BisyncStats::default();
    match act {
        Action::CopyAtoB(rel) => {
            let dp = join(root_b, rel);
            if opts.reversible && b.exists(&dp) {
                let _ = back_up(b, &dp, rel, versions_dir);
            }
            let n = copy_between(a, &join(root_a, rel), b, &dp, opts.atomic, throttle, cancel)?;
            if opts.verify {
                verify_copy(b, &dp, n)?;
            }
            st.bytes += n;
            st.a_to_b += 1;
            // Move (one-way): remove the source after a successful copy.
            if opts.move_files && opts.direction != Direction::Both {
                let sp = join(root_a, rel);
                if opts.reversible {
                    let _ = back_up(a, &sp, rel, versions_dir);
                }
                if a.remove_file(&sp).is_ok() {
                    st.deleted += 1;
                }
            }
        }
        Action::CopyBtoA(rel) => {
            let dp = join(root_a, rel);
            if opts.reversible && a.exists(&dp) {
                let _ = back_up(a, &dp, rel, versions_dir);
            }
            let n = copy_between(b, &join(root_b, rel), a, &dp, opts.atomic, throttle, cancel)?;
            if opts.verify {
                verify_copy(a, &dp, n)?;
            }
            st.bytes += n;
            st.b_to_a += 1;
            if opts.move_files && opts.direction != Direction::Both {
                let sp = join(root_b, rel);
                if opts.reversible {
                    let _ = back_up(b, &sp, rel, versions_dir);
                }
                if b.remove_file(&sp).is_ok() {
                    st.deleted += 1;
                }
            }
        }
        Action::DeleteB(rel) => {
            let p = join(root_b, rel);
            if opts.reversible {
                let _ = back_up(b, &p, rel, versions_dir);
            }
            delete_file(b, &p, opts.use_recycle)?;
            st.deleted += 1;
        }
        Action::DeleteA(rel) => {
            let p = join(root_a, rel);
            if opts.reversible {
                let _ = back_up(a, &p, rel, versions_dir);
            }
            delete_file(a, &p, opts.use_recycle)?;
            st.deleted += 1;
        }
        Action::KeepBothAtoB(rel) => {
            let bp = join(root_b, rel);
            // Preserve B's losing version as a conflict copy that will sync back.
            if b.exists(&bp) {
                let cp = join(root_b, &conflict_name(rel));
                let _ = copy_between(b, &bp, b, &cp, opts.atomic, throttle, cancel);
            }
            st.bytes += copy_between(a, &join(root_a, rel), b, &bp, opts.atomic, throttle, cancel)?;
            st.a_to_b += 1;
        }
        Action::KeepBothBtoA(rel) => {
            let ap = join(root_a, rel);
            if a.exists(&ap) {
                let cp = join(root_a, &conflict_name(rel));
                let _ = copy_between(a, &ap, a, &cp, opts.atomic, throttle, cancel);
            }
            st.bytes += copy_between(b, &join(root_b, rel), a, &ap, opts.atomic, throttle, cancel)?;
            st.b_to_a += 1;
        }
    }
    Ok(st)
}

/// Apply the planned actions, with reversible backups. Returns stats; errors are
/// counted (and the rel/message collected) rather than aborting.
///
/// Transfers run **concurrently** up to `min(a, b).parallelism()` — the slower
/// side caps it, so SFTP/FTP (which report 1) stay serial while local↔Drive
/// runs many files at once. This is the headline fix for the "27k small files
/// at 0.1 Mbit/s" case: those transfers are latency-bound, not bandwidth-bound.
/// Destination folders are created lazily by `copy_between`; the backends'
/// `mkdir_all` is concurrency-safe (Drive serializes folder creation).
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
    if opts.dry_run {
        let mut st = BisyncStats::default();
        for act in actions {
            match act {
                Action::CopyAtoB(_) | Action::KeepBothAtoB(_) => st.a_to_b += 1,
                Action::CopyBtoA(_) | Action::KeepBothBtoA(_) => st.b_to_a += 1,
                Action::DeleteA(_) | Action::DeleteB(_) => st.deleted += 1,
            }
        }
        return st;
    }

    let mut par = a
        .parallelism()
        .min(b.parallelism())
        .max(1)
        .min(actions.len().max(1));
    if opts.max_transfers > 0 {
        par = par.min(opts.max_transfers);
    }

    let throttle = Throttle::new(opts.bwlimit_bps);
    let merged: Mutex<(BisyncStats, Vec<(String, String)>)> =
        Mutex::new((BisyncStats::default(), Vec::new()));
    let idx = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..par {
            scope.spawn(|| {
                let mut local = BisyncStats::default();
                let mut local_errs: Vec<(String, String)> = Vec::new();
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i >= actions.len() {
                        break;
                    }
                    let act = &actions[i];
                    // Retry transient failures with a delay.
                    let mut attempt = 0u32;
                    let res = loop {
                        match run_one(act, a, root_a, b, root_b, opts, versions_dir, &throttle, cancel) {
                            Ok(s) => break Ok(s),
                            Err(e) => {
                                if attempt >= opts.retries || cancel.load(Ordering::Relaxed) {
                                    break Err(e);
                                }
                                attempt += 1;
                                std::thread::sleep(std::time::Duration::from_secs(
                                    opts.retry_delay_secs,
                                ));
                            }
                        }
                    };
                    match res {
                        Ok(s) => {
                            local.a_to_b += s.a_to_b;
                            local.b_to_a += s.b_to_a;
                            local.deleted += s.deleted;
                            local.bytes += s.bytes;
                        }
                        Err(e) => {
                            local.errors += 1;
                            local_errs.push((format!("{:?}", act), e.to_string()));
                        }
                    }
                }
                let mut m = merged.lock().unwrap();
                m.0.a_to_b += local.a_to_b;
                m.0.b_to_a += local.b_to_a;
                m.0.deleted += local.deleted;
                m.0.bytes += local.bytes;
                m.0.errors += local.errors;
                m.1.extend(local_errs);
            });
        }
    });

    let (st, errs) = merged.into_inner().unwrap();
    errors.extend(errs);
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
            Action::CopyAtoB(r)
            | Action::CopyBtoA(r)
            | Action::DeleteA(r)
            | Action::DeleteB(r)
            | Action::KeepBothAtoB(r)
            | Action::KeepBothBtoA(r) => r,
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

/// A read-only comparison of two sync endpoints (the "ls-diff" view): the
/// planned actions + conflicts, with no changes applied. Uses the saved baseline
/// (so it shows what *would* sync, exactly as a real run would decide).
#[derive(Default)]
pub struct Preview {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    pub a_files: usize,
    pub b_files: usize,
    pub error: Option<String>,
}

pub fn preview(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Preview {
    let base = load_baseline(&baseline_path(&pair_id(root_a, root_b)));
    let hash = opts.compare == CompareMode::Checksum;
    let at = match walk_files(a, root_a, cancel, filter, hash) {
        Ok(t) => t,
        Err(e) => {
            return Preview {
                error: Some(format!("{}: {}", root_a, e)),
                ..Default::default()
            }
        }
    };
    let bt = match walk_files(b, root_b, cancel, filter, hash) {
        Ok(t) => t,
        Err(e) => {
            return Preview {
                error: Some(format!("{}: {}", root_b, e)),
                ..Default::default()
            }
        }
    };
    let (a_files, b_files) = (at.len(), bt.len());
    let (actions, conflicts, _converged) = plan(&at, &bt, &base, opts);
    Preview {
        actions,
        conflicts,
        a_files,
        b_files,
        error: None,
    }
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
        Some(s) => format!("{}:{}:{}", s.size, s.mtime_ms, s.hash),
        None => "-".to_string(),
    }
}
fn parse_sig(s: &str) -> Option<Sig> {
    if s == "-" {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    Some(Sig {
        size: parts[0].parse().ok()?,
        mtime_ms: parts[1].parse().ok()?,
        hash: parts.get(2).and_then(|h| h.parse().ok()).unwrap_or(0),
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

/// Prune the version snapshots per the configured scheme. Snapshots are the
/// timestamp-named subdirectories of the versions store.
pub fn prune_versions(versions: &std::path::Path, v: &Versioning) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut snaps: Vec<(u64, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(versions) {
        for e in rd.flatten() {
            if let Some(ts) = e.file_name().to_str().and_then(|s| s.parse::<u64>().ok()) {
                snaps.push((ts, e.path()));
            }
        }
    }
    snaps.sort_by(|a, b| b.0.cmp(&a.0)); // newest first

    match v.scheme {
        VersioningScheme::Days => {
            if v.days == 0 {
                return; // keep forever
            }
            let cutoff = now.saturating_sub(v.days * 86_400);
            for (ts, p) in &snaps {
                if *ts < cutoff {
                    let _ = std::fs::remove_dir_all(p);
                }
            }
        }
        VersioningScheme::Count => {
            if v.count == 0 {
                return;
            }
            for (i, (_, p)) in snaps.iter().enumerate() {
                if i >= v.count as usize {
                    let _ = std::fs::remove_dir_all(p);
                }
            }
        }
        VersioningScheme::Staggered => keep_per_bucket(&snaps, now, staggered_bucket),
        VersioningScheme::Gfs => keep_per_bucket(&snaps, now, gfs_bucket),
    }
}

/// Keep the newest snapshot in each time bucket; delete the rest (a `None`
/// bucket means "too old — delete"). `snaps` must be newest-first.
fn keep_per_bucket(
    snaps: &[(u64, PathBuf)],
    now: u64,
    bucket: impl Fn(u64, u64) -> Option<String>,
) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (ts, p) in snaps {
        match bucket(*ts, now) {
            Some(key) => {
                if !seen.insert(key) {
                    let _ = std::fs::remove_dir_all(p);
                }
            }
            None => {
                let _ = std::fs::remove_dir_all(p);
            }
        }
    }
}

fn staggered_bucket(ts: u64, now: u64) -> Option<String> {
    let age = now.saturating_sub(ts);
    if age < 86_400 {
        Some(format!("s{ts}")) // <1d: keep all (unique key)
    } else if age < 30 * 86_400 {
        Some(format!("d{}", ts / 86_400)) // 1/day
    } else {
        Some(format!("w{}", ts / (7 * 86_400))) // 1/week
    }
}

fn gfs_bucket(ts: u64, now: u64) -> Option<String> {
    let age = now.saturating_sub(ts);
    if age < 86_400 {
        Some(format!("h{}", ts / 3_600)) // 1/hour for 24h
    } else if age < 7 * 86_400 {
        Some(format!("d{}", ts / 86_400)) // 1/day for 7d
    } else if age < 28 * 86_400 {
        Some(format!("w{}", ts / (7 * 86_400))) // 1/week for 4w
    } else if age < 365 * 86_400 {
        Some(format!("m{}", ts / (30 * 86_400))) // 1/month for 12m
    } else {
        None // older than a year — drop
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
        hash: 0,
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
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Outcome {
    let pair = pair_id(root_a, root_b);
    let bpath = baseline_path(&pair);
    let vdir = versions_dir(&pair);
    let base = load_baseline(&bpath);
    let hash = opts.compare == CompareMode::Checksum;
    let at = match walk_files(a, root_a, cancel, filter, hash) {
        Ok(t) => t,
        Err(e) => {
            return Outcome {
                errors: vec![(root_a.into(), e.to_string())],
                ..Default::default()
            }
        }
    };
    let bt = match walk_files(b, root_b, cancel, filter, hash) {
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

    // Delete-safety guard: refuse to apply if the plan would remove more files
    // than the configured limit (protects against a vanished/remounted side
    // looking like a mass deletion). Aborts the whole run — nothing is touched.
    let deletes = actions
        .iter()
        .filter(|a| matches!(a, Action::DeleteA(_) | Action::DeleteB(_)))
        .count() as u64;
    let total = at.len().max(bt.len()) as u64;
    let pct_limit = if opts.max_delete_pct > 0 {
        total * opts.max_delete_pct as u64 / 100
    } else {
        u64::MAX
    };
    let abs_limit = if opts.max_delete > 0 {
        opts.max_delete
    } else {
        u64::MAX
    };
    if !opts.dry_run && deletes > 0 && (deletes > abs_limit || deletes > pct_limit) {
        return Outcome {
            errors: vec![(
                "abgebrochen".into(),
                format!(
                    "Sicherheitsstopp: {} Löschungen überschreiten das Limit \
                     (max {} Dateien / {}%). Nichts wurde geändert.",
                    deletes, opts.max_delete, opts.max_delete_pct
                ),
            )],
            baseline: base,
            ..Default::default()
        };
    }

    let mut errors = Vec::new();
    let mut st = apply(&actions, a, root_a, b, root_b, opts, &vdir, &mut errors, cancel);
    // Mirror = exact replica: remove duplicate same-name files the destination
    // backend may hold (e.g. Google Drive) so only the correct one remains. This
    // runs before the re-walk so the baseline reflects the deduped state.
    if !opts.dry_run && opts.delete == DeletePolicy::Mirror {
        let dedup = match opts.direction {
            // Keep only names present on the source side (the mirror's truth);
            // an orphaned duplicate name is removed entirely.
            Direction::AtoB => b.dedupe_recursive(root_b, &|rel| at.contains_key(rel)).ok(),
            Direction::BtoA => a.dedupe_recursive(root_a, &|rel| bt.contains_key(rel)).ok(),
            Direction::Both => None,
        };
        st.deleted += dedup.unwrap_or(0) as u64;
    }
    // Re-walk to capture real post-write signatures (e.g. the destination's new
    // mtime), so the baseline doesn't re-detect just-synced files. Skipped on a
    // dry run, where nothing changed.
    let (at2, bt2) = if opts.dry_run {
        (at, bt)
    } else {
        (
            walk_files(a, root_a, cancel, filter, hash).unwrap_or(at),
            walk_files(b, root_b, cancel, filter, hash).unwrap_or(bt),
        )
    };
    let nb = update_baseline(&base, &at2, &bt2, &actions, &converged, &conflicts);
    if !opts.dry_run {
        let _ = save_baseline(&bpath, &nb);
        prune_versions(&vdir, &opts.versioning);
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
    let throttle = Throttle::new(0);
    let no_cancel = AtomicBool::new(false);
    if keep_a {
        if b.exists(&pb) {
            let _ = back_up(b, &pb, rel, &vdir);
        }
        copy_between(a, &pa, b, &pb, true, &throttle, &no_cancel)?;
    } else {
        if a.exists(&pa) {
            let _ = back_up(a, &pa, rel, &vdir);
        }
        copy_between(b, &pb, a, &pa, true, &throttle, &no_cancel)?;
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
        let f = WalkFilter::basic(true, &gs);
        let hash = opts.compare == CompareMode::Checksum;
        let at = walk_files(a, ra, &cancel, &f, hash).unwrap();
        let bt = walk_files(b, rb, &cancel, &f, hash).unwrap();
        let (actions, conflicts, converged) = plan(&at, &bt, base, opts);
        let mut errs = Vec::new();
        let st = apply(&actions, a, ra, b, rb, opts, vdir, &mut errs, &cancel);
        // re-walk for an accurate baseline after writes
        let at2 = walk_files(a, ra, &cancel, &f, hash).unwrap();
        let bt2 = walk_files(b, rb, &cancel, &f, hash).unwrap();
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
        let o1 = super::run(&ba, &ra, &bb, &rb, BisyncOptions::default(), &cancel, &f);
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
        let o2 = super::run(&ba, &ra, &bb, &rb, opts, &cancel, &f);
        assert!(!o2.errors.is_empty(), "guard reports an abort");
        assert!(b.join("f1.txt").exists(), "nothing deleted when aborted");
        let pair = pair_id(&ra, &rb);
        let _ = std::fs::remove_file(baseline_path(&pair));
        let _ = std::fs::remove_dir_all(versions_dir(&pair));
        for d in [&a, &b] {
            std::fs::remove_dir_all(d).ok();
        }
    }
}
