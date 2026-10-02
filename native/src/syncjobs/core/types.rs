use crate::bisync::{
    CompareMode, ConflictMode, DeletePolicy, Direction, VersioningScheme, VersionsLocation,
};

/// Version of the saved job settings. A job file without `config_version`
/// is 0 (saved before RV1) and is migrated when the jobs are loaded; after
/// that a stored 0 (e.g. `max_delete_pct`) is the user's choice (B09).
pub const CURRENT_CONFIG_VERSION: u32 = 1;

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
    /// RealTime: longest wait after the first change while changes keep
    /// coming (0 = automatic, see `effective_rt_max_latency_secs`).
    pub rt_max_latency_secs: u64,
    /// RealTime: seconds between polls of sides without change events
    /// (network drives, remote sides); 0 = never poll.
    pub rt_poll_secs: u64,
    /// Seconds between control runs that walk the watched source(s)
    /// completely; 0 = none.
    pub verify_interval_secs: u64,
    /// Seconds between complete verifications of pure target sides (the
    /// incremental mirror otherwise checks them at changed paths); 0 = none.
    pub verify_target_secs: u64,
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
    /// The percentage stop applies from this many deletions on.
    pub max_delete_min: u64,
    /// Where versions of replaced and deleted files go.
    pub versions_location: VersionsLocation,
    /// Descend into other file systems mounted inside a root (Linux): off
    /// for new jobs, on for jobs saved before RV1.
    pub cross_mounts: bool,
    /// Version of these settings (`CURRENT_CONFIG_VERSION`; 0 = before RV1).
    pub config_version: u32,

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
    /// Command run after a canceled or failed run (cleanup), like
    /// `run_after`; empty = none.
    pub run_cleanup: String,
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
    /// manual, hidden included, mass-delete stop at 25 files and 50 %, other
    /// mounted file systems left out). Every surface takes its defaults from
    /// here.
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
            rt_max_latency_secs: 0,
            rt_poll_secs: 300,
            verify_interval_secs: 3_600,
            verify_target_secs: 86_400,
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
            max_delete_pct: 50,
            max_delete_min: 25,
            versions_location: VersionsLocation::Auto,
            cross_mounts: false,
            config_version: CURRENT_CONFIG_VERSION,
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
            run_cleanup: String::new(),
        }
    }

    /// RealTime: the longest wait after the first change, `rt_max_latency_secs`
    /// or automatically max(5 × `rt_debounce_secs`, 300 s).
    pub fn effective_rt_max_latency_secs(&self) -> u64 {
        if self.rt_max_latency_secs > 0 {
            self.rt_max_latency_secs
        } else {
            self.rt_debounce_secs.saturating_mul(5).max(300)
        }
    }

    /// (min_size, max_size, after_mtime_ms, before_mtime_ms) for the walk filter,
    /// resolving the age windows against `now_secs`.
    pub fn filter_bounds(&self, now_secs: i64) -> (u64, u64, i64, i64) {
        self.checked_filter_bounds(now_secs)
            // Contradictory bounds reject every possible file. Callers that do
            // not surface validation errors therefore still fail closed.
            .unwrap_or((u64::MAX, 1, 0, 0))
    }

    pub fn checked_filter_bounds(&self, now_secs: i64) -> Result<(u64, u64, i64, i64), String> {
        self.validate()?;
        let min_size = self
            .filter_min_size_kb
            .checked_mul(1024)
            .ok_or_else(|| "filter_min_size_kb overflows bytes".to_string())?;
        let max_size = self
            .filter_max_size_kb
            .checked_mul(1024)
            .ok_or_else(|| "filter_max_size_kb overflows bytes".to_string())?;
        let after = checked_age_bound(now_secs, self.filter_max_age_days, false)?;
        let before = checked_age_bound(now_secs, self.filter_min_age_days, true)?;
        Ok((min_size, max_size, after, before))
    }

    /// Compile the ignore patterns. Invalid input yields a match-all set so an
    /// unchecked caller skips every path instead of silently dropping a rule.
    pub fn glob_set(&self) -> globset::GlobSet {
        self.checked_glob_set()
            .unwrap_or_else(|_| rejecting_glob_set())
    }

    /// Engine options derived from this job's settings.
    pub fn opts(&self, dry_run: bool) -> crate::bisync::BisyncOptions {
        self.checked_opts(dry_run)
            .unwrap_or_else(|_| crate::bisync::BisyncOptions {
                dry_run: true,
                delete: DeletePolicy::NoDelete,
                move_files: false,
                ..Default::default()
            })
    }

    pub fn checked_opts(&self, dry_run: bool) -> Result<crate::bisync::BisyncOptions, String> {
        self.validate()?;
        Ok(crate::bisync::BisyncOptions {
            direction: self.direction,
            conflict: self.conflict,
            reversible: true,
            dry_run,
            delete: self.delete_policy,
            move_files: self.move_files,
            compare: self.compare,
            modify_window_ms: i64::try_from(self.modify_window_sec)
                .map_err(|_| "modify_window_sec is too large".to_string())?
                .checked_mul(1000)
                .ok_or_else(|| "modify_window_sec overflows milliseconds".to_string())?,
            versioning: crate::bisync::Versioning {
                scheme: self.versioning_scheme,
                days: self.retain_days,
                count: self.retain_count,
            },
            use_recycle: self.use_recycle_bin,
            max_delete: self.max_delete,
            max_delete_pct: self.max_delete_pct,
            max_delete_min: self.max_delete_min,
            cross_mounts: self.cross_mounts,
            versions: self.versions_location,
            verify_target_secs: self.verify_target_secs,
            bwlimit_bps: self
                .bwlimit_kbps
                .checked_mul(1024)
                .ok_or_else(|| "bwlimit_kbps overflows bytes per second".to_string())?,
            max_transfers: usize::try_from(self.max_transfers)
                .map_err(|_| "max_transfers is too large for this platform".to_string())?,
            atomic: self.atomic_copy,
            verify: self.verify,
            retries: u32::try_from(self.retries)
                .map_err(|_| "retries must not exceed 4294967295".to_string())?,
            retry_delay_secs: self.retry_delay_secs,
        })
    }
}

