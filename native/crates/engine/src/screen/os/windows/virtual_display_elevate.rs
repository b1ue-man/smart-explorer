//! Entry of the driver step (plan 2026-11-17 P5) in the updater program
//! (`Smart Explorer Updater.exe`, which both the installer and the
//! auto-update run):
//! - `--display-driver install`: the installer's section; asks for the
//!   elevation (one UAC confirmation) when `driver_step` has something to do;
//! - `--display-driver after-update`: after a completed update; the same,
//!   but an offer the user declined once is not repeated unless se installed
//!   the driver before (`update_offers_step`);
//! - `--display-driver-elevated`: the elevated process itself; it decides
//!   again on its own, downloads by itself and trusts nothing its caller
//!   wrote.
//!
//! An unattended update must not wait for a confirmation only a person at
//! the device can give (plan 2026-11-17 P9). So `after-update` runs the step
//! in its own process when that is elevated already, and on a device where
//! se has its privileged component (the login broker service, LocalSystem,
//! whose own administrator approval per version a viewer gives through the
//! login screen) it leaves the step to se's one elevated call of the next
//! service start (`after_update_start`: party `Display` of
//! `rdp::host_setup::elevated`, the step `SYSTEM_STEP` of `se rdp host-setup
//! --system`), in-process when the worker is privileged itself. Only
//! without such a component does the update ask with its own UAC prompt.
//! The step itself is the same everywhere (`install::run`): it signs again
//! only when the driver's files change. A viewer that asks for se's display
//! on a host without a working driver starts the same step at use time
//! (`virtual_display_on_demand.rs`, user rule 2026-11-24) through
//! `run_step`; the decline of an update offer does not hold that back.
//!
//! Exit codes: 0 done or nothing to do, 1223 the elevation was declined,
//! 1 failed. The result line of the elevated step is in `last-run.txt` of
//! se's driver directory.
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Shell::{
    IsUserAnAdmin, ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::{install, pins, win_display};
use crate::login::os::shared::self_install::Elevation;
use crate::rdp::host_setup::elevated::{self as bundle, Launched, Party};
use crate::screen::core::virtual_display::windows_driver::{update_offers_step, DriverStep};

/// The argument of the unelevated entry.
pub const SWITCH: &str = "--display-driver";
/// The argument of the elevated process.
pub const ELEVATED_SWITCH: &str = "--display-driver-elevated";
const DECLINED: u32 = 1223;
/// The step of the driver in se's one elevated call (`se rdp host-setup
/// --system virtual-display-windows-driver`).
pub const SYSTEM_STEP: &str = "virtual-display-windows-driver";
/// Exit code of a failed step in that call (the display band 50..=55 of
/// `rdp::host_setup::elevated`).
const STEP_FAILED: i32 = 55;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The result line of the last step in se's driver directory.
pub(super) const LAST_RUN: &str = "last-run.txt";

/// The user declined the offer of an update once.
fn declined_marker() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("virtual-display-declined")
}

/// The driver is installed: a later update offers its step again.
pub(super) fn forget_decline() {
    let _ = std::fs::remove_file(declined_marker());
}

fn note(message: &str) {
    if let Ok(dir) = install::directory() {
        let _ = std::fs::write(dir.join(LAST_RUN), message);
    }
}

/// An update left the step to se's elevated call of the next service start.
fn pending_marker() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("virtual-display-pending")
}

fn write_marker(marker: &PathBuf, text: &[u8]) {
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(marker, text);
}

fn is_elevated() -> bool {
    // SAFETY: plain query of this process's token.
    unsafe { IsUserAnAdmin() != 0 }
}

/// se's privileged component on this device: the login broker service
/// (LocalSystem), installed with an administrator approval per version.
fn privileged_component() -> bool {
    use windows_sys::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS,
    };
    let name = wide(crate::login::os::platform::SERVICE);
    // SAFETY: plain service manager calls; every handle is closed below.
    unsafe {
        let manager = OpenSCManagerW(null(), null(), SC_MANAGER_CONNECT);
        if manager.is_null() {
            return false;
        }
        let service = OpenServiceW(manager, name.as_ptr(), SERVICE_QUERY_STATUS);
        let installed = !service.is_null();
        if installed {
            CloseServiceHandle(service);
        }
        CloseServiceHandle(manager);
        installed
    }
}

/// `se rdp host-setup --system virtual-display-windows-driver`: the step in
/// se's elevated call (or in a privileged worker); its line and exit code.
pub fn system_step() -> (String, i32) {
    if !is_elevated() {
        return ("display_state=failed:not_elevated".into(), STEP_FAILED);
    }
    let result = pins::pins()
        .map_err(|error| error.to_string())
        .and_then(install::run);
    match result {
        Ok(message) => {
            note(&message);
            ("display_state=done".into(), 0)
        }
        Err(error) => {
            note(&format!("virtueller Bildschirm: {error}"));
            let reason: String = error
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            (format!("display_state=failed:{reason}"), STEP_FAILED)
        }
    }
}

/// Service start of the worker: a driver step an update left
/// (`pending_marker`) joins the one approval of this start's self-setups
/// (announced before the login setup hands in its step), or runs here when
/// this process is privileged. A step that ran (also a failed or declined
/// one) is not asked again; a step that could not start waits for the next
/// start.
pub fn after_update_start() {
    if cfg!(any(test, feature = "test-support")) || !pending_marker().exists() {
        return;
    }
    let deciding = bundle::announce(Party::Display);
    let spawned = std::thread::Builder::new()
        .name("se-display-driver-step".into())
        .spawn(move || {
            run_pending();
            drop(deciding);
        });
    if let Err(error) = spawned {
        eprintln!("virtueller Bildschirm: Treiberschritt startet nicht: {error}");
    }
}

