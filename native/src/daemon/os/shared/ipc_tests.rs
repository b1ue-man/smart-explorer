//! The request authentication of the daemon IPC (`ipc.rs`): token checks,
//! the drive host's session token and the vault spool key.
use super::super::ipc_protocol::VaultIpcRequest;
use super::{require_request_auth, require_token, IpcRequest, ShareHost};
use crate::daemon::ipc_protocol_mount_host::MountHostRequest;
use crate::mount::MountId;

const DAEMON: &str = "daemon-token";

#[test]
fn token_rejects_mismatch() {
    assert!(require_token("abc", "abc").is_ok());
    assert!(require_token("abc", "def").is_err());
}

fn spool_key(token: &str, mount_id: &str) -> IpcRequest {
    IpcRequest::Vault {
        token: token.into(),
        req: VaultIpcRequest::SpoolKey {
            mount_id: mount_id.into(),
        },
    }
}

/// R5: the daemon token of the GUI, `se` or `se mcp` never reaches
/// `SpoolKey`; only the session token of the named drive's host does,
/// and a worker without that drive refuses every token.
#[test]
fn vault_task_spool_key_refuses_daemon_token_and_unknown_mount() {
    let host = ShareHost::new("0".repeat(32));
    let unknown = "f".repeat(32);
    assert_eq!(spool_key(DAEMON, &unknown).daemon_token(), None);
    for request in [
        spool_key(DAEMON, &unknown),
        spool_key("guessed-session-token", &unknown),
        spool_key(DAEMON, "../not-a-mount-id"),
    ] {
        let refused = require_request_auth(DAEMON, &request, &host).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    }
    // Every other vault request stays on the daemon token.
    let status = IpcRequest::Vault {
        token: DAEMON.into(),
        req: VaultIpcRequest::Status,
    };
    assert_eq!(status.daemon_token(), Some(DAEMON));
    assert!(require_request_auth(DAEMON, &status, &host).is_ok());
    assert!(require_request_auth("other", &status, &host).is_err());
}

/// R5: `MountHostExt` carries no daemon token; a session token that
/// belongs to no running drive host is refused.
#[test]
fn drive_task_mount_host_ext_requires_session_token_of_its_mount() {
    let host = ShareHost::new("0".repeat(32));
    let request = IpcRequest::MountHostExt {
        id: MountId::parse(&"e".repeat(32)).unwrap(),
        session_token: DAEMON.into(),
        req: MountHostRequest::PrepareLock { deadline_ms: 1000 },
    };
    assert_eq!(request.daemon_token(), None);
    let refused = require_request_auth(DAEMON, &request, &host).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
}
