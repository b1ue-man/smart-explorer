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

/// Status line while an administrator prompt may be open (threat T-R6);
/// `SetupStatus::lines` shows it on every surface.
pub(crate) const STATUS_TEXT: &str = "Remote Desktop wird eingerichtet (Administratorfreigabe)";

/// The host's RDP self-setup as the worker snapshot carries it
/// (`ShareWorkerSnapshot.rdp_setup`) for „Dieses Gerät“, `se share status`
/// and the Android status (design 3.4, threat T-R6).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SetupStatus {
    /// A setup runs now; an administrator prompt (UAC/polkit) may be open.
    pub(crate) running: bool,
    /// The marker of this version: the last result.
    pub(crate) state: Option<SetupState>,
    pub(crate) trigger: Option<SetupTrigger>,
    pub(crate) at: Option<i64>,
    /// A failure the marker could not carry.
    pub(crate) last_error: Option<String>,
}
impl SetupStatus {
    /// The German lines every surface shows.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.running {
            lines.push(format!(
                "{STATUS_TEXT}: Smart Explorer fragt nach der Freigabe des Betriebssystems"
            ));
        }
        match &self.state {
            Some(state) => lines.push(format!("Remote-Desktop-Einrichtung: {}", state.text())),
            None if !self.running => {
                lines.push("Remote-Desktop-Einrichtung: noch nicht gelaufen".into())
            }
            None => {}
        }
        if let Some(error) = &self.last_error {
            lines.push(format!("Remote-Desktop-Einrichtung: {error}"));
        }
        lines
    }
}

/// The current setup status of this host.
pub(crate) fn status() -> SetupStatus {
    let marker = read_marker();
    let stored = marker.as_ref().map(|marker| marker.state.clone());
    SetupStatus {
        running: in_progress(),
        state: platform::impossible().or(stored),
        trigger: marker.as_ref().map(|marker| marker.trigger),
        at: marker.as_ref().map(|marker| marker.at),
        last_error: last_error(),
    }
}

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
    /// The device whose „Remote Desktop“ (T4) led to this result.
    #[serde(default)]
    pub(crate) device: Option<String>,
    /// The session facts at this result (`platform::session_facts`): a
    /// missing interactive session is retried only once they changed.
    #[serde(default)]
    pub(crate) facts: Option<String>,
}

/// Who asks for a setup and in which session (the inputs of `decide`
/// beside the trigger and the marker).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Asking {
    /// The requesting device of T4.
    pub(crate) device: Option<String>,
    /// `platform::session_facts()` now.
    pub(crate) facts: String,
}

/// What a trigger does with the current marker (design 3.1, 3.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Run,
    Keep(SetupState),
}

