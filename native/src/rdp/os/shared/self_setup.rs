//! The host sets Remote Desktop up by itself (design 3, user rule r3 „die
//! eine freigabe muss reichen, der rest muss laufen“). Triggers T1–T4, one
//! marker per version, one setup at a time. The platform adapters do the
//! work; this module decides when, records the result and refreshes the
//! port list, listener owners and feed summary after every probe.
use super::super::platform::host_setup as platform;
use crate::rdp::core::{note_host_summary, HostProbe, RdpFeedSummary};
pub(crate) use crate::rdp::core::{SetupState, SetupTrigger};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

/// Status line while an administrator prompt may be open (threat T-R6).
// Consumer: the host page „Dieses Gerät“ (work package WP3).
#[allow(dead_code)]
pub(crate) const STATUS_TEXT: &str = "Remote Desktop wird eingerichtet (Administratorfreigabe)";

/// One setup at a time; a viewer's request (T4) waits for a running one.
static RUN: Mutex<()> = Mutex::new(());
static RUNNING: AtomicBool = AtomicBool::new(false);
static HOST_DEVICE: RwLock<Option<String>> = RwLock::new(None);
/// The last failure outside the marker (marker write, thread start); the
/// host page shows it next to the setup state.
static LAST_ERROR: RwLock<Option<String>> = RwLock::new(None);
/// Background requests of test builds are recorded instead of started.
#[cfg(test)]
pub(crate) static TEST_REQUESTS: Mutex<Vec<SetupTrigger>> = Mutex::new(Vec::new());

/// The marker of one version: `<sync_data_dir>/rdp-setup-<version>.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Marker {
    pub(crate) state: SetupState,
    pub(crate) trigger: SetupTrigger,
    pub(crate) at: i64,
}

/// What a trigger does with the current marker (design 3.1, 3.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Run,
    Keep(SetupState),
}

/// T1/T4 are a person's action and always run (they also ask again after
/// `declined`). T2 runs once per version. T3 runs without a marker or after
/// the failures that a present device can resolve (no interactive session,
/// locked keyring), so no administrator prompt repeats unattended.
pub(crate) fn decide(trigger: SetupTrigger, marker: Option<&Marker>) -> Decision {
    let Some(marker) = marker else {
        return Decision::Run;
    };
    if trigger.user_action() {
        return Decision::Run;
    }
    match (trigger, &marker.state) {
        (SetupTrigger::PeerOnline, SetupState::Failed(reason))
            if matches!(reason.as_str(), "no_interactive_session" | "keyring_locked") =>
        {
            Decision::Run
        }
        (_, state) => Decision::Keep(state.clone()),
    }
}

fn marker_path() -> PathBuf {
    let name = format!("rdp-setup-{}.json", env!("CARGO_PKG_VERSION"));
    crate::support_dirs::sync_data_dir().join(name)
}

