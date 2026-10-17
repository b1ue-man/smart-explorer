//! Why the background worker started (DRV-M3; decision S2 K7 and contracts
//! V9, V11 of plan 2026-10-17).
//!
//! The login entry of the drives (`drive_autostart.rs`) starts
//! `<program> --sync-daemon --drive-restore`. `--drive-restore` is no start
//! kind of its own (V9): it marks this worker as started for the drive
//! restore. Such a worker is restricted while the background-sync autostart
//! is off or switched off by the desktop (`disabled_by_system`, S2 C2): it
//! starts neither the Share host nor the Exec host and plans no sync run
//! (V11); it only supervises and restores drives that need no Share. The
//! first other trigger lifts the restriction for good: a request of the app,
//! `se` or `se mcp` (`admit_request`), or the background-sync autostart being
//! switched on (`tick`). With the background-sync autostart on, the worker
//! would run anyway, so it starts unrestricted. Every other start keeps the
//! behaviour of v0.5.182.
//!
//! The marker travels in the environment variable `SE_DAEMON_START_REASON`:
//! the `--sync-daemon` process first runs as guardian and starts its worker
//! child with `--sync-daemon` only; the child inherits the environment.
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::super::ipc_host::ShareHost;
use super::super::ipc_protocol::IpcRequest;
use super::super::state::log;

const REASON_ENV: &str = "SE_DAEMON_START_REASON";
const DRIVE_RESTORE_VALUE: &str = "drive-restore";
const DAEMON_FLAG: &str = "--sync-daemon";
const DRIVE_RESTORE_FLAG: &str = "--drive-restore";

/// Restricted worker (module comment); false in every other process.
static RESTRICTED: AtomicBool = AtomicBool::new(false);
/// Serializes the promotion, so a second request waits until Share runs.
static PROMOTION: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunStartReason {
    Normal,
    DriveRestore,
}

/// Process start (`lib.rs`, `bin/se.rs`) before `run_daemon`: a
/// `--drive-restore` start marks the worker and asks a worker that already
/// runs to restore its drives (a Linux worker may outlive a logout); a new
/// worker restores at its own start.
pub fn note_daemon_start_args<S: AsRef<OsStr>>(arguments: &[S]) {
    if !arguments
        .iter()
        .any(|argument| argument.as_ref() == OsStr::new(DRIVE_RESTORE_FLAG))
    {
        return;
    }
    std::env::set_var(REASON_ENV, DRIVE_RESTORE_VALUE);
    super::ipc_protocol_drives::request_restore_if_running();
}

/// The exact two-argument form the login entry starts (`bin/se.rs` accepts
/// `--sync-daemon` only as its single argument).
pub fn is_drive_restore_invocation<S: AsRef<OsStr>>(arguments: &[S]) -> bool {
    arguments.len() == 2
        && arguments[0].as_ref() == OsStr::new(DAEMON_FLAG)
        && arguments[1].as_ref() == OsStr::new(DRIVE_RESTORE_FLAG)
}

/// Worker start, in place of the initial Share load of v0.5.182.
pub(crate) fn begin(share_host: &ShareHost) {
    let reason = reason_from(std::env::var_os(REASON_ENV).as_deref());
    let restricted = restricted_at_start(
        reason,
        crate::autostart::is_enabled(),
        crate::autostart::disabled_by_system(),
    );
    RESTRICTED.store(restricted, Ordering::SeqCst);
    if restricted {
        log("daemon started for the drive restore at login: Share and Exec stay off until the app, se or the background-sync autostart asks for them");
        return;
    }
    if let Err(error) = share_host.reload_now() {
        log(&format!("share worker initial load failed: {error}"));
    }
}

/// Background sync may plan runs: the autostart is on and the worker is not
/// restricted.
pub(crate) fn sync_enabled() -> bool {
    sync_allowed(
        RESTRICTED.load(Ordering::SeqCst),
        crate::autostart::is_enabled(),
    )
}

/// Whether drives that reach other devices through Share may mount.
pub(crate) fn share_allowed() -> bool {
    !RESTRICTED.load(Ordering::SeqCst)
}

/// One worker tick, in place of `ShareHost::tick`.
pub(crate) fn tick(share_host: &ShareHost) {
    if RESTRICTED.load(Ordering::SeqCst) {
        if !crate::autostart::is_enabled() || crate::autostart::disabled_by_system() {
            // Only the drive hosts are supervised; Share reloads, LAN
            // presence and Share events wait for the promotion.
            share_host.mounts.tick();
            return;
        }
        promote(share_host, "background-sync autostart switched on");
    }
    share_host.tick();
}

