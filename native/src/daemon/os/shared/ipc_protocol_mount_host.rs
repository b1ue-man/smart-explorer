//! `IpcRequest::MountHostExt { id, session_token, req }`: requests of a
//! drive host to its daemon beyond the attach, backend and status requests
//! of v0.5.182 (plan 2026-10-17 W3; R5 sets the variant pair, the session
//! token of the mount authorizes it). DRV-M7 adds `PrepareLock`; DRV-M6
//! adds `ChildBackend` and `DeviceWatch` here.
//!
//! The lock channel of a vault drive (S2-V4, S2-V10): the host asks with
//! `PrepareLock { deadline_ms }`; the daemon holds the request until a lock
//! of the drive's vault begins, at the latest `deadline_ms` (at most
//! `MAX_WATCH_MS`), and answers `MountHostReply::PrepareLock` with the time
//! left before it aborts the drive, or `MountHostReply::Idle`, after which
//! the host asks again. After a `PrepareLock` reply the host prepares the
//! lock and writes one `LockPrepared` line on the same connection.
use serde::{Deserialize, Serialize};

/// The longest wait of one `PrepareLock` request.
pub(crate) const MAX_WATCH_MS: u64 = 30_000;
/// Longest `LockPrepared` line the daemon reads.
pub(crate) const MAX_PREPARED_LINE: usize = 64 * 1024;

/// One request of a drive host on its extension channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum MountHostRequest {
    /// Waits for a lock of the drive's vault (vault drives only).
    PrepareLock { deadline_ms: u64 },
}

/// The replies of `MountHostRequest` (`IpcResponse::MountHostExt`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MountHostReply {
    /// The vault locks: publish, drop clean cached content and unmount
    /// within `deadline_ms`. With `keep_unsaved` (a forced lock or
    /// „Sperren, Änderungen verschlüsselt aufbewahren“) unpublished changes
    /// stay sealed and the drive unmounts anyway; without it a drive with
    /// unpublished changes stays mounted and the lock is refused.
    PrepareLock {
        deadline_ms: u64,
        keep_unsaved: bool,
    },
    /// No lock began during the wait; ask again.
    Idle,
}

/// The host's answer after `MountHostReply::PrepareLock`, one JSON line on
/// the same connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LockPrepared {
    /// Changes, deletes and namespace intents still unpublished.
    pub(crate) pending: u64,
    /// The volume is gone; `false` when the drive refused the lock.
    pub(crate) unmounted: bool,
    /// Why publishing stopped (shown to the user, never logged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{LockPrepared, MountHostReply, MountHostRequest};

    #[test]
    fn mount_host_ext_wire_form() {
        let request = MountHostRequest::PrepareLock {
            deadline_ms: 25_000,
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(encoded, r#"{"op":"prepare_lock","deadline_ms":25000}"#);
        assert_eq!(
            serde_json::from_str::<MountHostRequest>(&encoded).unwrap(),
            request
        );
        let reply = MountHostReply::PrepareLock {
            deadline_ms: 4000,
            keep_unsaved: true,
        };
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"kind":"prepare_lock","deadline_ms":4000,"keep_unsaved":true}"#
        );
        assert_eq!(
            serde_json::to_string(&MountHostReply::Idle).unwrap(),
            r#"{"kind":"idle"}"#
        );
        let prepared = LockPrepared {
            pending: 2,
            unmounted: false,
            detail: None,
        };
        assert_eq!(
            serde_json::to_string(&prepared).unwrap(),
            r#"{"pending":2,"unmounted":false}"#
        );
    }
}
