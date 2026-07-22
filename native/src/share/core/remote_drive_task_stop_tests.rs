use super::service_tests::test_service;
use super::types::ShareCmd;
use std::sync::atomic::Ordering;

#[test]
fn remote_drive_task_stop_is_a_synchronous_idempotent_authorization_barrier() {
    let service = test_service();
    let initial_epoch = service.iroh.filesystem_authorization_epoch();
    assert!(service.iroh.require_sharing_active().is_ok());

    let first = service.cmd(ShareCmd::Stop);
    assert!(first
        .unwrap_err()
        .contains("Share-Kommando konnte nicht zugestellt werden"));
    assert!(service.iroh.require_sharing_active().is_err());
    assert_eq!(
        service.iroh.filesystem_authorization_epoch(),
        initial_epoch + 1
    );
    assert!(service.stopped.load(Ordering::Acquire));

    let second = service.cmd(ShareCmd::Stop);
    assert!(second.is_err());
    assert!(service.iroh.require_sharing_active().is_err());
    assert_eq!(
        service.iroh.filesystem_authorization_epoch(),
        initial_epoch + 1
    );
    assert!(service.stopped.load(Ordering::Acquire));
}
