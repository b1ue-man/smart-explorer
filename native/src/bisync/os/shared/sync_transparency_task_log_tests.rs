//! Live job log: file contract for viewers and the lines one real run writes.
use super::sync_reliability_task_fixture::{clean, forward, Pair, TIME};
use super::*;
use std::io::Write;

fn unique(name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("sync_transparency_task_{name}_{nanos}")
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(path) = job_log_path(&self.0) {
            let mut rotated = path.clone().into_os_string();
            rotated.push(".1");
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(rotated);
            let _ = std::fs::remove_file(path.with_extension("verbose"));
        }
    }
}

#[test]
fn sync_transparency_task_log_reader_returns_complete_lines_from_an_offset() {
    let id = unique("reader");
    let _cleanup = Cleanup(id.clone());
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    job_log_line(&id, "Start", "erste Zeile");
    assert!(last_activity(&id).is_some_and(|at| at >= before));
    let first = read_job_log(&id, None).unwrap();
    assert!(first.text.contains("Start") && first.text.contains("erste Zeile"));
    assert!(first.text.ends_with('\n'));
    assert_eq!(first.next, first.size);
    assert!(!first.restarted);
    // A partially written line is not returned until it is complete.
    let path = job_log_path(&id).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"2026-10-08 00:00:00.000 Aktion     halb")
        .unwrap();
    let partial = read_job_log(&id, Some(first.next)).unwrap();
    assert!(partial.text.is_empty());
    assert_eq!(partial.next, first.next);
    file.write_all(b"fertig\n").unwrap();
    let rest = read_job_log(&id, Some(partial.next)).unwrap();
    assert_eq!(rest.text, "2026-10-08 00:00:00.000 Aktion     halbfertig\n");
    // An offset beyond the file (rotation, truncation) restarts the view.
    let beyond = read_job_log(&id, Some(rest.size + 100)).unwrap();
    assert!(beyond.restarted);
    assert!(beyond.text.contains("erste Zeile"));
}

#[test]
fn sync_transparency_task_log_rejects_unsafe_ids_and_keeps_line_breaks_inside_one_line() {
    assert!(job_log_path("../escape").is_none());
    assert!(job_log_path("a/b").is_none());
    assert!(job_log_path("").is_none());
    assert!(read_job_log("../escape", None).is_err());
    assert!(set_job_log_verbose("../escape", true).is_err());
    let id = unique("newline");
    let _cleanup = Cleanup(id.clone());
    job_log_line(&id, "Fehler", "Zeile eins\nZeile zwei\r\n");
    let chunk = read_job_log(&id, None).unwrap();
    assert_eq!(chunk.text.lines().count(), 1);
    assert!(chunk.text.contains("Zeile eins ⏎ Zeile zwei"));
}

#[test]
fn sync_transparency_task_log_rotates_at_its_bound_and_switches_verbose_mode() {
    let id = unique("rotate");
    let _cleanup = Cleanup(id.clone());
    let path = job_log_path(&id).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(&path)
        .unwrap()
        .set_len(JOB_LOG_ROTATE_BYTES)
        .unwrap();
    job_log_line(&id, "Start", "nach der Rotation");
    let mut rotated = path.clone().into_os_string();
    rotated.push(".1");
    assert_eq!(
        std::fs::metadata(&rotated).unwrap().len(),
        JOB_LOG_ROTATE_BYTES
    );
    let current = read_job_log(&id, Some(JOB_LOG_ROTATE_BYTES)).unwrap();
    assert!(current.restarted);
    assert!(current.text.contains("nach der Rotation"));
    assert!(current.size < 1024);
    assert!(!job_log_verbose(&id));
    set_job_log_verbose(&id, true).unwrap();
    assert!(job_log_verbose(&id));
    set_job_log_verbose(&id, false).unwrap();
    set_job_log_verbose(&id, false).unwrap();
    assert!(!job_log_verbose(&id));
}

#[test]
fn sync_transparency_task_run_logs_every_listing_comparison_and_action() {
    let id = unique("run");
    let _cleanup = Cleanup(id.clone());
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    pair.put(PairSide::A, "folder/file.txt", b"content", TIME);
    let settings = RunSettings::for_job(&id);
    let out = pair.run_settings(forward(), &filter, settings.clone());
    clean(&out);
    let log = read_job_log(&id, None).unwrap().text;
    for expected in [
        "Start",
        "Gelesen",
        "Scan",
        "folder/file.txt: Quelle 7 B",
        "→ Quelle → Ziel kopieren",
        "Aktion",
        "folder/file.txt: kopiert aus Quelle",
        "Ordner",
        "erfolgreich beendet",
    ] {
        assert!(
            log.contains(expected),
            "missing {expected:?} in log:\n{log}"
        );
    }
    assert!(last_activity(&id).is_some());
    // A run without changes reports its comparisons as a count, and one by
    // one once the job's verbose switch is on.
    set_job_log_verbose(&id, true).unwrap();
    let from = read_job_log(&id, None).unwrap().next;
    let again = pair.run_settings(
        forward(),
        &filter,
        RunSettings {
            depth: ScanDepth::Full,
            ..settings
        },
    );
    clean(&again);
    let second = read_job_log(&id, Some(from)).unwrap().text;
    assert!(second.contains("0 Aktionen"), "{second}");
    assert!(second.contains("1 unverändert verglichen"), "{second}");
    assert!(second.contains("folder/file.txt: Quelle 7 B"), "{second}");
    assert!(second.contains("→ unverändert"), "{second}");
}

#[test]
fn sync_transparency_task_ad_hoc_runs_write_no_job_log() {
    let pair = Pair::new();
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    pair.put(PairSide::A, "a.txt", b"x", TIME);
    let out = pair.run(forward(), &filter);
    clean(&out);
    assert!(super::run_log::current().is_none());
}
