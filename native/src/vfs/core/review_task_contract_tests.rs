//! RV1 K1 milestone tests of the VFS contract: time precision, target
//! limits, filesystem profiles, stage names, error classes and the
//! fallbacks of backends without extensions.
use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::fs_profile::{linux_profile, mount_kind, windows_profile, FlushModel};
use super::*;
use crate::local_access::NotRegular;
use crate::types::Win32NameIssue;

#[test]
fn review_task_mtime_precision_compares_within_one_step() {
    assert!(MtimePrecision::Nanos.same_instant(1_000, 1_000));
    assert!(!MtimePrecision::Nanos.same_instant(1_000, 1_001));
    assert!(MtimePrecision::TenMillis.same_instant(5, 14));
    assert!(!MtimePrecision::TenMillis.same_instant(5, 15));
    assert!(MtimePrecision::TwoSeconds.same_instant(10_000, 11_999));
    assert!(!MtimePrecision::TwoSeconds.same_instant(10_000, 12_000));
    assert!(MtimePrecision::Days.same_instant(0, 86_399_999));
    assert!(MtimePrecision::Unknown.same_instant(7, 7));
    assert!(!MtimePrecision::Unknown.same_instant(7, 8));
    assert_eq!(
        MtimePrecision::Nanos.coarser(MtimePrecision::TwoSeconds),
        MtimePrecision::TwoSeconds
    );
    assert_eq!(
        MtimePrecision::Seconds.coarser(MtimePrecision::Unknown),
        MtimePrecision::Unknown
    );
    assert!(MtimePrecision::TwoSeconds.local_time_shifts());
    assert!(!MtimePrecision::TenMillis.local_time_shifts());
    assert_eq!(MtimePrecision::default(), MtimePrecision::Unknown);
}

#[test]
fn review_task_target_limits_report_impossible_names_and_sizes() {
    let ntfs = windows_profile("NTFS", 255).limits;
    assert_eq!(
        ntfs.name_issue("Meeting 10:30.txt"),
        Some(NameIssue::Windows(Win32NameIssue::InvalidCharacter))
    );
    assert_eq!(
        ntfs.name_issue("CON.txt"),
        Some(NameIssue::Windows(Win32NameIssue::ReservedDevice))
    );
    assert_eq!(
        ntfs.name_issue("notes."),
        Some(NameIssue::Windows(Win32NameIssue::TrailingDotOrSpace))
    );
    assert_eq!(ntfs.name_issue(&"x".repeat(256)), Some(NameIssue::TooLong));
    assert_eq!(ntfs.name_issue(&"x".repeat(255)), None);
    assert_eq!(ntfs.mtime_precision, MtimePrecision::Nanos);
    assert!(ntfs.fits_size(u64::MAX));

    let ext4 = linux_profile("ext4").limits;
    assert!(!ext4.windows_names);
    assert_eq!(ext4.name_issue("Meeting 10:30.txt"), None);
    assert_eq!(ext4.name_issue(&"é".repeat(128)), Some(NameIssue::TooLong));
    assert_eq!(ext4.name_issue(&"é".repeat(127)), None);

    for fat in [
        linux_profile("vfat").limits,
        windows_profile("FAT32", 255).limits,
    ] {
        assert!(fat.windows_names);
        assert_eq!(fat.mtime_precision, MtimePrecision::TwoSeconds);
        assert!(fat.fits_size(0xFFFF_FFFF));
        assert!(!fat.fits_size(0x1_0000_0000));
    }
    let exfat = linux_profile("exfat").limits;
    assert_eq!(exfat.mtime_precision, MtimePrecision::TenMillis);
    assert!(exfat.fits_size(0x1_0000_0000));
    assert_eq!(TargetLimits::default().name_issue(&"x".repeat(999)), None);
}

