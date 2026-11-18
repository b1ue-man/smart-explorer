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
use crate::screen::core::virtual_display::windows_driver::{update_offers_step, DriverStep};

/// The argument of the unelevated entry.
pub const SWITCH: &str = "--display-driver";
/// The argument of the elevated process.
pub const ELEVATED_SWITCH: &str = "--display-driver-elevated";
const DECLINED: u32 = 1223;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The user declined the offer of an update once.
fn declined_marker() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("virtual-display-declined")
}

fn note(message: &str) {
    if let Ok(dir) = install::directory() {
        let _ = std::fs::write(dir.join("last-run.txt"), message);
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
    match run_elevated() {
        Ok(DECLINED) => {
            let marker = declined_marker();
            if let Some(parent) = marker.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(marker, b"declined\n");
            DECLINED as i32
        }
        Ok(0) => {
            let _ = std::fs::remove_file(declined_marker());
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
