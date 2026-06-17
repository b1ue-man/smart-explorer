//! Persistent sync jobs — the saved "sync setups" (source, target, direction,
//! conflict mode, retention, schedule, hidden/ignore). Stored as one
//! human-readable `key=value` file per job under `<appdata>/smart_explorer/
//! sync/jobs/<id>.conf`, shared by the Sync UI, the split-view "sync these
//! folders" action, and the background worker — so a setup survives a restart
//! and every surface agrees.
//!
//! The `key=value` format is deliberately forward-compatible: unknown keys are
//! ignored and missing keys fall back to defaults, so new options can be added
//! over time without breaking old files or older builds. The previous single
//! positional `jobs.tsv` is auto-imported once on first load.

use crate::bisync::{CompareMode, ConflictMode, DeletePolicy, Direction};
use std::path::PathBuf;

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
            Trigger::Manual => "Manuell (nur „Jetzt“)",
            Trigger::Interval => "Intervall (alle N Min)",
            Trigger::Calendar => "Zeitplan (täglich/wöchentlich/monatlich)",
            Trigger::RealTime => "Echtzeit (bei Änderung)",
            Trigger::OnStartup => "Beim Start",
            Trigger::OnConnect => "Bei Geräte-/USB-Anschluss",
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

    // ── Group D: scheduling / triggers ───────────────────────────────────────
    pub trigger: Trigger,
    /// Calendar: minutes after local midnight to run (e.g. 9*60 = 09:00).
    pub cal_time_min: i32,
    /// Calendar weekdays bitmask, bit0=Mon … bit6=Sun. 0 = every day.
    pub cal_weekdays: u8,
    /// Calendar day-of-month 1..31 for monthly (0 = use weekdays instead).
    pub cal_monthday: u8,
    /// RealTime: settle/idle delay in seconds after the last change before running.
    pub rt_debounce_secs: u64,
    /// OnConnect: volume label / serial / drive-letter wildcard ("" = any removable).
    pub connect_match: String,
    /// Active-hours window (minutes after midnight). from==to ⇒ always allowed.
    pub active_from_min: i32,
    pub active_to_min: i32,
    /// Run a missed scheduled occurrence as soon as possible (else wait for next).
    pub catch_up: bool,

    // ── Group B/C: deletion handling, move, comparison ───────────────────────
    pub delete_policy: DeletePolicy,
    pub move_files: bool,
    pub compare: CompareMode,
    /// mtime tolerance in seconds for MtimeSize compare (FAT/DST: 1–2).
    pub modify_window_sec: u64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn gen_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}

/// Minutes after local midnight for a unix timestamp.
fn local_min_of_day(now: i64) -> i32 {
    use chrono::{Local, TimeZone, Timelike};
    match Local.timestamp_opt(now, 0).single() {
        Some(d) => d.hour() as i32 * 60 + d.minute() as i32,
        None => 0,
    }
}