#[test]
fn review_task_fs_profiles_choose_flushing_and_mount_kinds() {
    assert_eq!(linux_profile("ext4").flush, FlushModel::Batched);
    assert_eq!(linux_profile("vfat").flush, FlushModel::Batched);
    assert_eq!(linux_profile("nfs4").flush, FlushModel::PerFile);
    assert_eq!(linux_profile("fuse.sshfs").flush, FlushModel::PerFileOnly);
    assert_eq!(linux_profile("cifs").flush, FlushModel::PerFileOnly);
    assert_eq!(linux_profile("").flush, FlushModel::PerFileOnly);
    assert_eq!(windows_profile("NTFS", 255).flush, FlushModel::PerFile);
    assert_eq!(mount_kind("proc"), MountKind::Pseudo);
    assert_eq!(mount_kind("fusectl"), MountKind::Pseudo);
    assert_eq!(mount_kind("fuse.sshfs"), MountKind::Fuse);
    assert_eq!(mount_kind("fuseblk"), MountKind::Fuse);
    assert_eq!(mount_kind("nfs4"), MountKind::Network);
    assert_eq!(mount_kind("autofs"), MountKind::Automount);
    assert_eq!(mount_kind("ext4"), MountKind::Local);
}

#[test]
fn review_task_staging_names_are_recognized_and_fit_one_component() {
    for own in [
        "photo.jpg.se-bisync-0123456789abcdef",
        "photo.jpg.se-mount-delete-0123456789abcdef",
        ".photo.jpg.smart-explorer-0123456789abcdef.part",
        ".photo.jpg.smart-explorer-0123456789abcdef.move",
        ".photo.jpg.smart-explorer-4242-1a2b-0.part",
        ".photo.jpg.smart-explorer.part",
        "photo.jpg.se-agent-batch-0123456789abcdef-1.part",
        "photo.jpg.se-upload-4242-1a2b3c.part",
        ".se-agent-tree-1f-2a-0.spool",
    ] {
        assert!(is_staging_name(own), "{own}");
    }
    for user in [
        "photo.jpg",
        "notes.se-draft",
        "x.se-sync-012345",
        ".se-bisync-0123456789abcdef",
        "report.smart-explorer.pdf",
        ".photo.jpg.smart-explorer-android.part",
    ] {
        assert!(!is_staging_name(user), "{user}");
    }
    assert!(is_staging_name(
        ".se-private-0123456789abcdef0123456789abcdef.tmp"
    ));
    for user in [
        "notes.tmp",
        ".se-private-user.tmp",
        ".se-private-0123456789abcdef0123456789abcde.tmp",
        ".se-private-0123456789abcdef0123456789abcdef0.tmp",
        ".se-private-0123456789abcdef0123456789abcdeg.tmp",
        "prefix.se-private-0123456789abcdef0123456789abcdef.tmp",
    ] {
        assert!(!is_staging_name(user), "{user}");
    }
    let tail = ".se-sync-0123456789abcdef";
    assert_eq!(
        fit_stage_name("", "short.txt", tail),
        format!("short.txt{tail}")
    );
    for name in ["a".repeat(300), "日".repeat(120), "😀".repeat(200)] {
        let fitted = fit_stage_name("", &name, tail);
        assert!(fitted.len() <= 255, "{} bytes", fitted.len());
        assert!(fitted.encode_utf16().count() <= 255);
        assert!(fitted.ends_with(tail));
        assert!(is_staging_name(&fitted));
        let copied = fit_stage_name(".", &name, ".smart-explorer-0123456789abcdef.part");
        assert!(copied.len() <= 255 && is_staging_name(&copied));
    }
}

