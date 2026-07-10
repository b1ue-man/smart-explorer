use super::persistence::{atomic_write, load_dir, read_regular_utf8, san_id, write_job};
use super::persistence_codec::{parse_legacy, serialize_kv};
use super::types::SyncJob;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

const LEGACY_LIMIT_BYTES: u64 = 16 * 1024 * 1024;
const MARKER_LIMIT_BYTES: u64 = 1024 * 1024;
const MARKER_NAME: &str = ".legacy-import.pending";
const MARKER_HEADER: &str = "smart-explorer-legacy-import-v1";

pub(super) fn load_or_migrate(directory: &Path, legacy: &Path) -> io::Result<Vec<SyncJob>> {
    let marker = directory.join(MARKER_NAME);
    let marker_exists = exists_checked(&marker)?;
    let legacy_exists = exists_checked(legacy)?;

    match (marker_exists, legacy_exists) {
        (false, false) => load_dir(directory),
        (true, false) => recover_archived_import(directory, &marker),
        (_, true) => import_legacy(directory, legacy, &marker, marker_exists),
    }
}

fn import_legacy(
    directory: &Path,
    legacy: &Path,
    marker: &Path,
    marker_exists: bool,
) -> io::Result<Vec<SyncJob>> {
    let body = read_regular_utf8(legacy, LEGACY_LIMIT_BYTES, "legacy sync-job file")?;
    let imported = parse_legacy_jobs(&body)?;
    let expected = expected_entries(&imported)?;
    let marker_body = serialize_marker(&expected);

    if marker_exists {
        let persisted = read_marker(marker)?;
        if persisted != expected {
            return Err(invalid_data(
                "legacy sync-job import changed while a migration was pending",
            ));
        }
    } else {
        preflight_existing(directory, &expected)?;
        atomic_write(marker, marker_body.as_bytes())?;
    }

    let existing = jobs_by_id(load_dir(directory)?)?;
    for job in &imported {
        if existing.contains_key(&job.id) {
            continue;
        }
        write_job(directory, job)?;
    }

    let verified = load_dir(directory)?;
    verify_entries(&verified, &expected)?;
    let archive = archive_path(legacy);
    super::platform::rename_no_replace(legacy, &archive)?;
    if let Some(parent) = legacy.parent() {
        super::platform::sync_parent(parent)?;
    }
    std::fs::remove_file(marker)?;
    super::platform::sync_parent(directory)?;
    Ok(verified)
}

fn recover_archived_import(directory: &Path, marker: &Path) -> io::Result<Vec<SyncJob>> {
    let expected = read_marker(marker)?;
    let jobs = load_dir(directory)?;
    verify_entries(&jobs, &expected)?;
    std::fs::remove_file(marker)?;
    super::platform::sync_parent(directory)?;
    Ok(jobs)
}

fn parse_legacy_jobs(body: &str) -> io::Result<Vec<SyncJob>> {
    let mut jobs = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let job = parse_legacy(line).ok_or_else(|| {
            invalid_data(format!("legacy sync-job line {} is invalid", index + 1))
        })?;
        if job.id != san_id(&job.id) {
            return Err(invalid_data(format!(
                "legacy sync-job line {} has an unsafe id",
                index + 1
            )));
        }
        if !ids.insert(job.id.clone()) {
            return Err(invalid_data(format!(
                "legacy sync-job file contains duplicate id {:?}",
                job.id
            )));
        }
        jobs.push(job);
    }
    Ok(jobs)
}

fn expected_entries(jobs: &[SyncJob]) -> io::Result<BTreeMap<String, String>> {
    let mut expected = BTreeMap::new();
    for job in jobs {
        job.validate()
            .map_err(|error| invalid_data(format!("invalid legacy sync job: {error}")))?;
        if expected
            .insert(job.id.clone(), canonical_hash(job))
            .is_some()
        {
            return Err(invalid_data(format!(
                "legacy sync-job file contains duplicate id {:?}",
                job.id
            )));
        }
    }
    Ok(expected)
}

