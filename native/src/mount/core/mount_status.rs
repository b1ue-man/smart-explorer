//! Drive state and the outcomes of save and namespace operations. Moved out
//! of `types.rs` without any change of the serde form (DRV-M1); `types.rs`
//! re-exports every type, so every caller keeps its path. DRV-M3 adds
//! `MountedAt` for drives that appear at a folder.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::types::{DriveLetter, MountConflict};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountStatus {
    Unmounted,
    Mounting,
    Mounted {
        drive: DriveLetter,
    },
    /// Mounted at a folder: Linux FUSE drives (DRV-M4) and vault drives
    /// (contract V2, read through `list_mounts` for S3 K09/K11).
    MountedAt {
        path: PathBuf,
    },
    Unmounting,
    RuntimeUnavailable {
        detail: String,
    },
    Conflict {
        drive: DriveLetter,
        path: String,
        detail: String,
    },
    Failed {
        detail: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlushOutcome {
    NoChanges,
    Committed,
    /// The backend acknowledged the atomic promotion, but the new metadata or
    /// local journal could not be reconciled. Callers must return filesystem
    /// success (the namespace already changed) and surface this recovery state.
    CommittedPendingVerification(MountConflict),
    Conflict(MountConflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameOutcome {
    Complete,
    /// The namespace change is already committed. Filesystem callers must see
    /// success while Smart Explorer surfaces the retained recovery state.
    CommittedPendingVerification(MountConflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceOutcome {
    Complete,
    /// A namespace mutation was dispatched or acknowledged, but its final
    /// shape could not be verified. Windows must not replay it; the host stops
    /// and remounts after surfacing the detail.
    CommittedPendingVerification {
        path: String,
        detail: String,
    },
}
