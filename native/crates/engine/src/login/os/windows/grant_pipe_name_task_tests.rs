//! E45 (L15): the Windows grant channel takes a random pipe name that the
//! broker publishes in its SYSTEM-owned status; a process holding the fixed
//! name of older brokers no longer blocks it.
use super::*;
use crate::login::os::shared::host_status::{noted_grant_pipe, valid_grant_pipe};

#[test]
fn login_selbst_task_windows_malformed_status_pipe_name_is_dropped() {
    let good = r"\\.\pipe\SmartExplorerLoginGrants-0123456789abcdef0123456789ABCDEF";
    assert!(valid_grant_pipe(good));
    assert_eq!(grant_pipe_name(Some(good.into())), good);
    for bad in [
        // The fixed name itself, too short, too long, not hex, other prefix.
        r"\\.\pipe\SmartExplorerLoginGrants",
        r"\\.\pipe\SmartExplorerLoginGrants-0123456789abcdef",
        r"\\.\pipe\SmartExplorerLoginGrants-0123456789abcdef0123456789abcdef00",
        r"\\.\pipe\SmartExplorerLoginGrants-0123456789abcdef0123456789abcdeg",
        r"\\.\pipe\SmartExplorerLoginBroker-0123456789abcdef0123456789abcdef",
        r"\\server\pipe\SmartExplorerLoginGrants-0123456789abcdef0123456789abcdef",
        "",
    ] {
        assert!(!valid_grant_pipe(bad), "{bad}");
        assert_eq!(
            grant_pipe_name(Some(bad.into())),
            LEGACY_GRANT_PIPE,
            "{bad}"
        );
    }
    // A broker of an older version publishes no name: the fixed one.
    assert_eq!(grant_pipe_name(None), LEGACY_GRANT_PIPE);
}

#[test]
#[ignore = "Administratorrechte; Suite crates-task"]
fn login_selbst_task_windows_held_fixed_pipe_name_does_not_block_grant_channel() {
    static STOP: AtomicBool = AtomicBool::new(false);
    assert!(privileged(), "needs an elevated administrator");
    // Another local process holds the fixed name for good.
    let held = pipe::create_with(LEGACY_GRANT_PIPE, "D:P(A;;GA;;;WD)");
    start_grant_pipe(&STOP).expect("grant pipe");
    let name = noted_grant_pipe().expect("the broker notes its pipe");
    assert!(valid_grant_pipe(&name), "{name}");
    assert_ne!(name, LEGACY_GRANT_PIPE);
    // The broker's pipe exists: the worker reaches it and checks its server.
    // The test process is not LocalSystem, so the check refuses it.
    let reached = pipe::connect_grants(&name).err().map(|error| error.kind());
    assert_eq!(reached, Some(io::ErrorKind::PermissionDenied));
    STOP.store(true, Ordering::SeqCst);
    drop(held);
}
