use super::prelude::*;
use super::sync_paths_task_fixture::{forward, remote, Location};
use super::*;
use crate::bisync::versions::{self, VersionReason, VersionSide, VersionStore};
use crate::vfs::{Backend, CachingBackend, LocalBackend, Scheme};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn synchronize(a: &Location, b: &Location) -> crate::bisync::Outcome {
    let ignore = crate::bisync::empty_globset();
    crate::bisync::run(
        &*a.backend,
        &a.root,
        &*b.backend,
        &b.root,
        crate::bisync::BisyncOptions::default(),
        &AtomicBool::new(false),
        &crate::bisync::WalkFilter::basic(true, &ignore),
    )
}

fn finish(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            Instant::now() < deadline,
            "sync worker deadline: active={}; bisync={}; mirror={}; notice={:?}; error={:?}",
            app.desktop_sync_active(),
            app.bisync_running,
            app.sync_running,
            app.notice,
            app.error_msg
        );
        app.drain_job_connect();
        app.drain_bisync();
        app.drain_sync();
        let workers = app.drain_desktop_sync_workers();
        if workers == 0
            && !app.bisync_running
            && !app.sync_running
            && app.job_connect_rx.is_none()
            && app.bisync_rx.is_none()
            && app.sync_rx.is_none()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_all_backend_pairs_transfer_changes_and_keep_baselines_separate() {
    let schemes = [
        Scheme::Local,
        Scheme::Sftp,
        Scheme::Ftp,
        Scheme::Webdav,
        Scheme::GDrive,
        Scheme::Peer,
    ];
    for left in schemes {
        for right in schemes {
            let (a, b) = (Location::new(left), Location::new(right));
            std::fs::write(a.disk.join("from-a Ü %20.txt"), b"alpha").unwrap();
            std::fs::write(b.disk.join("from-b #.txt"), b"bravo").unwrap();
            let first = synchronize(&a, &b);
            assert!(
                first.errors.is_empty(),
                "{left:?} -> {right:?}: {:?}",
                first.errors
            );
            assert!(first.conflicts.is_empty());
            assert_eq!(
                std::fs::read(b.disk.join("from-a Ü %20.txt")).unwrap(),
                b"alpha"
            );
            assert_eq!(
                std::fs::read(a.disk.join("from-b #.txt")).unwrap(),
                b"bravo"
            );
            std::fs::write(a.disk.join("from-a Ü %20.txt"), b"changed alpha").unwrap();
            let updated = synchronize(&a, &b);
            assert!(updated.errors.is_empty(), "{:?}", updated.errors);
            assert_eq!(
                std::fs::read(b.disk.join("from-a Ü %20.txt")).unwrap(),
                b"changed alpha"
            );
            let stable = synchronize(&a, &b);
            assert!(stable.errors.is_empty());
            assert_eq!(
                stable.stats.a_to_b + stable.stats.b_to_a + stable.stats.deleted,
                0
            );
            assert_ne!(
                crate::bisync::pair_id_for(&*a.backend, &a.root, &*b.backend, &b.root),
                crate::bisync::pair_id_for(&*b.backend, &b.root, &*a.backend, &a.root)
            );
        }
    }
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_picker_setup_and_quick_actions_retain_remote_provenance() {
    for prefix in [
        "sftp://u@h:22",
        "ftp://u@h:21",
        "ftps://u@h:21",
        "webdav://u@h:443",
        "gdrive://",
        "share://direct/contact-a",
        "share://room/room-a/device-a",
    ] {
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
        let selection = app
            .picker_tab_locations()
            .into_iter()
            .find(|location| location.prefix == "sftp://other@second:22")
            .unwrap();
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
        assert_eq!(
            std::fs::read(b.disk.join("mirror.txt")).unwrap(),
            b"copy to chosen remote"
        );
        std::fs::write(b.disk.join("reverse.txt"), b"from target").unwrap();
        app.start_bisync(b.backend.clone(), b.root.clone());
        finish(&mut app);
        assert_eq!(
            std::fs::read(a.disk.join("reverse.txt")).unwrap(),
            b"from target"
        );
    }
}

#[test]
#[ignore = "requires the isolated remote sync-path task profile"]
fn sync_paths_task_split_same_paths_on_different_remotes_and_uncached_metadata() {
    let mut app = App::new_for_copy_task();
    let (a, b) = (Location::new(Scheme::Peer), Location::new(Scheme::Peer));
    let cached: crate::vfs::BackendHandle = Arc::new(CachingBackend::new(a.backend.clone()));
    assert!(cached.list_dir(&a.root).unwrap().is_empty());
    assert_ne!(
        a.backend.namespace_identity(),
        b.backend.namespace_identity()
    );
    assert_eq!(cached.state_identity(), a.backend.state_identity());
    assert_eq!(cached.namespace_identity(), a.backend.namespace_identity());
    assert!(Arc::ptr_eq(
        &crate::vfs::sync_backend(cached.clone()),
        &a.backend
    ));
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
    let context = app
        .bisync_ctx
        .as_ref()
        .expect("finished split sync context");
    assert!(
        context.state.is_some(),
        "split sync returned no engine state"
    );
    assert!(Arc::ptr_eq(&context.a, &a.backend));
    assert!(Arc::ptr_eq(&context.b, &b.backend));
    assert_eq!(context.root_a, a.root);
    assert_eq!(context.root_b, b.root);
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|(text, _)| text.starts_with("Sync:")),
        "split sync did not publish its terminal result: {:?}",
        app.notice
    );
    assert_eq!(
        std::fs::read(b.disk.join("fresh.txt")).unwrap_or_else(|error| {
            panic!(
                "split sync did not publish fresh.txt: {error}; notice={:?}; error={:?}",
                app.notice, app.error_msg
            )
        }),
        b"fresh after browser listing"
    );
    assert!(crate::vfs::validate_sync_roots(&*a.backend, &a.root, &*a.backend, &a.root).is_err());
    assert!(crate::vfs::validate_sync_roots(
        &*a.backend,
        &a.root,
        &*a.backend,
        &format!("{}/sub", a.root)
    )
    .is_err());
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
    assert!(app.bisync_running, "{:?}", app.error_msg);
    assert!(app.desktop_run.is_some());
    assert!(app.bisync_rx.is_some());
    assert!(app.bisync_cancel.is_some());
    assert_eq!(app.running_job.as_deref(), Some(id.as_str()));
    finish(&mut app);
    assert_eq!(
        std::fs::read(b.disk.join("local.txt")).unwrap(),
        b"local source"
    );
    let root = forward(a.disk.parent().unwrap());
    assert!(crate::vfs::validate_sync_roots(
        &LocalBackend::new(&root),
        &root,
        &*a.backend,
        &a.root
    )
    .is_err());
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
    let result = crate::bisync::run(
        &*first.backend,
        "/A",
        &*second.backend,
        "/A",
        crate::bisync::BisyncOptions::default(),
        &AtomicBool::new(false),
        &filter,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.conflicts.is_empty());
    assert_eq!(
        std::fs::read(second.root_a.join("first Ü.txt")).unwrap(),
        b"first peer"
    );
    assert_eq!(
        std::fs::read(first.root_a.join("second %20.txt")).unwrap(),
        b"second peer"
    );
    let pair = result
        .state
        .as_ref()
        .expect("completed Share pair state")
        .pair_id
        .clone();
    for backend in [&first.backend, &second.backend] {
        assert_eq!(
            backend.stat("/A/.se-versions").unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
    let cancel = AtomicBool::new(false);
    let sides = [
        VersionSide {
            side: crate::bisync::PairSide::A,
            backend: &*first.backend,
            root: "/A",
        },
        VersionSide {
            side: crate::bisync::PairSide::B,
            backend: &*second.backend,
            root: "/A",
        },
    ];
    let auto = crate::bisync::BisyncOptions {
        versions: crate::bisync::VersionsLocation::Auto,
        ..Default::default()
    };
    let changed = b"changed first peer with private backup";
    std::fs::write(first.root_a.join("first Ü.txt"), changed).unwrap();
    let replaced = crate::bisync::run(
        &*first.backend,
        "/A",
        &*second.backend,
        "/A",
        auto,
        &cancel,
        &filter,
    );
    assert!(replaced.errors.is_empty(), "{:?}", replaced.errors);
    assert!(replaced.conflicts.is_empty());
    assert_eq!(
        std::fs::read(second.root_a.join("first Ü.txt")).unwrap(),
        changed
    );
    let state = replaced.state.as_ref().expect("completed overwrite state");
    assert_eq!(state.pair_id, pair);
    let entries = versions::list_versions(&state.pair_id, &sides, &cancel).unwrap();
    let original = entries
        .iter()
        .find(|entry| {
            entry.side == Some(crate::bisync::PairSide::B)
                && entry.rel == "first Ü.txt"
                && entry.reason == Some(VersionReason::Replaced)
        })
        .expect("Share overwrite must keep the captured original");
    assert_eq!(original.store, VersionStore::AppData);
    assert_eq!(std::fs::read(&original.stored_path).unwrap(), b"first peer");
    {
        let lock = crate::bisync::PairLock::acquire(&state.lock_id).unwrap();
        versions::restore_version(&lock, &state.pair_id, original, &sides[1], &cancel).unwrap();
    }
    assert_eq!(
        std::fs::read(second.root_a.join("first Ü.txt")).unwrap(),
        b"first peer"
    );
    assert_eq!(
        std::fs::read(first.root_a.join("first Ü.txt")).unwrap(),
        changed
    );
    let entries = versions::list_versions(&state.pair_id, &sides, &cancel).unwrap();
    let retained = entries
        .iter()
        .find(|entry| {
            entry.side == Some(crate::bisync::PairSide::B)
                && entry.rel == "first Ü.txt"
                && entry.reason == Some(VersionReason::Restored)
        })
        .expect("restore must preserve the file it replaces");
    assert_eq!(retained.store, VersionStore::AppData);
    assert_eq!(std::fs::read(&retained.stored_path).unwrap(), changed);
    let restored = crate::bisync::run(
        &*first.backend,
        "/A",
        &*second.backend,
        "/A",
        auto,
        &cancel,
        &filter,
    );
    assert!(restored.errors.is_empty(), "{:?}", restored.errors);
    assert!(restored.conflicts.is_empty());
    assert_eq!(
        std::fs::read(first.root_a.join("first Ü.txt")).unwrap(),
        b"first peer"
    );
    assert_eq!(
        std::fs::read(second.root_a.join("first Ü.txt")).unwrap(),
        b"first peer"
    );
    let local = Location::new(Scheme::Local);
    let result = crate::bisync::run(
        &*first.backend,
        "/A",
        &*local.backend,
        &local.root,
        crate::bisync::BisyncOptions::default(),
        &AtomicBool::new(false),
        &filter,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        std::fs::read(local.disk.join("first Ü.txt")).unwrap(),
        b"first peer"
    );
}