/// The driver step with se's one elevation: in this process when it is
/// privileged, else in the elevated call of the bundle (party `Display`, one
/// UAC prompt on this desktop; `announce` tells the person before it). The
/// step's exit code (1 when an earlier step of the call failed first);
/// `Err` when it could not start.
pub(super) fn run_step(announce: fn()) -> Result<i32, String> {
    match crate::login::os::platform::elevation() {
        Ok(Elevation::InProcess) => Ok(system_step().1),
        Ok(Elevation::External(se)) => {
            let launcher: bundle::Launcher = Box::new(move |steps: &[String]| {
                announce();
                crate::login::os::platform::run_elevated(&se, steps).map(|code| Launched {
                    code: Some(code),
                    ..Launched::default()
                })
            });
            let (call, result) = bundle::submit(Party::Display, SYSTEM_STEP.to_string(), launcher);
            let launched = result?;
            Ok(bundle::step_code(&call, Party::Display, launched.code).unwrap_or(1))
        }
        Err(error) => Err(format!("keine Erhöhung: {error}")),
    }
}

fn run_pending() {
    let code = match run_step(crate::rdp::host_setup::prompt_follows) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("virtueller Bildschirm: {error}");
            return;
        }
    };
    let _ = std::fs::remove_file(pending_marker());
    match u32::try_from(code) {
        Ok(0) => {
            forget_decline();
            win_display::release_idle();
        }
        Ok(DECLINED) => write_marker(&declined_marker(), b"declined\n"),
        _ => eprintln!("virtueller Bildschirm: Treiberschritt endete mit {code}"),
    }
}

/// Starts this program elevated with `ELEVATED_SWITCH` and waits for it.
fn run_elevated() -> Result<u32, String> {
    let program = std::env::current_exe().map_err(|error| format!("Programmpfad: {error}"))?;
    let file = wide(&program.to_string_lossy());
    let verb = wide("runas");
    let parameters = wide(ELEVATED_SWITCH);
    // SAFETY: a plain C structure; all zero is a valid value.
    let mut execute: SHELLEXECUTEINFOW = unsafe { zeroed() };
    execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    execute.lpVerb = verb.as_ptr();
    execute.lpFile = file.as_ptr();
    execute.lpParameters = parameters.as_ptr();
    execute.lpDirectory = null();
    execute.hwnd = null_mut();
    execute.nShow = SW_HIDE;
    // SAFETY: the strings outlive the call; the structure has its size set.
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        // SAFETY: reads the calling thread's last error.
        let code = unsafe { GetLastError() };
        return if code == DECLINED {
            Ok(DECLINED)
        } else {
            Err(format!("Erhöhung fehlgeschlagen: Win32 {code}"))
        };
    }
    if execute.hProcess.is_null() {
        return Err("kein Prozess für den erhöhten Schritt".into());
    }
    // No time limit: the step waits for the download and the driver
    // installation, which both make progress on their own.
    // SAFETY: the handle belongs to this call and is closed below.
    let waited = unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    let mut code = 1u32;
    // SAFETY: as above.
    let read = unsafe { GetExitCodeProcess(execute.hProcess, &mut code) };
    // SAFETY: closed once.
    unsafe { CloseHandle(execute.hProcess) };
    if waited != WAIT_OBJECT_0 || read == 0 {
        return Err("Ende des erhöhten Schritts unbekannt".into());
    }
    Ok(code)
}

fn elevated() -> i32 {
    // SAFETY: plain query of this process's token.
    if unsafe { IsUserAnAdmin() } == 0 {
        note("virtueller Bildschirm: ohne Administratorrechte gestartet");
        return 1;
    }
    let result = pins::pins()
        .map_err(|error| error.to_string())
        .and_then(install::run);
    match result {
        Ok(message) => {
            note(&message);
            0
        }
        Err(error) => {
            note(&format!("virtueller Bildschirm: {error}"));
            1
        }
    }
}

fn unelevated(mode: &str) -> i32 {
    let pins = match pins::pins() {
        Ok(pins) => pins,
        Err(error) => {
            eprintln!("virtueller Bildschirm: {error}");
            return 1;
        }
    };
    let step = install::step(pins);
    let offered = match mode {
        "install" => step != DriverStep::Nothing,
        "after-update" => {
            let before = install::directory()
                .ok()
                .and_then(|dir| install::read_state(&dir))
                .is_some();
            update_offers_step(step, before, declined_marker().exists())
        }
        _ => {
            eprintln!("{SWITCH} install|after-update");
            return 2;
        }
    };
    if !offered {
        return 0;
    }
    if mode == "after-update" {
        // P9: no prompt an absent user would have to answer.
        if is_elevated() {
            return elevated();
        }
        if privileged_component() {
            write_marker(&pending_marker(), b"after-update\n");
            return 0;
        }
    }
    match run_elevated() {
        Ok(DECLINED) => {
            write_marker(&declined_marker(), b"declined\n");
            DECLINED as i32
        }
        Ok(0) => {
            forget_decline();
            win_display::release_idle();
            0
        }
        Ok(code) => i32::try_from(code).unwrap_or(1),
        Err(error) => {
            eprintln!("virtueller Bildschirm: {error}");
            1
        }
    }
}

/// The driver step for the arguments of the updater program; `None` when
/// the arguments name no driver step.
pub fn command(args: &[String]) -> Option<i32> {
    match args.get(1).map(String::as_str) {
        Some(ELEVATED_SWITCH) => Some(elevated()),
        Some(SWITCH) => Some(unelevated(args.get(2).map_or("", String::as_str))),
        _ => None,
    }
}
