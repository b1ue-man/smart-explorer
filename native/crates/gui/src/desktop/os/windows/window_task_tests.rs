//! DSK-M5 checks with the real GUI program on the runner's desktop
//! (`windows-2025`, interactive session): the surface is the lowest tool
//! window over its monitor and stays there after a click, keeps its styles
//! against a foreign change and survives Win+D; the jump list goes with
//! `se desktop remove`; an update stop and a GUI start with `--updated`
//! bring shell and surfaces back. `desktop_task_windows_show_desktop_keeps_surface`
//! needs Explorer (`Progman`); the suite reports it as not run without
//! (Annahme A7).
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_D, VK_LWIN};
use windows_sys::Win32::UI::Shell::FOLDERID_Recent;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IsIconic, IsWindowVisible, SetWindowLongPtrW, GWL_EXSTYLE, GWL_STYLE,
    WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_MINIMIZEBOX,
};

use super::desktop_folder::known_folder;
use super::monitors::{self, MonitorInfo};
use super::window_task_env::{
    below, chord, click, ex_style, progman_present, pump, rect, root_at, style, wait_for,
    PlainWindow, WinTaskEnv,
};
use super::PlatformIntegration;
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::ledger::Registration;
use crate::desktop::core::state::{Part, PartSet};
use crate::desktop::os::control::Control;
use crate::desktop::os::launch::SystemProcesses;

fn primary() -> MonitorInfo {
    let monitors = monitors::enumerate();
    monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or(monitors.first())
        .cloned()
        .expect("kein Bildschirm in dieser Sitzung")
}

/// A point of `monitor` where the surface `hwnd` is on top (no other
/// window covers it).
fn free_point(monitor: &MonitorInfo, hwnd: windows_sys::Win32::Foundation::HWND) -> [i32; 2] {
    let [left, top, right, bottom] = monitor.work;
    for row in 1..8 {
        for column in 1..8 {
            let point = [
                left + (right - left) * column / 8,
                top + (bottom - top) * row / 8,
            ];
            if root_at(point) == hwnd {
                return point;
            }
        }
    }
    panic!("keine freie Stelle der Fläche auf {}", monitor.gdi_name);
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_surface_bottom_toolwindow() {
    let mut env = WinTaskEnv::new();
    let monitor = primary();
    let surface = env.surface(&monitor.key);
    let hwnd = env.surface_window(surface);
    pump(Duration::from_millis(500));
    assert_ne!(ex_style(hwnd) & WS_EX_TOOLWINDOW, 0, "WS_EX_TOOLWINDOW");
    assert_eq!(ex_style(hwnd) & WS_EX_APPWINDOW, 0, "WS_EX_APPWINDOW");
    assert_eq!(style(hwnd) & WS_MINIMIZEBOX, 0, "WS_MINIMIZEBOX");
    assert_eq!(rect(hwnd), monitor.rect, "rcMonitor");
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_surface_stays_bottom_after_click() {
    let mut env = WinTaskEnv::new();
    let monitor = primary();
    let surface = env.surface(&monitor.key);
    let hwnd = env.surface_window(surface);
    let [left, top, _, _] = monitor.work;
    let program = PlainWindow::new([left + 40, top + 40, left + 440, top + 340]);
    assert!(
        below(program.0, hwnd),
        "Fläche liegt nicht unter dem Programmfenster"
    );
    let point = free_point(&monitor, hwnd);
    click(point);
    wait_for(
        Duration::from_secs(5),
        "Fläche aktiv nach Klick",
        || unsafe { GetForegroundWindow() == hwnd },
    );
    pump(Duration::from_millis(500));
    assert!(
        below(program.0, hwnd),
        "die Fläche ist nach dem Klick über das Programmfenster gesprungen"
    );
    assert_ne!(ex_style(hwnd) & WS_EX_TOOLWINDOW, 0);
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_style_enforced_after_change() {
    let mut env = WinTaskEnv::new();
    let monitor = primary();
    let surface = env.surface(&monitor.key);
    let hwnd = env.surface_window(surface);
    // A foreign change, as winit makes when it re-applies its styles.
    let foreign = (ex_style(hwnd) | WS_EX_APPWINDOW) & !WS_EX_TOOLWINDOW;
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, foreign as isize) };
    unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, (style(hwnd) | WS_MINIMIZEBOX) as isize) };
    pump(Duration::from_millis(300));
    assert_ne!(ex_style(hwnd) & WS_EX_TOOLWINDOW, 0, "WS_EX_TOOLWINDOW");
    assert_eq!(ex_style(hwnd) & WS_EX_APPWINDOW, 0, "WS_EX_APPWINDOW");
    assert_eq!(style(hwnd) & WS_MINIMIZEBOX, 0, "WS_MINIMIZEBOX");
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_show_desktop_keeps_surface() {
    assert!(
        progman_present(),
        "Vorprüfung A7: kein Explorer-Desktop (Progman) in dieser Sitzung"
    );
    let mut env = WinTaskEnv::new();
    let monitor = primary();
    let surface = env.surface(&monitor.key);
    let hwnd = env.surface_window(surface);
    chord(&[VK_LWIN, VK_D]);
    pump(Duration::from_millis(1500));
    let iconic = unsafe { IsIconic(hwnd) } != 0;
    let visible = unsafe { IsWindowVisible(hwnd) } != 0;
    // The second Win+D gives the desktop back as it was.
    chord(&[VK_LWIN, VK_D]);
    pump(Duration::from_millis(1000));
    assert!(!iconic, "Win+D hat die Fläche minimiert");
    assert!(visible, "Win+D hat die Fläche verborgen");
}

