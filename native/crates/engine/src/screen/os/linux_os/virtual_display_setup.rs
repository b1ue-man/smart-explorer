//! Driver setup of the virtual display with one elevation (plan 2026-11-17
//! P5 stage 2, decision E4). A host whose X server takes no display modes
//! (VESA) or whose driver limit is below the viewer's size, and that has no
//! physical monitor, gets the Xorg driver `dummy`: the elevated step
//! `virtual-display-install` installs its package, writes
//! `/etc/X11/smart-explorer-virtual.conf` and the drop-in of the display
//! manager (`virtual_display_config.rs`) and restarts the display manager
//! only when logind shows no user session; otherwise it takes effect after
//! sign-out or restart. The display stays `:0`, so the greeter and the
//! capture of a root worker keep their way. `virtual-display-remove` removes
//! se's files again after the last „Bildschirm steuern“ right (as the login
//! self-setup does); the package stays. The worker hands the step to the one
//! elevation of a trigger (`rdp/os/shared/self_setup_elevated.rs`, party
//! `Display`) or runs it itself as root.
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::login::os::shared::self_install::Elevation;
use crate::rdp::host_setup::elevated::{self as bundle, Launched, Party};
use crate::screen::core::virtual_display_config::{
    dummy_package, os_release_ids, restart_allowed, written_by_se, xorg_conf, DisplayManager,
    DisplayManagerKind, GDM_XORG_PATH, LIGHTDM_DROPIN_PATH, SDDM_DROPIN_PATH, XORG_CONF_PATH,
};
use crate::screen::core::virtual_display_host::SetupProgress;

pub const INSTALL_STEP: &str = "virtual-display-install";
pub const REMOVE_STEP: &str = "virtual-display-remove";

/// Exit codes of the display steps (`self_setup_elevated::display_code`).
pub mod exit {
    /// Done; takes effect after sign-out or restart.
    pub const AFTER_RESTART: i32 = 50;
    pub const PACKAGE: i32 = 51;
    pub const DISTRIBUTION: i32 = 52;
    pub const WRITE: i32 = 53;
    pub const DISPLAY_MANAGER: i32 = 54;
    pub const STEP: i32 = 55;
}

const DISPLAY_MANAGER_UNIT: &str = "/etc/systemd/system/display-manager.service";
const OS_RELEASE: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];

/// The state of the setup in this process (one attempt per process and
/// version; the marker keeps it across restarts of the worker).
static STATE: Mutex<Option<SetupProgress>> = Mutex::new(None);

#[derive(Serialize, Deserialize)]
struct Marker {
    state: String,
    #[serde(default)]
    reason: Option<String>,
}

fn marker_path() -> std::path::PathBuf {
    let name = format!("virtual-display-setup-{}.json", env!("CARGO_PKG_VERSION"));
    crate::support_dirs::sync_data_dir().join(name)
}

fn read_marker() -> Option<SetupProgress> {
    let marker: Marker = serde_json::from_slice(&fs::read(marker_path()).ok()?).ok()?;
    Some(match marker.state.as_str() {
        "done" => SetupProgress::Done,
        "after_restart" => SetupProgress::AfterRestart,
        _ => SetupProgress::Failed(marker.reason.unwrap_or_else(|| "failed".into())),
    })
}

fn write_marker(progress: &SetupProgress) {
    let (state, reason) = match progress {
        SetupProgress::Running => return,
        SetupProgress::Done => ("done", None),
        SetupProgress::AfterRestart => ("after_restart", None),
        SetupProgress::Failed(reason) => ("failed", Some(reason.clone())),
    };
    let marker = Marker {
        state: state.into(),
        reason,
    };
    if let Ok(bytes) = serde_json::to_vec(&marker) {
        let _ = fs::write(marker_path(), bytes);
    }
}

