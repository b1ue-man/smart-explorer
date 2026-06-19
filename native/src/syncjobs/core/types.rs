use crate::bisync::{CompareMode, ConflictMode, DeletePolicy, Direction, VersioningScheme};

/// What makes a job run. Timer-based kinds (`Interval`, `Calendar`) are evaluated
/// by `due()`; the event kinds are driven by the daemon (`OnStartup` once at
/// launch, `RealTime` by a filesystem watch, `OnConnect` by device arrival).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    Manual,
    Interval,
    Calendar,
    RealTime,
    OnStartup,
    OnConnect,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::Manual => "manual",
            Trigger::Interval => "interval",
            Trigger::Calendar => "calendar",
            Trigger::RealTime => "realtime",
            Trigger::OnStartup => "onstartup",
            Trigger::OnConnect => "onconnect",
        }
    }

    pub fn parse(s: &str) -> Option<Trigger> {
        Some(match s {
            "manual" => Trigger::Manual,
            "interval" => Trigger::Interval,
            "calendar" => Trigger::Calendar,
            "realtime" => Trigger::RealTime,
            "onstartup" => Trigger::OnStartup,
            "onconnect" => Trigger::OnConnect,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Trigger::Manual => "Manuell (nur \u{201e}Jetzt\u{201c})",
            Trigger::Interval => "Intervall (alle N Min)",
            Trigger::Calendar => "Zeitplan (t\u{e4}glich/w\u{f6}chentlich/monatlich)",
            Trigger::RealTime => "Echtzeit (bei \u{c4}nderung)",
            Trigger::OnStartup => "Beim Start",
            Trigger::OnConnect => "Bei Ger\u{e4}te-/USB-Anschluss",
        }
    }

    pub const ALL: [Trigger; 6] = [
        Trigger::Manual,
        Trigger::Interval,
        Trigger::Calendar,
        Trigger::RealTime,
        Trigger::OnStartup,
        Trigger::OnConnect,
    ];
}

#[derive(Clone, Debug)]
pub struct SyncJob {
    pub id: String,
    pub name: String,
    /// "Side A": a local path or a remote target (e.g. sftp://user@host:port/p).
    pub source: String,
    /// "Side B".
    pub target: String,
    pub direction: Direction,
    pub conflict: ConflictMode,
    pub retain_days: u64,
    /// Auto-run every N minutes (used when `trigger == Interval`; 0 = off).
    pub interval_min: u64,
    pub include_hidden: bool,
    /// Glob patterns matched on the relative path; matches are skipped.
    pub ignore: Vec<String>,
    /// Unix seconds of the last successful run (0 = never).
    pub last_run: i64,
    pub enabled: bool,

    // Group D: scheduling / triggers
    pub trigger: Trigger,
    /// Calendar: minutes after local midnight to run (e.g. 9*60 = 09:00).
    pub cal_time_min: i32,
    /// Calendar weekdays bitmask, bit0=Mon ... bit6=Sun. 0 = every day.
    pub cal_weekdays: u8,
    /// Calendar day-of-month 1..31 for monthly (0 = use weekdays instead).
    pub cal_monthday: u8,
    /// RealTime: settle/idle delay in seconds after the last change before running.
    pub rt_debounce_secs: u64,
    /// OnConnect: volume label / serial / drive-letter wildcard ("" = any removable).
    pub connect_match: String,
    /// Active-hours window (minutes after midnight). from==to means always allowed.
    pub active_from_min: i32,
    pub active_to_min: i32,
    /// Run a missed scheduled occurrence as soon as possible (else wait for next).
    pub catch_up: bool,

    // Group B/C: deletion handling, move, comparison
    pub delete_policy: DeletePolicy,
    pub move_files: bool,
    pub compare: CompareMode,
    /// mtime tolerance in seconds for MtimeSize compare (FAT/DST: 1-2).
    pub modify_window_sec: u64,

    // Group F: versioning & deletion safety
    pub versioning_scheme: VersioningScheme,
    /// Keep-last-N versions (used by Count scheme).
    pub retain_count: u64,
    /// Send deletes to the OS Recycle Bin (local paths) instead of removing.
    pub use_recycle_bin: bool,
    /// Abort if a run would delete more than this many files (0 = no limit).
    pub max_delete: u64,
    /// ...or more than this percent of a side's files (0 = no limit).
    pub max_delete_pct: u8,

    // Group G: filters (0 = off)
    pub filter_min_size_kb: u64,
    pub filter_max_size_kb: u64,
    /// Only sync files modified within the last N days.
    pub filter_max_age_days: u64,
    /// Only sync files older than N days.
    pub filter_min_age_days: u64,