/// Is `cur` (minutes after midnight) within the active window? `from == to`
/// means "always". A window with `from > to` wraps past midnight.
pub fn within_window(cur: i32, from: i32, to: i32) -> bool {
    if from == to {
        return true;
    }
    if from < to {
        cur >= from && cur < to
    } else {
        cur >= from || cur < to
    }
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
        }
    }

    /// Timer-due now? Honours the trigger kind and the active-hours window.
    /// Event triggers (RealTime/OnStartup/OnConnect) are driven by the daemon,
    /// not this timer check, so they return false here.
    pub fn due(&self, now: i64) -> bool {
        if !self.enabled || !self.active_now(now) {
            return false;
        }
        match self.trigger {
            Trigger::Interval => {
                self.interval_min > 0 && (now - self.last_run) >= (self.interval_min as i64 * 60)
            }
            Trigger::Calendar => match self.last_occurrence(now) {
                Some(occ) => {
                    if self.last_run >= occ {
                        false
                    } else if self.catch_up {
                        true
                    } else {
                        // No catch-up: only fire close to the scheduled instant
                        // (within one daemon check window's grace ≈ 2 min).
                        (now - occ) <= 120
                    }
                }
                None => false,
            },
            _ => false,
        }
    }

    /// Is `now` inside this job's active-hours window? (true when no window set).
    pub fn active_now(&self, now: i64) -> bool {
        within_window(local_min_of_day(now), self.active_from_min, self.active_to_min)
    }

    /// Does `day` (Mon=0..Sun=6 weekday, plus day-of-month) match this calendar?
    fn day_matches(&self, weekday_mon0: u32, day_of_month: u32) -> bool {
        if self.cal_monthday != 0 {
            return day_of_month == self.cal_monthday as u32;
        }
        if self.cal_weekdays == 0 {
            return true; // every day
        }
        (self.cal_weekdays >> weekday_mon0) & 1 == 1
    }

    /// Unix-seconds of the most recent scheduled occurrence at or before `now`
    /// (searching back up to a year), or None if the calendar never matches.
    fn last_occurrence(&self, now: i64) -> Option<i64> {
        use chrono::{Datelike, Duration, Local, TimeZone};
        let now_dt = Local.timestamp_opt(now, 0).single()?;
        let (h, m) = (
            (self.cal_time_min / 60).clamp(0, 23) as u32,
            (self.cal_time_min % 60).clamp(0, 59) as u32,
        );
        for back in 0..366 {
            let d = (now_dt - Duration::days(back)).date_naive();
            if !self.day_matches(d.weekday().num_days_from_monday(), d.day()) {
                continue;
            }
            let naive = d.and_hms_opt(h, m, 0)?;
            if let Some(inst) = Local.from_local_datetime(&naive).single() {
                let ts = inst.timestamp();
                if ts <= now {
                    return Some(ts);
                }
            }
        }
        None
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
        }
    }
}

// ── persistence ──────────────────────────────────────────────────────────────

fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer").join("sync");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Legacy single-file store (positional TSV), kept only for one-time import.
pub fn jobs_path() -> PathBuf {
    app_data_dir().join("jobs.tsv")
}

