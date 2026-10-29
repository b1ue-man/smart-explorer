use super::*;
use std::{
    fs,
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn source(root: &std::path::Path) -> Source {
    Source {
        device: format!("test-optical:{}", root.display()),
        generation: "disc-a".into(),
        read_only: true,
    }
}
fn wait_until(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "optical task synchronization timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn batches_task_optical_two_copy_jobs_and_scan_share_device() {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("disc");
    let alias = fixture.path().join("alias");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&alias).unwrap();
    fs::create_dir(alias.join("child")).unwrap();
    let bytes: Vec<u8> = (0..3 * OPTICAL_BLOCK_BYTES)
        .map(|index| (index % 251) as u8)
        .collect();
    fs::write(first.join("file.bin"), &bytes).unwrap();
    fs::write(alias.join("file.bin"), &bytes).unwrap();
    let device = source(&first);
    let _profile = test_profiles::register(&first, device.clone());
    let _alias = test_profiles::register(&alias, device.clone());
    let cancel = AtomicBool::new(false);
    let held = acquire(&first, Priority::Foreground, &cancel)
        .unwrap()
        .unwrap();
    let (index_tx, index_rx) = crossbeam_channel::unbounded();
    crate::folder_index::FolderIndex::build_async(
        vec![alias.clone()],
        fixture.path().join("index.bin"),
        index_tx,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let copy = |root: &std::path::Path, name: &str| {
        let from = root.join("file.bin");
        let to = fixture.path().join(name);
        let request = crate::copy::LocalRequest {
            source: &from,
            target: &to,
            conflict: crate::types::Conflict::Overwrite,
            mode: crate::types::CopyMode::Copy,
            cancel: &cancel,
            parallel: 4,
            probe: None,
        };
        let result = crate::copy::transfer_local(&request, &mut |_| {});
        assert!(
            matches!(result, Ok(crate::copy::LocalOutcome::Completed { .. })),
            "{result:?}"
        );
    };
    std::thread::scope(|scope| {
        let a = scope.spawn(|| copy(&first, "a.bin"));
        let b = scope.spawn(|| copy(&alias, "b.bin"));
        let scan = scope.spawn(|| crate::scanner::collect_recursive(&alias, false, 1, &cancel));
        wait_until(|| {
            snapshots().iter().any(|status| {
                status.source.device == device.device && status.foreground_waiters >= 3
            })
        });
        drop(held);
        a.join().unwrap();
        b.join().unwrap();
        assert!(scan.join().unwrap().is_complete());
    });
    loop {
        match index_rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            crate::folder_index::IndexMsg::Complete(index) => {
                assert_eq!(index.len(), 2);
                break;
            }
            crate::folder_index::IndexMsg::Progress { .. } => {}
            crate::folder_index::IndexMsg::Failed(error) => panic!("{error}"),
            crate::folder_index::IndexMsg::Canceled => {
                panic!("background index unexpectedly canceled")
            }
        }
    }
    assert_eq!(test_profiles::maximum(&device.device), 1);
    use sha2::{Digest, Sha256};
    for name in ["a.bin", "b.bin"] {
        assert_eq!(
            Sha256::digest(fs::read(fixture.path().join(name)).unwrap()),
            Sha256::digest(&bytes)
        );
    }
}

#[test]
fn batches_task_optical_wait_cancel_foreground_priority_and_independent_drive() {
    let fixture = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let device = source(fixture.path());
    let _profile = test_profiles::register(fixture.path(), device.clone());
    let _second = test_profiles::register(second.path(), source(second.path()));
    let cancel = AtomicBool::new(false);
    let first = acquire(fixture.path(), Priority::Foreground, &cancel)
        .unwrap()
        .unwrap();
    let independent = acquire(second.path(), Priority::Foreground, &cancel)
        .unwrap()
        .unwrap();
    drop(independent);
    let waiting_cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let waiting =
            scope.spawn(|| acquire(fixture.path(), Priority::Background, &waiting_cancel));
        wait_until(|| {
            snapshots().iter().any(|status| {
                status.source.device == device.device && status.background_waiters == 1
            })
        });
        waiting_cancel.store(true, Ordering::Release);
        assert!(
            matches!(waiting.join().unwrap(), Err(error) if error.kind() == std::io::ErrorKind::Interrupted)
        );
    });
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    std::thread::scope(|scope| {
        let fg = scope.spawn(|| {
            let _guard = acquire(fixture.path(), Priority::Foreground, &cancel).unwrap();
            order.lock().unwrap().push("foreground");
        });
        wait_until(|| {
            snapshots().iter().any(|status| {
                status.source.device == device.device && status.foreground_waiters == 1
            })
        });
        let bg = scope.spawn(|| {
            let _guard = acquire(fixture.path(), Priority::Background, &cancel).unwrap();
            order.lock().unwrap().push("background");
        });
        wait_until(|| {
            snapshots().iter().any(|status| {
                status.source.device == device.device && status.background_waiters == 1
            })
        });
        drop(first);
        fg.join().unwrap();
        bg.join().unwrap();
    });
    assert_eq!(*order.lock().unwrap(), ["foreground", "background"]);
}

