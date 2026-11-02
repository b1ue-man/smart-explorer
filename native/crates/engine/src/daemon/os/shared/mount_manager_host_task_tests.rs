use super::host_status_is_terminal;
use crate::mount::MountStatus;

#[test]
fn remote_drive_task_mount_host_failure_needs_a_terminal_recovery_boundary() {
    let callback_failure = MountStatus::Failed {
        detail: "write callback failed".into(),
    };
    assert!(!host_status_is_terminal(&callback_failure, false));
    assert!(host_status_is_terminal(&callback_failure, true));
    assert!(host_status_is_terminal(
        &MountStatus::RuntimeUnavailable {
            detail: "Dokany unavailable".into(),
        },
        false
    ));
    assert!(!host_status_is_terminal(&MountStatus::Mounting, true));
}
