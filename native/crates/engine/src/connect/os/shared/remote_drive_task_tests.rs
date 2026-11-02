use super::connector::{validate_sftp_agent_requirement, AgentFallback};
use crate::{creds::Protocol, mount::MountRootSecurity};

#[test]
fn remote_drive_task_strict_sftp_mount_requires_enabled_agent() {
    let strict = AgentFallback::for_mount(Protocol::Sftp, MountRootSecurity::Enforced);
    assert_eq!(strict, AgentFallback::RequireConfined);
    assert!(validate_sftp_agent_requirement(false, strict).is_err());
    assert!(validate_sftp_agent_requirement(true, strict).is_ok());
    assert!(!strict.permits_deploy_failure());
}

#[test]
fn remote_drive_task_browsing_and_trusted_mount_keep_transport_fallback() {
    let browse = AgentFallback::Allow;
    let trusted = AgentFallback::for_mount(Protocol::Sftp, MountRootSecurity::Trusted);
    let non_sftp = AgentFallback::for_mount(Protocol::Webdav, MountRootSecurity::Enforced);

    for policy in [browse, trusted, non_sftp] {
        assert_eq!(policy, AgentFallback::Allow);
        assert!(validate_sftp_agent_requirement(false, policy).is_ok());
        assert!(policy.permits_deploy_failure());
    }
}
