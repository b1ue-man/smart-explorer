//! Explorer on the surface (DSK-M5, C1.2): "Weitere Optionen anzeigen"
//! opens Explorer's own context menu for the selected desktop files
//! (`shell_menu::show_for_paths`, with every shell extension) or for the
//! desktop folder's background (`show_background_menu`); files dragged out
//! of the surface go to other programs through `dragout`. Menu and drag run
//! modal loops on the surface's UI thread, which then draws no frame. A
//! thread timer, dispatched by those loops, calls the surface's heartbeat
//! meanwhile, so the shell does not take a menu left open for ten seconds
//! for a hang (C5.5).
use std::cell::Cell;

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, KillTimer, PostMessageW, SetTimer, WindowFromPoint, GA_ROOT,
    WM_LBUTTONUP,
};

use crate::app::desktop_surface_host::{NativeWindow, ShellAction, ShellOutcome};
use crate::dragout::DragOutOutcome;
use crate::shell_menu::MenuResult;

/// The surface sends `alive` at most once a second (C5.5).
const HEARTBEAT_EVERY_MS: u32 = 1000;

type Heartbeat = *mut (dyn FnMut() + 'static);

thread_local! {
    /// The heartbeat of the modal loop running on this thread.
    static HEARTBEAT: Cell<Option<Heartbeat>> = const { Cell::new(None) };
}

unsafe extern "system" fn tick(_hwnd: HWND, _message: u32, _id: usize, _time: u32) {
    if let Some(heartbeat) = HEARTBEAT.with(Cell::get) {
        // SAFETY: set only while `with_heartbeat` runs on this thread and
        // borrows the closure; the guard clears it before the borrow ends.
        (*heartbeat)();
    }
}

/// Ends the timer and restores the previous heartbeat, also on a panic.
struct TimerGuard {
    timer: usize,
    previous: Option<Heartbeat>,
}

impl Drop for TimerGuard {
    fn drop(&mut self) {
        if self.timer != 0 {
            unsafe { KillTimer(std::ptr::null_mut(), self.timer) };
        }
        HEARTBEAT.with(|slot| slot.set(self.previous));
    }
}

/// Runs `modal` while a thread timer calls `heartbeat` every second.
fn with_heartbeat<R>(heartbeat: &mut dyn FnMut(), modal: impl FnOnce() -> R) -> R {
    let borrowed: *mut (dyn FnMut() + '_) = heartbeat;
    // SAFETY: only the lifetime is erased; the guard below removes the
    // pointer before `heartbeat` goes out of scope.
    let erased: Heartbeat = unsafe { std::mem::transmute(borrowed) };
    let previous = HEARTBEAT.with(|slot| slot.replace(Some(erased)));
    let timer = unsafe { SetTimer(std::ptr::null_mut(), 0, HEARTBEAT_EVERY_MS, Some(tick)) };
    let _guard = TimerGuard { timer, previous };
    modal()
}

fn shown(paths: &[std::path::PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.to_string_lossy().replace('/', "\\"))
        .collect()
}

fn menu_outcome(result: windows::core::Result<MenuResult>) -> Result<ShellOutcome, String> {
    match result {
        Ok(MenuResult::Shell | MenuResult::Own(_)) => Ok(ShellOutcome::Done),
        Ok(MenuResult::None) => Ok(ShellOutcome::Nothing),
        Err(error) => Err(error.to_string()),
    }
}

/// Runs one Explorer action of a surface; `heartbeat` keeps the surface
/// alive while its modal loop runs.
pub(crate) fn shell_action(
    action: ShellAction<'_>,
    heartbeat: &mut dyn FnMut(),
) -> Result<ShellOutcome, String> {
    match action {
        ShellAction::ItemMenu(paths) => {
            let paths = shown(paths);
            menu_outcome(with_heartbeat(heartbeat, || {
                crate::shell_menu::show_for_paths(&paths, None, None, &[])
            }))
        }
        ShellAction::FolderMenu(folder) => {
            let folder = folder.to_string_lossy().replace('/', "\\");
            menu_outcome(with_heartbeat(heartbeat, || {
                crate::shell_menu::show_background_menu(&folder, &[])
            }))
        }
        ShellAction::DragOut(paths) => {
            let files = shown(paths);
            match with_heartbeat(heartbeat, || crate::dragout::drag_out(&files)) {
                Ok(DragOutOutcome::Dropped(_)) => Ok(ShellOutcome::Done),
                Ok(DragOutOutcome::Cancelled) => Ok(ShellOutcome::Nothing),
                Err(error) => Err(error.to_string()),
            }
        }
    }
}

fn hwnd_of(window: NativeWindow) -> Option<HWND> {
    match window {
        NativeWindow::Win32 { hwnd } => Some(hwnd as HWND),
        _ => None,
    }
}

/// The pointer is over another program's window (or the task bar), which
/// lies above the surface although inside its rectangle.
pub(crate) fn pointer_outside(window: NativeWindow) -> bool {
    let Some(hwnd) = hwnd_of(window) else {
        return false;
    };
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return false;
    }
    let hit = unsafe { WindowFromPoint(point) };
    !hit.is_null() && unsafe { GetAncestor(hit, GA_ROOT) } != hwnd
}

/// After a drag out the button's release went to the drag loop, not to the
/// surface; this hands the surface its release at the pointer.
pub(crate) fn release_pointer(window: NativeWindow) {
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    let mut point = POINT::default();
    unsafe {
        if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point).is_err() {
            return;
        }
        let window = windows::Win32::Foundation::HWND(hwnd);
        let _ = ScreenToClient(window, &mut point);
        let position = ((point.y as u16 as u32) << 16) | (point.x as u16 as u32);
        PostMessageW(hwnd, WM_LBUTTONUP, 0, position as isize);
    }
}
