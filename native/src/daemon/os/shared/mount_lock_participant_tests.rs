//! Signal of the lock deadline of vault drives (DRV-M7, stage drive-vault;
//! S2-V4): the host takes its order once, an answer in time reaches the
//! participant, silence until the deadline aborts the drive (its journal
//! keeps the sealed changes, `host_storage_tests.rs`), a refusal counts the
//! changes without naming a path, and a forced lock never accepts one.
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::MountManager;
use super::{verdict, DriveVerdict, LockDesk};
use crate::daemon::ipc_protocol_mount_host::LockPrepared;

const SITE: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn drive_task_vault_lock_deadline_keeps_sealed_and_aborts() {
    let desk = LockDesk::default();
    let deadline = Instant::now() + Duration::from_millis(300);
    desk.post("drive-a", deadline, false);
    // The host takes the order once, with the lock's deadline and kind.
    assert_eq!(
        desk.take("drive-a", Duration::from_secs(1)),
        Some((deadline, false))
    );
    assert_eq!(desk.take("drive-a", Duration::from_millis(20)), None);
    assert_eq!(desk.take("drive-b", Duration::from_millis(20)), None);
    // No answer by the deadline: the participant aborts the drive.
    let waited = Instant::now();
    assert!(desk.wait_answer("drive-a", deadline).is_none());
    assert!(waited.elapsed() < Duration::from_secs(2));
    assert_eq!(verdict(None, false), DriveVerdict::Abort);
    assert_eq!(verdict(None, true), DriveVerdict::Abort);

    let unmounted = LockPrepared {
        pending: 1,
        unmounted: true,
        detail: None,
    };
    assert_eq!(verdict(Some(&unmounted), false), DriveVerdict::Unmounted);
    let refused = LockPrepared {
        pending: 2,
        unmounted: false,
        detail: Some("Remote /Geheim/Plan.txt nicht erreichbar".into()),
    };
    match verdict(Some(&refused), false) {
        DriveVerdict::Refused(text) => {
            assert!(text.starts_with("2 "), "{text}");
            assert!(!text.contains("Geheim"), "a path left the drive: {text}");
        }
        other => panic!("unexpected verdict {other:?}"),
    }
    // A forced lock (or „verschlüsselt aufbewahren“) never waits for a
    // refusing drive: it is aborted, its changes stay sealed.
    assert_eq!(verdict(Some(&refused), true), DriveVerdict::Abort);

    // An answer in time reaches the participant across threads.
    let desk = Arc::new(LockDesk::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    desk.post("drive-c", deadline, true);
    let host_desk = Arc::clone(&desk);
    let host = std::thread::spawn(move || {
        let (_, keep_unsaved) = host_desk.take("drive-c", Duration::from_secs(5)).unwrap();
        assert!(keep_unsaved);
        host_desk.answer(
            "drive-c",
            LockPrepared {
                pending: 0,
                unmounted: true,
                detail: None,
            },
        );
    });
    let answer = desk.wait_answer("drive-c", deadline);
    host.join().unwrap();
    assert_eq!(answer.map(|answer| answer.unmounted), Some(true));

    // Without running vault drives a lock has nothing to wait for.
    let manager = MountManager::from_registry(Ok(Vec::new()), |_| Vec::new());
    let quick = Instant::now() + Duration::from_secs(4);
    assert_eq!(manager.prepare_vault_lock(SITE, quick, true), Ok(()));
}
