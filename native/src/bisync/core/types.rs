use std::collections::BTreeMap;
use std::sync::Mutex;

/// Size + mtime (+ optional content hash) signature of one file on one side.
/// `hash` is 0 when this side wasn't hashed (see `HashMode`/`hash_mode`); when
/// non-zero it's the MD5-derived content key and takes priority in compares.
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
            CompareMode::MtimeSize => "Größe + Änderungszeit (schnell; nutzt gratis Server-Hash bei Drive/Nextcloud → kein erneuter Transfer)",
            CompareMode::SizeOnly => "Nur Größe (Änderungszeit ignorieren)",
            CompareMode::Checksum => "Prüfsumme (Server-Hash bei Drive/Nextcloud = kein Download; sonst Inhalt lesen)",
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
    // ── Groups H/I: bandwidth & reliability ───────────────────────────────
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
    pub(super) fn consume(&self, n: u64) {
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
