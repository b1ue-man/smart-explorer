//! The folder of vault sync jobs, `<SYNC>/vault-jobs/<id>.conf` (vault plan
//! C5.8): the same file format as `jobs/` plus `kind=vault`, `vault_site_a`
//! and `vault_site_b`. Older versions never read this folder, so their
//! job list, their runs and their files stay exactly as they were.
//!
//! Every caller loads it tolerantly: a vault job that cannot be read is
//! listed as broken and never empties the job list of `jobs/`, whose load
//! semantics stay unchanged for each caller. Job IDs are unique over both
//! folders.
use std::io;
use std::path::{Path, PathBuf};

use super::persistence::{
    app_data_dir, job_file, jobs_dir, load_job_file, san_id, write_job, BrokenJob, JobLoadReport,
};
use super::types::SyncJob;

/// Folder name of the vault sync jobs below `<SYNC>`.
pub const VAULT_JOBS_DIR: &str = "vault-jobs";

/// `<SYNC>/vault-jobs`.
pub fn vault_jobs_dir() -> PathBuf {
    app_data_dir().join(VAULT_JOBS_DIR)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// One vault job file: a job of another kind there is broken.
pub(super) fn load_vault_job_file(path: &Path) -> io::Result<SyncJob> {
    let job = load_job_file(path)?;
    if !job.is_vault() {
        return Err(invalid(format!(
            "kein Tresor-Job (kind=vault fehlt): {}",
            path.display()
        )));
    }
    Ok(job)
}

/// Every vault job of `dir` on its own (C5.8): valid ones sorted by name,
/// unreadable ones listed. A missing folder holds no job.
pub(super) fn load_report_in(dir: &Path) -> io::Result<JobLoadReport> {
    let mut report = JobLoadReport::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error),
    };
    for entry in entries {
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
        match load_vault_job_file(&path) {
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
    report.jobs.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(report)
}

/// The vault jobs of `dir` for a caller without a broken list: an
/// unreadable folder or job is left out (it shows up in `load_report`).
pub(super) fn load_valid_in(dir: &Path) -> Vec<SyncJob> {
    load_report_in(dir)
        .map(|report| report.jobs)
        .unwrap_or_default()
}

/// The vault job `id`, if its file exists.
pub(super) fn find_in(dir: &Path, id: &str) -> io::Result<Option<SyncJob>> {
    if id.is_empty() || id != san_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe sync job id",
        ));
    }
    match load_vault_job_file(&job_file(dir, id)) {
        Ok(job) => Ok(Some(job)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The vault job `id` of this installation.
pub fn load_vault_job(id: &str) -> io::Result<Option<SyncJob>> {
    find_in(&vault_jobs_dir(), id)
}

/// Whether a job file `<id>.conf` exists in `dir` (readable or not).
fn has_file(dir: &Path, id: &str) -> bool {
    job_file(dir, id).symlink_metadata().is_ok()
}

/// Writes a vault job to `vault_dir`, refused when the job is no vault job
/// or when `general_dir` holds a job with its ID (unique over both).
pub(super) fn write_in(general_dir: &Path, vault_dir: &Path, job: &SyncJob) -> io::Result<()> {
    if !job.is_vault() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nur Tresor-Jobs gehören nach vault-jobs/",
        ));
    }
    if has_file(general_dir, &job.id) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Job-ID ist schon an einen Sync-Job vergeben",
        ));
    }
    ensure_regular_dir(vault_dir)?;
    write_job(vault_dir, job)
}

/// Whether a general job with `id` may be written: no vault job holds it.
pub(super) fn general_id_free(vault_dir: &Path, id: &str) -> io::Result<()> {
    if has_file(vault_dir, id) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Job-ID ist schon an einen Tresor-Job vergeben",
        ));
    }
    Ok(())
}

/// Removes the vault job file `id` from `dir`; a missing one is no error.
pub(super) fn remove_in(dir: &Path, id: &str) -> io::Result<()> {
    match std::fs::remove_file(job_file(dir, id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Whether `id` names a job in `jobs/` or `vault-jobs/` of this installation
/// (C5.8: new IDs are checked against both folders).
pub fn job_id_taken(id: &str) -> bool {
    has_file(&jobs_dir(), id) || has_file(&vault_jobs_dir(), id)
}

fn ensure_regular_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let metadata = std::fs::symlink_metadata(dir)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "vault job path is not a regular directory: {}",
            dir.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "vault_jobs_dir_tests.rs"]
mod tests;
