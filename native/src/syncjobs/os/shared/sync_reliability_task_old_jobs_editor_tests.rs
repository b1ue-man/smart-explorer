//! C08 codec and editor retain the same stored endpoints, owner and options.
use super::sync_reliability_task_old_jobs_tests::root_path;
use crate::daemon::sync_reliability_task_old_jobs_tests::SavedJob;

#[test]
fn sync_reliability_task_old_jobs_literal_remote_locators_keep_options_through_migration() {
    let endpoints = [
        ("gdrive:///Notebook ", "sftp://one@example.test:2222/the same root"),
        ("sftp://one@example.test:2222/shared", "webdav://two@example.test:443/shared"),
        ("//server/share/literal %2F", "ftp://other@example.test:2121/literal %2F"),
    ];
    for (source, target) in endpoints {
        let saved = SavedJob::old(source, target, "calendar");
        let before = super::persistence_codec::parse_kv_checked(&saved.original).unwrap();
        assert_eq!(before.config_version, 0);
        saved.assert_options(source, target);
        let job = saved.load();
        let original = super::persistence_codec::serialize_kv(&job);
        let editor = crate::syncjobs::editor::JobEditor::from_job(&job);
        let edited = editor.build_sync_job(Some(&job)).unwrap();
        assert_eq!(super::persistence_codec::serialize_kv(&edited), original);
        crate::syncjobs::upsert(&edited).unwrap();
        saved.assert_options(source, target);
        assert_eq!((&*saved.load().source, &*saved.load().target), (source, target));
    }
}

#[test]
fn sync_reliability_task_old_jobs_edit_keeps_owner_and_missing_id_cannot_create_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("source");
    let b = temp.path().join("target");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    std::fs::write(a.join("data.txt"), b"same saved job owner").unwrap();
    let root_a = root_path(&a);
    let root_b = root_path(&b);
    let saved = SavedJob::old(&root_a, &root_b, "interval");
    saved.run();
    let job = saved.load();
    let local_a = crate::vfs::LocalBackend::new(&root_a);
    let local_b = crate::vfs::LocalBackend::new(&root_b);
    let key = saved.key(&local_a, &root_a, &local_b, &root_b);
    let baseline = crate::bisync::baseline_file(&key).unwrap();
    let before = std::fs::read(&baseline).unwrap();
    let editor = crate::syncjobs::editor::JobEditor::from_job(&job);
    assert!(editor.build_sync_job(None).is_err());
    let mut unrelated = job.clone();
    unrelated.id.push_str("_other");
    assert!(editor.build_sync_job(Some(&unrelated)).is_err());
    assert_eq!(std::fs::read(&baseline).unwrap(), before);
    let edited = editor.build_sync_job(Some(&job)).unwrap();
    assert_eq!(edited.id, job.id);
    assert_eq!(edited.last_run, job.last_run);
    crate::syncjobs::upsert(&edited).unwrap();
    let result = saved.run().last_result.unwrap();
    assert_eq!((result.a_to_b, result.b_to_a, result.deleted, result.errors), (0, 0, 0, 0));
    assert_eq!(saved.key(&local_a, &root_a, &local_b, &root_b), key);
    assert_eq!(std::fs::read(baseline).unwrap(), before);
}

#[test]
fn sync_reliability_task_old_jobs_explicit_zero_after_upgrade_survives_restart() {
    let saved = SavedJob::old("gdrive:///old", "sftp://saved/root", "onstartup");
    let mut job = saved.load();
    job.max_delete_pct = 0;
    job.max_delete_min = 0;
    job.rt_poll_secs = 0;
    job.verify_interval_secs = 0;
    job.verify_target_secs = 0;
    job.cross_mounts = false;
    job.versions_location = crate::bisync::VersionsLocation::AppData;
    crate::syncjobs::upsert(&job).unwrap();
    let back = saved.load();
    assert_eq!((back.max_delete_pct, back.max_delete_min, back.rt_poll_secs,
        back.verify_interval_secs, back.verify_target_secs), (0, 0, 0, 0, 0));
    assert!(!back.cross_mounts);
    assert_eq!(back.versions_location, crate::bisync::VersionsLocation::AppData);
    assert_eq!((&*back.id, &*back.source, &*back.target), (&*job.id, &*job.source, &*job.target));
}
