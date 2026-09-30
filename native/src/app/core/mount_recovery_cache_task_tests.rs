use super::*;
use crate::mount::*;

fn failed() -> MountSnapshot {
    MountSnapshot {
        config: MountConfig::new(MountId::parse("startup-recovery").unwrap(),
            MountSource::SavedRemote { account: "saved-test".into(), root: BackendRoot::parse("/").unwrap() },
            DriveSelection::Automatic, MountMode::ReadOnly, "Smart Explorer").unwrap(),
        status: MountStatus::Failed { detail: "Laufwerk-Recovery lokal pruefen".into() },
        recovery: MountRecovery::Required,
        recovery_required_compat: true,
    }
}

#[test]
fn mount_recovery_cache_task_startup_shows_details_and_later_failures_still_alert() {
    let mount = failed();
    assert!(has_mount_attention(std::slice::from_ref(&mount)));
    assert!(mount_list_alert(&[], std::slice::from_ref(&mount), true).is_none());
    assert!(mount_list_alert(&[], std::slice::from_ref(&mount), false).is_some());
    assert!(mount_list_alert(std::slice::from_ref(&mount), std::slice::from_ref(&mount), false).is_none());
    let mut changed = mount.clone();
    changed.status = MountStatus::Failed { detail: "New upload failure".into() };
    assert!(mount_list_alert(&[mount], std::slice::from_ref(&changed), false).is_some());
    assert!(mount_status_alert(None, &changed).is_some());
    assert!(!has_mount_attention(&[]));
}
