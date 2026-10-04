//! C08 migration and editing retain executable saved-job identity and state.
use crate::daemon::sync_reliability_task_old_jobs_tests::SavedJob;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

struct HistoricalFile {
    path: PathBuf,
    before: Option<Vec<u8>>,
    written: Vec<u8>,
}

impl HistoricalFile {
    fn write(path: PathBuf, bytes: Vec<u8>) -> Self {
        let before = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("read historical fixture input: {error}"),
        };
        let mut written = before.clone().unwrap_or_default();
        if !written.is_empty() && !written.ends_with(b"\n") {
            written.push(b'\n');
        }
        written.extend_from_slice(&bytes);
        std::fs::write(&path, &written).unwrap();
        Self { path, before, written }
    }
}

impl Drop for HistoricalFile {
    fn drop(&mut self) {
        let result = if let Some(before) = &self.before {
            match std::fs::read(&self.path) {
                Ok(current) if current == self.written => std::fs::write(&self.path, before),
                Ok(_) => Err(std::io::Error::other("historical fixture changed by another writer; preserve it")),
                Err(error) => Err(error),
            }
        } else {
            match std::fs::read(&self.path) {
                Ok(current) if current == self.written => std::fs::remove_file(&self.path),
                Ok(_) => Err(std::io::Error::other("historical fixture changed by another writer; preserve it")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        };
        if let Err(error) = result {
            if std::thread::panicking() {
                eprintln!("C08 historical fixture cleanup: {error}");
            } else {
                panic!("C08 historical fixture cleanup: {error}");
            }
        }
    }
}

pub(super) fn root_path(path: &Path) -> String {
    path.to_str().unwrap().replace('\\', "/")
}

fn contains_bytes(root: &Path, expected: &[u8]) -> bool {
    std::fs::read_dir(root).unwrap().any(|entry| {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            contains_bytes(&entry.path(), expected)
        } else {
            kind.is_file() && std::fs::read(entry.path()).unwrap() == expected
        }
    })
}

#[test]
fn sync_reliability_task_old_jobs_tsv_and_text_baseline_import_execute_same_job() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("source root");
    let b = temp.path().join("target root");
    let configs = temp.path().join("historical jobs");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::create_dir(&configs).unwrap();
    std::fs::write(a.join("deleted-on-b.txt"), b"old counterpart deleted at B").unwrap();
    std::fs::write(a.join("source-only.txt"), b"source bytes after import").unwrap();
    std::fs::write(b.join("target-only.txt"), b"target bytes after import").unwrap();
    let mut nonce = [0u8; 8];
    getrandom::getrandom(&mut nonce).unwrap();
    let id = format!("c08_tsv_{:016x}", u64::from_be_bytes(nonce));
    let root_a = root_path(&a);
    let root_b = root_path(&b);
    let row = [&*id, "historical TSV job", &*root_a, &*root_b, "both", "strict", "17", "1", "0",
        "*.skip\u{1f}ignored/**", "1700000000", "1"].join("\t");
    let legacy_jobs = temp.path().join("jobs.tsv");
    std::fs::write(&legacy_jobs, format!("{row}\n")).unwrap();
    let imported = super::migration::load_or_migrate(&configs, &legacy_jobs).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].id, id);
    assert_eq!(imported[0].config_version, 0);
    assert_eq!((&*imported[0].source, &*imported[0].target), (&*root_a, &*root_b));
    assert!(!legacy_jobs.exists());
    let archives: Vec<_> = std::fs::read_dir(temp.path()).unwrap().map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("jobs.tsv.imported.")).collect();
    assert_eq!(archives.len(), 1, "discover the actual durable legacy archive");
    assert_eq!(std::fs::read(&archives[0]).unwrap(), format!("{row}\n").as_bytes());
    let saved = SavedJob::with_body(id.clone(), super::persistence_codec::serialize_kv_core(&imported[0]));
    let job = saved.load();
    assert_eq!(job.id, id);
    assert_eq!((job.max_delete_pct, job.max_delete_min), (50, 25));
    assert!(job.cross_mounts);
    assert_eq!(job.last_run, 1_700_000_000);
    let old_result = format!("{id}\t1700000001\t9\t2\t1\t0\t0\told successful result\n");
    let result_fixture = HistoricalFile::write(crate::support_dirs::sync_data_dir().join("results.tsv"), old_result.into_bytes());
    let seeded = crate::syncjobs::load_job_state(&id).unwrap();
    assert_eq!(seeded.last_success, Some(1_700_000_000));
    assert_eq!(seeded.last_attempt, Some(1_700_000_001));
    assert_eq!(seeded.last_result.as_ref().unwrap().a_to_b, 9);

    let local_a = crate::vfs::LocalBackend::new(&root_a);
    let local_b = crate::vfs::LocalBackend::new(&root_b);
    let pair = crate::bisync::pair_id_for(&local_a, &root_a, &local_b, &root_b);
    let metadata = std::fs::metadata(a.join("deleted-on-b.txt")).unwrap();
    let mtime = metadata.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let sig = format!("{}:{mtime}:0", metadata.len());
    let baseline_fixture = HistoricalFile::write(crate::bisync::baseline_path(&pair),
        format!("deleted-on-b.txt\t{sig}\t{sig}\n").into_bytes());
    assert!(crate::syncjobs::legacy_baseline_pending(&id).unwrap());
    crate::daemon::sync_reliability_task_old_jobs_tests::run_saved_job(&job, &AtomicBool::new(false));
    let state = crate::syncjobs::load_job_state(&id).unwrap();
    assert!(state.last_error.is_none() && state.blocked.is_none(), "{state:?}");
    assert_eq!(std::fs::read(&result_fixture.path).unwrap(), result_fixture.written,
        "current attempts write job-state JSON and retain the old TSV source");
    assert_eq!(std::fs::read(&baseline_fixture.path).unwrap(), baseline_fixture.written,
        "migration copies the old text source into the job-owned baseline");
    assert_eq!(state.last_runner, Some(crate::syncjobs::Runner::Daemon));
    assert_eq!(state.last_result.as_ref().unwrap().deleted, 1,
        "the old baseline must interpret B's missing file as a deletion, not first-run copy");
    assert!(!a.join("deleted-on-b.txt").exists() && !b.join("deleted-on-b.txt").exists());
    assert_eq!(std::fs::read(a.join("target-only.txt")).unwrap(), b"target bytes after import");
    assert_eq!(std::fs::read(b.join("source-only.txt")).unwrap(), b"source bytes after import");
    assert!(contains_bytes(temp.path(), b"old counterpart deleted at B"));
    let key = saved.key(&local_a, &root_a, &local_b, &root_b);
    assert_eq!(key.owner, crate::bisync::StateOwner::Job(id.clone()));
    let owned_baseline = crate::bisync::baseline_file(&key).unwrap();
    assert!(!crate::bisync::load_baseline(&owned_baseline).unwrap().contains_key("deleted-on-b.txt"));
    let before = std::fs::read(&owned_baseline).unwrap();
    let noop = saved.run().last_result.unwrap();
    assert_eq!((noop.a_to_b, noop.b_to_a, noop.deleted, noop.conflicts, noop.errors), (0, 0, 0, 0, 0));
    assert_eq!(std::fs::read(owned_baseline).unwrap(), before);
    assert_eq!(saved.key(&local_a, &root_a, &local_b, &root_b), key);
}
