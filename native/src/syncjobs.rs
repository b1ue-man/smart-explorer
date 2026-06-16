//! Persistent sync jobs — the saved "sync setups" (source, target, direction,
//! conflict mode, retention, schedule, hidden/ignore). One TSV file in appdata
//! shared by the Sync UI, the split-view "sync these folders" action, and the
//! background worker — so a setup survives a restart and every surface agrees.

use crate::bisync::{ConflictMode, Direction};
use std::path::PathBuf;

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
    /// Auto-run every N minutes (0 = manual only).
    pub interval_min: u64,
    pub include_hidden: bool,
    /// Glob patterns matched on the relative path; matches are skipped.
    pub ignore: Vec<String>,
    /// Unix seconds of the last successful run (0 = never).
    pub last_run: i64,
    pub enabled: bool,
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
        }
    }

    /// Due to auto-run now?
    pub fn due(&self, now: i64) -> bool {
        self.enabled
            && self.interval_min > 0
            && (now - self.last_run) >= (self.interval_min as i64 * 60)
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
}

// ── persistence (TSV in appdata) ─────────────────────────────────────────────

fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer").join("sync");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn jobs_path() -> PathBuf {
    app_data_dir().join("jobs.tsv")
}

fn san(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn serialize(j: &SyncJob) -> String {
    [
        san(&j.id),
        san(&j.name),
        san(&j.source),
        san(&j.target),
        j.direction.as_str().to_string(),
        j.conflict.as_str().to_string(),
        j.retain_days.to_string(),
        j.interval_min.to_string(),
        if j.include_hidden { "1" } else { "0" }.to_string(),
        // ignore patterns joined by unit-separator (won't appear in globs)
        j.ignore.iter().map(|p| san(p)).collect::<Vec<_>>().join("\u{1f}"),
        j.last_run.to_string(),
        if j.enabled { "1" } else { "0" }.to_string(),
    ]
    .join("\t")
}

fn parse(line: &str) -> Option<SyncJob> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 12 {
        return None;
    }
    Some(SyncJob {
        id: f[0].to_string(),
        name: f[1].to_string(),
        source: f[2].to_string(),
        target: f[3].to_string(),
        direction: Direction::parse(f[4]).unwrap_or(Direction::Both),
        conflict: ConflictMode::parse(f[5]).unwrap_or(ConflictMode::FileLevel),
        retain_days: f[6].parse().unwrap_or(30),
        interval_min: f[7].parse().unwrap_or(0),
        include_hidden: f[8] != "0",
        ignore: if f[9].is_empty() {
            Vec::new()
        } else {
            f[9].split('\u{1f}').map(|s| s.to_string()).collect()
        },
        last_run: f[10].parse().unwrap_or(0),
        enabled: f[11] != "0",
    })
}

pub fn load_from(path: &std::path::Path) -> Vec<SyncJob> {
    match std::fs::read_to_string(path) {
        Ok(s) => s.lines().filter_map(parse).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn save_to(path: &std::path::Path, jobs: &[SyncJob]) -> std::io::Result<()> {
    std::fs::write(path, jobs.iter().map(serialize).collect::<Vec<_>>().join("\n"))
}

pub fn load() -> Vec<SyncJob> {
    load_from(&jobs_path())
}

pub fn save(jobs: &[SyncJob]) -> std::io::Result<()> {
    save_to(&jobs_path(), jobs)
}

/// Add or replace a job (by id) and persist.
pub fn upsert(job: &SyncJob) -> std::io::Result<()> {
    let mut jobs = load();
    jobs.retain(|j| j.id != job.id);
    jobs.push(job.clone());
    save(&jobs)
}

pub fn remove(id: &str) -> std::io::Result<()> {
    let mut jobs = load();
    jobs.retain(|j| j.id != id);
    save(&jobs)
}

/// Mark a job as just-run (updates last_run and persists).
pub fn mark_run(id: &str) {
    let mut jobs = load();
    if let Some(j) = jobs.iter_mut().find(|j| j.id == id) {
        j.last_run = now_secs();
        let _ = save(&jobs);
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

    #[test]
    fn serialize_parse_roundtrip() {
        let j = sample();
        let back = parse(&serialize(&j)).unwrap();
        assert_eq!(back.id, j.id);
        assert_eq!(back.name, "Docs");
        assert_eq!(back.source, "C:/a");
        assert_eq!(back.target, "D:/b");
        assert_eq!(back.direction, Direction::AtoB);
        assert_eq!(back.conflict, ConflictMode::NewerWins);
        assert_eq!(back.retain_days, 7);
        assert_eq!(back.interval_min, 15);
        assert!(!back.include_hidden);
        assert_eq!(back.ignore, vec!["**/*.tmp".to_string(), "node_modules/**".to_string()]);
    }

    #[test]
    fn file_roundtrip_and_upsert_semantics() {
        let mut p = std::env::temp_dir();
        p.push(format!("jobs_{}_{}.tsv", std::process::id(), now_secs()));
        let mut a = SyncJob::new("A".into(), "s".into(), "t".into());
        let b = SyncJob::new("B".into(), "s2".into(), "t2".into());
        save_to(&p, &[a.clone(), b.clone()]).unwrap();
        let loaded = load_from(&p);
        assert_eq!(loaded.len(), 2);
        // replace by id
        a.name = "A2".into();
        let mut jobs = load_from(&p);
        jobs.retain(|j| j.id != a.id);
        jobs.push(a.clone());
        save_to(&p, &jobs).unwrap();
        let l2 = load_from(&p);
        assert_eq!(l2.iter().find(|j| j.id == a.id).unwrap().name, "A2");
        assert_eq!(l2.len(), 2);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn due_logic() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
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
    fn glob_set_matches_ignores() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.ignore = vec!["**/*.tmp".into(), "cache/**".into()];
        let gs = j.glob_set();
        assert!(gs.is_match("foo/bar.tmp"));
        assert!(gs.is_match("cache/x/y"));
        assert!(!gs.is_match("keep/me.txt"));
    }
}
