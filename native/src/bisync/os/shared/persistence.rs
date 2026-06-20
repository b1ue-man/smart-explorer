use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::{Baseline, Sig, Versioning, VersioningScheme};

// ── persistence (baseline TSV in appdata, keyed by the two roots) ────────────

fn app_data_dir() -> PathBuf {
    crate::support_dirs::sync_data_dir()
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

pub fn load_baseline(path: &Path) -> Baseline {
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

pub fn save_baseline(path: &Path, bl: &Baseline) -> io::Result<()> {
    let body: String = bl
        .iter()
        .map(|(rel, (a, b))| format!("{}\t{}\t{}", rel.replace('\t', " "), sig_str(a), sig_str(b)))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, body)
}

/// Prune the version snapshots per the configured scheme. Snapshots are the
/// timestamp-named subdirectories of the versions store.
pub fn prune_versions(versions: &Path, v: &Versioning) {
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
