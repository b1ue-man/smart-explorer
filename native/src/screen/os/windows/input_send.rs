//! Input on a Windows host (SCR-M6, C5): `SendInput` in the host process at
//! the user's integrity level. The pointer goes absolute over the virtual
//! desktop (`MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`), positions
//! 0–65535 across the session's source (its monitor, or its window as it
//! lies now); keys as set-1 scan codes (`KEYEVENTF_SCANCODE`), text as
//! UTF-16 units (`KEYEVENTF_UNICODE`), touch as a single pointer.
//!
//! Before every batch: on the secure desktop (UAC prompt, lock screen) the
//! batch is dropped with `secure_desktop`; with a foreground window of a
//! higher integrity level, or when that cannot be told, with
//! `elevated_window` (`input_integrity`). Ctrl+Alt+Delete is never sent
//! (Windows takes it from no program anyway; `no_ctrl_alt_del`). No event
//! is ever logged.
use std::sync::{Mutex, MutexGuard, PoisonError};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    XBUTTON1, XBUTTON2,
};

use super::{input_integrity, monitors, secure_desktop};
use crate::screen::core::input_model::{InputEvent, PointerButton, TouchPhase};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::os::input_keys::{windows_key, SasGuard, WinKey};

/// A Windows host never sends Ctrl+Alt+Delete.
pub(crate) const LIMIT_NO_CTRL_ALT_DEL: &str = "no_ctrl_alt_del";
/// Input to an administrator window is blocked and reported.
pub(crate) const LIMIT_ELEVATED_WINDOWS_BLOCKED: &str = "elevated_windows_blocked";

/// Where this host process's pointer positions point to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Monitor(String),
    Window(usize),
}

struct State {
    target: Option<Target>,
    sas: SasGuard,
}

