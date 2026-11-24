//! Releases on an X11 host (split from `x11_input.rs`, round 13): the
//! session's compensating ups (`release`) and the presses of a batch whose
//! sending failed partway (`let_go`, round 13, S3). The X server carries out
//! each fake input it took even when a later one of the same batch fails, so
//! a batch that pressed a key and failed before (or at) its release left the
//! key down while the session, counting only the calls that went whole
//! (`input_calls.rs`, `Injection::went`), never let go of it. XTest drops a
//! release of a key or button that is not down, so letting go of a press
//! that never took effect changes nothing.
use std::io;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::{button_code, state, State, BUTTON_RELEASE};
use crate::screen::core::input_model::{InputEvent, PointerButton, TouchPhase};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::os::input_keys::evdev_code;
use crate::screen::os::x11_keymap;

/// Session-owned compensating ups do not need an existing source window or
/// root-space pointer position, and never emit MotionNotify on touch cancel.
pub fn release(events: &[InputEvent]) -> Result<(), ControlBlock> {
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
    let State { sas, conn, .. } = &mut *state;
    let Some((conn, root)) = conn.as_ref() else {
        return Err(ControlBlock::ControlUnavailable);
    };
    let sent = (|| {
        for event in &releases {
            let (kind, detail) = match event {
                InputEvent::Button { button, .. } => (BUTTON_RELEASE, button_code(*button)),
                InputEvent::Key { hid_usage, .. } => {
                    if !sas.allows(*hid_usage, false) {
                        continue;
                    }
                    let Some(code) =
                        evdev_code(*hid_usage).and_then(|code| u8::try_from(code + 8).ok())
                    else {
                        continue;
                    };
                    (x11_keymap::KEY_RELEASE, code)
                }
                _ => return Err(io::Error::other("non_release_event")),
            };
            // Check the server response before dropping the pending held state.
            conn.xtest_fake_input(kind, detail, 0, *root, 0, 0, 0)
                .map_err(io::Error::other)?
                .check()
                .map_err(io::Error::other)?;
        }
        Ok::<_, io::Error>(())
    })();
    if sent.is_err() {
        state.conn = None;
        return Err(ControlBlock::ControlUnavailable);
    }
    Ok(())
}

/// The releases of what `events` leaves pressed, in the order of the
/// presses: keys and buttons pressed and not released after, the left button
/// of a first finger that started and did not end.
pub(super) fn left_pressed(events: &[InputEvent]) -> Vec<InputEvent> {
    let mut held: Vec<InputEvent> = Vec::new();
    for event in events {
        let (release, down) = match event {
            InputEvent::Key { hid_usage, down } => (
                InputEvent::Key {
                    hid_usage: *hid_usage,
                    down: false,
                },
                *down,
            ),
            InputEvent::Button { button, down } => (
                InputEvent::Button {
                    button: *button,
                    down: false,
                },
                *down,
            ),
            InputEvent::Touch {
                id: 0,
                phase: TouchPhase::Move,
                ..
            } => continue,
            InputEvent::Touch { id: 0, phase, .. } => (
                InputEvent::Button {
                    button: PointerButton::Left,
                    down: false,
                },
                matches!(phase, TouchPhase::Start),
            ),
            _ => continue,
        };
        held.retain(|known| *known != release);
        if down {
            held.push(release);
        }
    }
    held
}

/// Round 13 (S3): the sending of `events` failed partway; what it pressed
/// and did not release goes up again (on a new connection, the one that
/// failed is dropped), so no key stays down that the session does not know
/// as held. A failure here is named in the host log.
pub(super) fn let_go(events: &[InputEvent]) {
    let releases = left_pressed(events);
    if releases.is_empty() {
        return;
    }
    if release(&releases).is_err() {
        eprintln!("screen host process: keys of a failed input batch could not be let go");
    }
}
