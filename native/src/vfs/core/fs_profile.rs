//! What the code knows about a local filesystem from its type name (Linux
//! mountinfo type, Windows filesystem name): storable names, file size, time
//! resolution, how writes become durable, and what kind of mount it is.
use super::{MountKind, MtimePrecision, NameLimit, TargetLimits};

/// Largest file FAT12/16/32 can store (4 GiB − 1).
const FAT_MAX_FILE: u64 = 0xFFFF_FFFF;
/// Longest name component: 255 bytes (Linux) or 255 UTF-16 units (Windows filesystems).
const NAME_MAX: usize = 255;

/// How finished stages on a filesystem reach stable storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlushModel {
    /// `syncfs` flushes data and metadata of everything written before
    /// (local block filesystems): new files can wait for one call.
    Batched,
    /// Each file is flushed on its own; the server commits renames before it
    /// answers (NFS), so `syncfs` completes a run's durability.
    PerFile,
    /// Each file is flushed on its own; nothing vouches for renames (FUSE,
    /// SMB/CIFS, unknown types).
    PerFileOnly,
}

/// Properties of one filesystem type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FsProfile {
    pub(crate) limits: TargetLimits,
    pub(crate) flush: FlushModel,
}

/// The type part of a Linux mountinfo type (`fuse.sshfs` → `fuse`).
fn base_type(fs_type: &str) -> &str {
    fs_type.split('.').next().unwrap_or(fs_type)
}

/// Profile of a Linux/Android filesystem by its mountinfo type.
pub(crate) fn linux_profile(fs_type: &str) -> FsProfile {
    let base = base_type(fs_type);
    let windows_names = matches!(
        base,
        "vfat" | "msdos" | "exfat" | "sdfat" | "ntfs" | "ntfs3" | "fuseblk" | "cifs" | "smb3"
    );
    let mtime_precision = match base {
        "ext4" | "xfs" | "btrfs" | "f2fs" | "tmpfs" | "ntfs" | "ntfs3" | "overlay" | "zfs"
        | "bcachefs" | "jfs" | "nilfs2" => MtimePrecision::Nanos,
        "reiserfs" | "hfsplus" => MtimePrecision::Seconds,
        "vfat" | "msdos" => MtimePrecision::TwoSeconds,
        "exfat" => MtimePrecision::TenMillis,
        _ => MtimePrecision::Unknown,
    };
    let flush = match base {
        "ext2" | "ext3" | "ext4" | "xfs" | "btrfs" | "f2fs" | "jfs" | "reiserfs" | "bcachefs"
        | "zfs" | "nilfs2" | "hfsplus" | "udf" | "vfat" | "msdos" | "exfat" | "ntfs3" | "ntfs"
        | "tmpfs" | "ramfs" | "overlay" => FlushModel::Batched,
        "nfs" | "nfs4" => FlushModel::PerFile,
        _ => FlushModel::PerFileOnly,
    };
    FsProfile {
        limits: TargetLimits {
            windows_names,
            max_name: Some(if windows_names {
                NameLimit::Utf16Units(NAME_MAX)
            } else {
                NameLimit::Bytes(NAME_MAX)
            }),
            max_file_size: matches!(base, "vfat" | "msdos").then_some(FAT_MAX_FILE),
            mtime_precision,
        },
        flush,
    }
}

/// Profile of a Windows volume by its filesystem name (`GetVolumeInformation`)
/// and the reported longest name component.
pub(crate) fn windows_profile(fs_name: &str, max_component: u32) -> FsProfile {
    let name = fs_name.to_ascii_uppercase();
    let mtime_precision = match name.as_str() {
        "NTFS" | "REFS" => MtimePrecision::Nanos,
        "FAT" | "FAT12" | "FAT16" | "FAT32" => MtimePrecision::TwoSeconds,
        "EXFAT" => MtimePrecision::TenMillis,
        "CDFS" => MtimePrecision::Seconds,
        _ => MtimePrecision::Unknown,
    };
    let units = match usize::try_from(max_component) {
        Ok(units) if units > 0 => units.min(NAME_MAX),
        _ => NAME_MAX,
    };
    FsProfile {
        limits: TargetLimits {
            windows_names: true,
            max_name: Some(NameLimit::Utf16Units(units)),
            max_file_size: name.starts_with("FAT").then_some(FAT_MAX_FILE),
            mtime_precision,
        },
        // Stages are flushed one by one (FlushFileBuffers) and published by
        // write-through renames; there is no unprivileged volume flush.
        flush: FlushModel::PerFile,
    }
}

/// Kind of a Linux/Android mount by its mountinfo type.
pub(crate) fn mount_kind(fs_type: &str) -> MountKind {
    match base_type(fs_type) {
        "proc" | "sysfs" | "devpts" | "devtmpfs" | "cgroup" | "cgroup2" | "securityfs"
        | "debugfs" | "tracefs" | "pstore" | "bpf" | "mqueue" | "hugetlbfs" | "configfs"
        | "fusectl" | "binfmt_misc" | "efivarfs" | "selinuxfs" | "nsfs" | "rpc_pipefs" => {
            MountKind::Pseudo
        }
        "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "9p" | "ceph" | "afs" | "lustre" | "ncpfs"
        | "coda" | "davfs" => MountKind::Network,
        "autofs" => MountKind::Automount,
        base if base.starts_with("fuse") => MountKind::Fuse,
        _ => MountKind::Local,
    }
}