fn preflight_existing(directory: &Path, expected: &BTreeMap<String, String>) -> io::Result<()> {
    let existing = jobs_by_id(load_dir(directory)?)?;
    for (id, job) in existing {
        match expected.get(&id) {
            Some(hash) if *hash == canonical_hash(&job) => {}
            Some(_) => {
                return Err(invalid_data(format!(
                    "saved sync job {id:?} conflicts with the pending legacy import"
                )))
            }
            None => {
                return Err(invalid_data(format!(
                    "saved sync job {id:?} is not part of the pending legacy import"
                )))
            }
        }
    }
    Ok(())
}

fn verify_entries(jobs: &[SyncJob], expected: &BTreeMap<String, String>) -> io::Result<()> {
    let actual = jobs_by_id(jobs.to_vec())?;
    for (id, hash) in expected {
        let job = actual.get(id).ok_or_else(|| {
            invalid_data(format!(
                "legacy sync-job import is incomplete; missing {id:?}"
            ))
        })?;
        if canonical_hash(job) != *hash {
            return Err(invalid_data(format!(
                "legacy sync-job import verification failed for {id:?}"
            )));
        }
    }
    Ok(())
}

fn jobs_by_id(jobs: Vec<SyncJob>) -> io::Result<BTreeMap<String, SyncJob>> {
    let mut by_id = BTreeMap::new();
    for job in jobs {
        let id = job.id.clone();
        if by_id.insert(id.clone(), job).is_some() {
            return Err(invalid_data(format!("duplicate saved sync-job id {id:?}")));
        }
    }
    Ok(by_id)
}

fn canonical_hash(job: &SyncJob) -> String {
    format!("{:x}", Sha256::digest(serialize_kv(job).as_bytes()))
}

fn serialize_marker(expected: &BTreeMap<String, String>) -> String {
    let mut body = String::from(MARKER_HEADER);
    body.push('\n');
    for (id, hash) in expected {
        body.push_str(id);
        body.push('\t');
        body.push_str(hash);
        body.push('\n');
    }
    body
}

fn read_marker(marker: &Path) -> io::Result<BTreeMap<String, String>> {
    let body = read_regular_utf8(marker, MARKER_LIMIT_BYTES, "legacy import marker")?;
    let mut lines = body.lines();
    if lines.next() != Some(MARKER_HEADER) {
        return Err(invalid_data("legacy import marker has an unknown format"));
    }
    let mut entries = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let (id, hash) = line.split_once('\t').ok_or_else(|| {
            invalid_data(format!(
                "legacy import marker line {} is invalid",
                index + 2
            ))
        })?;
        if id != san_id(id)
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || entries
                .insert(id.to_string(), hash.to_ascii_lowercase())
                .is_some()
        {
            return Err(invalid_data(format!(
                "legacy import marker line {} is invalid",
                index + 2
            )));
        }
    }
    Ok(entries)
}

fn archive_path(legacy: &Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    legacy.with_extension(format!("tsv.imported.{}.{}", std::process::id(), nonce))
}

fn exists_checked(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_line(id: &str, name: &str) -> String {
        [
            id, name, "/a", "/b", "both", "strict", "30", "15", "1", "", "0", "1",
        ]
        .join("\t")
    }

    #[test]
    fn malformed_legacy_input_is_not_partially_imported_or_retired() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("jobs");
        std::fs::create_dir(&directory).unwrap();
        let legacy = root.path().join("jobs.tsv");
        std::fs::write(&legacy, format!("{}\ninvalid\n", legacy_line("one", "One"))).unwrap();

        assert_eq!(
            load_or_migrate(&directory, &legacy).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(legacy.exists());
        assert!(load_dir(&directory).unwrap().is_empty());
        assert!(!directory.join(MARKER_NAME).exists());
    }

    #[test]
    fn verified_import_retires_legacy_only_after_all_jobs_exist() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("jobs");
        std::fs::create_dir(&directory).unwrap();
        let legacy = root.path().join("jobs.tsv");
        std::fs::write(
            &legacy,
            format!(
                "{}\n{}\n",
                legacy_line("one", "One"),
                legacy_line("two", "Two")
            ),
        )
        .unwrap();

        let jobs = load_or_migrate(&directory, &legacy).unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(!legacy.exists());
        assert!(!directory.join(MARKER_NAME).exists());
        assert_eq!(load_dir(&directory).unwrap().len(), 2);
    }
}
