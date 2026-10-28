//! „Remote Desktop“ as one action without a form (design 2.1, E1, user
//! rule r3 „die eine freigabe muss reichen, der rest muss laufen“): the
//! present relation, a fresh authenticated probe, the host's self-setup
//! when nothing is startable yet, the service of the smallest rank, this
//! OS's client, the whole desktop (or one published RemoteApp), then the
//! process-bound tunnel on the device address.
use super::super::clients::{choose_client, Credentials};
use super::{ownership::adopt_owned, RdpSession, SessionView, StartContext};
use crate::rdp::core::StartRequest;
use std::time::Duration;

/// Overrides of the derived choices; the default derives everything.
#[derive(Debug, Default)]
pub(crate) struct OpenOptions {
    /// One published RemoteApp alias instead of the whole desktop.
    pub(crate) remote_app: Option<String>,
    /// `--service` (hidden CLI override of the service rank).
    pub(crate) service: Option<String>,
    /// `--client` (hidden CLI override of the client order).
    pub(crate) client: Option<String>,
    /// Typed into the desktop GUI's own dialog (Linux without
    /// `sdl-freerdp3`, design 2.8); se-managed credentials need none.
    pub(crate) credentials: Option<Credentials>,
    /// `None` (default): no time limit for the host's answers.
    pub(crate) wait: Option<Duration>,
}

/// Shown when no stored relation to the device is present (design 2.1).
pub(crate) const DEVICE_OFFLINE: &str = "rdp_device_offline: Gerät ist nicht erreichbar";

/// Design 9.1 `open_device`: GUI and MCP hand the session to the owner
/// registry; the returned view describes it.
#[cfg(any(target_os = "android", all(unix, test)))]
pub(crate) fn open_device(
    device_id: &str,
    public_key: &str,
    remote_app: Option<&str>,
) -> Result<SessionView, String> {
    let options = OpenOptions {
        remote_app: remote_app.map(str::to_string),
        ..OpenOptions::default()
    };
    open_device_with(device_id, public_key, options, || {})
}

/// `open_device` with overrides and a callback for each host question.
pub(crate) fn open_device_with(
    device_id: &str,
    public_key: &str,
    options: OpenOptions,
    pending: impl FnMut(),
) -> Result<SessionView, String> {
    adopt_owned(open_device_session(
        device_id, public_key, options, pending,
    )?)
}

/// The CLI waits as the owner of the returned session.
pub(crate) fn open_device_session(
    device_id: &str,
    public_key: &str,
    options: OpenOptions,
    mut pending: impl FnMut(),
) -> Result<RdpSession, String> {
    // The client first: a device without one (Android) stops here.
    let client = choose_client(options.client.as_deref())?;
    let snapshot = crate::daemon::share_worker_snapshot()?;
    let target =
        crate::share::peer_select::present_relation(&snapshot.profiles, device_id, public_key)
            .ok_or_else(|| DEVICE_OFFLINE.to_string())?;
    let wait = options.wait;
    let (mut probe, setup) = crate::daemon::apps_rdp_probe_v2(target.clone(), wait, &mut pending)?;
    if options.service.is_none() && probe.report().preferred_service().is_none() {
        if !setup.setup_possible() {
            return Err(setup.text());
        }
        // Trigger T4: the host sets RDP up (its UAC/polkit dialog), then
        // answers with a new probe.
        let (prepared, setup) =
            crate::daemon::apps_rdp_prepare(target.clone(), wait, &mut pending)?;
        if prepared.report().preferred_service().is_none() {
            let reason = prepared.choose_service(None).err().unwrap_or_default();
            return Err(format!("{}; {reason}", setup.text()));
        }
        probe = prepared;
    }
    let service = probe.choose_service(options.service.as_deref())?.clone();
    let request = StartRequest::derive(&service, &client, options.remote_app);
    request.validate(&probe)?;
    let credentials = if service.managed_credentials {
        let (username, password) =
            crate::daemon::apps_rdp_credential(target.clone(), &service.id, wait)?;
        Some(Credentials {
            username,
            domain: None,
            password,
        })
    } else {
        options.credentials
    };
    let bind = crate::share::device_address::device_loopback(device_id, public_key)
        .map_err(|error| format!("Geräteadresse nicht verfügbar: {error}"))?;
    let context = StartContext {
        bind: Some(bind),
        credentials,
        wait,
    };
    RdpSession::start(target, probe, &request, context, pending)
}