/// Directory holding one `<id>.conf` per job.
pub fn jobs_dir() -> PathBuf {
    let d = app_data_dir().join("jobs");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn job_file(dir: &std::path::Path, id: &str) -> PathBuf {
    dir.join(format!("{}.conf", san_id(id)))
}

/// Strip anything that could escape the filename (ids are hex, but be safe).
fn san_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Strip characters that would break the one-value-per-line format.
fn san(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// Serialize one job as a `key=value` block (the body of its `.conf` file).
fn serialize_kv(j: &SyncJob) -> String {
    let mut s = String::new();
    s.push_str("# Smart Explorer sync job\n");
    s.push_str(&format!("id={}\n", san(&j.id)));
    s.push_str(&format!("name={}\n", san(&j.name)));
    s.push_str(&format!("source={}\n", san(&j.source)));
    s.push_str(&format!("target={}\n", san(&j.target)));
    s.push_str(&format!("direction={}\n", j.direction.as_str()));
    s.push_str(&format!("conflict={}\n", j.conflict.as_str()));
    s.push_str(&format!("retain_days={}\n", j.retain_days));
    s.push_str(&format!("interval_min={}\n", j.interval_min));
    s.push_str(&format!(
        "include_hidden={}\n",
        if j.include_hidden { 1 } else { 0 }
    ));
    for pat in &j.ignore {
        let p = san(pat);
        if !p.trim().is_empty() {
            s.push_str(&format!("ignore={}\n", p));
        }
    }
    s.push_str(&format!("last_run={}\n", j.last_run));
    s.push_str(&format!("enabled={}\n", if j.enabled { 1 } else { 0 }));
    // Group D — scheduling / triggers
    s.push_str(&format!("trigger={}\n", j.trigger.as_str()));
    s.push_str(&format!("cal_time_min={}\n", j.cal_time_min));
    s.push_str(&format!("cal_weekdays={}\n", j.cal_weekdays));
    s.push_str(&format!("cal_monthday={}\n", j.cal_monthday));
    s.push_str(&format!("rt_debounce_secs={}\n", j.rt_debounce_secs));
    s.push_str(&format!("connect_match={}\n", san(&j.connect_match)));
    s.push_str(&format!("active_from_min={}\n", j.active_from_min));
    s.push_str(&format!("active_to_min={}\n", j.active_to_min));
    s.push_str(&format!("catch_up={}\n", if j.catch_up { 1 } else { 0 }));
    // Group B/C — direction detail & comparison
    s.push_str(&format!("delete_policy={}\n", j.delete_policy.as_str()));
    s.push_str(&format!("move_files={}\n", if j.move_files { 1 } else { 0 }));
    s.push_str(&format!("compare={}\n", j.compare.as_str()));
    s.push_str(&format!("modify_window_sec={}\n", j.modify_window_sec));
    s
}

/// Parse one `key=value` job block. Tolerant: unknown keys ignored, missing keys
/// take defaults, repeated `ignore=` lines accumulate.
fn parse_kv(body: &str) -> Option<SyncJob> {
    let mut j = SyncJob::new(String::new(), String::new(), String::new());
    j.id.clear(); // require an explicit `id=` line
    j.ignore.clear();
    let mut saw_any = false;
    let mut saw_trigger = false;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        saw_any = true;
        match k {
            "id" => j.id = v.to_string(),
            "name" => j.name = v.to_string(),
            "source" => j.source = v.to_string(),
            "target" => j.target = v.to_string(),
            "direction" => j.direction = Direction::parse(v).unwrap_or(Direction::Both),
            "conflict" => j.conflict = ConflictMode::parse(v).unwrap_or(ConflictMode::FileLevel),
            "retain_days" => j.retain_days = v.parse().unwrap_or(30),
            "interval_min" => j.interval_min = v.parse().unwrap_or(0),
            "include_hidden" => j.include_hidden = v != "0",
            "ignore" => {
                if !v.is_empty() {
                    j.ignore.push(v.to_string())
                }
            }
            "last_run" => j.last_run = v.parse().unwrap_or(0),
            "enabled" => j.enabled = v != "0",
            "trigger" => {
                j.trigger = Trigger::parse(v).unwrap_or(Trigger::Manual);
                saw_trigger = true;
            }
            "cal_time_min" => j.cal_time_min = v.parse().unwrap_or(540),
            "cal_weekdays" => j.cal_weekdays = v.parse().unwrap_or(0),
            "cal_monthday" => j.cal_monthday = v.parse().unwrap_or(0),
            "rt_debounce_secs" => j.rt_debounce_secs = v.parse().unwrap_or(10),
            "connect_match" => j.connect_match = v.to_string(),
            "active_from_min" => j.active_from_min = v.parse().unwrap_or(0),
            "active_to_min" => j.active_to_min = v.parse().unwrap_or(0),
            "catch_up" => j.catch_up = v != "0",
            "delete_policy" => {
                j.delete_policy = DeletePolicy::parse(v).unwrap_or(DeletePolicy::Propagate)
            }
            "move_files" => j.move_files = v != "0",
            "compare" => j.compare = CompareMode::parse(v).unwrap_or(CompareMode::MtimeSize),
            "modify_window_sec" => j.modify_window_sec = v.parse().unwrap_or(0),
            _ => {} // unknown / future key — ignored
        }
    }
    // Back-compat: a job saved before triggers existed but with an interval set
    // was an interval job.
    if !saw_trigger && j.interval_min > 0 {
        j.trigger = Trigger::Interval;
    }
    if saw_any && !j.id.is_empty() {
        Some(j)
    } else {
        None
    }
}

/// Legacy positional-TSV line parser (for one-time import of the old jobs.tsv).
fn parse_legacy(line: &str) -> Option<SyncJob> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 12 {
        return None;
    }
    let mut j = SyncJob::new(f[1].to_string(), f[2].to_string(), f[3].to_string());
    j.id = f[0].to_string();
    j.direction = Direction::parse(f[4]).unwrap_or(Direction::Both);
    j.conflict = ConflictMode::parse(f[5]).unwrap_or(ConflictMode::FileLevel);
    j.retain_days = f[6].parse().unwrap_or(30);
    j.interval_min = f[7].parse().unwrap_or(0);
    j.include_hidden = f[8] != "0";
    j.ignore = if f[9].is_empty() {
        Vec::new()
    } else {
        f[9].split('\u{1f}').map(|s| s.to_string()).collect()
    };
    j.last_run = f[10].parse().unwrap_or(0);
    j.enabled = f[11] != "0";
    // The old format predates triggers: an interval meant an interval job.
    j.trigger = if j.interval_min > 0 {
        Trigger::Interval
    } else {
        Trigger::Manual
    };
    Some(j)
}

