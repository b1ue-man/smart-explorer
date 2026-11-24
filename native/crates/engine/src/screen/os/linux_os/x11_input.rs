//! Input on an X11 host (SCR-M6, C5): XTest (`x11rb`, feature `xtest`) in
//! the host process. Pointer positions 0–65535 across the session's source
//! (its monitor or its window as it lies now) become root coordinates;
//! keys go as the evdev keycode plus 8 (the keycodes of an evdev keyboard),
//! text as keysyms (`x11_keymap`, with a spare keycode for characters the
//! layout does not carry), the wheel as buttons 4–7, one finger as the left
//! button. Ctrl+Alt+Delete is dropped. No event is ever logged.
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{ConnectionExt as _, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::x11_keymap::Checked;
use super::{x11_keymap, x11_monitors, x11_windows};
use crate::screen::core::input_model::{InputEvent, PointerButton, TouchPhase};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::os::input_keys::{evdev_code, SasGuard};

#[path = "x11_input_release.rs"]
mod letgo;
use letgo::let_go;
pub use letgo::release;

const BUTTON_PRESS: u8 = 4;
const BUTTON_RELEASE: u8 = 5;
const MOTION_NOTIFY: u8 = 6;
/// One wheel notch in the core's units (C5: 1/120).
const NOTCH: i32 = 120;

struct State {
    target: Option<SourceId>,
    sas: SasGuard,
    conn: Option<(RustConnection, Window)>,
    /// The part of a notch per axis (x, y) not sent yet: viewers send
    /// fractions of a notch (a touchpad), X11 only whole notches.
    wheel_rest: (i32, i32),
}

static STATE: Mutex<State> = Mutex::new(State {
    target: None,
    sas: SasGuard::new(),
    conn: None,
    wheel_rest: (0, 0),
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The source this host process opened (`host_worker_main`).
pub fn set_target(source: &SourceId) {
    let mut state = state();
    state.target = (!matches!(source, SourceId::Synthetic)).then(|| source.clone());
    state.wheel_rest = (0, 0);
}

/// A rectangle in root coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The root position of `x`, `y` (0–65535 over `area`).
pub(super) fn root_point(area: Area, x: u16, y: u16) -> (i16, i16) {
    let along = |pos: u16, start: i32, length: i32| {
        let span = (i64::from(length) - 1).max(0);
        let pixel = i64::from(start) + i64::from(pos) * span / 65_535;
        pixel.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
    };
    (along(x, area.x, area.width), along(y, area.y, area.height))
}

fn area_of<C: Connection>(conn: &C, root: Window, source: &SourceId) -> io::Result<Area> {
    match source {
        SourceId::Monitor(id) => x11_monitors::list(conn, root)?
            .into_iter()
            .find(|monitor| &monitor.id == id)
            .map(|monitor| Area {
                x: monitor.x,
                y: monitor.y,
                width: monitor.width as i32,
                height: monitor.height as i32,
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "monitor gone")),
        SourceId::Window(id) => {
            let window = x11_windows::parse_window_id(id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "window id"))?;
            let geometry = conn
                .get_geometry(window)
                .map_err(io::Error::other)?
                .reply()
                .map_err(io::Error::other)?;
            let origin = conn
                .translate_coordinates(window, root, 0, 0)
                .map_err(io::Error::other)?
                .reply()
                .map_err(io::Error::other)?;
            Ok(Area {
                x: i32::from(origin.dst_x),
                y: i32::from(origin.dst_y),
                width: i32::from(geometry.width),
                height: i32::from(geometry.height),
            })
        }
        SourceId::Synthetic => Err(io::Error::from(io::ErrorKind::Unsupported)),
    }
}

fn button_code(button: PointerButton) -> u8 {
    match button {
        PointerButton::Left => 1,
        PointerButton::Middle => 2,
        PointerButton::Right => 3,
        PointerButton::Back => 8,
        PointerButton::Forward => 9,
    }
}

/// One fake input; its X11 error is checked with `sent` (round 12, Codex
/// 10: a dropped cookie left a refusal unread, the batch counted as sent).
fn fake<'c, C: Connection>(
    (conn, sent): (&'c C, &mut Checked<'c, C>),
    root: Window,
    kind: u8,
    detail: u8,
    at: (i16, i16),
) -> io::Result<()> {
    sent.push(conn.xtest_fake_input(kind, detail, 0, root, at.0, at.1, 0))
}

/// Whole notches of `rest` plus `amount` (1/120 notch); the remainder stays
/// in `rest` for the next event of the axis.
fn whole_notches(rest: &mut i32, amount: i16) -> i32 {
    let total = rest.saturating_add(i32::from(amount));
    *rest = total % NOTCH;
    total / NOTCH
}

fn wheel<'c, C: Connection>(
    (conn, sent): (&'c C, &mut Checked<'c, C>),
    root: Window,
    rest: &mut i32,
    amount: i16,
    positive: u8,
    negative: u8,
) -> io::Result<()> {
    let notches = whole_notches(rest, amount);
    let button = if notches > 0 { positive } else { negative };
    for _ in 0..notches.abs() {
        fake((conn, &mut *sent), root, BUTTON_PRESS, button, (0, 0))?;
        fake((conn, &mut *sent), root, BUTTON_RELEASE, button, (0, 0))?;
    }
    Ok(())
}

