//! RV1 K1 milestone tests of the local backend: stage finishing, durable
//! and private stages, folder creation below links, tolerant listings,
//! special files, refused links, target limits, identities and Windows
//! names and read-only replacement.
use std::io::Write;

use super::*;

fn fixture() -> (tempfile::TempDir, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_string_lossy().replace('\\', "/");
    (directory, root)
}

#[test]
fn review_task_finish_stage_applies_time_mode_and_flush() {
    let (_directory, root) = fixture();
    let backend = LocalBackend::new(&root);
    let stage = format!("{root}/report.txt.se-sync-0123456789abcdef");
    let mut writer = open_write_copy_stage_timed(&backend, &stage, 3, 0).unwrap();
    writer.write_all(b"abc").unwrap();
    drop(writer);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&stage).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "a stage starts private");
    }
    let mtime_ms = 1_600_000_000_123;
    let finished = finish_stage(
        &backend,
        &stage,
        StageFinish {
            mtime_ms: Some(mtime_ms),
            mode: Some(0o4640),
            durability: StageDurability::Now,
        },
    )
    .unwrap();
    assert!(finished.mtime_applied && finished.durable);
    let destination = format!("{root}/report.txt");
    promote_staged_create(&backend, &stage, &destination).unwrap();
    let published = backend.stat(&destination).unwrap();
    assert_eq!(published.mtime_ms, mtime_ms, "publishing keeps the time");
    assert_eq!(published.size, 3);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&destination)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o640, "set-id bits are never copied");
        assert_eq!(unix_mode(&backend, &destination).unwrap(), Some(0o640));
    }
    let nothing = finish_stage(&backend, &destination, StageFinish::default()).unwrap();
    assert_eq!(nothing, StageFinished::default());
}

#[test]
fn review_task_sized_stages_are_durable_and_unsynced_stages_plain() {
    let (_directory, root) = fixture();
    let backend = LocalBackend::new(&root);
    for (name, durable) in [("durable.bin", true), ("plain.bin", false)] {
        let stage = format!("{root}/{name}");
        let mut writer = if durable {
            backend.open_write_copy_stage_sized(&stage, 5).unwrap()
        } else {
            backend.open_write_copy_stage_unsynced(&stage, 5).unwrap()
        };
        writer.write_all(b"hello").unwrap();
        writer.flush().unwrap();
        drop(writer);
        assert_eq!(std::fs::read(&stage).unwrap(), b"hello");
        assert_eq!(
            backend
                .open_write_copy_stage_sized(&stage, 5)
                .err()
                .map(|e| e.kind()),
            Some(std::io::ErrorKind::AlreadyExists),
            "a stage never replaces an existing name"
        );
    }
}

#[test]
fn review_task_sync_filesystem_and_target_limits_of_a_local_root() {
    let (_directory, root) = fixture();
    let backend = LocalBackend::new(&root);
    let durable = sync_filesystem(&backend, &root).unwrap();
    let limits = target_limits(&backend, &root);
    let missing = target_limits(&backend, &format!("{root}/not/yet/created"));
    assert_eq!(missing, limits, "a missing root is judged by its ancestor");
    assert!(limits.max_name.is_some());
    if cfg!(windows) {
        assert!(durable);
        assert!(limits.windows_names);
    } else if limits.mtime_precision == MtimePrecision::Nanos {
        assert!(durable, "a local disk is flushed as a whole");
    }
}

#[test]
fn review_task_volume_identity_is_stable_for_one_location() {
    let (_directory, root) = fixture();
    let first = local_volume_identity(&root).unwrap();
    let second = volume_identity(&LocalBackend::new(&root), &root).unwrap();
    assert_eq!(first, second);
    if cfg!(windows) {
        let identity = first.expect("NTFS volumes have a serial number");
        assert_eq!(identity.volume_id.len(), 16);
        let leaf = root.rsplit('/').next().unwrap().to_lowercase();
        assert!(identity.relative_path.to_lowercase().ends_with(&leaf));
    } else if let Some(identity) = first {
        let leaf = root.rsplit('/').next().unwrap();
        assert!(identity.relative_path.ends_with(leaf), "{identity:?}");
        assert!(!identity.volume_id.is_empty());
    }
}

