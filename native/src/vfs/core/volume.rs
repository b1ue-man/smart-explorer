//! Identity of the local filesystem a location lives on, independent of where
//! it is mounted: the fallback identity of a sync replica whose root cannot
//! hold a marker file.

/// Which filesystem stores a local location and where inside it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct VolumeIdentity {
    /// Stable id of the filesystem: Linux/Android the filesystem UUID as in
    /// `/dev/disk/by-uuid` (FAT/exFAT: the volume serial), Windows the 64-bit
    /// volume serial number (`FILE_ID_INFO`), lowercase hex.
    pub volume_id: String,
    /// Forward-slash path of the location inside that filesystem, the same
    /// for every mount point, drive letter or bind mount ("" = its root).
    pub relative_path: String,
    /// Filesystem type ("ext4", "vfat", "NTFS", "exFAT", …), for reports
    /// only: one volume can show different driver names.
    pub fs_type: String,
}

/// Kind of filesystem mounted at a local directory inside a tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MountKind {
    /// Another local disk or partition.
    Local,
    /// NFS, SMB/CIFS and other network filesystems.
    Network,
    /// FUSE filesystems (sshfs, rclone, ntfs-3g, …).
    Fuse,
    /// autofs trigger points (mounting on access, possibly hanging).
    Automount,
    /// proc, sysfs, devpts, cgroup and similar pseudo filesystems.
    Pseudo,
}

impl VolumeIdentity {
    /// Comparable text of filesystem and location (without `fs_type`).
    pub fn key(&self) -> String {
        format!("{}:{}", self.volume_id, self.relative_path)
    }
}

impl PartialEq for VolumeIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.volume_id == other.volume_id && self.relative_path == other.relative_path
    }
}

impl Eq for VolumeIdentity {}

impl std::hash::Hash for VolumeIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.volume_id.hash(state);
        self.relative_path.hash(state);
    }
}