/// The state an exit code of a display step means.
pub fn progress_for_exit(code: i32) -> SetupProgress {
    match code {
        0 => SetupProgress::Done,
        exit::AFTER_RESTART => SetupProgress::AfterRestart,
        exit::PACKAGE => SetupProgress::Failed("package_install_failed".into()),
        exit::DISTRIBUTION => SetupProgress::Failed("distribution_unknown".into()),
        exit::WRITE => SetupProgress::Failed("write_failed".into()),
        exit::DISPLAY_MANAGER => SetupProgress::Failed("display_manager_unknown".into()),
        126 => SetupProgress::Failed("declined".into()),
        127 => SetupProgress::Failed("no_prompt".into()),
        other => SetupProgress::Failed(format!("exit_{other}")),
    }
}

/// Runs `step` with one elevation: in this process as root, else handed to
/// the elevation bundle under pkexec. The step's exit code.
fn elevated(step: &str) -> Result<i32, String> {
    match crate::login::os::platform::elevation()? {
        Elevation::InProcess => Ok(run_system_step(step).1),
        Elevation::External(se) => {
            let launcher: bundle::Launcher = Box::new(move |steps: &[String]| {
                crate::rdp::host_setup::prompt_follows();
                crate::login::os::platform::run_elevated(&se, steps).map(|code| Launched {
                    code: Some(code),
                    ..Launched::default()
                })
            });
            let (call, result) = bundle::submit(Party::Display, step.to_string(), launcher);
            let launched = result?;
            bundle::step_code(&call, Party::Display, launched.code)
                .ok_or_else(|| "bundled_step_unreported".to_string())
        }
    }
}

/// Starts the setup in the background unless this process or version tried
/// already; its state now.
pub fn request() -> SetupProgress {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(known) = state.as_ref() {
        return known.clone();
    }
    if let Some(stored) = read_marker() {
        *state = Some(stored.clone());
        return stored;
    }
    if cfg!(any(test, feature = "test-support")) {
        return SetupProgress::Failed("test_build".into());
    }
    *state = Some(SetupProgress::Running);
    let spawned = std::thread::Builder::new()
        .name("virtual-display-setup".into())
        .spawn(|| {
            let progress = match elevated(INSTALL_STEP) {
                Ok(code) => progress_for_exit(code),
                Err(reason) => SetupProgress::Failed(reason),
            };
            write_marker(&progress);
            *STATE.lock().unwrap_or_else(PoisonError::into_inner) = Some(progress);
        });
    if let Err(error) = spawned {
        let failed = SetupProgress::Failed(format!("thread: {error}"));
        *state = Some(failed.clone());
        return failed;
    }
    SetupProgress::Running
}

/// A device's „Bildschirm steuern“ right ended: after the last one se's
/// files go (in the background, one elevation); a later setup may run again.
pub fn coverage_ended(steering_remains: bool) {
    if steering_remains || cfg!(any(test, feature = "test-support")) {
        return;
    }
    if !Path::new(XORG_CONF_PATH).exists() {
        return;
    }
    *STATE.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let _ = fs::remove_file(marker_path());
    let spawned = std::thread::Builder::new()
        .name("virtual-display-remove".into())
        .spawn(|| {
            if let Err(reason) = elevated(REMOVE_STEP) {
                eprintln!("virtual display: removal: {reason}");
            }
        });
    if let Err(error) = spawned {
        eprintln!("virtual display: removal thread: {error}");
    }
}

/// `se rdp host-setup --system virtual-display-…` (root): the line and the
/// exit code of one step.
pub fn run_system_step(step: &str) -> (String, i32) {
    let result = match step {
        INSTALL_STEP => install(),
        REMOVE_STEP => remove(),
        _ => Err(("unknown_step", exit::STEP)),
    };
    match result {
        Ok(0) => ("display_state=done".into(), 0),
        Ok(code) => ("display_state=after_restart".into(), code),
        Err((reason, code)) => (format!("display_state=failed:{reason}"), code),
    }
}

