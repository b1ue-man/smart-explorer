//! What the filesystem behind an open descriptor does with owner modes and
//! with growing files, told by its `fstatfs` type (the values of Linux's
//! `include/uapi/linux/magic.h`, the same ones `vfs::confined_local` and
//! `ConfinedDirectory::stable_identity` know).
use std::fs::File;
use std::io;
use std::os::unix::io::AsRawFd;

/// FAT12/16/32 (`vfat`, `msdos`): `MSDOS_SUPER_MAGIC`.
pub(super) const MSDOS: u64 = 0x4d44;
/// The kernel's exFAT driver: `EXFAT_SUPER_MAGIC`.
pub(super) const EXFAT: u64 = 0x2011_bab0;
/// The kernel's ntfs3 driver (`NTFS3_SUPER_MAGIC`).
pub(super) const NTFS3: u64 = 0x7366_746e;
/// The old read-mostly kernel NTFS driver (`NTFS_SB_MAGIC`).
pub(super) const NTFS: u64 = 0x5346_544e;
/// Every FUSE driver (ntfs-3g, exfat-fuse, sshfs …): `FUSE_SUPER_MAGIC`.
pub(super) const FUSE: u64 = 0x6573_5546;

/// The `f_type` of the filesystem holding `file`.
pub(super) fn magic(file: &File) -> io::Result<u64> {
    let mut facts = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: a live descriptor and a correctly sized output buffer.
    if unsafe { libc::fstatfs(file.as_raw_fd(), facts.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fstatfs filled the buffer on success. The field's width and
    // sign differ between targets; the magic is its low 32 bits.
    Ok(unsafe { facts.assume_init() }.f_type as u64 & 0xffff_ffff)
}

/// Filesystems whose owner and permission bits may come from the mount
/// (`uid=`, `gid=`, `fmask=`, `dmask=`) instead of the entry: FAT and exFAT
/// store none, NTFS keeps them only with extra mount options, and a FUSE
/// driver decides itself. A fresh entry there can show `0755`/`0644` however
/// it was created, so an owner-private capture folder may be refused. ext4,
/// XFS, Btrfs and every other type are not in this list: there the refusal
/// stays a refusal.
pub(super) fn owner_modes_unstorable(magic: u64) -> bool {
    matches!(magic, MSDOS | EXFAT | NTFS3 | NTFS | FUSE)
}

/// Filesystems that write zeros up to a new length or a write offset the
/// moment a file grows: FAT on `truncate` (`fat_cont_expand`) and on a write
/// beyond the end (`cont_write_begin`); exFAT the same before its valid data
/// length (Linux 6.8), and since then still for the gap before a write past
/// that length (`exfat_extend_valid_size`). A pre-sized stage or ranges
/// written out of order therefore write the file twice there.
pub(super) fn extends_by_zero_fill(magic: u64) -> bool {
    matches!(magic, MSDOS | EXFAT)
}

/// Whether `writer` lies on such a filesystem. An unanswered `fstatfs`
/// keeps the copy as it was (`false`).
pub(super) fn zero_fill_target(writer: &File) -> bool {
    magic(writer).is_ok_and(extends_by_zero_fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which filesystem types may skip the private capture folder (U1) and
    /// which get no pre-sized stage and no ranges (U6); ext4 (0xef53), XFS
    /// (0x58465342), Btrfs (0x9123683e), tmpfs (0x01021994), f2fs (0xf2f52010)
    /// and overlayfs (0x794c7630) keep the old behaviour.
    #[test]
    fn removable_task_filesystem_types_select_their_capabilities() {
        for kind in [MSDOS, EXFAT, NTFS3, NTFS, FUSE] {
            assert!(owner_modes_unstorable(kind), "{kind:#x}");
        }
        for kind in [MSDOS, EXFAT] {
            assert!(extends_by_zero_fill(kind), "{kind:#x}");
        }
        for kind in [NTFS3, NTFS, FUSE] {
            assert!(!extends_by_zero_fill(kind), "{kind:#x}");
        }
        for kind in [
            0xef53,
            0x5846_5342,
            0x9123_683e,
            0x0102_1994,
            0xf2f5_2010,
            0x794c_7630,
        ] {
            assert!(!owner_modes_unstorable(kind), "{kind:#x}");
            assert!(!extends_by_zero_fill(kind), "{kind:#x}");
        }
        // The descriptor query itself: a stage in an ordinary temporary
        // folder is no zero-filling target.
        let file = tempfile::tempfile().unwrap();
        let kind = magic(&file).unwrap();
        assert!(!extends_by_zero_fill(kind), "{kind:#x}");
        assert!(!zero_fill_target(&file));
    }
}
