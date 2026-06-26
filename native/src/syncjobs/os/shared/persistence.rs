use super::types::{SyncJob, Trigger};
use crate::bisync::{CompareMode, ConflictMode, DeletePolicy, Direction, VersioningScheme};
use std::path::PathBuf;

pub(super) fn app_data_dir() -> PathBuf {
    crate::support_dirs::sync_data_dir()
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

pub(super) fn job_file(dir: &std::path::Path, id: &str) -> PathBuf {
    dir.join(format!("{}.conf", san_id(id)))
}

/// Strip anything that could escape the filename (ids are hex, but be safe).
fn san_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Strip characters that would break the one-value-per-line format.
pub(super) fn san(s: &str) -> String {
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
    s.push_str(&format!("trigger={}\n", j.trigger.as_str()));
    s.push_str(&format!("cal_time_min={}\n", j.cal_time_min));
    s.push_str(&format!("cal_weekdays={}\n", j.cal_weekdays));
    s.push_str(&format!("cal_monthday={}\n", j.cal_monthday));
    s.push_str(&format!("rt_debounce_secs={}\n", j.rt_debounce_secs));
    s.push_str(&format!("connect_match={}\n", san(&j.connect_match)));
    s.push_str(&format!("active_from_min={}\n", j.active_from_min));
    s.push_str(&format!("active_to_min={}\n", j.active_to_min));
    s.push_str(&format!("catch_up={}\n", if j.catch_up { 1 } else { 0 }));
    s.push_str(&format!("delete_policy={}\n", j.delete_policy.as_str()));
    s.push_str(&format!(
        "move_files={}\n",
        if j.move_files { 1 } else { 0 }
    ));
    s.push_str(&format!("compare={}\n", j.compare.as_str()));
    s.push_str(&format!("modify_window_sec={}\n", j.modify_window_sec));
    s.push_str(&format!(
        "versioning_scheme={}\n",
        j.versioning_scheme.as_str()
    ));
    s.push_str(&format!("retain_count={}\n", j.retain_count));
    s.push_str(&format!(
        "use_recycle_bin={}\n",
        if j.use_recycle_bin { 1 } else { 0 }
    ));
    s.push_str(&format!("max_delete={}\n", j.max_delete));
    s.push_str(&format!("max_delete_pct={}\n", j.max_delete_pct));
    s.push_str(&format!("filter_min_size_kb={}\n", j.filter_min_size_kb));
    s.push_str(&format!("filter_max_size_kb={}\n", j.filter_max_size_kb));
    s.push_str(&format!("filter_max_age_days={}\n", j.filter_max_age_days));
    s.push_str(&format!("filter_min_age_days={}\n", j.filter_min_age_days));
    s.push_str(&format!("bwlimit_kbps={}\n", j.bwlimit_kbps));
    s.push_str(&format!("max_transfers={}\n", j.max_transfers));
    s.push_str(&format!(
        "atomic_copy={}\n",
        if j.atomic_copy { 1 } else { 0 }
    ));
    s.push_str(&format!("verify={}\n", if j.verify { 1 } else { 0 }));
    s.push_str(&format!("retries={}\n", j.retries));
    s.push_str(&format!("retry_delay_secs={}\n", j.retry_delay_secs));
    s.push_str(&format!("run_before={}\n", san(&j.run_before)));
    s.push_str(&format!("run_after={}\n", san(&j.run_after)));
    s
}

/// Parse one `key=value` job block. Tolerant: unknown keys ignored, missing keys
/// take defaults, repeated `ignore=` lines accumulate.
pub(super) fn parse_kv(body: &str) -> Option<SyncJob> {
    let mut j = SyncJob::new(String::new(), String::new(), String::new());
    j.id.clear();
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
            "ignore" if !v.is_empty() => {
                j.ignore.push(v.to_string());
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
            "versioning_scheme" => {
                j.versioning_scheme = VersioningScheme::parse(v).unwrap_or(VersioningScheme::Days)
            }
            "retain_count" => j.retain_count = v.parse().unwrap_or(0),
            "use_recycle_bin" => j.use_recycle_bin = v != "0",
            "max_delete" => j.max_delete = v.parse().unwrap_or(0),
            "max_delete_pct" => j.max_delete_pct = v.parse().unwrap_or(0),
            "filter_min_size_kb" => j.filter_min_size_kb = v.parse().unwrap_or(0),
            "filter_max_size_kb" => j.filter_max_size_kb = v.parse().unwrap_or(0),
            "filter_max_age_days" => j.filter_max_age_days = v.parse().unwrap_or(0),
            "filter_min_age_days" => j.filter_min_age_days = v.parse().unwrap_or(0),
            "bwlimit_kbps" => j.bwlimit_kbps = v.parse().unwrap_or(0),
            "max_transfers" => j.max_transfers = v.parse().unwrap_or(0),
            "atomic_copy" => j.atomic_copy = v != "0",
            "verify" => j.verify = v != "0",
            "retries" => j.retries = v.parse().unwrap_or(0),
            "retry_delay_secs" => j.retry_delay_secs = v.parse().unwrap_or(2),
            "run_before" => j.run_before = v.to_string(),
            "run_after" => j.run_after = v.to_string(),
            _ => {}
        }
    }
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
    j.trigger = if j.interval_min > 0 {
        Trigger::Interval
    } else {
        Trigger::Manual
    };
    Some(j)
}

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
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    out
}

pub(super) fn write_job(dir: &std::path::Path, job: &SyncJob) -> std::io::Result<()> {
    std::fs::write(job_file(dir, &job.id), serialize_kv(job))
}

fn save_dir(dir: &std::path::Path, jobs: &[SyncJob]) -> std::io::Result<()> {
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

pub fn load() -> Vec<SyncJob> {
    let dir = jobs_dir();
    let jobs = load_dir(&dir);
    if jobs.is_empty() {
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

/// Add or replace a job (by id) - rewrites just that job's file.
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
        let body = "id=abc\nname=X\nsource=s\ntarget=t\nfuture_option=42\n";
        let j = parse_kv(body).unwrap();
        assert_eq!(j.id, "abc");
        assert_eq!(j.name, "X");
        assert_eq!(j.direction, Direction::Both);
        assert_eq!(j.retain_days, 30);
        assert!(j.ignore.is_empty());
        assert!(parse_kv("name=NoId\n").is_none());
    }

    #[test]
    fn dir_store_roundtrip_upsert_and_remove() {
        let dir = temp_dir();
        let a = SyncJob::new("A".into(), "s".into(), "t".into());
        let b = SyncJob::new("B".into(), "s2".into(), "t2".into());
        save_dir(&dir, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(load_dir(&dir).len(), 2);

        let mut a2 = a.clone();
        a2.name = "A2".into();
        write_job(&dir, &a2).unwrap();
        let l2 = load_dir(&dir);
        assert_eq!(l2.iter().find(|j| j.id == a.id).unwrap().name, "A2");
        assert_eq!(l2.len(), 2);

        save_dir(&dir, std::slice::from_ref(&b)).unwrap();
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
}
