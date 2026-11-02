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
        out.push(load_general_job_file(&path)?);
    }
    sort_jobs(&mut out);
    Ok(out)
}

fn sort_jobs(jobs: &mut [SyncJob]) {
    jobs.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
}

/// Every job file loaded on its own (Y20): one broken file stops only itself.
#[derive(Clone, Debug, Default)]
pub struct JobLoadReport {
    pub jobs: Vec<SyncJob>,
    pub broken: Vec<BrokenJob>,
}

/// A job file (or the pending import of the old `jobs.tsv`) that could not
/// be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokenJob {
    /// The job id from the file name ("" for the import or an unreadable
    /// directory entry).
    pub id: String,
    pub path: PathBuf,
    pub error: String,
}

fn load_dir_report(dir: &Path) -> io::Result<JobLoadReport> {
    let mut report = JobLoadReport::default();
    for entry in std::fs::read_dir(dir)? {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                report.broken.push(BrokenJob {
                    id: String::new(),
                    path: dir.to_path_buf(),
                    error: error.to_string(),
                });
                continue;
            }
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("conf") {
            continue;
        }
        match load_general_job_file(&path) {
            Ok(job) => report.jobs.push(job),
            Err(error) => report.broken.push(BrokenJob {
                id: path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default()
                    .to_string(),
                path,
                error: error.to_string(),
            }),
        }
    }
    sort_jobs(&mut report.jobs);
    Ok(report)
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

/// A job file of `jobs/`: a vault job there is refused, it belongs to
/// `vault-jobs/` (C5.8).
fn load_general_job_file(path: &Path) -> io::Result<SyncJob> {
    let job = load_job_file(path)?;
    if job.is_vault() {
        return Err(invalid_data(format!(
            "Tresor-Job gehört nach vault-jobs/: {}",
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

/// Every job: those of `jobs/` strictly (one broken file fails the list, as
/// before), then the valid vault jobs of `vault-jobs/` (C5.8: a broken
/// vault job never empties the list; `load_report` names it).
pub fn load() -> io::Result<Vec<SyncJob>> {
    let directory = ensure_jobs_dir()?;
    let mut jobs = super::migration::load_or_migrate(&directory, &jobs_path())?;
    super::baseline_migration::migrate(&mut jobs)?;
    jobs.extend(super::vault_jobs_dir::load_valid_in(
        &super::vault_jobs_dir::vault_jobs_dir(),
    ));
    Ok(jobs)
}

/// Like [`load`], but every job file on its own: valid jobs are returned and
/// broken files are listed instead of failing the whole list (Y20). A failed
/// pending import of the old `jobs.tsv` is listed as well.
pub fn load_report() -> io::Result<JobLoadReport> {
    let directory = ensure_jobs_dir()?;
    let import = super::migration::complete_pending_import(&directory, &jobs_path());
    let mut report = load_dir_report(&directory)?;
    let mut jobs = Vec::with_capacity(report.jobs.len());
    for mut job in std::mem::take(&mut report.jobs) {
        // While the original import is unresolved, changing its core fields
        // would invalidate its manifest hash. The running job gets the safe
        // defaults and eligibility proof; its old file stays byte-identical.
        match super::baseline_migration::migrate_for_report(
            std::slice::from_mut(&mut job),
            import.is_ok(),
        ) {
            Ok(()) => jobs.push(job),
            Err(error) => report.broken.push(BrokenJob {
                path: job_file(&directory, &job.id),
                id: job.id,
                error: format!("Löschschutz-Migration: {error}"),
            }),
        }
    }
    report.jobs = jobs;
    if let Err(error) = import {
        report.broken.push(BrokenJob {
            id: String::new(),
            path: jobs_path(),
            error: error.to_string(),
        });
    }
    // Vault jobs after the general ones; an unreadable folder is one entry.
    let vault_dir = super::vault_jobs_dir::vault_jobs_dir();
    match super::vault_jobs_dir::load_report_in(&vault_dir) {
        Ok(vault) => {
            report.jobs.extend(vault.jobs);
            report.broken.extend(vault.broken);
        }
        Err(error) => report.broken.push(BrokenJob {
            id: String::new(),
            path: vault_dir,
            error: error.to_string(),
        }),
    }
    Ok(report)
}

/// Add or replace a job (by id) - rewrites just that job's file.
/// A vault job goes to `vault-jobs/`, every other job to `jobs/`; an ID
/// is never used in both folders (C5.8).
pub fn upsert(job: &SyncJob) -> io::Result<()> {
    let directory = ensure_jobs_dir()?;
    let vault_dir = super::vault_jobs_dir::vault_jobs_dir();
    if job.is_vault() {
        super::vault_jobs_dir::write_in(&directory, &vault_dir, job)?;
    } else {
        super::vault_jobs_dir::general_id_free(&vault_dir, &job.id)?;
        write_job(&directory, job)?;
    }
    super::job_state_store::recheck_after_edit(&job.id);
    Ok(())
}

/// Removes a job's configuration and its runtime state (one deletion path
/// for every surface). Its synchronization state and versions are offered
/// separately (`bisync::forget_job_state`, `bisync::versions::remove_versions`).
pub fn remove(id: &str) -> io::Result<()> {
    if id != san_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe sync job id",
        ));
    }
    match std::fs::remove_file(job_file(&jobs_dir(), id)) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    super::vault_jobs_dir::remove_in(&super::vault_jobs_dir::vault_jobs_dir(), id)?;
    super::baseline_migration::remove_pending(id)?;
    super::job_state_store::remove_job_state(id)
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
#[path = "persistence_tests.rs"]
mod tests;