#[test]
fn review_task_error_classes_map_refusals_and_omissions() {
    for kind in [
        io::ErrorKind::StorageFull,
        io::ErrorKind::QuotaExceeded,
        io::ErrorKind::ReadOnlyFilesystem,
    ] {
        assert!(is_target_refusal(&io::Error::from(kind)));
        let wrapped = io::Error::other(io::Error::new(kind, "voll"));
        assert!(is_target_refusal(&wrapped), "{kind:?} wrapped");
    }
    assert!(!is_target_refusal(&io::Error::from(
        io::ErrorKind::PermissionDenied
    )));
    let reason = |error: io::Error| omission_reason(&error);
    assert_eq!(
        reason(io::ErrorKind::NotFound.into()),
        Some(OmissionReason::Vanished)
    );
    assert_eq!(
        reason(io::ErrorKind::PermissionDenied.into()),
        Some(OmissionReason::Unreadable)
    );
    assert_eq!(
        reason(io::ErrorKind::InvalidFilename.into()),
        Some(OmissionReason::Unrepresentable)
    );
    assert_eq!(reason(NotRegular::Link.error()), Some(OmissionReason::Link));
    assert_eq!(
        reason(NotRegular::Special.error()),
        Some(OmissionReason::Special)
    );
    assert_eq!(
        reason(NotRegular::Directory.error()),
        Some(OmissionReason::Vanished)
    );
    assert_eq!(reason(io::Error::other("timeout")), None);
}

/// A backend with only the core interface.
struct Plain;

impl Backend for Plain {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        Ok(vec![VfsMeta {
            name: "a".into(),
            ..VfsMeta::default()
        }])
    }
    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(io::empty()))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(io::sink()))
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Ok(())
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
}

#[test]
fn review_task_backends_without_extensions_get_safe_fallbacks() {
    let backend = Plain;
    let listing = list_dir_tolerant(&backend, "/").unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert!(listing.omitted.is_empty());
    let finished = finish_stage(
        &backend,
        "/a",
        StageFinish {
            mtime_ms: Some(1),
            mode: Some(0o600),
            durability: StageDurability::Now,
        },
    )
    .unwrap();
    assert_eq!(finished, StageFinished::default());
    assert!(!sync_filesystem(&backend, "/").unwrap());
    assert_eq!(target_limits(&backend, "/"), TargetLimits::default());
    assert_eq!(mtime_precision(&backend, "/"), MtimePrecision::Unknown);
    assert_eq!(unix_mode(&backend, "/a").unwrap(), None);
    assert_eq!(volume_identity(&backend, "/").unwrap(), None);
    assert!(!supports_duplicate_search(&backend, "/").unwrap());
    let progress = crate::analytics::ReclaimProgress::default();
    assert!(find_duplicates(&backend, "/", 1, &progress)
        .unwrap()
        .is_none());
    let (tx, _rx) = crossbeam_channel::unbounded();
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
    };
    assert!(!hash_walk(&backend, "/", request, tx, &AtomicBool::new(false)).unwrap());
    let expected = RecycleExpectation {
        size: 1,
        sha256: None,
    };
    assert_eq!(
        recycle(&backend, "/a", &expected).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(change_signal_mode(&backend, "/").unwrap(), None);
    let (tx, _rx) = crossbeam_channel::unbounded();
    assert!(change_signal(&backend, "/", Duration::from_secs(300), tx)
        .unwrap()
        .is_none());
}

#[test]
fn review_task_caching_backend_forwards_local_extensions() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().to_string_lossy().replace('\\', "/");
    let local = LocalBackend::new(&root);
    let cached = CachingBackend::new(std::sync::Arc::new(LocalBackend::new(&root)));
    assert_eq!(target_limits(&cached, &root), target_limits(&local, &root));
    let stage = format!("{root}/kept.txt.se-sync-0123456789abcdef");
    let mut writer = open_write_copy_stage_timed(&cached, &stage, 2, 0).unwrap();
    writer.write_all(b"ok").unwrap();
    drop(writer);
    let finished = finish_stage(
        &cached,
        &stage,
        StageFinish {
            mtime_ms: Some(1_600_000_000_000),
            mode: None,
            durability: StageDurability::Deferred,
        },
    )
    .unwrap();
    assert!(finished.mtime_applied && finished.durable);
    assert_eq!(local.stat(&stage).unwrap().mtime_ms, 1_600_000_000_000);
    let listing = list_dir_tolerant(&cached, &root).unwrap();
    assert_eq!(listing.entries.len(), 1);
}
