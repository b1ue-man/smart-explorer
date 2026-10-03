//! What a host reports with every Capabilities reply beyond the staged-write
//! contract (RV1): the features it serves and the limits of the storage that
//! holds the requested path.
use serde::{Deserialize, Serialize};

use super::is_false;
use crate::vfs::{MtimePrecision, NameLimit, TargetLimits};

/// Host features of RV1 beyond transfer v1. Every flag is the capability of
/// its name; older hosts send none, and a client sends the matching request
/// only to a host that offers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsHostFeatures {
    /// `DuplicateSearch`: the host's own duplicate finder.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) duplicate_search_v1: bool,
    /// `HashWalk`: content hashes computed on the host.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) hash_walk_v1: bool,
    /// `ListDirBatch`: folder listings in portions.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) list_batches_v1: bool,
    /// `Recycle`: the host's trash, after a check of the file.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) remote_trash_v1: bool,
    /// `FinishStage`/`SyncFilesystem`: source time, permissions and
    /// durability of finished stages.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) stage_finish_v1: bool,
    /// `StorageAnalysis { compress }`: the tree as one deflate stream.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) analysis_deflate_v1: bool,
    /// `request_id` of `StorageAnalysis`/`DuplicateSearch`: kept for 10
    /// minutes after a lost connection, attachable again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) analysis_reattach_v1: bool,
    /// `WatchExport`: change generations of an export.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) watch_v1: bool,
    /// The host enforces `FsWriteCapabilities::access` and the peer's write
    /// right.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) export_access_v1: bool,
    /// `SyncChildPath`: build a child from a literal listing name.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) literal_children_v1: bool,
}

impl FsHostFeatures {
    /// Features actually connected to this build's authenticated dispatcher.
    pub(crate) fn host() -> Self {
        Self {
            duplicate_search_v1: true,
            hash_walk_v1: true,
            list_batches_v1: true,
            remote_trash_v1: crate::analytics::host_recycle_available(),
            stage_finish_v1: true,
            analysis_deflate_v1: true,
            analysis_reattach_v1: true,
            watch_v1: true,
            export_access_v1: true,
            literal_children_v1: true,
        }
    }

    pub(crate) fn is_absent(&self) -> bool {
        *self == Self::default()
    }

    /// The offered features by capability name, for logs.
    pub(crate) fn names(&self) -> Vec<&'static str> {
        [
            (self.duplicate_search_v1, "duplicate_search_v1"),
            (self.hash_walk_v1, "hash_walk_v1"),
            (self.list_batches_v1, "list_batches_v1"),
            (self.remote_trash_v1, "remote_trash_v1"),
            (self.stage_finish_v1, "stage_finish_v1"),
            (self.analysis_deflate_v1, "analysis_deflate_v1"),
            (self.analysis_reattach_v1, "analysis_reattach_v1"),
            (self.watch_v1, "watch_v1"),
            (self.export_access_v1, "export_access_v1"),
            (self.literal_children_v1, "literal_children_v1"),
        ]
        .into_iter()
        .filter_map(|(offered, name)| offered.then_some(name))
        .collect()
    }
}

/// Resolution of stored modification times (`vfs::MtimePrecision`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FsMtimePrecision {
    Nanos,
    Millis,
    TenMillis,
    Seconds,
    TwoSeconds,
    Minutes,
    Days,
    /// Not determined, or a value of a later version.
    #[default]
    #[serde(other)]
    Unknown,
}

impl From<MtimePrecision> for FsMtimePrecision {
    fn from(value: MtimePrecision) -> Self {
        match value {
            MtimePrecision::Nanos => Self::Nanos,
            MtimePrecision::Millis => Self::Millis,
            MtimePrecision::TenMillis => Self::TenMillis,
            MtimePrecision::Seconds => Self::Seconds,
            MtimePrecision::TwoSeconds => Self::TwoSeconds,
            MtimePrecision::Minutes => Self::Minutes,
            MtimePrecision::Days => Self::Days,
            MtimePrecision::Unknown => Self::Unknown,
        }
    }
}

impl From<FsMtimePrecision> for MtimePrecision {
    fn from(value: FsMtimePrecision) -> Self {
        match value {
            FsMtimePrecision::Nanos => Self::Nanos,
            FsMtimePrecision::Millis => Self::Millis,
            FsMtimePrecision::TenMillis => Self::TenMillis,
            FsMtimePrecision::Seconds => Self::Seconds,
            FsMtimePrecision::TwoSeconds => Self::TwoSeconds,
            FsMtimePrecision::Minutes => Self::Minutes,
            FsMtimePrecision::Days => Self::Days,
            FsMtimePrecision::Unknown => Self::Unknown,
        }
    }
}

/// What the storage below a path can hold (`vfs::TargetLimits`), so a client
/// plans names, sizes and time comparisons for a Share target like for a
/// local one. Absent from older hosts: everything unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsTargetLimits {
    /// Windows naming rules apply (Windows hosts, SMB shares).
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) windows_names: bool,
    /// Longest name component in UTF-8 bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_name_bytes: Option<u32>,
    /// Longest name component in UTF-16 code units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_name_utf16: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_file_size: Option<u64>,
    #[serde(default)]
    pub(crate) mtime_precision: FsMtimePrecision,
}

impl FsTargetLimits {
    pub(crate) fn is_unknown(&self) -> bool {
        *self == Self::default()
    }
}

impl From<TargetLimits> for FsTargetLimits {
    fn from(limits: TargetLimits) -> Self {
        let clamp = |limit: usize| u32::try_from(limit).unwrap_or(u32::MAX);
        Self {
            windows_names: limits.windows_names,
            max_name_bytes: match limits.max_name {
                Some(NameLimit::Bytes(limit)) => Some(clamp(limit)),
                _ => None,
            },
            max_name_utf16: match limits.max_name {
                Some(NameLimit::Utf16Units(limit)) => Some(clamp(limit)),
                _ => None,
            },
            max_file_size: limits.max_file_size,
            mtime_precision: limits.mtime_precision.into(),
        }
    }
}

impl From<FsTargetLimits> for TargetLimits {
    fn from(limits: FsTargetLimits) -> Self {
        // A host names one of the two; bytes win if a later one sends both.
        let max_name = match (limits.max_name_bytes, limits.max_name_utf16) {
            (Some(bytes), _) => Some(NameLimit::Bytes(bytes as usize)),
            (None, Some(units)) => Some(NameLimit::Utf16Units(units as usize)),
            (None, None) => None,
        };
        Self {
            windows_names: limits.windows_names,
            max_name,
            max_file_size: limits.max_file_size,
            mtime_precision: limits.mtime_precision.into(),
        }
    }
}