/// Sends `events` through XTest; returns the characters without a key.
pub(super) fn send<C: Connection>(
    conn: &C,
    root: Window,
    area: Area,
    events: &[InputEvent],
    sas: &mut SasGuard,
    wheel_rest: &mut (i32, i32),
) -> io::Result<usize> {
    let mut skipped = 0;
    let mut sent = Checked::new();
    for event in events {
        match event {
            InputEvent::PointerAbs { x, y } => fake(
                (conn, &mut sent),
                root,
                MOTION_NOTIFY,
                0,
                root_point(area, *x, *y),
            )?,
            InputEvent::Button { button, down } => {
                let kind = if *down { BUTTON_PRESS } else { BUTTON_RELEASE };
                fake((conn, &mut sent), root, kind, button_code(*button), (0, 0))?;
            }
            // Up and right are positive (C5); X11 scrolls up with 4, right with 7.
            InputEvent::Wheel { dx, dy } => {
                wheel((conn, &mut sent), root, &mut wheel_rest.1, *dy, 4, 5)?;
                wheel((conn, &mut sent), root, &mut wheel_rest.0, *dx, 7, 6)?;
            }
            InputEvent::Key { hid_usage, down } => {
                if !sas.allows(*hid_usage, *down) {
                    continue;
                }
                if let Some(code) =
                    evdev_code(*hid_usage).and_then(|code| u8::try_from(code + 8).ok())
                {
                    let kind = if *down {
                        x11_keymap::KEY_PRESS
                    } else {
                        x11_keymap::KEY_RELEASE
                    };
                    fake((conn, &mut sent), root, kind, code, (0, 0))?;
                }
            }
            InputEvent::Text { text } => skipped += x11_keymap::type_text(conn, root, text)?,
            InputEvent::Touch { id: 0, phase, x, y } => {
                fake(
                    (conn, &mut sent),
                    root,
                    MOTION_NOTIFY,
                    0,
                    root_point(area, *x, *y),
                )?;
                match phase {
                    TouchPhase::Start => fake((conn, &mut sent), root, BUTTON_PRESS, 1, (0, 0))?,
                    TouchPhase::End | TouchPhase::Cancel => {
                        fake((conn, &mut sent), root, BUTTON_RELEASE, 1, (0, 0))?
                    }
                    TouchPhase::Move => {}
                }
            }
            InputEvent::Touch { .. } | InputEvent::Action { .. } => {}
        }
    }
    // Sends what waits and fails on the first request the server refused.
    sent.check()?;
    Ok(skipped)
}

/// Injects `events` into this X11 display, or names why not.
pub fn inject(events: &[InputEvent]) -> Result<(), ControlBlock> {
    let mut state = state();
    let target = state
        .target
        .clone()
        .ok_or(ControlBlock::ControlUnavailable)?;
    if state.conn.is_none() {
        let (conn, screen) =
            RustConnection::connect(None).map_err(|_| ControlBlock::ControlUnavailable)?;
        let present = conn
            .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some();
        if !present {
            return Err(ControlBlock::UnsupportedOnHost);
        }
        let root = conn.setup().roots[screen].root;
        // Codex 10/16: spare keys a text before could not empty again.
        crate::screen::os::x11_keymap::restore_pending(&conn);
        state.conn = Some((conn, root));
    }
    let State {
        sas,
        conn,
        wheel_rest,
        ..
    } = &mut *state;
    let Some((conn, root)) = conn.as_ref() else {
        return Err(ControlBlock::ControlUnavailable);
    };
    let area = area_of(conn, *root, &target).map_err(|_| ControlBlock::ControlUnavailable)?;
    texts_typable(conn, events)?;
    match send(conn, *root, area, events, sas, wheel_rest) {
        Ok(0) => Ok(()),
        Ok(skipped) => {
            eprintln!("screen host process: {skipped} characters have no key on this host");
            Ok(())
        }
        Err(_) => {
            // A broken connection is made anew with the next batch.
            state.conn = None;
            drop(state);
            let_go(events);
            Err(ControlBlock::ControlUnavailable)
        }
    }
}

