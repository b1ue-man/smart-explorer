use super::connector::{validate_sftp_agent_requirement, AgentFallback};
use crate::{creds::Protocol, mount::MountRootSecurity};

#[test]
fn remote_drive_task_strict_sftp_mount_requires_enabled_agent() {
    let strict = AgentFallback::for_mount(Protocol::Sftp, MountRootSecurity::Enforced);
    assert_eq!(strict, AgentFallback::RequireConfined);
    assert!(validate_sftp_agent_requirement(false, strict).is_err());
    assert!(validate_sftp_agent_requirement(true, strict).is_ok());
    assert!(!strict.permits_deploy_failure());

    // Cases that keep the transport fallback: browsing and a trusted SFTP mount, and non-SFTP mounts.
    for (case, policy) in [
        ("browsing", AgentFallback::Allow),
        (
            "trusted SFTP mount",
            AgentFallback::for_mount(Protocol::Sftp, MountRootSecurity::Trusted),
        ),
        (
            "non-SFTP mount",
            AgentFallback::for_mount(Protocol::Webdav, MountRootSecurity::Enforced),
        ),
    ] {
        assert_eq!(policy, AgentFallback::Allow, "{case}");
        assert!(
            validate_sftp_agent_requirement(false, policy).is_ok(),
            "{case}"
        );
        assert!(policy.permits_deploy_failure(), "{case}");
    }
}