fn checked_age_bound(now_secs: i64, days: u64, is_upper_bound: bool) -> Result<i64, String> {
    if days == 0 {
        return Ok(0);
    }
    let seconds = days
        .checked_mul(86_400)
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| "filter age overflows seconds".to_string())?;
    let cutoff_secs = now_secs
        .checked_sub(seconds)
        .ok_or_else(|| "filter age underflows the timestamp".to_string())?;
    let cutoff_ms = cutoff_secs
        .checked_mul(1000)
        .ok_or_else(|| "filter age overflows milliseconds".to_string())?;
    if cutoff_ms <= 0 {
        if is_upper_bound {
            return Err("minimum-age filter predates the supported timestamp range".into());
        }
        return Ok(0);
    }
    Ok(cutoff_ms)
}

fn rejecting_glob_set() -> globset::GlobSet {
    let mut builder = globset::GlobSetBuilder::new();
    builder.add(
        globset::Glob::new("**")
            .expect("the built-in match-all glob must remain valid for fail-closed filtering"),
    );
    builder
        .build()
        .expect("the built-in match-all glob set must compile")
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

    #[test]
    fn invalid_glob_and_numeric_settings_fail_closed() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.ignore = vec!["[".into()];
        assert!(j.glob_set().is_match("anything/at/all"));

        j.ignore.clear();
        j.max_delete_pct = 101;
        assert!(j.opts(false).dry_run);
        assert_eq!(j.opts(false).delete, DeletePolicy::NoDelete);
        assert_eq!(j.filter_bounds(1_000), (u64::MAX, 1, 0, 0));
    }

    #[test]
    fn checked_bounds_and_options_use_exact_conversions() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.filter_min_size_kb = 2;
        j.filter_max_size_kb = 8;
        j.filter_max_age_days = 2;
        j.filter_min_age_days = 1;
        j.modify_window_sec = 3;
        j.bwlimit_kbps = 4;
        j.retries = 5;
        let now = 1_700_000_000;

        assert_eq!(
            j.checked_filter_bounds(now).unwrap(),
            (2048, 8192, (now - 2 * 86_400) * 1000, (now - 86_400) * 1000,)
        );
        let opts = j.checked_opts(false).unwrap();
        assert_eq!(opts.modify_window_ms, 3000);
        assert_eq!(opts.bwlimit_bps, 4096);
        assert_eq!(opts.retries, 5);
    }
}
