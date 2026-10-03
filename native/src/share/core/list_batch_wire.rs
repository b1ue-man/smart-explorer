//! Wire form of entries a listing or walk had to leave out (`list_batches_v1`,
//! `hash_walk_v1`) and the size of one listing portion.
use serde::{Deserialize, Serialize};

use crate::vfs::{OmissionReason, VfsOmission};

/// One listing portion stays far below the receiver's frame limit: a name of
/// at most 255 bytes costs at most about 1 KiB of JSON (escaped), so 2048
/// entries stay below 2 MiB even then, and ordinary names below 512 KiB.
pub(crate) const LIST_BATCH_MAX_ENTRIES: usize = 2048;
/// Estimated JSON bytes of a portion before it is sent.
pub(crate) const LIST_BATCH_MAX_BYTES: usize = 512 * 1024;
/// JSON of one `FsMeta` besides its name and id (field names and numbers).
pub(crate) const META_JSON_BYTES: usize = 192;
/// Frame limit a client reads portions and walk messages with.
pub(crate) const STREAM_FRAME_LIMIT: usize = 4 * 1024 * 1024;

/// Why an existing entry is missing (`vfs::OmissionReason`). Values of later
/// versions read as `Unreadable`: still an omission, never an absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FsOmissionReason {
    Link,
    Special,
    Vanished,
    Unrepresentable,
    #[serde(other)]
    Unreadable,
}

/// One entry left out of a listing (`rel` = name) or a walk (`rel` = path
/// below the walk root); everything below it counts as omitted too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsOmission {
    pub(crate) rel: String,
    pub(crate) reason: FsOmissionReason,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) detail: String,
}

impl FsOmission {
    /// JSON bytes this omission adds to a portion (estimate).
    pub(crate) fn wire_bytes(&self) -> usize {
        self.rel.len() + self.detail.len() + 64
    }
}

impl From<OmissionReason> for FsOmissionReason {
    fn from(reason: OmissionReason) -> Self {
        match reason {
            OmissionReason::Link => Self::Link,
            OmissionReason::Special => Self::Special,
            OmissionReason::Unreadable => Self::Unreadable,
            OmissionReason::Vanished => Self::Vanished,
            OmissionReason::Unrepresentable => Self::Unrepresentable,
        }
    }
}

impl From<FsOmissionReason> for OmissionReason {
    fn from(reason: FsOmissionReason) -> Self {
        match reason {
            FsOmissionReason::Link => Self::Link,
            FsOmissionReason::Special => Self::Special,
            FsOmissionReason::Unreadable => Self::Unreadable,
            FsOmissionReason::Vanished => Self::Vanished,
            FsOmissionReason::Unrepresentable => Self::Unrepresentable,
        }
    }
}

impl From<VfsOmission> for FsOmission {
    fn from(omission: VfsOmission) -> Self {
        Self {
            rel: omission.rel,
            reason: omission.reason.into(),
            detail: omission.detail,
        }
    }
}

impl From<FsOmission> for VfsOmission {
    fn from(omission: FsOmission) -> Self {
        Self {
            rel: omission.rel,
            reason: omission.reason.into(),
            detail: omission.detail,
        }
    }
}
