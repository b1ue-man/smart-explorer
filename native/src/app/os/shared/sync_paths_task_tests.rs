use super::prelude::*;
use super::*;
use super::sync_paths_task_fixture::{forward, remote, Location};
use crate::vfs::{Backend, CachingBackend, LocalBackend, Scheme};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn synchronize(a: &Location, b: &Location) -> crate::bisync::Outcome {
    let ignore = crate::bisync::empty_globset();
    crate::bisync::run(&*a.backend, &a.root, &*b.backend, &b.root,
        crate::bisync::BisyncOptions::default(), &AtomicBool::new(false),
        &crate::bisync::WalkFilter::basic(true, &ignore))
}

fn finish(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.bisync_running || app.sync_running || app.job_connect_rx.is_some() {
        assert!(Instant::now() < deadline, "sync worker deadline");
        app.drain_job_connect();
        app.drain_bisync();
        app.drain_sync();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_all_backend_pairs_transfer_changes_and_keep_baselines_separate() {
    let schemes = [Scheme::Local, Scheme::Sftp, Scheme::Ftp, Scheme::Webdav, Scheme::GDrive, Scheme::Peer];
    for left in schemes {
        for right in schemes {
            let (a, b) = (Location::new(left), Location::new(right));
            std::fs::write(a.disk.join("from-a Ü %20.txt"), b"alpha").unwrap();
            std::fs::write(b.disk.join("from-b #.txt"), b"bravo").unwrap();
            let first = synchronize(&a, &b);
            assert!(first.errors.is_empty(), "{left:?} -> {right:?}: {:?}", first.errors);
            assert!(first.conflicts.is_empty());
            assert_eq!(std::fs::read(b.disk.join("from-a Ü %20.txt")).unwrap(), b"alpha");
            assert_eq!(std::fs::read(a.disk.join("from-b #.txt")).unwrap(), b"bravo");
            std::fs::write(a.disk.join("from-a Ü %20.txt"), b"changed alpha").unwrap();
            let updated = synchronize(&a, &b);
            assert!(updated.errors.is_empty(), "{:?}", updated.errors);
            assert_eq!(std::fs::read(b.disk.join("from-a Ü %20.txt")).unwrap(), b"changed alpha");
            let stable = synchronize(&a, &b);
            assert!(stable.errors.is_empty());
            assert_eq!(stable.stats.a_to_b + stable.stats.b_to_a + stable.stats.deleted, 0);
            assert_ne!(crate::bisync::pair_id_for(&*a.backend, &a.root, &*b.backend, &b.root),
                crate::bisync::pair_id_for(&*b.backend, &b.root, &*a.backend, &a.root));
        }
    }
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_picker_setup_and_quick_actions_retain_remote_provenance() {
    for prefix in ["sftp://u@h:22", "ftp://u@h:21", "ftps://u@h:21", "webdav://u@h:443",
        "gdrive://", "share://direct/contact-a", "share://room/room-a/device-a"] {
        let mut app = App::new_for_copy_task();
        let (a, b) = (Location::new(Scheme::Sftp), Location::new(Scheme::GDrive));
        app.root_path = a.root.clone();
        app.remote = Some(remote(a.backend.clone(), prefix));
        let mut tab = TabState::default();
        tab.root_path = b.root.clone();
        tab.remote = Some(remote(b.backend.clone(), "sftp://other@second:22"));
        app.tabs.push(tab);
        let second = app.tabs.len() - 1;
        app.begin_pane_sync_setup(app.active_tab, Some(second));
        let editor = app.job_editor.as_ref().unwrap();
        assert_eq!(editor.source, format!("{prefix}{}", a.root));
        assert_eq!(editor.target, format!("sftp://other@second:22{}", b.root));
        let job = editor.build_sync_job(None).unwrap();
        assert_eq!(job.source, editor.source);
        app.open_picker(PickerPurpose::MirrorDest, "");
        assert!(!app.picker.as_ref().unwrap().purpose.local_only());
        let selection = app.picker_tab_locations().into_iter().find(|location|
            location.prefix == "sftp://other@second:22").unwrap();
        app.picker_use_location(selection);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.picker.as_ref().unwrap().listing {
            app.drain_picker_list();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let picker = app.picker.take().unwrap();
        assert_eq!(App::picker_value(&picker), job.target);
        std::fs::write(a.disk.join("mirror.txt"), b"copy to chosen remote").unwrap();
        app.start_mirror(picker.backend.unwrap(), picker.cwd);
        finish(&mut app);
        assert_eq!(std::fs::read(b.disk.join("mirror.txt")).unwrap(), b"copy to chosen remote");
        std::fs::write(b.disk.join("reverse.txt"), b"from target").unwrap();
        app.start_bisync(b.backend.clone(), b.root.clone());
        finish(&mut app);
        assert_eq!(std::fs::read(a.disk.join("reverse.txt")).unwrap(), b"from target");
    }
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_split_same_paths_on_different_remotes_and_uncached_metadata() {
    let mut app = App::new_for_copy_task();
    let (a, b) = (Location::new(Scheme::Peer), Location::new(Scheme::Peer));
    let cached: crate::vfs::BackendHandle = Arc::new(CachingBackend::new(a.backend.clone()));
    assert!(cached.list_dir(&a.root).unwrap().is_empty());
    std::fs::write(a.disk.join("fresh.txt"), b"fresh after browser listing").unwrap();
    app.root_path = a.root.clone();
    app.remote = Some(remote(cached, "share://direct/first"));
    let mut tab = TabState::default();
    tab.root_path = b.root.clone();
    tab.remote = Some(remote(b.backend.clone(), "share://direct/second"));
    app.tabs.push(tab);
    app.split = true;
    app.panes = [app.active_tab, app.tabs.len() - 1];
    assert_eq!(a.root, b.root);
    app.sync_split_panes();
    assert!(app.bisync_running, "{:?}", app.error_msg);
    finish(&mut app);
    assert_eq!(std::fs::read(b.disk.join("fresh.txt")).unwrap(), b"fresh after browser listing");
    assert!(crate::vfs::validate_sync_roots(&*a.backend, &a.root, &*a.backend, &a.root).is_err());
    assert!(crate::vfs::validate_sync_roots(&*a.backend, &a.root, &*a.backend, &format!("{}/sub", a.root)).is_err());
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_saved_local_job_uses_worker_resolution_and_preserves_old_behavior() {
    let mut app = App::new_for_copy_task();
    let (a, b) = (Location::new(Scheme::Local), Location::new(Scheme::Local));
    std::fs::write(a.disk.join("local.txt"), b"local source").unwrap();
    let job = crate::syncjobs::SyncJob::new("local".into(), a.root.clone(), b.root.clone());
    let id = job.id.clone();
    crate::syncjobs::upsert(&job).unwrap();
    app.sync_jobs.push(job);
    app.run_job(&id);
    assert!(app.job_connect_rx.is_some());
    finish(&mut app);
    assert_eq!(std::fs::read(b.disk.join("local.txt")).unwrap(), b"local source");
    let root = forward(a.disk.parent().unwrap());
    assert!(crate::vfs::validate_sync_roots(&LocalBackend::new(&root), &root, &*a.backend, &a.root).is_err());
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_real_share_cross_peer_sync_and_local_roundtrip() {
    let first = crate::share::CopyPastePeerFixture::new().unwrap();
    let second = crate::share::CopyPastePeerFixture::new().unwrap();
    std::fs::write(first.root_a.join("first Ü.txt"), b"first peer").unwrap();
    std::fs::write(second.root_a.join("second %20.txt"), b"second peer").unwrap();
    let ignore = crate::bisync::empty_globset();
    let filter = crate::bisync::WalkFilter::basic(true, &ignore);
    let result = crate::bisync::run(&*first.backend, "/A", &*second.backend, "/A",
        crate::bisync::BisyncOptions::default(), &AtomicBool::new(false), &filter);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.conflicts.is_empty());
    assert_eq!(std::fs::read(second.root_a.join("first Ü.txt")).unwrap(), b"first peer");
    assert_eq!(std::fs::read(first.root_a.join("second %20.txt")).unwrap(), b"second peer");
    let local = Location::new(Scheme::Local);
    let result = crate::bisync::run(&*first.backend, "/A", &*local.backend, &local.root,
        crate::bisync::BisyncOptions::default(), &AtomicBool::new(false), &filter);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(std::fs::read(local.disk.join("first Ü.txt")).unwrap(), b"first peer");
}
