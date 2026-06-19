use super::persistence::{app_data_dir, job_file, jobs_dir, parse_kv, san, write_job};
use super::schedule::now_secs;
use std::collections::BTreeMap;
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
    /// Short status word: "ok", "Konflikte", "Fehler", "abgebrochen".
    pub note: String,
}

fn results_path() -> PathBuf {
    app_data_dir().join("results.tsv")
}

pub fn load_results() -> BTreeMap<String, JobResult> {
    let mut out = BTreeMap::new();
    if let Ok(txt) = std::fs::read_to_string(results_path()) {
        for line in txt.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() >= 8 {
                out.insert(
                    f[0].to_string(),
                    JobResult {
                        when: f[1].parse().unwrap_or(0),
                        a_to_b: f[2].parse().unwrap_or(0),
                        b_to_a: f[3].parse().unwrap_or(0),
                        deleted: f[4].parse().unwrap_or(0),
                        conflicts: f[5].parse().unwrap_or(0),
                        errors: f[6].parse().unwrap_or(0),
                        note: f[7..].join("\t"),
                    },
                );
            }
        }
    }
    out
}

/// Record (upsert) a job's latest run result.
pub fn record_result(id: &str, r: &JobResult) {
    let mut all = load_results();
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
    let _ = std::fs::write(results_path(), body);
}

/// Mark a job as just-run (updates last_run and rewrites only its file).
pub fn mark_run(id: &str) {
    let dir = jobs_dir();
    if let Ok(body) = std::fs::read_to_string(job_file(&dir, id)) {
        if let Some(mut j) = parse_kv(&body) {
            j.last_run = now_secs();
            let _ = write_job(&dir, &j);
        }
    }
}
