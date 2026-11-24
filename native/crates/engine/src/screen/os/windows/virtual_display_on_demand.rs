//! The driver of the virtual display at use time (user rule of 2026-11-24:
//! se installs every dependency it uses at the latest when it is used, an
//! option se shows works end to end). A viewer that asks a host without a
//! working driver for se's display (`windows_driver::installs_on_use`)
//! starts the driver step here instead of getting an error: in this process
//! when it is privileged (an elevated administrator, the LocalSystem worker
//! of the login broker service), else in se's one elevated call
//! (`elevate::run_step`), which raises one UAC prompt on the host's desktop.
//! The decline of an update offer (`virtual-display-declined`) does not hold
//! it back, and a decline here is not written there: that marker only
//! governs the unsolicited offers of updates. One install runs at a time in
//! this process (`windows_driver::OnDemand`), further requests wait for it;
//! the installer, an update and this step also wait for each other in the
//! step itself (`install::run`). After the step the driver's device gets
//! its monitor; then the sessions that waited hear the end
//! (`virtual_display_host::setup_ended`): after success they switch to the
//! virtual display they asked for, after a decline or a failure they name
//! its cause, and a later request tries again.
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use super::super::{device, elevate, install, pins};
use crate::login::os::shared::self_install::Elevation;
use crate::screen::core::virtual_display::windows_driver::{InstallEnd, OnDemand};
use crate::screen::core::virtual_display_host::{self as host, SetupProgress};

static BOARD: Mutex<OnDemand> = Mutex::new(OnDemand::new());

/// How long the device may take after the step until its monitor shows
/// (assumption: IddCx starts the adapter within seconds; not measured).
const MONITOR_WAIT: Duration = Duration::from_secs(30);
const MONITOR_POLL: Duration = Duration::from_millis(250);

/// The notice on the host's desktop before the UAC prompt.
const PROMPT_NOTICE: &str =
    "Treiber für den virtuellen Bildschirm wird installiert (Administrator-Bestätigung folgt)";

fn board() -> MutexGuard<'static, OnDemand> {
    BOARD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether this host can install the driver now: the pins of this release
/// are valid and se reaches an elevation (it is privileged itself, or it
/// runs in a desktop session with `se.exe` beside it for the UAC prompt).
pub fn possible() -> bool {
    pins::pins().is_ok() && crate::login::os::platform::elevation().is_ok()
}

/// The reason of a `Setup` report while an install runs.
pub fn reason() -> Option<String> {
    board().reason().map(str::to_string)
}

fn failed(message: String) -> SetupProgress {
    SetupProgress::Failed(InstallEnd::Failed(message).reason().unwrap_or_default())
}

/// A viewer asked: starts the install unless one runs (that one is joined).
pub fn request() -> SetupProgress {
    if cfg!(any(test, feature = "test-support")) {
        return SetupProgress::Failed("test_build".into());
    }
    let prompt = match crate::login::os::platform::elevation() {
        Ok(Elevation::InProcess) => false,
        Ok(Elevation::External(_)) => true,
        Err(reason) => return failed(format!("keine Erhöhung: {reason}")),
    };
    if !board().request(prompt) {
        return SetupProgress::Running;
    }
    let spawned = std::thread::Builder::new()
        .name("se-display-driver-use".into())
        .spawn(run);
    if let Err(error) = spawned {
        board().finished();
        return failed(format!("Installationsthread: {error}"));
    }
    SetupProgress::Running
}

fn announce() {
    crate::notify_desktop::notify_access("Smart Explorer", PROMPT_NOTICE);
}

fn last_run() -> Option<PathBuf> {
    install::directory()
        .ok()
        .map(|dir| dir.join(elevate::LAST_RUN))
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

/// The device started and its monitor shows (or the wait ended).
fn wait_for_monitor() {
    let deadline = Instant::now() + MONITOR_WAIT;
    while !(device::working() && super::ours(&super::outputs()).is_some()) {
        if Instant::now() >= deadline {
            eprintln!("virtueller Bildschirm: kein Monitor des Treibers nach der Installation");
            return;
        }
        std::thread::sleep(MONITOR_POLL);
    }
}

/// The install thread: the step, its end, the waiting sessions.
fn run() {
    let note = last_run();
    let before = note.as_deref().and_then(modified);
    let code = elevate::run_step(announce);
    // Only a line the step wrote this time names its cause.
    let fresh = note
        .as_deref()
        .filter(|path| modified(path).is_some_and(|after| Some(after) > before))
        .and_then(|path| std::fs::read_to_string(path).ok());
    let mut end = InstallEnd::from_exit(code, fresh.as_deref());
    // An earlier step of the same elevated call may have failed first while
    // the driver step itself succeeded, or another step (the installer, an
    // update) installed it meanwhile: a working device decides.
    if end != InstallEnd::Done && device::working() {
        end = InstallEnd::Done;
    }
    if end == InstallEnd::Done {
        elevate::forget_decline();
        wait_for_monitor();
        // As after an update: the new monitor leaves the desktop where a
        // physical one is; the waiting sessions attach it in their size.
        super::release_idle();
    } else if let Some(reason) = end.reason() {
        eprintln!("virtueller Bildschirm: Installation bei Nutzung: {reason}");
    }
    board().finished();
    host::setup_ended(match end.reason() {
        None => SetupProgress::Done,
        Some(reason) => SetupProgress::Failed(reason),
    });
}