    // Groups H/I: bandwidth & reliability
    pub bwlimit_kbps: u64,
    pub max_transfers: u64,
    pub atomic_copy: bool,
    pub verify: bool,
    pub retries: u64,
    pub retry_delay_secs: u64,
    /// Commands run before / after the job (background daemon runs).
    pub run_before: String,
    pub run_after: String,
}

fn gen_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}

impl SyncJob {
    /// New job with safe defaults (two-way, strict conflicts, 30-day retention,
    /// manual, hidden included).
    pub fn new(name: String, source: String, target: String) -> Self {
        SyncJob {
            id: gen_id(),
            name,
            source,
            target,
            direction: Direction::Both,
            conflict: ConflictMode::FileLevel,
            retain_days: 30,
            interval_min: 0,
            include_hidden: true,
            ignore: Vec::new(),
            last_run: 0,
            enabled: true,
            trigger: Trigger::Manual,
            cal_time_min: 9 * 60,
            cal_weekdays: 0,
            cal_monthday: 0,
            rt_debounce_secs: 10,
            connect_match: String::new(),
            active_from_min: 0,
            active_to_min: 0,
            catch_up: true,
            delete_policy: DeletePolicy::Propagate,
            move_files: false,
            compare: CompareMode::MtimeSize,
            modify_window_sec: 0,
            versioning_scheme: VersioningScheme::Days,
            retain_count: 0,
            use_recycle_bin: false,
            max_delete: 0,
            max_delete_pct: 0,
            filter_min_size_kb: 0,
            filter_max_size_kb: 0,
            filter_max_age_days: 0,
            filter_min_age_days: 0,
            bwlimit_kbps: 0,
            max_transfers: 0,
            atomic_copy: true,
            verify: false,
            retries: 0,
            retry_delay_secs: 2,
            run_before: String::new(),
            run_after: String::new(),
        }
    }

    /// (min_size, max_size, after_mtime_ms, before_mtime_ms) for the walk filter,
    /// resolving the age windows against `now_secs`.
    pub fn filter_bounds(&self, now_secs: i64) -> (u64, u64, i64, i64) {
        let min_size = self.filter_min_size_kb.saturating_mul(1024);
        let max_size = self.filter_max_size_kb.saturating_mul(1024);
        let after = if self.filter_max_age_days > 0 {
            (now_secs - self.filter_max_age_days as i64 * 86_400) * 1000
        } else {
            0
        };
        let before = if self.filter_min_age_days > 0 {
            (now_secs - self.filter_min_age_days as i64 * 86_400) * 1000
        } else {
            0
        };
        (min_size, max_size, after, before)
    }

    /// Compile the ignore patterns into a GlobSet (bad patterns are skipped).
    pub fn glob_set(&self) -> globset::GlobSet {
        let mut b = globset::GlobSetBuilder::new();
        for pat in &self.ignore {
            let pat = pat.trim();
            if pat.is_empty() {
                continue;
            }
            if let Ok(g) = globset::Glob::new(pat) {
                b.add(g);
            }
        }
        b.build().unwrap_or_else(|_| crate::bisync::empty_globset())
    }

    /// Engine options derived from this job's settings.
    pub fn opts(&self, dry_run: bool) -> crate::bisync::BisyncOptions {
        crate::bisync::BisyncOptions {
            direction: self.direction,
            conflict: self.conflict,
            reversible: true,
            dry_run,
            delete: self.delete_policy,
            move_files: self.move_files,
            compare: self.compare,
            modify_window_ms: self.modify_window_sec as i64 * 1000,
            versioning: crate::bisync::Versioning {
                scheme: self.versioning_scheme,
                days: self.retain_days,
                count: self.retain_count,
            },
            use_recycle: self.use_recycle_bin,
            max_delete: self.max_delete,
            max_delete_pct: self.max_delete_pct,
            bwlimit_bps: self.bwlimit_kbps.saturating_mul(1024),
            max_transfers: self.max_transfers as usize,
            atomic: self.atomic_copy,
            verify: self.verify,
            retries: self.retries as u32,
            retry_delay_secs: self.retry_delay_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_set_matches_ignores() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.ignore = vec!["**/*.tmp".into(), "cache/**".into()];
        let gs = j.glob_set();
        assert!(gs.is_match("foo/bar.tmp"));
        assert!(gs.is_match("cache/x/y"));
        assert!(!gs.is_match("keep/me.txt"));
    }
}