static STATE: Mutex<State> = Mutex::new(State {
    target: None,
    sas: SasGuard::new(),
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The source this host process opened (`host_worker_main`).
pub(crate) fn set_target(source: &SourceId) {
    state().target = match source {
        SourceId::Monitor(id) => Some(Target::Monitor(id.clone())),
        SourceId::Window(text) => text.parse().ok().map(Target::Window),
        SourceId::Synthetic => None,
    };
}

/// A rectangle in virtual-desktop pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Area {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

fn target_area(target: &Target) -> Option<Area> {
    match target {
        Target::Monitor(id) => monitors::find(id).map(|entry| Area {
            left: entry.info.x,
            top: entry.info.y,
            width: entry.info.width as i32,
            height: entry.info.height as i32,
        }),
        // The window picture of Graphics Capture covers the visible frame.
        // GetWindowRect "may include invisible resize borders"; Microsoft
        // names DWMWA_EXTENDED_FRAME_BOUNDS for the visible bounds (physical
        // pixels, "not adjusted for DPI"), the same rectangle the family path
        // (`remote_window::os::windows::facts`) maps its input onto.
        Target::Window(handle) => {
            let mut rect = windows::Win32::Foundation::RECT::default();
            // SAFETY: a plain query on a window handle value into a local of
            // the size passed.
            let read = unsafe {
                windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                    windows::Win32::Foundation::HWND(*handle as HWND),
                    windows::Win32::Graphics::Dwm::DWMWA_EXTENDED_FRAME_BOUNDS,
                    (&mut rect as *mut windows::Win32::Foundation::RECT).cast(),
                    std::mem::size_of_val(&rect) as u32,
                )
            };
            if read.is_err() {
                return None;
            }
            window_area(rect.left, rect.top, rect.right, rect.bottom)
        }
    }
}

/// The area of a window rectangle; none for an empty or inverted one.
fn window_area(left: i32, top: i32, right: i32, bottom: i32) -> Option<Area> {
    let width = right.checked_sub(left).filter(|width| *width > 0)?;
    let height = bottom.checked_sub(top).filter(|height| *height > 0)?;
    Some(Area {
        left,
        top,
        width,
        height,
    })
}

fn virtual_desktop() -> Area {
    // SAFETY: plain metric queries.
    unsafe {
        Area {
            left: GetSystemMetrics(SM_XVIRTUALSCREEN),
            top: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

/// The `SendInput` coordinates (0–65535 over the virtual desktop) of the
/// source position `x`, `y` (0–65535 over `source`).
pub(super) fn absolute(source: Area, desk: Area, x: u16, y: u16) -> (i32, i32) {
    let along = |pos: u16, start: i32, length: i32, desk_start: i32, desk_length: i32| {
        let pixel = i64::from(start) + i64::from(pos) * i64::from((length - 1).max(0)) / 65_535;
        let span = i64::from((desk_length - 1).max(1));
        ((pixel - i64::from(desk_start)) * 65_535 / span).clamp(0, 65_535) as i32
    };
    (
        along(x, source.left, source.width, desk.left, desk.width),
        along(y, source.top, source.height, desk.top, desk.height),
    )
}

fn mouse(flags: u32, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as _,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// The `INPUT` records of `events`; `area` is the source on the virtual
/// desktop `desk`.
pub(super) fn records(
    events: &[InputEvent],
    area: Area,
    desk: Area,
    sas: &mut SasGuard,
) -> Vec<INPUT> {
    records_with_position(events, area, desk, sas, absolute)
}

fn records_with_position(
    events: &[InputEvent],
    area: Area,
    desk: Area,
    sas: &mut SasGuard,
    position: fn(Area, Area, u16, u16) -> (i32, i32),
) -> Vec<INPUT> {
    const MOVE: u32 = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let mut out = Vec::with_capacity(events.len() * 2);
    for event in events {
        match event {
            InputEvent::PointerAbs { x, y } => {
                let (ax, ay) = position(area, desk, *x, *y);
                out.push(mouse(MOVE, ax, ay, 0));
            }
            InputEvent::Button { button, down } => {
                let (flags, data) = match (button, down) {
                    (PointerButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                    (PointerButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (PointerButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (PointerButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (PointerButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (PointerButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (PointerButton::Back, true) => (MOUSEEVENTF_XDOWN, XBUTTON1 as i32),
                    (PointerButton::Back, false) => (MOUSEEVENTF_XUP, XBUTTON1 as i32),
                    (PointerButton::Forward, true) => (MOUSEEVENTF_XDOWN, XBUTTON2 as i32),
                    (PointerButton::Forward, false) => (MOUSEEVENTF_XUP, XBUTTON2 as i32),
                };
                out.push(mouse(flags, 0, 0, data));
            }
            // 120 units are one notch, as `WHEEL_DELTA`; up and right are positive.
            InputEvent::Wheel { dx, dy } => {
                if *dy != 0 {
                    out.push(mouse(MOUSEEVENTF_WHEEL, 0, 0, i32::from(*dy)));
                }
                if *dx != 0 {
                    out.push(mouse(MOUSEEVENTF_HWHEEL, 0, 0, i32::from(*dx)));
                }
            }
            InputEvent::Key { hid_usage, down } => {
                if !sas.allows(*hid_usage, *down) {
                    continue;
                }
                let up = if *down { 0 } else { KEYEVENTF_KEYUP };
                match windows_key(*hid_usage) {
                    Some(WinKey::Scan { code, extended }) => {
                        let extended = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
                        out.push(key(0, code, KEYEVENTF_SCANCODE | extended | up));
                    }
                    Some(WinKey::Virtual(vk)) => out.push(key(vk, 0, up)),
                    None => {}
                }
            }
            InputEvent::Text { text } => {
                for unit in text.encode_utf16() {
                    out.push(key(0, unit, KEYEVENTF_UNICODE));
                    out.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
            }
            // One finger as the left button; further fingers are left out.
            InputEvent::Touch { id: 0, phase, x, y } => {
                let (ax, ay) = position(area, desk, *x, *y);
                out.push(mouse(MOVE, ax, ay, 0));
                match phase {
                    TouchPhase::Start => out.push(mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0)),
                    TouchPhase::End | TouchPhase::Cancel => {
                        out.push(mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0))
                    }
                    TouchPhase::Move => {}
                }
            }
            // Android's system actions do not exist on Windows.
            InputEvent::Touch { .. } | InputEvent::Action { .. } => {}
        }
    }
    out
}

/// Injects `events` into this Windows session, or names why not.
pub(crate) fn inject(events: &[InputEvent]) -> Result<(), ControlBlock> {
    if secure_desktop::on_secure_desktop() {
        return Err(ControlBlock::SecureDesktop);
    }
    if input_integrity::foreground_blocked() {
        return Err(ControlBlock::ElevatedWindow);
    }
    let mut state = state();
    let target = state
        .target
        .clone()
        .ok_or(ControlBlock::ControlUnavailable)?;
    let area = target_area(&target).ok_or(ControlBlock::ControlUnavailable)?;
    let inputs = records(events, area, virtual_desktop(), &mut state.sas);
    drop(state);
    submit(&inputs)
}

/// Family positions use the very physical rectangle admitted by HostSession.
/// The raw monitor/window path above deliberately retains its old mapping.
pub(crate) fn inject_family(
    geometry: crate::remote_window::CaptureGeometry,
    events: &[InputEvent],
) -> Result<(), ControlBlock> {
    if secure_desktop::on_secure_desktop() {
        return Err(ControlBlock::SecureDesktop);
    }
    if input_integrity::foreground_blocked() {
        return Err(ControlBlock::ElevatedWindow);
    }
    if !geometry.valid() {
        return Err(ControlBlock::ControlUnavailable);
    }
    monitors::ensure_dpi_aware();
    let area = Area {
        left: geometry.x,
        top: geometry.y,
        width: i32::try_from(geometry.width).map_err(|_| ControlBlock::ControlUnavailable)?,
        height: i32::try_from(geometry.height).map_err(|_| ControlBlock::ControlUnavailable)?,
    };
    let desk = virtual_desktop();
    if desk.width <= 0 || desk.height <= 0 {
        return Err(ControlBlock::ControlUnavailable);
    }
    for event in events {
        if let InputEvent::PointerAbs { x, y } | InputEvent::Touch { x, y, .. } = event {
            let px = i64::from(area.left) + i64::from(*x) * i64::from(area.width - 1) / 65_535;
            let py = i64::from(area.top) + i64::from(*y) * i64::from(area.height - 1) / 65_535;
            if px < i64::from(desk.left)
                || py < i64::from(desk.top)
                || px >= i64::from(desk.left) + i64::from(desk.width)
                || py >= i64::from(desk.top) + i64::from(desk.height)
            {
                return Err(ControlBlock::ControlUnavailable);
            }
            let (ax, ay) = family_absolute(area, desk, *x, *y);
            // Reject a pixel the 16-bit desktop coordinate cannot represent.
            if i64::from(ax) * i64::from(desk.width) / 65_536 + i64::from(desk.left) != px
                || i64::from(ay) * i64::from(desk.height) / 65_536 + i64::from(desk.top) != py
            {
                return Err(ControlBlock::ControlUnavailable);
            }
        }
    }
    let inputs = records_with_position(events, area, desk, &mut state().sas, family_absolute);
    submit(&inputs)
}

fn family_absolute(area: Area, desk: Area, x: u16, y: u16) -> (i32, i32) {
    // Pixel centers avoid the edge-rounding error described by Microsoft's
    // UIAutomation Input.SendMouseInput implementation (source.dot.net).
    let axis = |pos: u16, start: i32, length: i32, origin: i32, span: i32| {
        let pixel =
            i64::from(start) - i64::from(origin) + i64::from(pos) * i64::from(length - 1) / 65_535;
        (((pixel * 2 + 1) * 32_768) / i64::from(span)).clamp(0, 65_535) as i32
    };
    (
        axis(x, area.left, area.width, desk.left, desk.width),
        axis(y, area.top, area.height, desk.top, desk.height),
    )
}

/// Compensating ups never query or activate the now-unusable target. UIPI is
/// still enforced by SendInput itself; failed releases remain pending upstream.
pub(crate) fn release(events: &[InputEvent]) -> Result<(), ControlBlock> {
    if secure_desktop::on_secure_desktop() {
        return Err(ControlBlock::SecureDesktop);
    }
    let mut releases = Vec::with_capacity(events.len());
    for event in events {
        match event {
            InputEvent::Key { down: false, .. } | InputEvent::Button { down: false, .. } => {
                releases.push(event.clone())
            }
            InputEvent::Touch {
                id,
                phase: TouchPhase::End | TouchPhase::Cancel,
                ..
            } => {
                if *id == 0 {
                    releases.push(InputEvent::Button {
                        button: PointerButton::Left,
                        down: false,
                    });
                }
            }
            _ => return Err(ControlBlock::ControlUnavailable),
        }
    }
    // No accepted event uses coordinates; the one-pixel value is never read.
    let unused = Area {
        left: 0,
        top: 0,
        width: 1,
        height: 1,
    };
    let inputs = records(&releases, unused, unused, &mut state().sas);
    submit(&inputs)
}

fn submit(inputs: &[INPUT]) -> Result<(), ControlBlock> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: `inputs` holds `len` initialized records of `size_of::<INPUT>()`.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    // Windows refuses input it blocks (UIPI) without saying more.
    if sent as usize != inputs.len() {
        return Err(ControlBlock::ElevatedWindow);
    }
    Ok(())
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
