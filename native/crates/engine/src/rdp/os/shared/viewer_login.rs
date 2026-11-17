//! Viewer side of the sign-in with the device's account (plan 2026-11-17
//! P4): asks the host (`RdpLogin`) which account and secret the session
//! uses, and lets the person store the password of that account on the
//! device once (`RdpStorePassword`: the desktop GUI and `se rdp password`
//! store it, MCP only forgets it). This device keeps no password: a received
//! secret lives only in the session's `Credentials` (mstsc: the
//! session-bound `TERMSRV` credential, deleted with the last session).
use super::clients::Credentials;
use crate::rdp::core::{viewer_plan, HostAccount, LoginInfo, LoginView, LOGIN_FALLBACK};
use crate::share::{apps_wire::ManagedSecret, PeerOpenTarget};
use std::time::Duration;

/// What one start hands its client.
pub struct Login {
    pub credentials: Option<Credentials>,
    /// The account without a password (the client asks for it).
    pub account: Option<HostAccount>,
    pub view: Option<LoginView>,
}
impl Login {
    /// Credentials se already has (GNOME-managed, typed in the GUI form).
    pub fn given(credentials: Option<Credentials>) -> Self {
        Self {
            credentials,
            account: None,
            view: None,
        }
    }
}

/// The host's sign-in for `service_id`. A host that refuses `RdpLogin` (up
/// to 0.5.197) yields a start as before, with the reason in `view`.
pub fn host_login(
    target: &PeerOpenTarget,
    service_id: &str,
    wait: Option<Duration>,
    ask_password: bool,
) -> Result<Login, String> {
    let reply = crate::daemon::apps_rdp_login(target.clone(), service_id, false, wait);
    let plan = viewer_plan(reply, ask_password)?;
    let credentials = plan.credentials.map(|(account, password)| Credentials {
        username: account.username().to_string(),
        domain: account.domain().map(str::to_string),
        password,
    });
    Ok(Login {
        credentials,
        account: plan.account,
        view: Some(plan.view),
    })
}

/// The sign-in state of the device for this viewer, without a password.
pub fn login_status(
    device_id: &str,
    public_key: &str,
    wait: Option<Duration>,
) -> Result<LoginInfo, String> {
    let (target, service) = login_target(device_id, public_key, wait)?;
    // A host up to 0.5.197 refuses `RdpLogin`: named as such.
    crate::daemon::apps_rdp_login(target, &service, true, wait)
        .map_err(|error| format!("{LOGIN_FALLBACK} ({error})"))
}

/// Stores the password of the device's account on the device (it checks
/// it first, at most five wrong ones per hour); `None` deletes the stored
/// one there.
pub fn store_password(
    device_id: &str,
    public_key: &str,
    password: Option<ManagedSecret>,
    wait: Option<Duration>,
) -> Result<LoginInfo, String> {
    let (target, service) = login_target(device_id, public_key, wait)?;
    crate::daemon::apps_rdp_store_password(target, &service, password, wait)
}

/// The present relation and the service a start would use.
fn login_target(
    device_id: &str,
    public_key: &str,
    wait: Option<Duration>,
) -> Result<(PeerOpenTarget, String), String> {
    let snapshot = crate::daemon::share_worker_snapshot()?;
    let target =
        crate::share::peer_select::present_relation(&snapshot.profiles, device_id, public_key)
            .ok_or_else(|| super::session::DEVICE_OFFLINE.to_string())?;
    let (probe, _) = crate::daemon::apps_rdp_probe_v2(target.clone(), wait, || {})?;
    let report = probe.report();
    let service = report
        .preferred_service()
        .or_else(|| report.services.first())
        .map(|service| service.id.clone())
        .ok_or_else(|| "Das Gerät meldet keinen Remote-Desktop-Dienst".to_string())?;
    Ok((target, service))
}