/// Every authenticated IPC request before it is served (`ipc.rs`).
pub(crate) fn admit_request(host: &ShareHost, req: &IpcRequest) {
    if RESTRICTED.load(Ordering::SeqCst) && request_promotes(req) {
        promote(host, "request of the app, se or se mcp");
    }
}

/// Restores the registered drives on a thread of its own, so the worker
/// loop and the IPC reply never wait for a remote connection.
pub(crate) fn restore_drives(host: &ShareHost) {
    let share = share_allowed();
    let host = host.clone();
    let spawned = std::thread::Builder::new()
        .name("drive-restore".into())
        .spawn(move || {
            super::drive_autostart::reconcile_registered();
            host.mounts.restore(&host, share);
        });
    if let Err(error) = spawned {
        log(&format!("drive restore not started: {error}"));
    }
}

fn promote(host: &ShareHost, cause: &str) {
    let Ok(_promotion) = PROMOTION.lock() else {
        return;
    };
    if !RESTRICTED.load(Ordering::SeqCst) {
        return;
    }
    log(&format!(
        "drive-restore daemon promoted ({cause}): starting Share and Exec"
    ));
    if let Err(error) = host.reload_now() {
        log(&format!("share worker initial load failed: {error}"));
    }
    RESTRICTED.store(false, Ordering::SeqCst);
    // Drives on other devices waited for Share.
    restore_drives(host);
}

fn reason_from(value: Option<&OsStr>) -> RunStartReason {
    if value == Some(OsStr::new(DRIVE_RESTORE_VALUE)) {
        RunStartReason::DriveRestore
    } else {
        RunStartReason::Normal
    }
}

/// S2 K7: only a drive-restore start while background sync would not start
/// the worker anyway.
fn restricted_at_start(
    reason: RunStartReason,
    sync_autostart: bool,
    disabled_by_system: bool,
) -> bool {
    reason == RunStartReason::DriveRestore && (!sync_autostart || disabled_by_system)
}

fn sync_allowed(restricted: bool, sync_autostart: bool) -> bool {
    sync_autostart && !restricted
}

