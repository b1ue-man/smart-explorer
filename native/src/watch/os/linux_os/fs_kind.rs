//! How completely inotify reports changes below a root (Linux, Android):
//! inotify sees operations made through this machine's VFS, so network and
//! most FUSE file systems miss changes made elsewhere (`LocalOnly`, the
//! consumer polls as well), and Android's shared storage misses writes the
//! media provider makes on the lower file system (`SharedStorage`).

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
#[cfg(not(target_os = "android"))]
use std::path::PathBuf;

use super::types::Coverage;

const FUSE_SUPER_MAGIC: u64 = 0x6573_5546;
/// `statfs` magics of file systems whose content other machines change:
/// NFS, SMB, CIFS, SMB2, 9P, Ceph, AFS (two), Coda, NCP, Lustre, OrangeFS,
/// GFS2, OCFS2, VirtualBox shared folders.
const REMOTE_MAGICS: [u64; 15] = [
    0x6969,
    0x517B,
    0xFF53_4D42,
    0xFE53_4D42,
    0x0102_1997,
    0x00C3_6400,
    0x5346_414F,
    0x6B41_4653,
    0x7375_7245,
    0x564C,
    0x0BD0_0BD0,
    0x1366_1366,
    0x0116_1970,
    0x7461_636F,
    0x786F_4256,
];
/// Block-device backed FUSE has no independent backing directory. Local
/// overlays can also change below the mount, so they still need polling.
#[cfg(not(target_os = "android"))]
const LOCAL_FUSE: [&str; 1] = ["fuseblk"];

pub(super) fn coverage(root: &Path) -> Coverage {
    let Some(magic) = fs_magic(root) else {
        return Coverage::LocalOnly;
    };
    if REMOTE_MAGICS.contains(&magic) {
        return Coverage::LocalOnly;
    }
    if magic == FUSE_SUPER_MAGIC {
        return fuse_coverage(root);
    }
    Coverage::Complete
}

#[cfg(target_os = "android")]
fn fuse_coverage(_root: &Path) -> Coverage {
    // The FUSE daemon on Android is the local media provider (shared storage).
    Coverage::SharedStorage
}

#[cfg(not(target_os = "android"))]
fn fuse_coverage(root: &Path) -> Coverage {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    match mount_type(&mountinfo, root) {
        Some(kind) if LOCAL_FUSE.contains(&kind.as_str()) => Coverage::Complete,
        _ => Coverage::LocalOnly,
    }
}

/// The 32-bit `statfs` magic of the file system holding `path`.
fn fs_magic(path: &Path) -> Option<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: an all-zero `statfs` is a valid output buffer.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: NUL-terminated path and a writable buffer of the right type.
    if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    // The field is signed on some targets; the magics are 32-bit values.
    u64::try_from(i128::from(stat.f_type) & 0xFFFF_FFFF).ok()
}

/// The file system type (`mountinfo` field after ` - `) of the mount that
/// holds `path` (longest mount point that contains it).
#[cfg(not(target_os = "android"))]
fn mount_type(mountinfo: &str, path: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let Some((fields, rest)) = line.split_once(" - ") else {
            continue;
        };
        let Some(mount_point) = fields.split(' ').nth(4).map(unescape) else {
            continue;
        };
        let Some(kind) = rest.split(' ').next() else {
            continue;
        };
        let mount_point = PathBuf::from(mount_point);
        if !path.starts_with(&mount_point) {
            continue;
        }
        let depth = mount_point.components().count();
        // Later lines win at equal depth (mounts stacked on the same point).
        if best.as_ref().is_none_or(|(known, _)| depth >= *known) {
            best = Some((depth, kind.to_string()));
        }
    }
    best.map(|(_, kind)| kind)
}

/// Undoes the octal escapes of `mountinfo` (`\040` space, `\011` tab,
/// `\012` newline, `\134` backslash).
#[cfg(not(target_os = "android"))]
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            if let Some(value) = octal(bytes.get(index + 1..index + 4)) {
                out.push(value);
                index += 4;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Three octal digits as one byte.
#[cfg(not(target_os = "android"))]
fn octal(digits: Option<&[u8]>) -> Option<u8> {
    let mut value = 0u32;
    for digit in digits? {
        if !(b'0'..=b'7').contains(digit) {
            return None;
        }
        value = value * 8 + u32::from(digit - b'0');
    }
    u8::try_from(value).ok()
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
40 22 0:44 / /home/u/nas rw,nosuid shared:20 - fuse.sshfs nas:/data rw
41 22 8:17 / /media/u/My\\040Disk rw shared:21 - fuseblk /dev/sdb1 rw
";

    #[test]
    fn review_task_watch_finds_the_mount_of_a_root() {
        assert_eq!(
            mount_type(MOUNTINFO, Path::new("/home/u/nas/photos")).as_deref(),
            Some("fuse.sshfs")
        );
        assert_eq!(
            mount_type(MOUNTINFO, Path::new("/media/u/My Disk/backup")).as_deref(),
            Some("fuseblk")
        );
        assert_eq!(
            mount_type(MOUNTINFO, Path::new("/home/u/docs")).as_deref(),
            Some("ext4")
        );
    }

    #[test]
    fn review_task_watch_local_roots_are_completely_covered() {
        let directory = tempfile::tempdir().unwrap();
        // Test runners keep temporary files on local disks or tmpfs.
        assert_ne!(coverage(directory.path()), Coverage::LocalOnly);
    }
}