/// Name, size and time of every file of the jump-list storage.
fn custom_destinations() -> BTreeMap<String, (u64, Option<SystemTime>)> {
    let folder: PathBuf = known_folder(&FOLDERID_Recent)
        .expect("FOLDERID_Recent")
        .join("CustomDestinations");
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let meta = entry.metadata().ok()?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    Some((name, (meta.len(), meta.modified().ok())))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_jump_list_removed_from_gui_image() {
    let mut env = WinTaskEnv::new();
    let before = custom_destinations();
    env.enable(&[Part::JumpList]);
    env.spawn(&["--desktop-shell"]);
    let dir = env.dir();
    wait_for(Duration::from_secs(60), "Sprungliste geschrieben", || {
        let recorded = dir.read_state().ok().flatten().is_some_and(|state| {
            state
                .ledger
                .iter()
                .any(|entry| entry.kind == Registration::JumpList)
        });
        recorded && !env.events("jump_list").is_empty()
    });
    assert_ne!(
        custom_destinations(),
        before,
        "Annahme A6: die Liste liegt nicht in Recent\\CustomDestinations"
    );
    // `se desktop remove` from another image: the GUI program deletes the
    // list with `--desktop-jumplist-clear`.
    let mut integration = PlatformIntegration::new(Some(env.gui.clone()));
    let mut available = PartSet::default();
    for part in Part::ALL {
        available.set(part, true);
    }
    let mut control = Control {
        dir: env.dir(),
        integration: &mut integration,
        processes: &SystemProcesses,
        gui: Ok(env.gui.clone()),
        available,
        unavailable: Vec::new(),
        system_items: SystemItems::initial(None, None),
        version: env!("CARGO_PKG_VERSION").into(),
        now: 1_700_000_000,
    };
    let outcome = control.remove().unwrap();
    assert!(outcome.complete(), "{outcome:?}");
    assert_eq!(custom_destinations(), before);
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_update_restarts_surface() {
    let mut env = WinTaskEnv::new();
    env.enable(&[Part::Surface]);
    let shell = env.spawn(&["--desktop-shell"]);
    wait_for(Duration::from_secs(30), "Fläche gestartet", || {
        !env.events("surface_start").is_empty()
    });
    // Stop as the updater helper does: its marker.
    let marker = crate::support_dirs::sync_data_dir().join("daemon.stop");
    std::fs::write(&marker, "stop").unwrap();
    let mut code = None;
    wait_for(Duration::from_secs(10), "Wächter endet mit 75", || {
        code = env
            .child(shell)
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.code());
        code.is_some()
    });
    assert_eq!(code, Some(Some(crate::daemon::EXIT_UPDATE_STOP)));
    let starts = env.events("start").len();
    let surfaces = env.events("surface_start").len();
    // The helper starts the app with `--updated`; it resumes the shell.
    env.spawn(&["--updated"]);
    wait_for(
        Duration::from_secs(60),
        "Wächter und Fläche wieder da",
        || env.events("start").len() > starts && env.events("surface_start").len() > surfaces,
    );
}
