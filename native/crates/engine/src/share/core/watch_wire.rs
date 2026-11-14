//! Notices of a watched export (`watch_v1`, `FsResponse::Watch`). They only
//! trigger runs: what changed is decided by the rescan.
use serde::{Deserialize, Serialize};

use crate::vfs::ChangeNotice;

/// Paths a `Changed` notice names at most; beyond them only the generation
/// changes (the subscriber rescans the whole subtree anyway).
pub const MAX_NOTICE_PATHS: usize = 64;
/// The host says it still serves the subscription this often.
pub const ALIVE_SECS: u64 = 60;
/// A subscriber gives up after this long without any notice.
pub const SILENT_SECS: u64 = 3 * ALIVE_SECS;
/// Start of the `Ended` text of a subscription whose host cannot report
/// changes of this path at all: the subscriber polls instead of subscribing
/// again (the same words as a backend without change signal).
pub const POLL_ONLY: &str = "Keine Änderungsereignisse; Abfrage aktiv";
/// What hosts up to v0.5.196 sent for that case without `unsupported`: the
/// watch service's reason and the Windows pin refusal.
const LEGACY_UNSUPPORTED: [&str; 2] = ["Unsupported", "directory pin has no safe watch capability"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "w", rename_all = "snake_case")]
pub enum FsWatchEvent {
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
    /// `unsupported`: it never can (no change notifications there); the
    /// subscriber polls instead of subscribing again. Older peers ignore the
    /// field and keep the unchanged `reason`.
    Unavailable {
        reason: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        unsupported: bool,
    },
    /// Keepalive of an idle subscription.
    Alive,
}

impl FsWatchEvent {
    /// The client side; `None` for keepalives.
    pub fn into_notice(self) -> Option<ChangeNotice> {
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
            Self::Unavailable {
                reason,
                unsupported,
            } if unsupported || LEGACY_UNSUPPORTED.contains(&reason.as_str()) => {
                Some(ChangeNotice::Ended(format!("{POLL_ONLY}: {reason}")))
            }
            Self::Unavailable { reason, .. } => Some(ChangeNotice::Ended(reason)),
            Self::Alive => None,
        }
    }
}

#[cfg(test)]
mod windows_watch_task_tests {
    use super::*;

    #[test]
    fn windows_watch_task_unsupported_reaches_the_subscriber_distinguishably() {
        // The daemon's remote watch recognizes these words (remote_watch.rs).
        assert_eq!(POLL_ONLY, "Keine Änderungsereignisse; Abfrage aktiv");
        let event = FsWatchEvent::Unavailable {
            reason: "Unsupported".into(),
            unsupported: true,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["w"], "unavailable");
        assert_eq!(json["reason"], "Unsupported");
        assert_eq!(json["unsupported"], true);
        assert_eq!(serde_json::from_value::<FsWatchEvent>(json).unwrap(), event);
        // An ordinary end keeps the old wire form and the old text.
        let ordinary = FsWatchEvent::Unavailable {
            reason: "RootMissing".into(),
            unsupported: false,
        };
        let json = serde_json::to_value(&ordinary).unwrap();
        assert!(json.get("unsupported").is_none());
        assert_eq!(
            ordinary.into_notice(),
            Some(ChangeNotice::Ended("RootMissing".into()))
        );
        // A notice of an older host without the field still deserializes.
        let older: FsWatchEvent =
            serde_json::from_str(r#"{"w":"unavailable","reason":"x"}"#).unwrap();
        assert_eq!(older.into_notice(), Some(ChangeNotice::Ended("x".into())));
        for reason in ["Unsupported", "directory pin has no safe watch capability"] {
            let legacy = FsWatchEvent::Unavailable {
                reason: reason.into(),
                unsupported: false,
            };
            assert_eq!(
                legacy.into_notice(),
                Some(ChangeNotice::Ended(format!("{POLL_ONLY}: {reason}")))
            );
        }
        assert_eq!(
            event.into_notice(),
            Some(ChangeNotice::Ended(format!("{POLL_ONLY}: Unsupported")))
        );
    }
}
