//! Contract V3 for saved jobs: defaults, the keys RV1 added and what files
//! saved before RV1 mean.
use super::*;
use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::CURRENT_CONFIG_VERSION;

#[test]
fn review_task_new_jobs_carry_the_rv1_defaults_and_round_trip() {
    let mut job = SyncJob::new("Docs".into(), "/a".into(), "/b".into());
    assert_eq!(job.max_delete_pct, 50);
    assert_eq!(job.max_delete_min, 25);
    assert!(!job.cross_mounts);
    assert_eq!(job.versions_location, VersionsLocation::Auto);
    assert_eq!(job.config_version, CURRENT_CONFIG_VERSION);
    assert_eq!(
        (
            job.rt_poll_secs,
            job.verify_interval_secs,
            job.verify_target_secs
        ),
        (300, 3_600, 86_400)
    );
    let opts = job.checked_opts(false).unwrap();
    assert_eq!((opts.max_delete_pct, opts.max_delete_min), (50, 25));
    assert!(!opts.cross_mounts);
    assert_eq!(opts.versions, VersionsLocation::Auto);

    job.rt_max_latency_secs = 45;
    job.rt_poll_secs = 0;
    job.versions_location = VersionsLocation::AppData;
    job.cross_mounts = true;
    job.max_delete_min = 3;
    job.run_cleanup = "cleanup --all".into();
    let back = parse_kv_checked(&serialize_kv(&job)).unwrap();
    assert_eq!(back.run_cleanup, "cleanup --all");
    assert_eq!(back.rt_max_latency_secs, 45);
    assert_eq!(back.rt_poll_secs, 0);
    assert_eq!(back.versions_location, VersionsLocation::AppData);
    assert!(back.cross_mounts);
    assert_eq!(back.max_delete_min, 3);
    assert_eq!(back.config_version, CURRENT_CONFIG_VERSION);
}

#[test]
fn review_task_files_saved_before_rv1_keep_their_meaning() {
    let job = parse_kv_checked("id=abc\nname=X\nsource=s\ntarget=t\nmax_delete_pct=0\n").unwrap();
    assert_eq!(job.config_version, 0);
    assert_eq!((job.max_delete_pct, job.max_delete_min), (0, 0));
    assert!(job.cross_mounts);
    assert_eq!(job.versions_location, VersionsLocation::Auto);
    assert_eq!(job.rt_poll_secs, 300);

    let legacy = parse_legacy(
        &[
            "id1", "Name", "/a", "/b", "both", "strict", "30", "0", "1", "", "0", "1",
        ]
        .join("\t"),
    )
    .unwrap();
    assert_eq!((legacy.config_version, legacy.max_delete_pct), (0, 0));
    assert!(legacy.cross_mounts);
}

#[test]
fn review_task_legacy_import_hash_covers_only_the_original_keys() {
    let job = SyncJob::new("Docs".into(), "/a".into(), "/b".into());
    let core = serialize_kv_core(&job);
    let full = serialize_kv(&job);
    assert!(full.starts_with(&core));
    assert!(!core.contains("config_version="));
    assert!(full.contains("config_version="));
    assert!(full.contains("cross_mounts=0\n"));
}

#[test]
fn review_task_latency_and_case_insensitive_ignore_patterns() {
    let mut job = SyncJob::new("Docs".into(), "/a".into(), "/b".into());
    assert_eq!(job.effective_rt_max_latency_secs(), 300);
    job.rt_debounce_secs = 120;
    assert_eq!(job.effective_rt_max_latency_secs(), 600);
    job.rt_max_latency_secs = 45;
    assert_eq!(job.effective_rt_max_latency_secs(), 45);

    job.ignore = vec!["**/*.tmp".into()];
    let folded = job.checked_glob_set_for(true).unwrap();
    let exact = job.checked_glob_set_for(false).unwrap();
    assert!(folded.is_match("dir/X.TMP"));
    assert!(!exact.is_match("dir/X.TMP"));
    assert!(exact.is_match("dir/x.tmp"));
}

#[test]
fn review_task_editor_carries_the_rv1_settings() {
    let mut editor = JobEditor::blank("/a".into(), "/b".into());
    assert!(editor.id.is_none());
    assert_eq!(editor.max_delete_pct, "50");
    assert_eq!(editor.max_delete_min, "25");
    assert!(!editor.cross_mounts);
    editor.rt_poll = "60".into();
    editor.verify_target = "0".into();
    editor.versions_location = VersionsLocation::AppData;
    let job = editor.build_sync_job(None).unwrap();
    assert_eq!((job.rt_poll_secs, job.verify_target_secs), (60, 0));
    assert_eq!(job.versions_location, VersionsLocation::AppData);
    assert_eq!(job.config_version, CURRENT_CONFIG_VERSION);

    editor.rt_poll = "bald".into();
    assert!(editor
        .build_sync_job(None)
        .unwrap_err()
        .contains("Abfrageintervall"));
}