#[test]
fn review_task_unique_stage_names_fit_long_destination_names() {
    let (_directory, root) = fixture();
    let backend = LocalBackend::new(&root);
    let long = "Ä".repeat(120);
    let destination = format!("{root}/{long}.pdf");
    let stage = unique_staging_path(&backend, &destination, "bisync").unwrap();
    let name = stage.rsplit('/').next().unwrap();
    assert!(name.len() <= 255 && name.encode_utf16().count() <= 255);
    assert!(is_staging_name(name));
    let mut writer = backend.open_write_new(&stage).unwrap();
    writer.write_all(b"x").unwrap();
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;

    #[test]
    fn review_task_mkdir_all_follows_links_above_the_root_only() {
        let (_directory, base) = fixture();
        std::fs::create_dir_all(format!("{base}/real")).unwrap();
        std::fs::create_dir_all(format!("{base}/victim")).unwrap();
        symlink(format!("{base}/real"), format!("{base}/alias")).unwrap();
        let root = format!("{base}/alias/backup");
        let backend = LocalBackend::new(&root);
        backend.mkdir_all(&format!("{root}/a/b")).unwrap();
        assert!(std::path::Path::new(&format!("{base}/real/backup/a/b")).is_dir());
        backend.mkdir_all(&format!("{root}/a/b")).unwrap();
        symlink(format!("{base}/victim"), format!("{root}/escape")).unwrap();
        let error = backend.mkdir_all(&format!("{root}/escape/x")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!std::path::Path::new(&format!("{base}/victim/x")).exists());
        let global = LocalBackend::new("/");
        assert!(
            global.mkdir_all(&format!("{base}/alias/other")).is_err(),
            "outside a chosen root every link is refused"
        );
    }

    #[test]
    fn review_task_tolerant_listing_names_unlistable_entries() {
        let (_directory, root) = fixture();
        std::fs::write(format!("{root}/ok.txt"), b"1").unwrap();
        let raw = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
        std::fs::write(std::path::Path::new(&root).join(raw), b"2").unwrap();
        let backend = LocalBackend::new(&root);
        assert!(
            backend.list_dir(&root).is_err(),
            "the strict listing is unchanged"
        );
        let listing = list_dir_tolerant(&backend, &root).unwrap();
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["ok.txt"]);
        assert_eq!(listing.omitted.len(), 1);
        assert_eq!(listing.omitted[0].reason, OmissionReason::Unrepresentable);
        assert_eq!(listing.omitted[0].rel, "caf\u{FFFD}.txt");
    }

    #[test]
    fn review_task_tolerant_listing_keeps_going_past_unreadable_entries() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: plain getter without arguments.
        if unsafe { libc::geteuid() } == 0 {
            return; // root reads everything; the denial cannot be staged
        }
        let (_directory, root) = fixture();
        let locked = format!("{root}/locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(format!("{locked}/inside.txt"), b"1").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        let listing = list_dir_tolerant(&LocalBackend::new(&root), &locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let listing = listing.unwrap();
        assert!(listing.entries.is_empty());
        assert_eq!(listing.omitted.len(), 1);
        assert_eq!(listing.omitted[0].rel, "inside.txt");
        assert_eq!(listing.omitted[0].reason, OmissionReason::Unreadable);
    }

    #[test]
    fn review_task_special_files_are_flagged_and_never_block() {
        let (_directory, root) = fixture();
        let fifo = format!("{root}/pipe");
        let path = std::ffi::CString::new(fifo.as_bytes()).unwrap();
        // SAFETY: NUL-terminated path, plain mode bits.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let socket = format!("{root}/socket");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let backend = LocalBackend::new(&root);
        for entry in backend.list_dir(&root).unwrap() {
            assert!(
                entry.special && !entry.is_dir && entry.size == 0,
                "{entry:?}"
            );
        }
        assert!(backend.stat(&fifo).unwrap().special);
        assert!(backend.stat(&socket).unwrap().special);
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let backend = LocalBackend::new("/");
            let plain = backend.open_read(&fifo).err();
            let regular = open_read_regular(&backend, &fifo, None).err();
            let _ = sender.send((plain, regular));
        });
        let (plain, regular) = receiver
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("opening a FIFO must not wait for a writer");
        reader.join().unwrap();
        for error in [plain, regular] {
            let error = error.expect("a FIFO is refused");
            assert_eq!(omission_reason(&error), Some(OmissionReason::Special));
        }
    }

    #[test]
    fn review_task_reading_listed_files_refuses_links_explorer_reads_follow() {
        let (_directory, root) = fixture();
        std::fs::write(format!("{root}/target.txt"), b"data").unwrap();
        symlink(format!("{root}/target.txt"), format!("{root}/link.txt")).unwrap();
        let backend = LocalBackend::new(&root);
        let error = open_read_regular(&backend, &format!("{root}/link.txt"), None)
            .err()
            .expect("a link is never read as a listed file");
        assert_eq!(omission_reason(&error), Some(OmissionReason::Link));
        let mut text = String::new();
        backend
            .open_read(&format!("{root}/link.txt"))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "data");
    }

    #[test]
    fn review_task_mount_boundaries_name_pseudo_mounts() {
        let (_directory, root) = fixture();
        std::fs::create_dir(format!("{root}/plain")).unwrap();
        assert_eq!(
            local_mount_boundary(&format!("{root}/plain")).unwrap(),
            None
        );
        if std::path::Path::new("/proc/self").exists() {
            assert_eq!(
                local_mount_boundary("/proc").unwrap(),
                Some(MountKind::Pseudo)
            );
        }
    }

    fn suite_directory(variable: &str) -> String {
        std::env::var(variable)
            .unwrap_or_else(|_| panic!("{variable} names the prepared mount"))
            .replace('\\', "/")
    }

    /// Suite stage: a writable directory on NFS, sshfs or ntfs-3g.
    #[test]
    #[ignore = "needs an NFS/FUSE mount named by SE_REVIEW_NOREPLACE_DIR"]
    fn review_task_publishing_never_replaces_on_network_and_fuse_mounts() {
        let directory = suite_directory("SE_REVIEW_NOREPLACE_DIR");
        let backend = LocalBackend::new(&directory);
        let destination = format!("{directory}/review-task-published.txt");
        let _ = std::fs::remove_file(&destination);
        for round in 0..2 {
            let stage = unique_staging_path(&backend, &destination, "sync").unwrap();
            std::fs::write(&stage, format!("round {round}")).unwrap();
            let published = promote_staged_create(&backend, &stage, &destination);
            if round == 0 {
                published.unwrap();
            } else {
                assert_eq!(
                    published.unwrap_err().kind(),
                    std::io::ErrorKind::AlreadyExists
                );
                std::fs::remove_file(&stage).unwrap();
            }
        }
        assert_eq!(std::fs::read(&destination).unwrap(), b"round 0");
        std::fs::remove_file(&destination).unwrap();
    }

    /// Suite stage: FAT32 and exFAT loop images mounted for this user.
    #[test]
    #[ignore = "needs FAT32/exFAT mounts named by SE_REVIEW_FAT_DIR and SE_REVIEW_EXFAT_DIR"]
    fn review_task_fat_and_exfat_targets_report_their_limits() {
        for (variable, precision, max_file_size) in [
            (
                "SE_REVIEW_FAT_DIR",
                MtimePrecision::TwoSeconds,
                Some(0xFFFF_FFFF),
            ),
            ("SE_REVIEW_EXFAT_DIR", MtimePrecision::TenMillis, None),
        ] {
            let directory = suite_directory(variable);
            let backend = LocalBackend::new(&directory);
            let limits = target_limits(&backend, &directory);
            assert_eq!(limits.mtime_precision, precision, "{variable}");
            assert_eq!(limits.max_file_size, max_file_size, "{variable}");
            assert!(limits.windows_names, "{variable}");
            let identity = local_volume_identity(&directory)
                .unwrap()
                .expect("the real image has a UUID");
            let batched_flush =
                crate::vfs::fs_profile::mount_kind(&identity.fs_type) != MountKind::Fuse;
            let stage = format!("{directory}/review-task.txt.se-sync-0123456789abcdef");
            std::fs::write(&stage, b"x").unwrap();
            let wanted = 1_600_000_001_234;
            let finish = StageFinish {
                mtime_ms: Some(wanted),
                mode: Some(0o600),
                durability: StageDurability::Deferred,
            };
            assert!(
                finish_stage(&backend, &stage, finish)
                    .unwrap()
                    .mtime_applied
            );
            let stored = backend.stat(&stage).unwrap().mtime_ms;
            assert!(
                precision.same_instant(stored, wanted),
                "{stored} vs {wanted}"
            );
            assert_eq!(
                sync_filesystem(&backend, &directory).unwrap(),
                batched_flush
            );
            std::fs::remove_file(&stage).unwrap();
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;

    #[test]
    fn review_task_windows_colon_names_never_become_streams() {
        let (_directory, root) = fixture();
        let backend = LocalBackend::new(&root);
        std::fs::write(format!("{root}/Meeting 10"), b"keep").unwrap();
        let error = backend
            .open_write_new(&format!("{root}/Meeting 10:30.txt"))
            .err()
            .expect("':' is refused");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidFilename);
        assert!(backend.mkdir_all(&format!("{root}/a:b/c")).is_err());
        assert_eq!(
            std::fs::read(format!("{root}/Meeting 10")).unwrap(),
            b"keep"
        );
        let limits = target_limits(&backend, &root);
        assert!(limits.name_issue("Meeting 10:30.txt").is_some());
        assert!(limits.name_issue("aux.txt").is_some());
    }

    #[test]
    fn review_task_windows_read_only_destination_is_replaced() {
        let (_directory, root) = fixture();
        let backend = LocalBackend::new(&root);
        let destination = format!("{root}/locked.txt");
        std::fs::write(&destination, b"old").unwrap();
        let mut permissions = std::fs::metadata(&destination).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&destination, permissions).unwrap();
        let stage = unique_staging_path(&backend, &destination, "sync").unwrap();
        std::fs::write(&stage, b"new").unwrap();
        promote_staged_replace(&backend, &stage, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        let metadata = std::fs::metadata(&destination).unwrap();
        assert!(metadata.permissions().readonly(), "the attribute stays");
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&destination, permissions).unwrap();
    }

    #[test]
    fn review_task_windows_junction_above_root_is_followed_below_refused() {
        let (_directory, base) = fixture();
        std::fs::create_dir_all(format!("{base}/real")).unwrap();
        std::fs::create_dir_all(format!("{base}/victim")).unwrap();
        let junction = |link: &str, target: &str| {
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link.replace('/', "\\"))
                .arg(target.replace('/', "\\"))
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        };
        junction(&format!("{base}/alias"), &format!("{base}/real"));
        let root = format!("{base}/alias/backup");
        let backend = LocalBackend::new(&root);
        backend.mkdir_all(&format!("{root}/a/b")).unwrap();
        assert!(std::path::Path::new(&format!("{base}/real/backup/a/b")).is_dir());
        junction(&format!("{root}/escape"), &format!("{base}/victim"));
        let error = backend.mkdir_all(&format!("{root}/escape/x")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!std::path::Path::new(&format!("{base}/victim/x")).exists());
    }
}