/// Requests of the app, `se` and `se mcp` promote, `ShareSnapshot` included
/// (every caller of `share_worker_snapshot` sends it). The reachability
/// check (`Ping`), the read-only view of se Desktop (`ShareSnapshotPeek`,
/// V11) and the login entry itself (`Drive::Restore`) do not; neither does a
/// drive host, which carries no daemon token.
fn request_promotes(req: &IpcRequest) -> bool {
    match req {
        IpcRequest::Ping { .. } | IpcRequest::ShareSnapshotPeek { .. } => false,
        IpcRequest::Drive { req, .. } => req.promotes_daemon(),
        other => other.daemon_token().is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::ipc_protocol_drives::DriveIpcRequest;
    use super::*;
    use crate::mount::MountId;

    fn token() -> String {
        "token".into()
    }

    #[test]
    fn drive_task_drive_restore_no_jobs_without_sync_autostart() {
        // The login entry of the drives starts the worker, background sync
        // has no autostart: restricted, so no Share host and no sync run.
        let restricted = restricted_at_start(RunStartReason::DriveRestore, false, false);
        assert!(restricted);
        assert!(!sync_allowed(restricted, false));
        // The autostart switched on later lifts it (S2 C2).
        assert!(sync_allowed(false, true));
        // v0.5.182 behaviour of every other start: Share starts, sync stays
        // off without its autostart.
        assert!(!restricted_at_start(RunStartReason::Normal, false, false));
        assert!(!sync_allowed(false, false));
        // The marker reaches the worker child through the environment.
        assert_eq!(
            reason_from(Some(OsStr::new("drive-restore"))),
            RunStartReason::DriveRestore
        );
        assert_eq!(reason_from(None), RunStartReason::Normal);
        assert_eq!(
            reason_from(Some(OsStr::new("other"))),
            RunStartReason::Normal
        );
        // The login entry and the read-only view of se Desktop never lift it;
        // app, se and se mcp requests do; drive hosts carry no daemon token.
        let restore = IpcRequest::Drive {
            token: token(),
            req: DriveIpcRequest::Restore,
        };
        assert!(!request_promotes(&restore));
        assert!(!request_promotes(&IpcRequest::Ping { token: token() }));
        assert!(!request_promotes(&IpcRequest::ShareSnapshotPeek {
            token: token()
        }));
        assert!(request_promotes(&IpcRequest::ListMounts { token: token() }));
        assert!(request_promotes(&IpcRequest::DrainShareEvents {
            token: token()
        }));
        let options = IpcRequest::Drive {
            token: token(),
            req: DriveIpcRequest::SetOptions {
                id: MountId::parse("drive-1").unwrap(),
                auto_remount: true,
            },
        };
        assert!(request_promotes(&options));
        let host_status = IpcRequest::MountHostStatus {
            id: MountId::parse("drive-1").unwrap(),
            session_token: token(),
            status: crate::mount::MountStatus::Mounting,
            recovery: None,
            recovery_required: None,
        };
        assert!(!request_promotes(&host_status));
        // `se` accepts the login form only exactly.
        assert!(is_drive_restore_invocation(&[
            "--sync-daemon",
            "--drive-restore"
        ]));
        assert!(!is_drive_restore_invocation(&["--drive-restore"]));
        assert!(!is_drive_restore_invocation(&[
            "--drive-restore",
            "--sync-daemon"
        ]));
        assert!(!is_drive_restore_invocation(&[
            "--sync-daemon",
            "--drive-restore",
            "x"
        ]));
    }

    #[test]
    fn drive_task_drive_restore_no_jobs_when_sync_entry_disabled_by_system() {
        // The sync entry exists, but the desktop switched it off: the login
        // entry of the drives must not plan sync runs nor start Share.
        let restricted = restricted_at_start(RunStartReason::DriveRestore, true, true);
        assert!(restricted);
        assert!(!sync_allowed(restricted, true));
        // After the app or `--sync-daemon` (switched on again) promote it,
        // runs are planned as by every worker with the entry present.
        assert!(sync_allowed(false, true));
        // With the sync entry on, the worker would start at login anyway:
        // the drive-restore start is not restricted.
        let unrestricted = restricted_at_start(RunStartReason::DriveRestore, true, false);
        assert!(!unrestricted);
        assert!(sync_allowed(unrestricted, true));
        // v0.5.182: a normal start plans runs whenever the entry exists,
        // even when the desktop switched it off.
        let normal = restricted_at_start(RunStartReason::Normal, true, true);
        assert!(!normal);
        assert!(sync_allowed(normal, true));
    }

    #[test]
    fn drive_task_status_command_promotes_restricted_daemon() {
        // Every caller of `share_worker_snapshot` sends `ShareSnapshot`, for
        // example `se share status`: it lifts the restriction of a worker
        // started for the drive restore.
        let status = IpcRequest::ShareSnapshot { token: token() };
        assert!(request_promotes(&status));
        // se Desktop probes with `Ping` and reads the same answer with
        // `ShareSnapshotPeek` (`share_worker_peek`): neither starts Share
        // (V11), and neither does the login entry itself.
        let peek = IpcRequest::ShareSnapshotPeek { token: token() };
        assert!(!request_promotes(&peek));
        assert!(!request_promotes(&IpcRequest::Ping { token: token() }));
        assert!(!request_promotes(&IpcRequest::Drive {
            token: token(),
            req: DriveIpcRequest::Restore,
        }));
        // Both snapshot requests pass the same daemon-token check.
        assert_eq!(status.daemon_token(), Some("token"));
        assert_eq!(peek.daemon_token(), Some("token"));
        // `ShareSnapshot` keeps its wire form, so older and newer programs
        // stay compatible; the new variant has a tag of its own.
        let status_line = r#"{"t":"share_snapshot","token":"token"}"#;
        let peek_line = r#"{"t":"share_snapshot_peek","token":"token"}"#;
        assert_eq!(serde_json::to_string(&status).unwrap(), status_line);
        assert_eq!(serde_json::to_string(&peek).unwrap(), peek_line);
        let decoded: IpcRequest = serde_json::from_str(status_line).unwrap();
        assert!(matches!(decoded, IpcRequest::ShareSnapshot { .. }));
        assert!(request_promotes(&decoded));
        let decoded: IpcRequest = serde_json::from_str(peek_line).unwrap();
        assert!(matches!(decoded, IpcRequest::ShareSnapshotPeek { .. }));
        assert!(!request_promotes(&decoded));
    }
}
