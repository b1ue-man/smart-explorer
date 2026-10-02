//! Last-run results of the sync jobs. Since RV1 they live in the job state
//! (`JobState::last_result`, contract V4); the `results.tsv` of older
//! versions is only read to seed jobs that have no state yet.
//! `record_result` and `mark_run` stay for runners that still report the old
//! way: the result becomes a classified attempt in the job state, the run
//! time keeps updating the configuration's `last_run` for older displays.

use super::job_state::{AttemptOutcome, AttemptReport, FailureKind, JobError, RunCause, Runner};
use super::job_state_store::{record_attempt, stored_results};
use super::persistence::{app_data_dir, job_file, jobs_dir, load_job_file, write_job};
use super::schedule::now_secs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// Per-job last-run result (runtime state, shown in the UI). Also stored as
/// `JobState::last_result`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobResult {
    pub when: i64,
    pub a_to_b: u64,
    pub b_to_a: u64,
    pub deleted: u64,
    pub conflicts: u64,
    pub errors: u64,
    /// Short status summary, including pre/post-command failures when present.
    pub note: String,
}

fn results_path() -> PathBuf {
    app_data_dir().join("results.tsv")
}

/// The readable rows of a legacy `results.tsv` (malformed rows are skipped).
fn legacy_results_from(path: &Path) -> BTreeMap<String, JobResult> {
    let mut out = BTreeMap::new();
    if let Ok(txt) = std::fs::read_to_string(path) {
        for (index, line) in txt.lines().enumerate() {
            if let Ok((id, result)) = parse_result_line(line, index) {
                out.insert(id, result);
            }
        }
    }
    out
}

/// The legacy row of one job, used to seed its missing state.
pub(super) fn legacy_result(id: &str) -> Option<JobResult> {
    legacy_results_from(&results_path()).remove(id)
}

/// The last result of every job (job states first, legacy rows otherwise).
pub fn load_results() -> BTreeMap<String, JobResult> {
    stored_results(legacy_results_from(&results_path()))
}

/// Records a finished run reported the old way as an attempt in the job
/// state, classified from its note and error count. New code reports with
/// `classify_run` and `record_attempt`.
pub fn record_result(id: &str, r: &JobResult) -> io::Result<()> {
    record_attempt(id, &legacy_attempt(r)).map(|_| ())
}

fn legacy_attempt(r: &JobResult) -> AttemptReport {
    let outcome = if r.note.starts_with("abgebrochen") {
        AttemptOutcome::Cancelled
    } else if r.errors > 0 || r.note.starts_with("Fehler") || r.note.contains("fehlgeschlagen") {
        let kind = if r.note.contains("Befehl") {
            FailureKind::Hook
        } else {
            FailureKind::Run
        };
        AttemptOutcome::Failed(JobError {
            kind,
            message: r.note.clone(),
        })
    } else {
        AttemptOutcome::Success
    };
    AttemptReport {
        runner: Runner::Other,
        cause: RunCause::Manual,
        started: r.when,
        finished: r.when,
        outcome,
        result: Some(r.clone()),
    }
}

fn parse_result_line(line: &str, index: usize) -> io::Result<(String, JobResult)> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 8 || fields[0].is_empty() {
        return Err(invalid_result(index, "expected at least eight fields"));
    }
    let number = |field: usize, name: &str| {
        fields[field]
            .parse::<u64>()
            .map_err(|_| invalid_result(index, &format!("invalid {name}")))
    };
    let when = fields[1]
        .parse::<i64>()
        .map_err(|_| invalid_result(index, "invalid timestamp"))?;
    Ok((
        fields[0].to_string(),
        JobResult {
            when,
            a_to_b: number(2, "A-to-B count")?,
            b_to_a: number(3, "B-to-A count")?,
            deleted: number(4, "delete count")?,
            conflicts: number(5, "conflict count")?,
            errors: number(6, "error count")?,
            note: fields[7..].join("\t"),
        },
    ))
}

fn invalid_result(index: usize, detail: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid sync result on line {}: {detail}", index + 1),
    )
}

/// Mark a job as just-run (updates the configuration's `last_run`, which only
/// older displays read; schedules count from `JobState::last_success`).
pub fn mark_run(id: &str) -> std::io::Result<()> {
    let dir = jobs_dir();
    let path = job_file(&dir, id);
    let mut job = load_job_file(&path)?;
    job.last_run = now_secs();
    write_job(&dir, &job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_legacy_results_skip_malformed_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("results.tsv");
        std::fs::write(&path, "old\t1700000000\t1\t2\t3\t4\t5\tok\nbroken").unwrap();

        let rows = legacy_results_from(&path);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows["old"].when, 1_700_000_000);
        assert_eq!(rows["old"].errors, 5);
    }

    #[test]
    fn review_task_legacy_reports_are_classified() {
        let report = |note: &str, errors: u64| {
            legacy_attempt(&JobResult {
                when: 10,
                errors,
                note: note.into(),
                ..Default::default()
            })
            .outcome
        };
        assert_eq!(report("ok", 0), AttemptOutcome::Success);
        assert_eq!(report("Konflikte", 0), AttemptOutcome::Success);
        assert_eq!(report("abgebrochen", 0), AttemptOutcome::Cancelled);
        assert!(matches!(
            report("Fehler", 3),
            AttemptOutcome::Failed(JobError {
                kind: FailureKind::Run,
                ..
            })
        ));
        assert!(matches!(
            report("Befehl davor fehlgeschlagen", 1),
            AttemptOutcome::Failed(JobError {
                kind: FailureKind::Hook,
                ..
            })
        ));
    }
}
