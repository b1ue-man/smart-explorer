//! RV1 K1 milestone tests of the mountinfo reader.
use std::path::Path;

use super::mountinfo::{boundary, containing, parse, resolve_existing, split_device};

#[test]
fn review_task_mountinfo_lines_parse_with_escapes() {
    let text = b"22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw\n\
36 22 0:45 /@home /home rw - btrfs /dev/nvme0n1p3 rw,subvol=/@home\n\
40 22 0:50 / /mnt/My\\040Disk rw - vfat /dev/sdb1 rw\n\
41 40 0:51 / /mnt/My\\040Disk/nested rw master:2 - fuse.sshfs user@host:/srv rw\n\
broken line\n";
    let mounts = parse(text);
    assert_eq!(mounts.len(), 4);
    assert_eq!(mounts[0].device, (8, 2));
    assert_eq!(mounts[1].root, Path::new("/@home"));
    assert_eq!(mounts[2].mount_point, Path::new("/mnt/My Disk"));
    assert_eq!(mounts[3].fs_type, "fuse.sshfs");
    assert_eq!(mounts[3].source, "user@host:/srv");
    let holder =
        |path: &str| containing(&mounts, Path::new(path)).map(|mount| mount.fs_type.as_str());
    assert_eq!(holder("/home/user/x"), Some("btrfs"));
    assert_eq!(holder("/mnt/My Disk/nested/a"), Some("fuse.sshfs"));
    assert_eq!(holder("/mnt/My Disk/a"), Some("vfat"));
    assert_eq!(holder("/homes"), Some("ext4"));
    let covered = parse(b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 0:9 / / rw - tmpfs none rw\n");
    assert_eq!(
        containing(&covered, Path::new("/x")).map(|mount| mount.fs_type.as_str()),
        Some("tmpfs"),
        "a later mount over the same point covers the earlier one"
    );
}

#[test]
fn review_task_device_numbers_split_like_glibc() {
    assert_eq!(split_device(0x0802), (8, 2));
    assert_eq!(split_device(0x1_0301), (259, 1));
    assert_eq!(split_device(0x1230_0045), (0, 0x12345));
}

#[test]
fn review_task_missing_paths_resolve_through_their_ancestor() {
    let fixture = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(fixture.path()).unwrap();
    let resolved = resolve_existing(&fixture.path().join("a/b")).unwrap();
    assert_eq!(resolved, canonical.join("a").join("b"));
}

#[test]
fn review_task_mount_boundaries_include_same_device_binds_and_autofs() {
    let mounts = parse(b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 8:1 /home/shared /backup/bind rw - ext4 /dev/sda1 rw\n\
3 1 0:7 / /backup/trigger rw - autofs systemd-1 rw\n\
4 1 0:8 / /backup/network rw - nfs4 host:/srv rw\n\
5 1 0:9 / /backup/fuse rw - fuse.sshfs host:/srv rw\n");
    assert_eq!(boundary(&mounts, Path::new("/backup/bind")), Some(crate::vfs::MountKind::Local));
    assert_eq!(boundary(&mounts, Path::new("/backup/trigger")), Some(crate::vfs::MountKind::Automount));
    assert_eq!(boundary(&mounts, Path::new("/backup/network")), Some(crate::vfs::MountKind::Network));
    assert_eq!(boundary(&mounts, Path::new("/backup/fuse")), Some(crate::vfs::MountKind::Fuse));
    assert_eq!(boundary(&mounts, Path::new("/backup/plain")), None);
}
