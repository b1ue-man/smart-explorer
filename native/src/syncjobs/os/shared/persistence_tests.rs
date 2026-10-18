//! Tests of the job store in `jobs/` (moved out of `persistence.rs`
//! unchanged, so the store has room for the vault job folder, C5.8).
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

#[test]
fn review_task_tolerant_load_lists_broken_files_and_keeps_valid_jobs() {
    let dir = temp_dir();
    let valid = SyncJob::new("good".into(), "s".into(), "t".into());
    write_job(&dir, &valid).unwrap();
    std::fs::write(dir.join("broken.conf"), "id=broken\nmax_delete_pct=101\n").unwrap();
    assert!(load_dir(&dir).is_err());
    let report = load_dir_report(&dir).unwrap();
    assert_eq!(report.jobs.len(), 1);
    assert_eq!(report.jobs[0].id, valid.id);
    assert_eq!(report.broken.len(), 1);
    assert_eq!(report.broken[0].id, "broken");
    assert!(!report.broken[0].error.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}
