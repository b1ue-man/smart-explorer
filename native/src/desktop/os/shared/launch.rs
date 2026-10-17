//! The GUI program se Desktop runs on and the shell process as seen from
//! outside it (DSK-M3, C5.6, C5.7). `se` finds the app beside itself through
//! the updater's installation detection; inside the GUI the running program
//! is it. The shell always starts detached from its caller; a stop waits for
//! the shell's own end and only then ends what remains by PID, after the
//! image check of the platform adapter.
use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::control::{Control, Integration, ShellProcesses, StopReport};
use super::platform::{self, PlatformIntegration};
use super::store::DesktopDir;
use super::Session;

pub(crate) const NO_APP: &str = "keine Smart-Explorer-App neben diesem se installiert";
pub(crate) const SHELL_SWITCH: &str = "--desktop-shell";
/// The shell reads `state.json` and `shell.stop` once per second and gives
/// its surfaces five seconds (C5.5); one more second covers its own end.
const STOP_WAIT: Duration = Duration::from_secs(7);
/// After the forced end the instance must be free again.
const KILL_WAIT: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(100);

/// The running GUI program, or the app installed beside `se`.
pub(crate) fn gui_binary() -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        if platform::is_gui_image(&exe) {
            return Ok(exe);
        }
    }
    match crate::updater::detect_installation()? {
        crate::updater::Installation::Desktop { app, .. } => Ok(app),
        crate::updater::Installation::TerminalOnly { .. } => Err(NO_APP.into()),
    }
}

/// Waits for a detached child in the background, so a long-running caller
/// (the GUI) keeps no zombie.
pub(crate) fn reap(mut child: Child) {
    let _ = std::thread::Builder::new()
        .name("se-desktop-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

/// Starts `"<GUI>" --desktop-shell` detached from the caller.
pub(crate) fn start_shell(gui: &Path) -> Result<(), String> {
    let child = platform::process::spawn_detached(gui, &[SHELL_SWITCH])
        .map_err(|error| format!("{}: {error}", gui.display()))?;
    reap(child);
    Ok(())
}

/// How a shell is started (tests record instead).
pub(crate) type Starter = fn(&Path) -> Result<(), String>;

/// The module is enabled with a part on, no halt holds for this version
/// (Ä30) and no shell runs. Reads only; never creates the folder.
pub(crate) fn resume_wanted(dir: &DesktopDir) -> bool {
    matches!(dir.read_state(), Ok(Some(state)) if state.should_resume(env!("CARGO_PKG_VERSION")))
        && !platform::instance::is_running(dir)
}

/// C5.6: after the updater steps of every GUI start, without blocking it,
/// a thread starts the shell when [`resume_wanted`]. The thread's result
/// tells whether a shell was started.
pub(crate) fn resume_with(
    gui: PathBuf,
    dir: Option<DesktopDir>,
    start: Starter,
) -> Option<JoinHandle<bool>> {
    std::thread::Builder::new()
        .name("se-desktop-resume".into())
        .spawn(move || {
            let dir = dir.unwrap_or_else(DesktopDir::production);
            resume_wanted(&dir) && start(&gui).is_ok()
        })
        .ok()
}

/// The shell process of this user.
pub(crate) struct SystemProcesses;

impl ShellProcesses for SystemProcesses {
    fn running(&self, dir: &DesktopDir) -> bool {
        platform::instance::is_running(dir)
    }

    fn start(&self, gui: &Path) -> Result<(), String> {
        start_shell(gui)
    }

    fn await_stop(&self, dir: &DesktopDir, gui: Option<&Path>) -> StopReport {
        if wait_until_free(dir, STOP_WAIT) {
            return StopReport {
                stopped: true,
                ..StopReport::default()
            };
        }
        let Some(gui) = gui else {
            return StopReport {
                error: Some(format!(
                    "se Desktop reagiert nicht und lässt sich ohne Smart-Explorer-App nicht \
                     sicher beenden ({NO_APP})"
                )),
                ..StopReport::default()
            };
        };
        let mut report = StopReport::default();
        if let Some(runtime) = dir.read_shell() {
            let pids = std::iter::once(runtime.shell_pid)
                .chain(runtime.surfaces.iter().map(|surface| surface.pid));
            for pid in pids {
                match platform::process::terminate_checked(pid, gui) {
                    Ok(true) => report.killed.push(pid),
                    Ok(false) => {}
                    Err(error) => report.error = Some(format!("Prozess {pid}: {error}")),
                }
            }
        }
        report.stopped = wait_until_free(dir, KILL_WAIT);
        if report.stopped {
            dir.clear_shell();
        } else if report.error.is_none() {
            report.error = Some("se Desktop hat sich nicht beenden lassen".into());
        }
        report
    }
}

fn wait_until_free(dir: &DesktopDir, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    while platform::instance::is_running(dir) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}

/// One control call with the production pieces: the folder, the platform's
/// registrations, the shell processes and the session's parts.
pub(crate) fn with_control<T>(run: impl FnOnce(&mut Control<'_>) -> T) -> T {
    let dir = DesktopDir::production();
    let gui = gui_binary();
    let config = dir.read_config().ok().flatten();
    let session = Session::probe(config.as_ref());
    let mut integration = PlatformIntegration::new(gui.as_ref().ok().cloned());
    let processes = SystemProcesses;
    let mut control = Control {
        dir,
        integration: &mut integration as &mut dyn Integration,
        processes: &processes,
        gui,
        available: session.available,
        unavailable: session.unavailable,
        system_items: platform::initial_system_items(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        now: super::store::unix_now(),
    };
    run(&mut control)
}
