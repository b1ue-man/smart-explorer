//! Worker side of the login self-setup (E21): „Bildschirm steuern:
//! Erlauben“ is the only step. The worker installs the login service with
//! one administrator approval per version (or directly as root), keeps the
//! broker's grants equal to the steering devices, sends the configured
//! server's relays and removes the service after the last revocation. The
//! RDP self-setup's triggers start it (`rdp/os/shared/self_setup.rs`).
use super::{installation, self_channel, self_install, self_watch};
use crate::login::{
    core::{
        gateway_types::GatewayTarget,
        self_grants::{one_per_device, SelfReply, SelfRequest, SteeringGrant, SELF_FORMAT},
        self_plan::{
            decide, removal_due, state_for_exit, Installed, LoginTrigger, Marker, Plan, SelfState,
            Step,
        },
    },
    os::platform,
};
use crate::rdp::host_setup::elevated::{self as bundle, Launched, Party, Rounds};
use self_install::Elevation;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex, RwLock,
};
use std::time::Duration;

pub use self_install::run_system_step;

/// What every surface shows (`rdp::host_setup::SetupStatus::lines`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SelfSetupStatus {
    /// A setup runs; an administrator prompt may be open.
    pub running: bool,
    pub state: Option<SelfState>,
    pub trigger: Option<LoginTrigger>,
    pub at: Option<i64>,
    pub last_error: Option<String>,
    /// Devices whose login grant the broker confirmed last.
    pub granted_devices: usize,
}

impl SelfSetupStatus {
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.running {
            lines.push(
                "Maschinenanmeldung wird eingerichtet (Administratorfreigabe): Smart Explorer \
                 fragt nach der Freigabe des Betriebssystems"
                    .into(),
            );
        }
        if let Some(state) = &self.state {
            lines.push(format!("Maschinenanmeldung: {}", state.text()));
            if *state == SelfState::Done {
                lines.push(format!(
                    "Maschinenanmeldung: {} Gerät(e) dürfen den Anmeldebildschirm steuern",
                    self.granted_devices
                ));
            }
        }
        if let Some(error) = &self.last_error {
            lines.push(format!("Maschinenanmeldung: {error}"));
        }
        lines
    }
}

/// `None` until the share runtime noted the steering devices once.
static STEERING: RwLock<Option<Vec<SteeringGrant>>> = RwLock::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// An elevated step runs now; an administrator prompt may be open.
static PROMPTING: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Option<LoginTrigger>> = Mutex::new(None);
static CURRENT: RwLock<Option<(SelfState, usize)>> = RwLock::new(None);
static LAST_ERROR: RwLock<Option<String>> = RwLock::new(None);
/// The latest trigger's announcement in the elevation bundle (E44): the RDP
/// step of the same trigger waits for this setup's step until it is handed
/// in or not needed. `ROUND` is the number the running round took.
static ANNOUNCED: Rounds = Rounds::new(Party::Login);
static ROUND: AtomicU64 = AtomicU64::new(0);
/// Test builds record requests instead of starting a setup.
#[cfg(any(test, feature = "test-support"))]
pub static TEST_REQUESTS: Mutex<Vec<LoginTrigger>> = Mutex::new(Vec::new());

/// The share runtime's steering devices with the end of each right. A
/// change of devices or ends syncs the broker (never a new prompt). A device
/// named twice (Direct grant and room) keeps its later end.
pub fn note_steering(grants: Vec<SteeringGrant>) {
    let grants = one_per_device(grants);
    let changed = match STEERING.write() {
        Ok(mut current) => {
            let changed = current.as_ref() != Some(&grants);
            *current = Some(grants);
            changed
        }
        Err(_) => false,
    };
    if changed {
        request(LoginTrigger::ListChanged);
    }
}

