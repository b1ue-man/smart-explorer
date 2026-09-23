use super::*;
use super::sync_paths_task_fixture::Location;
use crate::bisync::{link_fixture, BisyncOptions, WalkFilter};
use crate::vfs::Scheme;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_links_task_saved_job_and_gui_retain_partial_result_notice() {
    let mut app = App::new_for_copy_task();
    let (a, b) = (Location::new(Scheme::Local), Location::new(Scheme::Local));
    let outside = tempfile::tempdir().unwrap();
    link_fixture::directory(outside.path(), &a.disk.join("node_modules"));
    std::fs::write(a.disk.join("normal.txt"), b"saved job").unwrap();
    let job = crate::syncjobs::SyncJob::new("linked folder".into(), a.root.clone(), b.root.clone());
    let id = job.id.clone();
    crate::syncjobs::upsert(&job).unwrap();
    app.sync_jobs.push(job);
    app.run_job(&id);
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.bisync_running || app.job_connect_rx.is_some() {
        assert!(Instant::now() < deadline, "saved sync deadline");
        app.drain_job_connect();
        app.drain_bisync();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
    assert!(app.notice.as_ref().unwrap().0.contains("node_modules"));
    let results = crate::syncjobs::load_results();
    assert!(results[&id].note.starts_with("mit Auslassungen"));
    assert!(results[&id].note.contains("node_modules"));
    assert_eq!(results[&id].errors, 0);
    assert_eq!(std::fs::read(b.disk.join("normal.txt")).unwrap(), b"saved job");
    link_fixture::remove_directory(&a.disk.join("node_modules"));
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_links_task_cross_remote_contract_protects_counterpart_subtree() {
    let (a, b) = (Location::new(Scheme::Sftp), Location::new(Scheme::Webdav));
    let outside = tempfile::tempdir().unwrap();
    link_fixture::directory(outside.path(), &a.disk.join("node_modules"));
    std::fs::write(a.disk.join("regular Ü %20 #.txt"), b"cross remote").unwrap();
    std::fs::create_dir(b.disk.join("node_modules")).unwrap();
    std::fs::write(b.disk.join("node_modules/keep.txt"), b"preserve").unwrap();
    let globs = crate::bisync::empty_globset();
    let result = crate::bisync::run(&*a.backend, &a.root, &*b.backend, &b.root,
        BisyncOptions::default(), &AtomicBool::new(false), &WalkFilter::basic(true, &globs));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.omissions.reported_paths().collect::<Vec<_>>(), ["node_modules"]);
    assert_eq!(std::fs::read(b.disk.join("regular Ü %20 #.txt")).unwrap(), b"cross remote");
    assert_eq!(std::fs::read(b.disk.join("node_modules/keep.txt")).unwrap(), b"preserve");
    assert!(!outside.path().join("keep.txt").exists());
    link_fixture::remove_directory(&a.disk.join("node_modules"));
}
