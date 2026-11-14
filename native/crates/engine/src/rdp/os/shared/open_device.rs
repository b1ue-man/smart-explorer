//! „Remote Desktop“ as one action without a form (design 2.1, E1, user
//! rule r3 „die eine freigabe muss reichen, der rest muss laufen“): the
//! present relation, a fresh authenticated probe, the host's self-setup
//! when nothing is startable yet, the service of the smallest rank, this
//! OS's client, the whole desktop (or one published RemoteApp), then the
//! process-bound tunnel on the device address.
use super::super::clients::{choose_client, Credentials};
use super::{ownership::adopt_owned, RdpSession, SessionView, StartContext};
use crate::rdp::core::{
    AuthenticatedProbe, CertificatePolicy, ClientCapability, HostService, SessionMode, StartRequest,
};
use crate::rdp::host_setup::switched_off_between_sessions;
use std::time::Duration;

/// Overrides of the derived choices; the default derives everything.
#[derive(Debug, Default)]
pub struct OpenOptions {
    /// One published RemoteApp alias instead of the whole desktop.
    pub remote_app: Option<String>,
    /// `--service` (hidden CLI override of the service rank).
    pub service: Option<String>,
    /// `--client` (hidden CLI override of the client order).
    pub client: Option<String>,
    /// Typed into the desktop GUI's own dialog (Linux without
    /// `sdl-freerdp3`, design 2.8); se-managed credentials need none.
    pub credentials: Option<Credentials>,
    /// `None` (default): no time limit for the host's answers.
    pub wait: Option<Duration>,
}

/// Shown when no stored relation to the device is present (design 2.1).
pub const DEVICE_OFFLINE: &str = "rdp_device_offline: Gerät ist nicht erreichbar";

/// Design 9.1 `open_device`: GUI and MCP hand the session to the owner
/// registry; the returned view describes it.
#[cfg(any(target_os = "android", all(unix, any(test, feature = "test-support"))))]
pub fn open_device(
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
pub fn open_device_with(
    device_id: &str,
    public_key: &str,
    options: OpenOptions,
    pending: impl FnMut(),
) -> Result<SessionView, String> {
    adopt_owned(open_device_session(
        device_id, public_key, options, pending,
    )?)
}

/// The service that is switched off between sessions (E29). The condition is
/// `switched_off_between_sessions`, defined once in `self_setup_state.rs`; no
/// second version exists here. Without a wanted service it is the choice only
/// when no service starts. The session start switches it on.
fn off_between_sessions<'a>(
    probe: &'a AuthenticatedProbe,
    wanted: Option<&str>,
) -> Option<&'a HostService> {
    match wanted {
        Some(id) => probe
            .service(id)
            .ok()
            .filter(|service| switched_off_between_sessions(service)),
        None if probe.report().preferred_service().is_some() => None,
        None => probe
            .report()
            .services
            .iter()
            .find(|service| switched_off_between_sessions(service)),
    }
}

/// The checks of `StartRequest::validate` that need no listener (E29, P3): the
/// verification policy, TLS, a known session mode, the proved certificate and a
/// RemoteApp alias. They run before the switch-on, because the switch-on is the
/// side effect that a start which cannot succeed must not leave behind.
fn checked_before_switch_on(
    service: &HostService,
    client: &ClientCapability,
    remote_app: Option<&str>,
) -> Result<(), String> {
    if client.certificate_policy == CertificatePolicy::ExactPeerPin {
        return Err("No installed-client adapter proves exclusive peer-certificate pinning; select an explicitly documented verification policy".into());
    }
    if service.tls != Some(true) {
        return Err("TLS support of the RDP service is not proved".into());
    }
    if service.session_mode == SessionMode::Unknown {
        return Err(service.session_mode.notice().into());
    }
    if service.certificate_sha256.is_none() {
        return Err(
            "The host has not supplied a proved SHA-256 RDP certificate fingerprint".into(),
        );
    }
    if let Some(alias) = remote_app {
        if !service
            .remote_apps
            .iter()
            .any(|published| published.alias == alias)
        {
            return Err("RemoteApp alias was not published by this Windows Server RDS host".into());
        }
    }
    Ok(())
}

/// The CLI waits as the owner of the returned session.
pub fn open_device_session(
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
    let wanted = options.service.as_deref();
    let (mut probe, setup) = crate::daemon::apps_rdp_probe_v2(target.clone(), wait, &mut pending)?;
    if wanted.is_none()
        && probe.report().preferred_service().is_none()
        && off_between_sessions(&probe, None).is_none()
    {
        if !setup.setup_possible() {
            return Err(setup.text());
        }
        // Trigger T4: the host sets RDP up (its UAC/polkit dialog), then
        // answers with a new probe.
        let (prepared, setup) =
            crate::daemon::apps_rdp_prepare(target.clone(), wait, &mut pending)?;
        if prepared.report().preferred_service().is_none()
            && off_between_sessions(&prepared, None).is_none()
        {
            let reason = prepared.choose_service(None).err().unwrap_or_default();
            return Err(format!("{}; {reason}", setup.text()));
        }
        probe = prepared;
    }
    // E29: a sharing that is off between sessions is switched on by the
    // session start; any startable service is used as before.
    let off = if wanted.is_none() && probe.report().preferred_service().is_some() {
        None
    } else {
        off_between_sessions(&probe, wanted).map(|found| found.id.clone())
    };
    let (probe, service, credentials) = match off {
        Some(id) => {
            // The checks that need no listener run before the switch-on (E29, P3).
            checked_before_switch_on(probe.service(&id)?, &client, options.remote_app.as_deref())?;
            // The host switches the sharing on and issues this session's
            // credential before the tunnel exists; the probe is taken again,
            // so the listener it proves is the one the tunnel will use.
            let (username, password) =
                crate::daemon::apps_rdp_credential(target.clone(), &id, wait)?;
            let (fresh, _) = crate::daemon::apps_rdp_probe_v2(target.clone(), wait, &mut pending)?;
            let service = fresh.choose_service(Some(id.as_str()))?.clone();
            (
                fresh,
                service,
                Some(Credentials {
                    username,
                    domain: None,
                    password,
                }),
            )
        }
        None => {
            let service = probe.choose_service(wanted)?.clone();
            StartRequest::derive(&service, &client, options.remote_app.clone()).validate(&probe)?;
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
            (probe, service, credentials)
        }
    };
    let request = StartRequest::derive(&service, &client, options.remote_app);
    let bind = crate::share::device_address::device_loopback(device_id, public_key)
        .map_err(|error| format!("Geräteadresse nicht verfügbar: {error}"))?;
    let context = StartContext {
        bind: Some(bind),
        credentials,
        wait,
    };
    RdpSession::start(target, probe, &request, context, pending)
}