// ── dir-based store (testable with an explicit directory) ────────────────────

fn load_dir(dir: &std::path::Path) -> Vec<SyncJob> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("conf") {
                if let Ok(body) = std::fs::read_to_string(&p) {
                    if let Some(j) = parse_kv(&body) {
                        out.push(j);
                    }
                }
            }
        }
    }
    // Stable, predictable order (by name, then id) regardless of fs ordering.
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    out
}

fn write_job(dir: &std::path::Path, job: &SyncJob) -> std::io::Result<()> {
    std::fs::write(job_file(dir, &job.id), serialize_kv(job))
}

fn save_dir(dir: &std::path::Path, jobs: &[SyncJob]) -> std::io::Result<()> {
    // Delete .conf files for jobs no longer present, then (over)write the rest.
    let keep: Vec<String> = jobs.iter().map(|j| san_id(&j.id)).collect();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("conf") {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                if !keep.contains(&stem) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
    for j in jobs {
        write_job(dir, j)?;
    }
    Ok(())
}

// ── public API (real appdata store) ──────────────────────────────────────────

pub fn load() -> Vec<SyncJob> {
    let dir = jobs_dir();
    let jobs = load_dir(&dir);
    if jobs.is_empty() {
        // One-time migration from the old single-file TSV.
        let legacy = jobs_path();
        if let Ok(s) = std::fs::read_to_string(&legacy) {
            let imported: Vec<SyncJob> = s.lines().filter_map(parse_legacy).collect();
            if !imported.is_empty() {
                let _ = save_dir(&dir, &imported);
                let _ = std::fs::rename(&legacy, legacy.with_extension("tsv.imported"));
                return load_dir(&dir);
            }
        }
    }
    jobs
}

pub fn save(jobs: &[SyncJob]) -> std::io::Result<()> {
    save_dir(&jobs_dir(), jobs)
}

/// Add or replace a job (by id) — rewrites just that job's file.
pub fn upsert(job: &SyncJob) -> std::io::Result<()> {
    write_job(&jobs_dir(), job)
}

pub fn remove(id: &str) -> std::io::Result<()> {
    match std::fs::remove_file(job_file(&jobs_dir(), id)) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Mark a job as just-run (updates last_run and rewrites only its file).
pub fn mark_run(id: &str) {
    let dir = jobs_dir();
    if let Ok(body) = std::fs::read_to_string(job_file(&dir, id)) {
        if let Some(mut j) = parse_kv(&body) {
            j.last_run = now_secs();
            let _ = write_job(&dir, &j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SyncJob {
        let mut j = SyncJob::new("Docs".into(), "C:/a".into(), "D:/b".into());
        j.interval_min = 15;
        j.retain_days = 7;
        j.include_hidden = false;
        j.ignore = vec!["**/*.tmp".into(), "node_modules/**".into()];
        j.conflict = ConflictMode::NewerWins;
        j.direction = Direction::AtoB;
        j
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn temp_dir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("se_jobs_{}_{}", std::process::id(), now_nanos()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn kv_roundtrip() {
        let j = sample();
        let back = parse_kv(&serialize_kv(&j)).unwrap();
        assert_eq!(back.id, j.id);
        assert_eq!(back.name, "Docs");
        assert_eq!(back.source, "C:/a");
        assert_eq!(back.target, "D:/b");
        assert_eq!(back.direction, Direction::AtoB);
        assert_eq!(back.conflict, ConflictMode::NewerWins);
        assert_eq!(back.retain_days, 7);
        assert_eq!(back.interval_min, 15);
        assert!(!back.include_hidden);
        assert_eq!(
            back.ignore,
            vec!["**/*.tmp".to_string(), "node_modules/**".to_string()]
        );
    }

    #[test]
    fn kv_tolerates_unknown_and_missing_keys() {
        // Unknown future key ignored; missing keys take defaults.
        let body = "id=abc\nname=X\nsource=s\ntarget=t\nfuture_option=42\n";
        let j = parse_kv(body).unwrap();
        assert_eq!(j.id, "abc");
        assert_eq!(j.name, "X");
        assert_eq!(j.direction, Direction::Both);
        assert_eq!(j.retain_days, 30);
        assert!(j.ignore.is_empty());
        // A block with no id is rejected.
        assert!(parse_kv("name=NoId\n").is_none());
    }

    #[test]
    fn dir_store_roundtrip_upsert_and_remove() {
        let dir = temp_dir();
        let a = SyncJob::new("A".into(), "s".into(), "t".into());
        let b = SyncJob::new("B".into(), "s2".into(), "t2".into());
        save_dir(&dir, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(load_dir(&dir).len(), 2);

        // Rewrite one job (upsert semantics).
        let mut a2 = a.clone();
        a2.name = "A2".into();
        write_job(&dir, &a2).unwrap();
        let l2 = load_dir(&dir);
        assert_eq!(l2.iter().find(|j| j.id == a.id).unwrap().name, "A2");
        assert_eq!(l2.len(), 2);

        // save_dir with fewer jobs deletes the dropped one's file.
        save_dir(&dir, &[b.clone()]).unwrap();
        let l3 = load_dir(&dir);
        assert_eq!(l3.len(), 1);
        assert_eq!(l3[0].id, b.id);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_tsv_line_imports() {
        let mut j = sample();
        j.id = "deadbeef".into();
        let line = [
            j.id.as_str(),
            &j.name,
            &j.source,
            &j.target,
            j.direction.as_str(),
            j.conflict.as_str(),
            &j.retain_days.to_string(),
            &j.interval_min.to_string(),
            "0",
            &j.ignore.join("\u{1f}"),
            &j.last_run.to_string(),
            "1",
        ]
        .join("\t");
        let back = parse_legacy(&line).unwrap();
        assert_eq!(back.id, "deadbeef");
        assert_eq!(back.direction, Direction::AtoB);
        assert_eq!(back.conflict, ConflictMode::NewerWins);
        assert_eq!(back.ignore.len(), 2);
    }

    #[test]
    fn due_logic_interval() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        assert!(!j.due(1000), "manual trigger is never timer-due");
        j.trigger = Trigger::Interval;
        assert!(!j.due(1000), "interval 0 = never due");
        j.interval_min = 10;
        j.last_run = 0;
        assert!(j.due(700), "10 min elapsed since epoch");
        j.last_run = 700;
        assert!(!j.due(900), "only 200s since last run");
        assert!(j.due(700 + 600));
        j.enabled = false;
        assert!(!j.due(99999), "disabled never due");
    }

    #[test]
    fn within_window_logic() {
        // always (from==to)
        assert!(within_window(0, 0, 0));
        assert!(within_window(720, 480, 480));
        // normal window 09:00–17:00
        assert!(within_window(600, 540, 1020));
        assert!(!within_window(1100, 540, 1020));
        assert!(!within_window(300, 540, 1020));
        // wraps midnight 22:00–06:00
        assert!(within_window(1380, 1320, 360)); // 23:00
        assert!(within_window(120, 1320, 360)); // 02:00
        assert!(!within_window(720, 1320, 360)); // 12:00
    }

    #[test]
    fn calendar_day_matches() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        // every day
        j.cal_weekdays = 0;
        j.cal_monthday = 0;
        assert!(j.day_matches(0, 15));
        assert!(j.day_matches(6, 1));
        // weekly: Mon + Fri (bit0 | bit4)
        j.cal_weekdays = 0b0001_0001;
        assert!(j.day_matches(0, 10)); // Mon
        assert!(j.day_matches(4, 10)); // Fri
        assert!(!j.day_matches(2, 10)); // Wed
        // monthly overrides weekdays
        j.cal_monthday = 15;
        assert!(j.day_matches(2, 15));
        assert!(!j.day_matches(0, 16));
    }

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
