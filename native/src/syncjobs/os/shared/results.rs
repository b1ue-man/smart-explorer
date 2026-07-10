use super::persistence::{
    app_data_dir, atomic_write, job_file, jobs_dir, load_job_file, san, write_job,
};
use super::schedule::now_secs;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;

/// Per-job last-run result (runtime state, shown in the UI).
#[derive(Clone, Debug, Default)]
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

pub fn load_results() -> BTreeMap<String, JobResult> {
    let mut out = BTreeMap::new();
    if let Ok(txt) = std::fs::read_to_string(results_path()) {
        for (index, line) in txt.lines().enumerate() {
            if let Ok((id, result)) = parse_result_line(line, index) {
                out.insert(id, result);
            }
        }
    }
    out
}

/// Record (upsert) a job's latest run result.
pub fn record_result(id: &str, r: &JobResult) -> std::io::Result<()> {
    record_result_to(&results_path(), id, r)
}

fn record_result_to(path: &Path, id: &str, r: &JobResult) -> io::Result<()> {
    let mut all = load_results_for_update(path)?;
    all.insert(id.to_string(), r.clone());
    let body: String = all
        .iter()
        .map(|(id, r)| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                san(id),
                r.when,
                r.a_to_b,
                r.b_to_a,
                r.deleted,
                r.conflicts,
                r.errors,
                san(&r.note)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    atomic_write(path, body.as_bytes())
}

fn load_results_for_update(path: &Path) -> io::Result<BTreeMap<String, JobResult>> {
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error),
    };
    body.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| parse_result_line(line, index))
        .collect()
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

/// Mark a job as just-run (updates last_run and rewrites only its file).
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
    fn record_rejects_malformed_history_without_erasing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("results.tsv");
        let original = "old\t1700000000\t1\t2\t3\t4\t5\tok\nbroken";
        std::fs::write(&path, original).unwrap();

        let error = record_result_to(
            &path,
            "new",
            &JobResult {
                when: 1_800_000_000,
                note: "ok".into(),
                ..Default::default()
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }
}
