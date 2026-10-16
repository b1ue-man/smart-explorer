//! The Android bridge carries every setting of the desktop sync editor in
//! both directions (api.md §4.7 `Job`) and sets the worker's check interval
//! like the desktop (`bg.setCadence`).
use serde_json::{json, Map, Value};

use super::background::set_cadence;
use super::job_json::{apply_draft, field_for_message, job_json};
use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::SyncJob;

#[test]
fn luecken_task_job_json_carries_every_editor_field() {
    let mut job = SyncJob::new("Fotos".into(), "/a".into(), "/b".into());
    job.direction = crate::bisync::Direction::AtoB;
    job.modify_window_sec = 2;
    job.filter_min_size_kb = 1;
    job.filter_max_size_kb = 900;
    job.filter_max_age_days = 30;
    job.filter_min_age_days = 2;
    job.bwlimit_kbps = 512;
    job.max_transfers = 4;
    job.atomic_copy = false;
    job.verify = true;
    job.retries = 3;
    job.retry_delay_secs = 9;
    job.connect_match = "BACKUP*".into();
    let value = job_json(&job, None, None);
    for key in [
        "modifyWindowSec",
        "filterMinSizeKb",
        "filterMaxSizeKb",
        "filterMaxAgeDays",
        "filterMinAgeDays",
        "bwlimitKbps",
        "maxTransfers",
        "atomicCopy",
        "verifyCopies",
        "retries",
        "retryDelaySecs",
        "connectMatch",
    ] {
        assert!(value.get(key).is_some(), "missing {key}");
    }
    let draft: Map<String, Value> = value.as_object().cloned().unwrap_or_default();
    let mut editor = JobEditor::blank(String::new(), String::new());
    let errors = apply_draft(&mut editor, &draft);
    assert!(errors.is_empty(), "{errors:?}");
    let back = editor.build_sync_job(None).expect("valid draft");
    assert_eq!(back.modify_window_sec, 2);
    assert_eq!(
        (
            back.filter_min_size_kb,
            back.filter_max_size_kb,
            back.filter_max_age_days,
            back.filter_min_age_days
        ),
        (1, 900, 30, 2)
    );
    assert_eq!((back.bwlimit_kbps, back.max_transfers), (512, 4));
    assert!(!back.atomic_copy && back.verify);
    assert_eq!((back.retries, back.retry_delay_secs), (3, 9));
    assert_eq!(back.connect_match, "BACKUP*");
}

#[test]
fn luecken_task_job_json_errors_name_the_new_fields() {
    for (message, field) in [
        (
            "Zeit-Toleranz enthält keine gültige nichtnegative Zahl.",
            "modifyWindowSec",
        ),
        (
            "Die Mindestgröße darf nicht über der Maximalgröße liegen.",
            "filterMinSizeKb",
        ),
        (
            "Maximalgröße enthält keine gültige nichtnegative Zahl.",
            "filterMaxSizeKb",
        ),
        (
            "Das Mindestalter darf nicht über dem Maximalalter liegen.",
            "filterMinAgeDays",
        ),
        (
            "Maximales Alter enthält keine gültige nichtnegative Zahl.",
            "filterMaxAgeDays",
        ),
        (
            "Bandbreite enthält keine gültige nichtnegative Zahl.",
            "bwlimitKbps",
        ),
        (
            "Parallele Übertragungen enthält keine gültige nichtnegative Zahl.",
            "maxTransfers",
        ),
        (
            "Wiederholungspause enthält keine gültige nichtnegative Zahl.",
            "retryDelaySecs",
        ),
        ("Die Zahl der Wiederholungen ist zu groß.", "retries"),
    ] {
        assert_eq!(field_for_message(message), field, "{message}");
    }
}

#[test]
fn luecken_task_bg_set_cadence_keeps_the_desktop_range() {
    for seconds in [-1, 0, 1, 3601] {
        assert!(
            set_cadence(&json!({ "seconds": seconds })).is_err(),
            "{seconds}"
        );
    }
    assert!(set_cadence(&json!({})).is_err());
}
