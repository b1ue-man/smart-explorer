//! One-time RV1 defaults and proof that a saved job may inherit its old
//! pair-wide baseline. Raw import parsing/hashing keeps its original meaning.
use std::io;
use std::path::PathBuf;

use super::persistence::{atomic_write, job_file, jobs_dir, load_job_file, read_regular_utf8, san_id, write_job};
use super::types::{SyncJob, CURRENT_CONFIG_VERSION};

pub(super) fn migrate(jobs: &mut [SyncJob]) -> io::Result<()> {
    migrate_for_report(jobs, true)
}

pub(super) fn migrate_for_report(jobs: &mut [SyncJob], persist: bool) -> io::Result<()> {
    migrate_at(jobs, &jobs_dir(), &super::persistence::app_data_dir().join("legacy-baseline-jobs"), persist)
}

pub(super) fn migrate_at(jobs: &mut [SyncJob], jobs_dir: &std::path::Path, pending_dir: &std::path::Path, persist: bool) -> io::Result<()> {
    for job in jobs {
        if job.config_version != 0 { continue; }
        pending_path(&job.id)?; // Validate without changing the stored locator.
        let path = pending_dir.join(format!("{}.pending", job.id));
        if !path.try_exists()? {
            let endpoints = serde_json::to_vec(&(job.source.clone(), job.target.clone())).map_err(io::Error::other)?;
            atomic_write(&path, &endpoints)?;
        }
        upgrade_defaults(job);
        if persist { write_job(jobs_dir, job)?; }
    }
    Ok(())
}

pub(super) fn upgrade_defaults(job: &mut SyncJob) {
    if job.config_version == 0 {
        if job.max_delete_pct == 0 {
            job.max_delete_pct = 50;
            job.max_delete_min = 25;
        } else {
            // An existing explicit threshold keeps the original effect.
            job.max_delete_min = 0;
        }
        job.config_version = CURRENT_CONFIG_VERSION;
    }
}

pub(crate) fn legacy_baseline_pending(id: &str) -> io::Result<bool> {
    let path = pending_path(id)?;
    let current = match load_job_file(&job_file(&jobs_dir(), id)) {
        Ok(job) => job,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if current.config_version == 0 { return Ok(true); }
    match read_regular_utf8(&path, 1024 * 1024, "legacy baseline eligibility") {
        Ok(body) => {
            let (source, target): (String, String) = serde_json::from_str(&body).map_err(io::Error::other)?;
            Ok(source == current.source && target == current.target)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn remove_pending(id: &str) -> io::Result<()> {
    match std::fs::remove_file(pending_path(id)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn pending_path(id: &str) -> io::Result<PathBuf> {
    if id.is_empty() || id != san_id(id) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe sync job id"));
    }
    Ok(super::persistence::app_data_dir().join("legacy-baseline-jobs").join(format!("{id}.pending")))
}

#[cfg(test)]
pub(super) fn migrate_defaults_for_test(job: &mut SyncJob) { upgrade_defaults(job); }
