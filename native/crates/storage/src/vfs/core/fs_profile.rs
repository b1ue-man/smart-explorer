//! What the code knows about a local filesystem from its type name (Linux
//! mountinfo type, Windows filesystem name): storable names, file size, time
//! resolution, how writes become durable, whether names are compared without
//! regard to case, and what kind of mount it is.
use super::{MountKind, MtimePrecision, NameIssue, NameLimit, TargetLimits};

/// Largest file FAT12/16/32 can store (4 GiB − 1).
const FAT_MAX_FILE: u64 = 0xFFFF_FFFF;
/// Longest name component: 255 bytes (Linux) or 255 UTF-16 units (Windows filesystems).
const NAME_MAX: usize = 255;

/// How finished stages on a filesystem reach stable storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushModel {
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

/// How a filesystem compares names: two names with the same key are one
/// entry there (`a.txt` and `A.txt` on FAT).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaseFolding {
    /// Names are compared byte for byte (ext4, btrfs, NTFS through
    /// ntfs3/ntfs-3g) or the rule is not known.
    #[default]
    None,
    /// Only ASCII letters are folded (Linux vfat with the utf8 option: its
    /// `nls_utf8` table folds ASCII only).
    Ascii,
    /// Letters are folded by the volume's up-case table (Windows volumes,
    /// Linux exFAT): approximated by the simple one-letter Unicode mapping.
    Unicode,
}

impl CaseFolding {
    /// Whether two different spellings can be one name here.
    pub fn folds(self) -> bool {
        self != Self::None
    }

    /// The key under which this filesystem finds `name`; `None` when names
    /// are not folded.
    pub fn key(self, name: &str) -> Option<String> {
        match self {
            Self::None => None,
            Self::Ascii => Some(name.to_ascii_uppercase()),
            Self::Unicode => Some(name.chars().map(simple_upper).collect()),
        }
    }
}

/// The one-letter upper case of `letter`, or the letter itself where Unicode
/// maps it to several letters (`ß`): an up-case table maps letter to letter.
fn simple_upper(letter: char) -> char {
    let mut upper = letter.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => letter,
    }
}

/// How names behave on a filesystem and what it is called in messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NameRules {
    /// Name of the filesystem in messages (`FAT`, `exFAT`, `NTFS`, …);
    /// `None` = not known by name.
    pub label: Option<&'static str>,
    pub folding: CaseFolding,
}

impl NameRules {
    fn named(&self) -> &'static str {
        self.label.unwrap_or("dem Ziel-Dateisystem")
    }

    /// Why a file larger than `limit` bytes is not copied (German, for the
    /// report of the one file).
    pub fn size_reason(&self, limit: u64) -> String {
        let limit = if limit == FAT_MAX_FILE {
            "4 GiB − 1 Byte".to_string()
        } else {
            format!("{limit} Byte")
        };
        let filesystem = self.label.unwrap_or("das Ziel-Dateisystem");
        format!("größer als {limit}: {filesystem} kann sie nicht speichern")
    }

    /// Why a name is not copied (German).
    pub fn name_reason(&self, issue: NameIssue, limit: Option<NameLimit>) -> String {
        let detail = match (issue, limit) {
            (NameIssue::Windows(issue), _) => issue.label_de().to_string(),
            (NameIssue::TooLong, Some(NameLimit::Bytes(bytes))) => {
                format!("länger als {bytes} Byte")
            }
            (NameIssue::TooLong, Some(NameLimit::Utf16Units(units))) => {
                format!("länger als {units} Zeichen")
            }
            (NameIssue::TooLong, None) => "zu lang".to_string(),
        };
        format!("Name nicht erlaubt auf {}: {detail}", self.named())
    }

    /// Why `name` is not copied next to `other`, which this filesystem
    /// stores under the same name (German, names both).
    pub fn fold_reason(&self, name: &str, other: &str) -> String {
        format!(
            "„{name}“ und „{other}“ sind auf {} derselbe Name (Groß- und Kleinschreibung werden nicht unterschieden); „{other}“ wird kopiert",
            self.named()
        )
    }
}

/// Properties of one filesystem type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsProfile {
    pub limits: TargetLimits,
    pub flush: FlushModel,
    pub names: NameRules,
}

/// The type part of a Linux mountinfo type (`fuse.sshfs` → `fuse`).
fn base_type(fs_type: &str) -> &str {
    fs_type.split('.').next().unwrap_or(fs_type)
}

/// Profile of a Linux/Android filesystem by its mountinfo type.
pub fn linux_profile(fs_type: &str) -> FsProfile {
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
    let names = NameRules {
        label: match base {
            "vfat" | "msdos" => Some("FAT"),
            "exfat" | "sdfat" => Some("exFAT"),
            "ntfs" | "ntfs3" => Some("NTFS"),
            "cifs" | "smb3" => Some("SMB"),
            _ => None,
        },
        // Folding as the Linux drivers do it; ntfs3, ntfs-3g and SMB
        // mounts with Unix extensions keep both spellings, so they are
        // not counted (a missed collision fails at publication, a wrong
        // one would refuse a storable file).
        folding: match base {
            "vfat" | "msdos" => CaseFolding::Ascii,
            "exfat" | "sdfat" => CaseFolding::Unicode,
            _ => CaseFolding::None,
        },
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
        names,
    }
}

/// Profile of a Windows volume by its filesystem name (`GetVolumeInformation`)
/// and the reported longest name component.
pub fn windows_profile(fs_name: &str, max_component: u32) -> FsProfile {
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
        names: NameRules {
            label: match name.as_str() {
                "FAT" => Some("FAT"),
                "FAT12" => Some("FAT12"),
                "FAT16" => Some("FAT16"),
                "FAT32" => Some("FAT32"),
                "EXFAT" => Some("exFAT"),
                "NTFS" => Some("NTFS"),
                "REFS" => Some("ReFS"),
                "UDF" => Some("UDF"),
                "CDFS" => Some("CDFS"),
                _ => None,
            },
            // Windows compares names through the volume's up-case table
            // (directories marked case-sensitive for WSL are the exception
            // the profile of a volume cannot see).
            folding: CaseFolding::Unicode,
        },
    }
}

/// Kind of a Linux/Android mount by its mountinfo type.
pub fn mount_kind(fs_type: &str) -> MountKind {
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

#[cfg(test)]
#[path = "fs_profile_tests.rs"]
mod tests;