/// Starts a run in the background; a run already going takes the request
/// as its next round (a grant outranks every other trigger, and the watch
/// never replaces a waiting trigger: every round syncs anyway).
pub fn request(trigger: LoginTrigger) {
    if recorded_for_test(trigger) {
        return;
    }
    // The watch never prompts, so it never makes the RDP step wait.
    if trigger != LoginTrigger::Watch {
        ANNOUNCED.announce();
    }
    self_watch::start();
    if let Ok(mut pending) = PENDING.lock() {
        if RUNNING.swap(true, Ordering::AcqRel) {
            let keep = *pending == Some(LoginTrigger::Grant)
                || (trigger == LoginTrigger::Watch && pending.is_some());
            if !keep {
                *pending = Some(trigger);
            }
            return;
        }
    }
    let spawned = std::thread::Builder::new()
        .name("login-self-setup".into())
        .spawn(move || {
            let mut next = Some(trigger);
            while let Some(trigger) = next {
                let round = ANNOUNCED.round();
                ROUND.store(round, Ordering::Release);
                run(trigger);
                ANNOUNCED.withdraw(round);
                // Taking the next round and clearing `RUNNING` happen under
                // the same lock as `request`'s check, so no request is lost
                // and no second run starts beside this one.
                next = match PENDING.lock() {
                    Ok(mut pending) => {
                        let taken = pending.take();
                        if taken.is_none() {
                            RUNNING.store(false, Ordering::Release);
                        }
                        taken
                    }
                    Err(_) => {
                        RUNNING.store(false, Ordering::Release);
                        None
                    }
                };
            }
        });
    if let Err(error) = spawned {
        RUNNING.store(false, Ordering::Release);
        ANNOUNCED.withdraw(ANNOUNCED.round());
        note_error(format!("Einrichtung startet nicht: {error}"));
    }
}

#[cfg(any(test, feature = "test-support"))]
fn recorded_for_test(trigger: LoginTrigger) -> bool {
    if let Ok(mut requests) = TEST_REQUESTS.lock() {
        requests.push(trigger);
    }
    true
}

#[cfg(not(any(test, feature = "test-support")))]
fn recorded_for_test(_: LoginTrigger) -> bool {
    false
}

fn note_error(error: String) {
    eprintln!("Maschinenanmeldung: {error}");
    if let Ok(mut last) = LAST_ERROR.write() {
        *last = Some(error);
    }
}

fn marker_path() -> std::path::PathBuf {
    let name = format!("login-setup-{}.json", env!("CARGO_PKG_VERSION"));
    crate::support_dirs::sync_data_dir().join(name)
}

fn read_marker() -> Option<Marker> {
    serde_json::from_slice(&std::fs::read(marker_path()).ok()?).ok()
}

fn write_marker(state: &SelfState, trigger: LoginTrigger) {
    let marker = Marker {
        state: state.clone(),
        trigger,
        at: crate::share::core_now_secs(),
    };
    let path = marker_path();
    let staged = path.with_extension("json.tmp");
    let written = serde_json::to_vec(&marker)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&staged, bytes))
        .and_then(|()| std::fs::rename(&staged, &path));
    if let Err(error) = written {
        note_error(format!("Einrichtungsstand nicht speicherbar: {error}"));
    }
}

/// The noted devices that still steer now; a revocation the list has not
/// reached yet already removes the device (fail closed).
fn effective_steering() -> Option<Vec<SteeringGrant>> {
    let noted = STEERING.read().ok()?.clone()?;
    Some(
        noted
            .into_iter()
            .filter(|grant| {
                crate::share::access_steering::steering_device(&grant.peer_id, &grant.public_key)
            })
            .collect(),
    )
}

fn installed() -> Installed {
    let activity = super::host_status::activity();
    match (activity.state.as_str(), activity.evidence) {
        ("idle" | "active", Some(evidence)) => Installed::Running {
            release: evidence.release,
        },
        ("unavailable", _) => Installed::Unavailable,
        _ => Installed::Missing,
    }
}

/// One elevated step: in this process when it is privileged, otherwise
/// handed to the elevation bundle (E44), whose one pkexec or UAC call names
/// Smart Explorer's own `se` and also carries the RDP step of the trigger.
fn elevated(step: impl FnOnce(&std::path::Path) -> Option<Step>, removing: bool) -> SelfState {
    let route = match platform::elevation() {
        Ok(route) => route,
        Err(reason) => return SelfState::Failed(reason),
    };
    let beside = match &route {
        Elevation::InProcess => std::env::current_exe().ok(),
        Elevation::External(se) => Some(se.clone()),
    };
    let Some(step) = beside.as_deref().and_then(step) else {
        return SelfState::Failed("image_missing".into());
    };
    PROMPTING.store(true, Ordering::Release);
    let state = match route {
        Elevation::InProcess => {
            state_for_exit(self_install::run_system_step(&step.name()).1, removing)
        }
        Elevation::External(se) => {
            let launcher: bundle::Launcher = Box::new(move |steps: &[String]| {
                crate::rdp::host_setup::prompt_follows();
                platform::run_elevated(&se, steps).map(|code| Launched {
                    code: Some(code),
                    ..Launched::default()
                })
            });
            let handed_in = bundle::submit(Party::Login, step.name(), launcher);
            ANNOUNCED.withdraw(ROUND.load(Ordering::Acquire));
            match handed_in {
                (call, Ok(launched)) => bundled_state(&call, &launched, removing),
                (_, Err(reason)) => SelfState::Failed(reason),
            }
        }
    };
    PROMPTING.store(false, Ordering::Release);
    state
}