/// T1 is the host person's action and always runs. T4 (a viewer's
/// „Remote Desktop“) runs too, also after `declined` — except when that
/// same device's own request was the one declined: then only a new grant
/// (T1) or a new version asks again, so one device cannot repeat the
/// administrator prompt (review S3, threat T-R6). T2 runs once per version.
/// T3 runs without a marker or after the failures a present device can
/// resolve: a locked keyring, and a missing interactive session once the
/// session facts changed (a process whose session cannot change is not
/// retried on every presence event, review R1).
pub(crate) fn decide(trigger: SetupTrigger, marker: Option<&Marker>, asking: &Asking) -> Decision {
    let Some(marker) = marker else {
        return Decision::Run;
    };
    let declined_by_same_device = marker.state == SetupState::Declined
        && marker.trigger == SetupTrigger::PeerRequest
        && marker.device.is_some()
        && marker.device == asking.device;
    match (trigger, &marker.state) {
        (SetupTrigger::Grant, _) => Decision::Run,
        (SetupTrigger::PeerRequest, _) if !declined_by_same_device => Decision::Run,
        (SetupTrigger::PeerOnline, SetupState::Failed(reason)) if reason == "keyring_locked" => {
            Decision::Run
        }
        (SetupTrigger::PeerOnline, SetupState::Failed(reason))
            if reason == "no_interactive_session"
                && marker.facts.as_deref() != Some(asking.facts.as_str()) =>
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

/// Threat T-R6: the platform adapter calls this right before it raises the
/// OS prompt (UAC, polkit), so the prompt is labelled as Smart Explorer's
/// by a system notice next to the status line on „Dieses Gerät“. Callers:
/// `windows::host_setup::ensure`, `linux_os::host_setup_pkexec::via_pkexec`.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub(crate) fn prompt_follows() {
    crate::notify_desktop::notify_access("Smart Explorer", STATUS_TEXT);
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
/// `device` names the requesting device of T4. A request that waited for a
/// setup running meanwhile takes that setup's result instead of starting
/// one more (no repeated prompt for requests queued behind an open one).
pub(crate) fn ensure_rdp_host(trigger: SetupTrigger, device: Option<String>) -> SetupState {
    let asked_at = crate::share::core_now_secs();
    let _turn = RUN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let asking = Asking {
        device,
        facts: platform::session_facts(),
    };
    let marker = read_marker();
    if let Some(marker) = &marker {
        if trigger != SetupTrigger::Grant && marker.at > asked_at {
            return marker.state.clone();
        }
    }
    if let Decision::Keep(state) = decide(trigger, marker.as_ref(), &asking) {
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
        device: asking.device,
        facts: Some(asking.facts),
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
            ensure_rdp_host(trigger, None);
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
                ensure_rdp_host(SetupTrigger::VersionStart, None);
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

/// The se-managed credential handed out last, in memory only (never on
/// disk), so concurrent sessions to the same service keep one valid
/// password (review B8). Cleared on every rotation.
type Issued = (String, String, zeroize::Zeroizing<String>);
static ISSUED: Mutex<Option<Issued>> = Mutex::new(None);

/// Se-managed credentials for `service_id` (design 2.8). While a session to
/// that service's port runs (`in_use`, the host's target tunnels), the same
/// credential is handed out again, so a second viewer's start does not
/// break the first one's reconnects; otherwise new ones are set. Only a
/// service whose credentials se manages qualifies.
pub(crate) fn issue_managed_credential(
    service_id: &str,
    in_use: impl Fn(u16) -> bool,
) -> Result<(String, String), String> {
    let probe = refresh_probe();
    let service = probe
        .services
        .iter()
        .find(|service| service.id == service_id && service.managed_credentials)
        .ok_or_else(|| {
            format!("RDP-Dienst {service_id} hat keine von se verwalteten Zugangsdaten")
        })?;
    let device = host_device().ok_or("Geräteidentität ist noch nicht bekannt")?;
    let mut issued = ISSUED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((id, username, password)) = issued.as_ref() {
        if id == service_id && service.port.is_some_and(&in_use) {
            return Ok((username.clone(), password.to_string()));
        }
    }
    let (username, password) = platform::issue_credential(service, &device)?;
    *issued = Some((
        service_id.to_string(),
        username.clone(),
        zeroize::Zeroizing::new(password.clone()),
    ));
    Ok((username, password))
}

fn forget_issued() {
    *ISSUED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// After a revocation of steering: new random credentials that nobody
/// receives, so a revoked device knows no valid password (threat T-R2).
pub(crate) fn rotate_managed_credentials() {
    coverage_ended(true);
}

/// A device's steering coverage ended (revocation, configuration change or
/// the grant's own expiry instant): the se-managed credentials are rotated
/// at once. When no device steers this screen any more, the network-wide
/// listener of an se-managed GNOME desktop sharing is switched off as well
/// (GNOME offers no loopback binding; `grdctl` knows only `set-port`); the
/// next grant (T1) or a viewer's „Remote Desktop“ (T4) switches it on again.
pub(crate) fn coverage_ended(steering_remains: bool) {
    if cfg!(test) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rdp-credential-rotate".into())
        .spawn(move || {
            // Not behind `RUN`: an open administrator prompt of a setup must
            // not delay the rotation.
            forget_issued();
            if let Some(device) = host_device() {
                if let Err(error) = platform::rotate_credentials(&device) {
                    note_error(format!("Zugangsdaten nach Entzug nicht erneuert: {error}"));
                }
            }
            if !steering_remains {
                match platform::withdraw_managed() {
                    Ok(true) => {
                        refresh_probe();
                    }
                    Ok(false) => {}
                    Err(error) => note_error(format!("RDP bleibt eingeschaltet: {error}")),
                }
            }
        });
    if let Err(error) = spawned {
        note_error(format!(
            "Erneuerung der Zugangsdaten startet nicht: {error}"
        ));
    }
}

/// `se rdp host-setup --system <step> [--user-sid <sid>]` (design 3.2,
/// 3.3): one fixed elevated step; the line to print and the exit code.
/// Linux runs its root steps through `pkexec`; Windows runs its setup
/// script in this elevated se, so the UAC prompt names Smart Explorer's se
/// instead of Windows PowerShell (threat T-R6). Called by `cli::rdp`.
pub(crate) fn run_system_step(step: &str, user_sid: Option<&str>) -> (String, i32) {
    platform::run_system_step(step, user_sid)
}

#[cfg(test)]
#[path = "einrichtung_setup_task_tests.rs"]
mod einrichtung_setup_task_tests;