pub(crate) fn read_marker() -> Option<Marker> {
    let text = std::fs::read_to_string(marker_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_marker(marker: &Marker) -> Result<(), String> {
    let path = marker_path();
    let bytes = serde_json::to_vec(marker).map_err(|error| error.to_string())?;
    let staged = path.with_extension("json.tmp");
    std::fs::write(&staged, bytes)
        .and_then(|()| std::fs::rename(&staged, &path))
        .map_err(|error| format!("RDP-Einrichtungsstand nicht speicherbar: {error}"))
}

fn note_error(error: String) {
    eprintln!("RDP-Selbsteinrichtung: {error}");
    if let Ok(mut last) = LAST_ERROR.write() {
        *last = Some(error);
    }
}

/// The last setup failure that the marker could not carry.
// Consumer: the host page „Dieses Gerät“ (work package WP3).
#[allow(dead_code)]
pub(crate) fn last_error() -> Option<String> {
    LAST_ERROR.read().ok().and_then(|last| last.clone())
}

pub(crate) fn note_host_device(device_id: &str) {
    if let Ok(mut current) = HOST_DEVICE.write() {
        *current = Some(device_id.to_string());
    }
}

fn host_device() -> Option<String> {
    HOST_DEVICE.read().ok().and_then(|current| current.clone())
}

/// Whether a setup (possibly with an open administrator prompt) runs now.
pub(crate) fn in_progress() -> bool {
    RUNNING.load(Ordering::Acquire)
}

/// The setup state a probe v2 reports with `probe` (design 9.2).
pub(crate) fn current_state(probe: &HostProbe) -> SetupState {
    if let Some(state) = platform::impossible() {
        return state;
    }
    let marker = read_marker().map(|marker| marker.state);
    if probe.preferred_service().is_some() {
        return match marker {
            Some(SetupState::Done) => SetupState::Done,
            _ => SetupState::NotNeeded,
        };
    }
    match marker {
        Some(SetupState::Done | SetupState::NotNeeded) => {
            SetupState::Failed(format!("no_listener: {}", unavailable_reason(probe)))
        }
        Some(state) => state,
        None => SetupState::Failed("not_set_up".into()),
    }
}

fn unavailable_reason(probe: &HostProbe) -> String {
    let mut reasons: Vec<String> = probe
        .services
        .iter()
        .filter_map(|service| {
            let reason = service.startable().err()?;
            Some(format!("{}: {reason}", service.id))
        })
        .collect();
    if let Some(reason) = &probe.unavailable_reason {
        reasons.push(reason.clone());
    }
    if reasons.is_empty() {
        "kein startbarer RDP-Dienst".into()
    } else {
        reasons.join("; ")
    }
}

/// A fresh probe of this host. It refreshes the RDP port list and listener
/// owners of `access_steering` and the feed summary (design 3.1, 5.2).
pub(crate) fn refresh_probe() -> HostProbe {
    let probe = super::super::probe();
    crate::share::access_steering::note_own_rdp_ports(
        probe.services.iter().filter_map(|service| service.port),
    );
    crate::share::access_steering::note_own_rdp_listeners(
        probe
            .services
            .iter()
            .filter_map(|service| Some((service.port?, service.listener_pid?))),
    );
    note_host_summary(RdpFeedSummary::from_probe(&probe, current_state(&probe)));
    probe
}

/// Runs the self-setup for `trigger` and returns its result. Blocks while an
/// administrator prompt (UAC, polkit) or a package installation runs; there
/// is no fixed limit (user rule r7).
pub(crate) fn ensure_rdp_host(trigger: SetupTrigger) -> SetupState {
    let _turn = RUN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Decision::Keep(state) = decide(trigger, read_marker().as_ref()) {
        return state;
    }
    let mut probe = refresh_probe();
    let state = if probe.preferred_service().is_some() {
        SetupState::NotNeeded
    } else {
        RUNNING.store(true, Ordering::Release);
        let device = host_device();
        let state = platform::ensure(&probe, device.as_deref());
        RUNNING.store(false, Ordering::Release);
        probe = refresh_probe();
        match state {
            SetupState::Done if probe.preferred_service().is_none() => {
                SetupState::Failed(format!("no_listener: {}", unavailable_reason(&probe)))
            }
            other => other,
        }
    };
    let marker = Marker {
        state: state.clone(),
        trigger,
        at: crate::share::core_now_secs(),
    };
    if let Err(error) = write_marker(&marker) {
        note_error(error);
    }
    note_host_summary(RdpFeedSummary::from_probe(&probe, state.clone()));
    state
}

/// Starts `ensure_rdp_host` in the background unless a setup already runs.
pub(crate) fn request(trigger: SetupTrigger) {
    if recorded_for_test(trigger) || in_progress() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rdp-self-setup".into())
        .spawn(move || {
            ensure_rdp_host(trigger);
        });
    if let Err(error) = spawned {
        note_error(format!("Einrichtungsthread startet nicht: {error}"));
    }
}

/// Test builds record background requests instead of starting a setup.
#[cfg(test)]
fn recorded_for_test(trigger: SetupTrigger) -> bool {
    if let Ok(mut requests) = TEST_REQUESTS.lock() {
        requests.push(trigger);
    }
    true
}

#[cfg(not(test))]
fn recorded_for_test(_: SetupTrigger) -> bool {
    false
}

/// Service start: probe once so the RDP port list is not empty after a
/// restart; T2 when a relation steers this screen.
pub(crate) fn on_service_start(steering: bool) {
    let spawned = std::thread::Builder::new()
        .name("rdp-self-setup-start".into())
        .spawn(move || {
            refresh_probe();
            if steering {
                ensure_rdp_host(SetupTrigger::VersionStart);
            }
        });
    if let Err(error) = spawned {
        note_error(format!("Startprobe startet nicht: {error}"));
    }
}

/// T3: a device became present; runs when it may steer this screen.
/// Called from the daemon's event drain (`ipc_host_events::peers_present`)
/// for DirectAvailable, RoomJoined and RoomRoster.
pub(crate) fn peer_present(device_id: &str, public_key: &str) {
    if crate::share::access_steering::steering_device(device_id, public_key) {
        request(SetupTrigger::PeerOnline);
    }
}

/// New se-managed credentials for `service_id` (design 2.8); se keeps no
/// copy. Only a service whose credentials se manages qualifies.
pub(crate) fn issue_managed_credential(service_id: &str) -> Result<(String, String), String> {
    let probe = refresh_probe();
    let service = probe
        .services
        .iter()
        .find(|service| service.id == service_id && service.managed_credentials)
        .ok_or_else(|| {
            format!("RDP-Dienst {service_id} hat keine von se verwalteten Zugangsdaten")
        })?;
    let device = host_device().ok_or("Geräteidentität ist noch nicht bekannt")?;
    platform::issue_credential(service, &device)
}

/// After a revocation of steering: new random credentials that nobody
/// receives, so a revoked device knows no valid password (threat T-R2).
pub(crate) fn rotate_managed_credentials() {
    if cfg!(test) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rdp-credential-rotate".into())
        .spawn(|| {
            if let Some(device) = host_device() {
                if let Err(error) = platform::rotate_credentials(&device) {
                    note_error(format!("Zugangsdaten nach Entzug nicht erneuert: {error}"));
                }
            }
        });
    if let Err(error) = spawned {
        note_error(format!(
            "Erneuerung der Zugangsdaten startet nicht: {error}"
        ));
    }
}

/// `se rdp host-setup --system <step>` (design 3.3): one fixed root step;
/// called by `cli::rdp`.
pub(crate) fn run_system_step(step: &str) -> Result<String, String> {
    platform::run_system_step(step)
}

#[cfg(test)]
#[path = "einrichtung_setup_task_tests.rs"]
mod einrichtung_setup_task_tests;
