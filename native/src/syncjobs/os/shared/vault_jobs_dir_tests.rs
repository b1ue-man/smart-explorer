//! The vault job folder next to `jobs/` (vault plan C5.8, stage
//! `vault_sync`): older versions and the strict loader of `jobs/` never see
//! vault jobs, a broken vault job is listed and never empties a list.
use super::super::persistence::{load_dir, write_job};
use super::super::persistence_codec::serialize_kv;
use super::*;

const SITE_A: &str = "0123456789abcdef0123456789abcdef";
const SITE_B: &str = "fedcba9876543210fedcba9876543210";

struct Folders {
    _dir: tempfile::TempDir,
    jobs: PathBuf,
    vault: PathBuf,
}

fn folders() -> Folders {
    let dir = tempfile::tempdir().expect("temporary folder");
    let jobs = dir.path().join("jobs");
    let vault = dir.path().join(VAULT_JOBS_DIR);
    std::fs::create_dir_all(&jobs).expect("jobs folder");
    Folders {
        _dir: dir,
        jobs,
        vault,
    }
}

fn general(name: &str) -> SyncJob {
    let mut job = SyncJob::new(
        name.into(),
        format!("/daten/{name}"),
        format!("/kopie/{name}"),
    );
    job.id = format!("general-{name}");
    job
}

fn vault_job(id: &str) -> SyncJob {
    let mut job = SyncJob::new_vault("Tresor".into(), SITE_A, SITE_B);
    job.id = id.to_string();
    job
}

/// Every file of `dir` with its bytes, sorted by name.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .expect("folder")
        .map(|entry| {
            let path = entry.expect("entry").path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).expect("file"))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn vault_task_vault_jobs_dir_invisible_to_jobs_loader() {
    let f = folders();
    let (one, two) = (general("eins"), general("zwei"));
    write_job(&f.jobs, &one).unwrap();
    write_job(&f.jobs, &two).unwrap();
    let before = snapshot(&f.jobs);
    // A general job is written exactly as by earlier versions: no new key.
    let text = serialize_kv(&one);
    for key in ["kind=", "vault_site_a=", "vault_site_b="] {
        assert!(!text.contains(key), "{key} in a general job");
    }

    // A vault job goes to vault-jobs/ only; jobs/ keeps its bytes.
    let vault = vault_job("tresor-job-1");
    write_in(&f.jobs, &f.vault, &vault).unwrap();
    assert_eq!(snapshot(&f.jobs), before, "jobs/ unchanged by a vault job");
    let saved = std::fs::read_to_string(job_file(&f.vault, &vault.id)).unwrap();
    for line in [
        "kind=vault",
        "vault_site_a=",
        "vault_site_b=",
        "source=vault-store://",
    ] {
        assert!(saved.contains(line), "{line} missing:\n{saved}");
    }

    // The strict loader of jobs/ (the GUI list, the folder older versions
    // read) succeeds with exactly the general jobs.
    let listed = load_dir(&f.jobs).expect("strict jobs/ load stays successful");
    let ids: Vec<&str> = listed.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(ids, [one.id.as_str(), two.id.as_str()]);
    assert!(listed.iter().all(|job| !job.is_vault()));

    // vault-jobs/ yields the vault job with its kind and sites.
    let report = load_report_in(&f.vault).unwrap();
    assert!(report.broken.is_empty(), "{:?}", report.broken);
    assert_eq!(report.jobs.len(), 1);
    assert!(report.jobs[0].is_vault());
    assert_eq!(report.jobs[0].vault_site_b, SITE_B);
    assert_eq!(find_in(&f.vault, &vault.id).unwrap().unwrap().id, vault.id);

    // IDs are unique over both folders.
    let mut clash = vault_job(&one.id);
    clash.name = "Kollision".into();
    assert_eq!(
        write_in(&f.jobs, &f.vault, &clash).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        general_id_free(&f.vault, &vault.id).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    general_id_free(&f.vault, "unbenutzt").unwrap();
    assert!(write_in(&f.jobs, &f.vault, &general("drei")).is_err());

    // A vault job file in jobs/ is never read as a general job.
    std::fs::write(
        job_file(&f.jobs, "verirrt"),
        saved.replace(&vault.id, "verirrt"),
    )
    .unwrap();
    assert!(load_dir(&f.jobs).is_err());
}

#[test]
fn vault_task_broken_vault_job_listed_not_fatal() {
    let f = folders();
    let one = general("eins");
    write_job(&f.jobs, &one).unwrap();
    let good = vault_job("tresor-gut");
    write_in(&f.jobs, &f.vault, &good).unwrap();
    // Broken: a fixed value changed by hand, and a file of the wrong kind.
    let broken = std::fs::read_to_string(job_file(&f.vault, &good.id))
        .unwrap()
        .replace(&good.id, "tresor-kaputt")
        .replace("conflict=strict", "conflict=newer");
    std::fs::write(job_file(&f.vault, "tresor-kaputt"), broken).unwrap();
    let mut wrong = general("falsch");
    wrong.id = "tresor-falsch".into();
    std::fs::write(job_file(&f.vault, &wrong.id), serialize_kv(&wrong)).unwrap();

    let report = load_report_in(&f.vault).expect("a broken vault job never fails the list");
    let ids: Vec<&str> = report.jobs.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(ids, [good.id.as_str()]);
    let mut broken_ids: Vec<&str> = report.broken.iter().map(|job| job.id.as_str()).collect();
    broken_ids.sort_unstable();
    assert_eq!(broken_ids, ["tresor-falsch", "tresor-kaputt"]);
    assert!(report.broken.iter().all(|job| !job.error.is_empty()));
    assert!(report
        .broken
        .iter()
        .any(|job| job.error.contains("Tresor-Job")));

    // The strict list of jobs/ stays complete; callers without a broken list
    // get the valid vault jobs after it (load: strict jobs/, then vault).
    let mut all = load_dir(&f.jobs).expect("strict jobs/ load");
    all.extend(load_valid_in(&f.vault));
    let ids: Vec<&str> = all.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(ids, [one.id.as_str(), good.id.as_str()]);

    // A missing vault-jobs/ folder holds no job and is no error.
    let empty = load_report_in(&f.vault.join("fehlt")).unwrap();
    assert!(empty.jobs.is_empty() && empty.broken.is_empty());
}