type StepResult = Result<i32, (&'static str, i32)>;

fn display_manager() -> DisplayManager {
    fs::read_link(DISPLAY_MANAGER_UNIT)
        .ok()
        .and_then(|target| target.file_name()?.to_str().map(DisplayManager::from_unit))
        .unwrap_or_else(|| DisplayManager::from_unit("none"))
}

/// The package of the driver `dummy` for this distribution.
fn package() -> Option<&'static str> {
    let text = OS_RELEASE
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let ids = os_release_ids(&text);
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    dummy_package(&ids)
}

/// Whether stage 2 can run on this host: the driver is installed or its
/// package is known, and the display manager is one se can point at the
/// configuration (LightDM, SDDM, GDM).
pub fn possible() -> bool {
    (super::probe::dummy_driver_installed() || package().is_some())
        && display_manager().dropin().is_some()
}

/// The driver's package through the shared package step
/// (`login/os/linux_os/package_install.rs`, which also refreshes stale apt
/// lists once).
fn install_package() -> Result<(), (&'static str, i32)> {
    use crate::login::os::platform::package_install::{install_as_root, RootError};
    if super::probe::dummy_driver_installed() {
        return Ok(());
    }
    let package = package().ok_or(("distribution_unknown", exit::DISTRIBUTION))?;
    match install_as_root(&[package]) {
        Ok(()) => Ok(()),
        Err(RootError::NoManager) => Err(("distribution_unknown", exit::DISTRIBUTION)),
        Err(RootError::Package(message)) => {
            eprintln!("virtual display: package {package}: {message}");
            Err(("package_install_failed", exit::PACKAGE))
        }
    }
}

/// Writes `text` to `path` (mode 0644) unless a file there is not se's.
fn write_own(path: &str, text: &str) -> io::Result<()> {
    let path = Path::new(path);
    if let Ok(existing) = fs::read_to_string(path) {
        if !written_by_se(&existing) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a file of another origin is in the way",
            ));
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged = path.with_extension("se-tmp");
    let _ = fs::remove_file(&staged);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&staged)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&staged, path)
}

/// Removes `path` when se wrote it.
fn remove_own(path: &str) {
    if fs::read_to_string(path).is_ok_and(|text| written_by_se(&text)) {
        let _ = fs::remove_file(path);
    }
}

/// The classes of logind's sessions (`user`, `greeter`, …).
fn session_classes() -> Vec<String> {
    let listed = Command::new("loginctl")
        .args(["list-sessions", "--no-legend"])
        .stdin(Stdio::null())
        .output();
    let Ok(listed) = listed else {
        // Unknown: keep the display manager running.
        return vec!["user".into()];
    };
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .map(|id| {
            Command::new("loginctl")
                .args(["show-session", id.as_str(), "-p", "Class", "--value"])
                .stdin(Stdio::null())
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .unwrap_or_else(|_| "user".into())
        })
        .collect()
}

/// Restarts the display manager when no user session runs: 0, else
/// `AFTER_RESTART`.
fn restart_if_idle(manager: &DisplayManager) -> i32 {
    if !restart_allowed(&session_classes()) {
        return exit::AFTER_RESTART;
    }
    let restarted = Command::new("systemctl")
        .args(["restart", manager.unit.as_str()])
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if restarted {
        0
    } else {
        exit::AFTER_RESTART
    }
}

fn install() -> StepResult {
    let manager = display_manager();
    let (dropin_path, dropin) = manager
        .dropin()
        .ok_or(("display_manager_unknown", exit::DISPLAY_MANAGER))?;
    install_package()?;
    write_own(XORG_CONF_PATH, &xorg_conf()).map_err(|_| ("write_failed", exit::WRITE))?;
    write_own(dropin_path, &dropin).map_err(|_| ("write_failed", exit::WRITE))?;
    Ok(restart_if_idle(&manager))
}

fn remove() -> StepResult {
    for path in [
        XORG_CONF_PATH,
        LIGHTDM_DROPIN_PATH,
        SDDM_DROPIN_PATH,
        GDM_XORG_PATH,
    ] {
        remove_own(path);
    }
    let manager = display_manager();
    if manager.kind == DisplayManagerKind::Other {
        return Ok(0);
    }
    Ok(restart_if_idle(&manager))
}