#[test]
fn batches_task_optical_media_change_rejects_open_reader_and_queued_index() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("file.bin"), b"original").unwrap();
    let initial = source(fixture.path());
    let profile = test_profiles::register(fixture.path(), initial.clone());
    let mut reader = guard_reader(&fixture.path().join("file.bin"), || {
        fs::File::open(fixture.path().join("file.bin"))
    })
    .unwrap();
    let mut changed = initial;
    changed.generation = "disc-b".into();
    profile.replace(changed);
    assert!(reader.read(&mut [0u8; 8]).is_err());
    drop(reader);
    let cancel = AtomicBool::new(false);
    let mut listing = GuardedIterator::new(fixture.path(), Priority::Background, &cancel, || {
        fs::read_dir(fixture.path())
    })
    .unwrap();
    let mut changed = source(fixture.path());
    changed.generation = "disc-c".into();
    profile.replace(changed);
    assert!(listing.next().unwrap().is_err());
}

#[test]
fn batches_task_optical_process_child() {
    let Some(root) = std::env::var_os("SE_TEST_OPTICAL_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let device = source(&root);
    let _profile = test_profiles::register(&root, device);
    fs::write(root.join("child-ready"), b"ready").unwrap();
    let cancel = AtomicBool::new(false);
    let _guard = acquire(&root, Priority::Foreground, &cancel)
        .unwrap()
        .unwrap();
    assert!(
        !root.join("parent-holds").exists(),
        "a second process entered the held physical reader"
    );
    fs::write(root.join("child-entered"), b"entered").unwrap();
}

#[test]
fn batches_task_optical_process_lock_excludes_second_process() {
    let fixture = tempfile::tempdir().unwrap();
    let _profile = test_profiles::register(fixture.path(), source(fixture.path()));
    let cancel = AtomicBool::new(false);
    let guard = acquire(fixture.path(), Priority::Foreground, &cancel)
        .unwrap()
        .unwrap();
    fs::write(fixture.path().join("parent-holds"), b"held").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "source_media::tests::batches_task_optical_process_child",
            "--exact",
        ])
        .env("SE_TEST_OPTICAL_ROOT", fixture.path())
        .spawn()
        .unwrap();
    wait_until(|| fixture.path().join("child-ready").exists());
    std::thread::sleep(Duration::from_millis(100));
    assert!(!fixture.path().join("child-entered").exists());
    fs::remove_file(fixture.path().join("parent-holds")).unwrap();
    drop(guard);
    wait_until(|| child.try_wait().unwrap().is_some());
    assert!(child.wait().unwrap().success());
    assert!(fixture.path().join("child-entered").exists());
}

#[test]
fn batches_task_optical_confined_reader_uses_held_root_and_cancel() {
    use crate::vfs::BackendExtensions;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("root");
    let moved = fixture.path().join("moved");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file.bin"), b"original").unwrap();
    let original = source(&root);
    let old_profile = test_profiles::register(&root, original.clone());
    // The confined reader probes its handle, whose final path has the long
    // names (`GetFinalPathNameByHandleW`); a temp folder may be spelled with
    // 8.3 short names (`C:\Users\RUNNER~1\…`). The device is the same for
    // both spellings.
    #[cfg(windows)]
    let _final_profile =
        test_profiles::register(&fs::canonicalize(&root).unwrap(), original.clone());
    let text = root.to_string_lossy().replace('\\', "/");
    let backend = crate::vfs::LocalBackend::new(&text)
        .confined_local_root(&text)
        .unwrap()
        .unwrap();
    #[cfg(windows)]
    {
        let error = fs::rename(&root, &moved).unwrap_err();
        assert_eq!(
            error.raw_os_error(),
            Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
        );
        assert!(!moved.exists());
        assert_eq!(fs::read(root.join("file.bin")).unwrap(), b"original");
    }
    #[cfg(not(windows))]
    let moved_profile = {
        fs::rename(&root, &moved).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file.bin"), b"replacement").unwrap();
        let profile = test_profiles::register(&moved, original.clone());
        old_profile.replace(Source {
            device: format!("replacement:{}", root.display()),
            generation: "different".into(),
            read_only: true,
        });
        profile
    };
    let effective_root = if cfg!(windows) { &root } else { &moved };
    let cancel = AtomicBool::new(false);
    let held = acquire(effective_root, Priority::Foreground, &cancel)
        .unwrap()
        .unwrap();
    let waiting_cancel = AtomicBool::new(false);
    let path = format!("{text}/file.bin");
    std::thread::scope(|scope| {
        let waiting =
            scope.spawn(|| with_cancellation(&waiting_cancel, || backend.open_read(&path)));
        wait_until(|| {
            snapshots().iter().any(|status| {
                status.source.device == original.device && status.foreground_waiters == 1
            })
        });
        waiting_cancel.store(true, Ordering::Release);
        assert!(
            matches!(waiting.join().unwrap(), Err(error) if error.kind() == std::io::ErrorKind::Interrupted)
        );
    });
    drop(held);
    let mut reader = backend.open_read(&path).unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original");
    #[cfg(not(windows))]
    drop(moved_profile);
    drop(old_profile);
}