/// The login step's result in the bundled call: its `login_state=` line
/// (pkexec), otherwise its exit code (UAC carries no output; the login
/// step runs first, so the shared code always tells its own).
fn bundled_state(call: &bundle::ElevatedCall, launched: &Launched, removing: bool) -> SelfState {
    let reported = launched
        .stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("login_state="))
        .and_then(|state| serde_json::from_str::<SelfState>(state).ok());
    if let Some(state) = reported {
        return state;
    }
    match bundle::step_code(call, Party::Login, launched.code) {
        Some(code) => state_for_exit(code, removing),
        None => SelfState::Failed("bundled_step_unreported".into()),
    }
}

/// The step that binds the approval to the app image beside `beside`.
fn install_step(beside: &std::path::Path) -> Option<Step> {
    let image = self_install::app_image(beside);
    let file = platform::open_image(&image).ok()?;
    let sha256 = installation::sha256_hex(&file).ok()?;
    Some(Step::Install { sha256 })
}

fn call_retrying(request: &SelfRequest, attempts: u32) -> std::io::Result<SelfReply> {
    let mut last = None;
    for attempt in 0..attempts.max(1) {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500));
        }
        match self_channel::call(request) {
            Ok(reply) => return Ok(reply),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(error)
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("login grant channel unavailable")))
}

/// Grants, then the configured server's relays (step 2 (d), (e)), then the
/// removal after the last revocation (step 2 (f)).
fn sync(steering: &[SteeringGrant], trigger: LoginTrigger, attempts: u32) -> (SelfState, usize) {
    // A right revoked while this round waited (elevation bundle, open prompt)
    // never reaches the broker with this round's older list (S1, fail closed).
    let steering: Vec<SteeringGrant> = steering
        .iter()
        .filter(|grant| {
            crate::share::access_steering::steering_device(&grant.peer_id, &grant.public_key)
        })
        .cloned()
        .collect();
    let request = SelfRequest::Grants {
        format: SELF_FORMAT,
        grants: steering.clone(),
    };
    let reply = match call_retrying(&request, attempts) {
        Ok(reply) => reply,
        Err(error) => {
            let refused = error.kind() == std::io::ErrorKind::PermissionDenied;
            // An unreachable broker is asked again by the watch (a
            // revocation must reach it); a refusal is not repeated.
            self_watch::note_unreachable(!refused);
            let reason = if refused {
                "grant_refused"
            } else {
                "service_unreachable"
            };
            note_error(format!("Abgleich der Freigaben: {error}"));
            return (SelfState::Failed(reason.into()), 0);
        }
    };
    let mut unreachable = false;
    if !steering.is_empty() {
        match crate::daemon::load_share_server() {
            Ok(server) => {
                let network = crate::share::login_transport::configured_server_network(&server);
                if let Err(error) = self_channel::call(&SelfRequest::Network { network }) {
                    unreachable = error.kind() != std::io::ErrorKind::PermissionDenied;
                    note_error(format!(
                        "Relays des Share-Servers nicht übernommen: {error}"
                    ));
                }
            }
            Err(error) => note_error(format!("Share-Server nicht lesbar: {error}")),
        }
    }
    self_watch::note_unreachable(unreachable);
    let own = reply.own_grants.len();
    // The watch never prompts: the removal waits for the next trigger; the
    // service without a grant admits no device meanwhile.
    if trigger != LoginTrigger::Watch
        && removal_due(
            !steering.is_empty(),
            own,
            reply.other_grants,
            read_marker().as_ref(),
        )
    {
        let state = elevated(|_| Some(Step::Remove), true);
        write_marker(&state, trigger);
        return (state, 0);
    }
    let state = if steering.is_empty() {
        SelfState::NotNeeded
    } else {
        SelfState::Done
    };
    (state, own)
}

