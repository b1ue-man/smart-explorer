//! Read-only installed-record report; it never starts or changes a service.
use crate::login::Policy;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct InstallationStatus {
    pub state: String,
    pub reason: Option<String>,
    pub installed_release: Option<String>,
    pub policy: Option<Policy>,
    /// Record validation is not a claim that SCM/systemd reports a live service.
    pub service_runtime: String,
    #[serde(default)]
    pub activity: super::host_status::HostActivity,
    pub limits: Vec<String>,
}
pub(crate) fn installation_status() -> InstallationStatus {
    let result = super::installation::load().and_then(|installed| {
        super::installation::verify_hash(&installed.image.file, &installed.manifest.sha256)?;
        Ok((installed.manifest.version, installed.policy))
    });
    let (state, reason, installed_release, policy) = match result {
        Ok((version, policy)) => ("installed", None, Some(version), Some(policy)),
        Err(error) => {
            let state = match error.kind() {
                std::io::ErrorKind::NotFound => "not_installed",
                std::io::ErrorKind::PermissionDenied => "policy_blocked",
                std::io::ErrorKind::Unsupported => "unavailable",
                _ => "invalid_installation",
            };
            (state, Some(error.to_string()), None, None)
        }
    };
    let activity = super::host_status::activity();
    InstallationStatus { state: state.into(), reason, installed_release, policy,
        service_runtime: activity.state.clone(), activity, limits: vec![
            "Only an explicit administrative service installation grants pre-login access.".into(),
            "The public machine target must be explicitly pinned; a normal device identity does not prove it.".into(),
            "Every operation is independently signed for its current boot, user, session, desktop, policy and sequence.".into(),
            "Five seconds without a new signed operation ends the broker bridge; status calls do not renew it.".into(),
            "Windows uses only its active console/input desktop and existing SAS policy; Linux requires an actually detected X11 greeter.".into(),
            "Wayland greeter access and Android privileged login hosting remain unavailable.".into(),
        ] }
}
