#[cfg(test)]
use super::persistence_codec::parse_kv;
pub(super) use super::persistence_codec::san;
use super::persistence_codec::{parse_kv_checked, serialize_kv};
use super::types::SyncJob;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_JOB_FILE_BYTES: u64 = 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn app_data_dir() -> PathBuf {
    crate::support_dirs::sync_data_dir()
}

/// Legacy single-file store (positional TSV), kept only for one-time import.
pub fn jobs_path() -> PathBuf {
    app_data_dir().join("jobs.tsv")
}

/// Directory holding one `<id>.conf` per job.
pub fn jobs_dir() -> PathBuf {
    app_data_dir().join("jobs")
}

fn ensure_jobs_dir() -> io::Result<PathBuf> {
    let directory = jobs_dir();
    std::fs::create_dir_all(&directory)?;
    let metadata = std::fs::symlink_metadata(&directory)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "sync jobs path is not a regular directory: {}",
                directory.display()
            ),
        ));
    }
    Ok(directory)
}

pub(super) fn job_file(dir: &std::path::Path, id: &str) -> PathBuf {
    dir.join(format!("{}.conf", san_id(id)))
}

/// Strip anything that could escape the filename (ids are hex, but be safe).
pub(super) fn san_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

pub(super) fn load_dir(dir: &Path) -> io::Result<Vec<SyncJob>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("conf") {
            continue;
        }
        out.push(load_job_file(&path)?);
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(out)
}

pub(super) fn load_job_file(path: &Path) -> io::Result<SyncJob> {
    let body = read_regular_utf8(path, MAX_JOB_FILE_BYTES, "sync job configuration")?;
    let job = parse_kv_checked(&body).map_err(|error| {
        invalid_data(format!(
            "invalid sync job configuration {}: {error}",
            path.display()
        ))
    })?;
    let expected_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| invalid_data("configuration filename is not valid UTF-8"))?;
    if job.id != expected_id {
        return Err(invalid_data(format!(
            "sync job id {:?} does not match filename {:?}: {}",
            job.id,
            expected_id,
            path.display()
        )));
    }
    Ok(job)
}

pub(super) fn write_job(dir: &Path, job: &SyncJob) -> io::Result<()> {
    job.validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if job.id != san_id(&job.id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sync job id contains characters that are unsafe in filenames",
        ));
    }
    atomic_write(&job_file(dir, &job.id), serialize_kv(job).as_bytes())
}

#[cfg(test)]
fn save_dir(dir: &Path, jobs: &[SyncJob]) -> io::Result<()> {
    for job in jobs {
        job.validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if job.id != san_id(&job.id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe sync job id",
            ));
        }
    }
    let keep: Vec<String> = jobs.iter().map(|j| san_id(&j.id)).collect();
    for j in jobs {
        write_job(dir, j)?;
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("conf") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| invalid_data("sync job filename is not valid UTF-8"))?;
        if !keep.iter().any(|id| id == stem) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn load() -> io::Result<Vec<SyncJob>> {
    let directory = ensure_jobs_dir()?;
    super::migration::load_or_migrate(&directory, &jobs_path())
}

/// Add or replace a job (by id) - rewrites just that job's file.
pub fn upsert(job: &SyncJob) -> io::Result<()> {
    let directory = ensure_jobs_dir()?;
    write_job(&directory, job)
}

pub fn remove(id: &str) -> io::Result<()> {
    if id != san_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe sync job id",
        ));
    }
    match std::fs::remove_file(job_file(&jobs_dir(), id)) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub(super) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut last_collision = None;
    for _ in 0..16 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{}.{}.{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("sync-job"),
            std::process::id(),
            sequence
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.write_all(contents)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            super::platform::atomic_replace(&temp, path)?;
            super::platform::sync_parent(parent)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        return result;
    }
    Err(last_collision.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::AlreadyExists, "temporary filename collision")
    }))
}

pub(super) fn read_regular_utf8(path: &Path, limit: u64, label: &str) -> io::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(invalid_data(format!(
            "{label} is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > limit {
        return Err(invalid_data(format!(
            "{label} exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| invalid_data(format!("{label} size does not fit this platform")))?;
    let mut bytes = Vec::with_capacity(capacity);
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid_data(format!(
            "{label} exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|error| invalid_data(format!("{label} is not UTF-8: {error}")))
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bisync::{ConflictMode, Direction};
    use crate::syncjobs::persistence_codec::parse_legacy;

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
        assert_eq!(load_dir(&dir).unwrap().len(), 2);

        let mut a2 = a.clone();
        a2.name = "A2".into();
        write_job(&dir, &a2).unwrap();
        let l2 = load_dir(&dir).unwrap();
        assert_eq!(l2.iter().find(|j| j.id == a.id).unwrap().name, "A2");
        assert_eq!(l2.len(), 2);

        save_dir(&dir, std::slice::from_ref(&b)).unwrap();
        let l3 = load_dir(&dir).unwrap();
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
    fn invalid_or_mismatched_config_is_never_loaded() {
        let dir = temp_dir();
        let mut invalid = SyncJob::new("bad".into(), "s".into(), "t".into());
        invalid.max_delete_pct = 101;
        assert_eq!(
            write_job(&dir, &invalid).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );

        let valid = SyncJob::new("good".into(), "s".into(), "t".into());
        std::fs::write(dir.join("different.conf"), serialize_kv(&valid)).unwrap();
        assert!(load_dir(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_preflights_every_job_before_removing_old_files() {
        let dir = temp_dir();
        let old = SyncJob::new("old".into(), "s".into(), "t".into());
        save_dir(&dir, std::slice::from_ref(&old)).unwrap();
        let mut invalid = SyncJob::new("bad".into(), "x".into(), "y".into());
        invalid.max_delete_pct = 101;
        assert!(save_dir(&dir, &[invalid]).is_err());
        assert_eq!(load_dir(&dir).unwrap().len(), 1);
        assert_eq!(load_dir(&dir).unwrap()[0].id, old.id);
        std::fs::remove_dir_all(&dir).ok();
    }
}