/// An older service that still runs gets every grant change all the same
/// (a revocation must reach it); the status keeps `kept`, this version's
/// setup result, unless the sync removed the service.
fn keep_synced(
    kept: Option<SelfState>,
    steering: &[SteeringGrant],
    trigger: LoginTrigger,
) -> (SelfState, usize) {
    match (sync(steering, trigger, 1), kept) {
        ((state @ (SelfState::Removed | SelfState::RemoveDeclined), own), _) => (state, own),
        ((_, own), Some(kept)) => (kept, own),
        (synced, None) => synced,
    }
}

fn run(trigger: LoginTrigger) {
    // Without the share runtime's list nothing is known: no change at all.
    let Some(steering) = effective_steering() else {
        return;
    };
    let installed = installed();
    // A platform without a privileged login host (Android) shows nothing.
    if installed == Installed::Unavailable {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    let marker = read_marker();
    let plan = decide(
        trigger,
        !steering.is_empty(),
        &installed,
        marker.as_ref(),
        version,
    );
    // Without an install or a possible removal no step follows: the RDP step
    // of this trigger need not wait for this round.
    if plan != Plan::Install && !steering.is_empty() {
        ANNOUNCED.withdraw(ROUND.load(Ordering::Acquire));
    }
    // The watch only syncs a running service; it never prompts and never
    // replaces the shown result.
    if trigger == LoginTrigger::Watch && !matches!(plan, Plan::Sync | Plan::SyncKeep(_)) {
        return;
    }
    let (state, own) = match plan {
        Plan::Nothing => (SelfState::NotNeeded, 0),
        // A setup of this version that no longer runs is not shown as done.
        Plan::Keep(SelfState::Done) if installed == Installed::Missing => {
            (SelfState::Failed("service_unreachable".into()), 0)
        }
        Plan::Keep(state) => (state, 0),
        Plan::Sync => sync(&steering, trigger, 1),
        Plan::SyncKeep(kept) => keep_synced(kept, &steering, trigger),
        Plan::Install => {
            let state = elevated(install_step, false);
            write_marker(&state, trigger);
            if state == SelfState::Done {
                // The new services need a moment before their channel answers.
                sync(&steering, trigger, 20)
            } else if matches!(installed, Installed::Running { .. }) {
                // Declined or failed update: the older service keeps running.
                keep_synced(Some(state), &steering, trigger)
            } else {
                (state, 0)
            }
        }
    };
    if let Ok(mut current) = CURRENT.write() {
        *current = Some((state, own));
    }
}

/// The status for every surface; `None` while nothing happened yet.
pub fn status() -> Option<SelfSetupStatus> {
    let marker = read_marker();
    let current = CURRENT.read().ok().and_then(|current| current.clone());
    let last_error = LAST_ERROR.read().ok().and_then(|last| last.clone());
    let running = PROMPTING.load(Ordering::Acquire);
    if !running && marker.is_none() && current.is_none() && last_error.is_none() {
        return None;
    }
    let granted_devices = current.as_ref().map(|(_, own)| *own).unwrap_or(0);
    Some(SelfSetupStatus {
        running,
        state: current
            .map(|(state, _)| state)
            .or_else(|| marker.as_ref().map(|marker| marker.state.clone())),
        trigger: marker.as_ref().map(|marker| marker.trigger),
        at: marker.as_ref().map(|marker| marker.at),
        last_error,
        granted_devices,
    })
}

/// The broker's view for the viewer side (p-login-viewer): the account a
/// viewer names as `target_user` and the published machine target.
pub fn broker_view() -> std::io::Result<SelfReply> {
    #[cfg(any(test, feature = "test-support"))]
    {
        if let Some(reply) = broker_stand_in::reply() {
            return Ok(reply);
        }
    }
    self_channel::call(&SelfRequest::Status)
}

/// The published machine target of the installed gateway, if it runs.
pub fn installed_target() -> std::io::Result<Option<GatewayTarget>> {
    broker_view().map(|reply| reply.target)
}

#[cfg(any(test, feature = "test-support"))]
#[path = "self_broker_stand_in.rs"]
pub mod broker_stand_in;