/// The root area of `source` now, read on the input's own connection (merge
/// W2: the input endpoint of an X11 source, `x11_input_lane.rs`, checks the
/// geometry without the capture's connection).
pub(super) fn area_now(source: &SourceId) -> Result<Area, ControlBlock> {
    let mut state = state();
    if state.conn.is_none() {
        let (conn, screen) =
            RustConnection::connect(None).map_err(|_| ControlBlock::ControlUnavailable)?;
        if conn
            .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_none()
        {
            return Err(ControlBlock::UnsupportedOnHost);
        }
        let root = conn.setup().roots[screen].root;
        // Codex 10/16: spare keys a text before could not empty again.
        crate::screen::os::x11_keymap::restore_pending(&conn);
        state.conn = Some((conn, root));
    }
    let Some((conn, root)) = state.conn.as_ref() else {
        return Err(ControlBlock::ControlUnavailable);
    };
    match area_of(conn, *root, source) {
        Ok(area) => Ok(area),
        Err(_) => {
            // A broken connection is made anew with the next call.
            state.conn = None;
            Err(ControlBlock::ControlUnavailable)
        }
    }
}

/// Positions belong to a captured source, not a freshly relocated output.
/// Callers retain their source/Frame/PRESENTED proof across the worker queue.
pub fn inject_bound(
    source: &SourceId,
    area: Area,
    events: &[InputEvent],
) -> Result<(), ControlBlock> {
    let mut state = state();
    if state.conn.is_none() {
        let (conn, screen) =
            RustConnection::connect(None).map_err(|_| ControlBlock::ControlUnavailable)?;
        if conn
            .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_none()
        {
            return Err(ControlBlock::UnsupportedOnHost);
        }
        let root = conn.setup().roots[screen].root;
        // Codex 10/16: spare keys a text before could not empty again.
        crate::screen::os::x11_keymap::restore_pending(&conn);
        state.conn = Some((conn, root));
    }
    let State {
        sas,
        conn,
        wheel_rest,
        ..
    } = &mut *state;
    let Some((conn, root)) = conn.as_ref() else {
        return Err(ControlBlock::ControlUnavailable);
    };
    if area_of(conn, *root, source).map_err(|_| ControlBlock::ControlUnavailable)? != area {
        return Err(ControlBlock::ControlUnavailable);
    }
    for event in events {
        if let InputEvent::PointerAbs { x, y } | InputEvent::Touch { x, y, .. } = event {
            let px =
                i64::from(area.x) + i64::from(*x) * (i64::from(area.width) - 1).max(0) / 65_535;
            let py =
                i64::from(area.y) + i64::from(*y) * (i64::from(area.height) - 1).max(0) / 65_535;
            if i16::try_from(px).is_err() || i16::try_from(py).is_err() {
                return Err(ControlBlock::ControlUnavailable);
            }
        }
    }
    texts_typable(conn, events)?;
    match send(conn, *root, area, events, sas, wheel_rest) {
        Ok(0) => Ok(()),
        // Only when the mapping changed since `texts_typable`; the batch
        // went (its presses count as held), so it is no refusal.
        Ok(skipped) => {
            eprintln!("screen host process: {skipped} characters have no key on this host");
            Ok(())
        }
        Err(_) => {
            state.conn = None;
            drop(state);
            let_go(events);
            Err(ControlBlock::ControlUnavailable)
        }
    }
}

/// Codex 21 (review 2026-11-19): a batch whose text cannot go whole is
/// refused before any of it is sent, with a reason the viewer shows
/// (before, the characters without a key were left out and the batch
/// counted as sent).
fn texts_typable<C: Connection>(conn: &C, events: &[InputEvent]) -> Result<(), ControlBlock> {
    let mut texts = events.iter().filter_map(|event| match event {
        InputEvent::Text { text } => Some(text.as_str()),
        _ => None,
    });
    let Some(first) = texts.next() else {
        return Ok(());
    };
    match x11_keymap::typable(conn, std::iter::once(first).chain(texts)) {
        Ok(true) => Ok(()),
        Ok(false) => {
            eprintln!("screen host process: text with characters that have no key on this host");
            Err(ControlBlock::UnsupportedOnHost)
        }
        Err(_) => Err(ControlBlock::ControlUnavailable),
    }
}

/// Keep the host gate's admitted member geometry through the final XTest
/// boundary. A movement/resize after that gate is denied by `inject_bound`.
pub(in crate::screen::os) fn inject_x11_family(
    member: &crate::remote_window::CaptureMember,
    events: &[InputEvent],
    inject: impl FnOnce(&SourceId, Area, &[InputEvent]) -> Result<(), ControlBlock>,
) -> Result<(), ControlBlock> {
    if !member.input || !member.geometry.valid() {
        return Err(ControlBlock::ControlUnavailable);
    }
    let window = u32::try_from(member.binding.native.0)
        .ok()
        .filter(|id| *id > 1)
        .ok_or(ControlBlock::ControlUnavailable)?;
    let area = Area {
        x: member.geometry.x,
        y: member.geometry.y,
        width: i32::try_from(member.geometry.width)
            .map_err(|_| ControlBlock::ControlUnavailable)?,
        height: i32::try_from(member.geometry.height)
            .map_err(|_| ControlBlock::ControlUnavailable)?,
    };
    inject(&SourceId::Window(window.to_string()), area, events)
}

#[cfg(test)]
#[path = "x11_input_tests.rs"]
mod tests;
