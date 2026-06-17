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
    s
}

/// Parse one `key=value` job block. Tolerant: unknown keys ignored, missing keys
/// take defaults, repeated `ignore=` lines accumulate.
fn parse_kv(body: &str) -> Option<SyncJob> {
    let mut j = SyncJob::new(String::new(), String::new(), String::new());
    j.id.clear(); // require an explicit `id=` line
    j.ignore.clear();
    let mut saw_any = false;
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
            _ => {} // unknown / future key — ignored
        }
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
