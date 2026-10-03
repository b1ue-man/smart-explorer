//! Notices of a watched export (`watch_v1`, `FsResponse::Watch`). They only
//! trigger runs: what changed is decided by the rescan.
use serde::{Deserialize, Serialize};

use crate::vfs::ChangeNotice;

/// Paths a `Changed` notice names at most; beyond them only the generation
/// changes (the subscriber rescans the whole subtree anyway).
pub(crate) const MAX_NOTICE_PATHS: usize = 64;
/// The host says it still serves the subscription this often.
pub(crate) const ALIVE_SECS: u64 = 60;
/// A subscriber gives up after this long without any notice.
pub(crate) const SILENT_SECS: u64 = 3 * ALIVE_SECS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "w", rename_all = "snake_case")]
pub(crate) enum FsWatchEvent {
    /// Watched from now on; earlier changes are not all known (check once).
    Ready {
        generation: u64,
        /// Partial OS coverage still requires periodic verification.
        #[serde(default)]
        complete: bool,
    },
    /// Something below the watched path changed; `paths` are hints relative
    /// to it (empty when there were more than `MAX_NOTICE_PATHS`).
    Changed {
        generation: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
    },
    /// Notices were lost: check the whole subtree.
    Overflow { generation: u64 },
    /// The host cannot watch this path (now); the stream ends after it.
    Unavailable { reason: String },
    /// Keepalive of an idle subscription.
    Alive,
}

impl FsWatchEvent {
    /// The client side; `None` for keepalives.
    pub(crate) fn into_notice(self) -> Option<ChangeNotice> {
        match self {
            Self::Ready {
                generation,
                complete: true,
            } => Some(ChangeNotice::Ready {
                generation: Some(generation),
            }),
            Self::Ready {
                generation,
                complete: false,
            } => Some(ChangeNotice::ReadyPartial {
                generation: Some(generation),
            }),
            Self::Changed { generation, paths } => Some(ChangeNotice::Changed {
                generation: Some(generation),
                paths,
            }),
            Self::Overflow { .. } => Some(ChangeNotice::Overflow),
            Self::Unavailable { reason } => Some(ChangeNotice::Ended(reason)),
            Self::Alive => None,
        }
    }
}
