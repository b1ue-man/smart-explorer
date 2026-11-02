//! Whether a Linux file system keeps the file IDs the digest cache keys on
//! (`hash_cache.rs`): the inode number of a file must name that file for as
//! long as the cached digest lives, across remounts too.
//!
//! The Linux FAT driver (`msdos`, `vfat`) and exFAT make inode numbers up
//! when they read a directory, so after a remount another file can carry the
//! number a digest was stored under; with the 2 s time resolution of those
//! file systems size and times may agree as well. FUSE without `use_ino`,
//! SMB/CIFS without server inode numbers, 9p and shared folders of virtual
//! machines hand out numbers of the client; optical media name files by their
//! position on the disc. None of them is cached (a stamp is `None`); every
//! other type keeps the inode numbers of its on-disk files.
use std::ffi::CString;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `f_type` values of `statfs(2)` (`linux/magic.h`).
const MSDOS_SUPER_MAGIC: i128 = 0x4d44;
const EXFAT_SUPER_MAGIC: i128 = 0x2011_BAB0;
const FUSE_SUPER_MAGIC: i128 = 0x6573_5546;
const SMB_SUPER_MAGIC: i128 = 0x517B;
const CIFS_SUPER_MAGIC: i128 = 0xFF53_4D42;
const SMB2_SUPER_MAGIC: i128 = 0xFE53_4D42;
const V9FS_MAGIC: i128 = 0x0102_1997;
const ISOFS_SUPER_MAGIC: i128 = 0x9660;
const UDF_SUPER_MAGIC: i128 = 0x1501_3346;
const VBOXSF_SUPER_MAGIC: i128 = 0x786f_4256;

/// Whether the file system type (`statfs.f_type`; a signed or unsigned word,
/// depending on the C library, so only the low 32 bits count) keeps durable
/// file IDs.
pub(super) fn type_has_durable_ids(f_type: impl Into<i128>) -> bool {
    !matches!(
        f_type.into() & 0xFFFF_FFFF,
        MSDOS_SUPER_MAGIC
            | EXFAT_SUPER_MAGIC
            | FUSE_SUPER_MAGIC
            | SMB_SUPER_MAGIC
            | CIFS_SUPER_MAGIC
            | SMB2_SUPER_MAGIC
            | V9FS_MAGIC
            | ISOFS_SUPER_MAGIC
            | UDF_SUPER_MAGIC
            | VBOXSF_SUPER_MAGIC
    )
}

/// The file system of an open file. A type that cannot be read counts as
/// "no durable IDs": the digest is then only computed again.
pub(super) fn file_has_durable_ids(file: &std::fs::File) -> bool {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: the descriptor is open for the call; the buffer is a `statfs`.
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: the call succeeded and filled the buffer.
    type_has_durable_ids(unsafe { stat.assume_init() }.f_type)
}

/// The file system holding `path` (for a regular file; a link's own file
/// system is not asked).
pub(super) fn path_has_durable_ids(path: &Path) -> bool {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is a NUL-terminated string; the buffer is a `statfs`.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: the call succeeded and filled the buffer.
    type_has_durable_ids(unsafe { stat.assume_init() }.f_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abgleich_task_fat_exfat_fuse_and_network_types_have_no_durable_ids() {
        for magic in [
            0x4d44i64,
            0x2011_BAB0,
            0x6573_5546,
            0xFF53_4D42,
            0xFE53_4D42,
            0x0102_1997,
            0x9660,
            0x1501_3346,
        ] {
            assert!(!type_has_durable_ids(magic), "{magic:#x}");
        }
        // 32-bit C libraries report 0xFF534D42 as a negative word.
        assert!(!type_has_durable_ids(0xFF53_4D42u32 as i32));
        // ext4, xfs, btrfs, tmpfs, nfs, ntfs.
        for magic in [
            0xEF53i64,
            0x5846_5342,
            0x9123_683E,
            0x0102_1994,
            0x6969,
            0x5346_544e,
        ] {
            assert!(type_has_durable_ids(magic), "{magic:#x}");
        }
    }

    #[test]
    fn abgleich_task_a_missing_path_has_no_durable_ids() {
        assert!(!path_has_durable_ids(Path::new(
            "/nonexistent/abgleich-task/file"
        )));
    }
}
