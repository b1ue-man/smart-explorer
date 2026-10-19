//! XTest input on an X server (SCR-M6): positions, a click and text reach a
//! window of this test, and a character the layout lacks comes through a
//! spare keycode that is empty again afterwards. Both need a display with
//! XTest (`fixture=xvfb`), so they are ignored outside that stage.
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as _, CreateWindowAux, EventMask, InputFocus, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use super::*;

#[test]
fn screen_task_x11_root_points_cover_the_source() {
    let area = Area {
        x: 1920,
        y: 0,
        width: 1280,
        height: 1024,
    };
    assert_eq!(root_point(area, 0, 0), (1920, 0));
    assert_eq!(root_point(area, 65_535, 65_535), (3199, 1023));
    assert_eq!(root_point(area, 32_768, 32_768), (2559, 511));
}

/// A mapped, focused window that receives keys and buttons.
fn window() -> (RustConnection, Window, Window) {
    let (conn, screen) = RustConnection::connect(None).expect("DISPLAY with XTest");
    let root = conn.setup().roots[screen].root;
    let window = conn.generate_id().unwrap();
    let aux = CreateWindowAux::new()
        .event_mask(EventMask::KEY_PRESS | EventMask::BUTTON_PRESS)
        .background_pixel(conn.setup().roots[screen].white_pixel);
    conn.create_window(
        0,
        window,
        root,
        50,
        50,
        300,
        200,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &aux,
    )
    .unwrap();
    conn.map_window(window).unwrap();
    conn.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    conn.set_input_focus(InputFocus::PARENT, window, 0u32)
        .unwrap();
    conn.flush().unwrap();
    (conn, root, window)
}

/// The keysyms of the key presses and the buttons that arrived.
fn arrived(conn: &RustConnection) -> (Vec<u32>, Vec<u8>) {
    let map = x11_keymap::KeyMap::read(conn).unwrap();
    let per = usize::from(map.per_code);
    let mut keys = Vec::new();
    let mut buttons = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match conn.poll_for_event().unwrap() {
            Some(Event::KeyPress(press)) => {
                let index = usize::from(press.detail - map.min) * per;
                keys.push(map.syms[index]);
            }
            Some(Event::ButtonPress(press)) => buttons.push(press.detail),
            Some(_) => {}
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    (keys, buttons)
}

#[test]
#[ignore = "needs an X server with XTest (fixture=xvfb)"]
fn screen_task_x11_xtest_text_and_click() {
    let (conn, root, window) = window();
    set_target(&SourceId::Window(x11_windows::window_id(window)));
    let events = [
        InputEvent::PointerAbs {
            x: 32_768,
            y: 32_768,
        },
        InputEvent::Button {
            button: PointerButton::Left,
            down: true,
        },
        InputEvent::Button {
            button: PointerButton::Left,
            down: false,
        },
        InputEvent::Text { text: "ab".into() },
    ];
    assert_eq!(inject(&events), Ok(()));
    let (keys, buttons) = arrived(&conn);
    assert_eq!(buttons, [1]);
    assert_eq!(keys, [0x61, 0x62]);
    let _ = root;
}

#[test]
#[ignore = "needs an X server with XTest (fixture=xvfb)"]
fn screen_task_x11_unmapped_char_via_spare_keycode() {
    let (conn, root, window) = window();
    let before = x11_keymap::KeyMap::read(&conn).unwrap();
    let spare = before.spare().expect("the layout has a free keycode");
    // U+01C5 has no key on the default layouts.
    let sym = x11_keymap::keysym_of('ǅ');
    assert!(before.find(sym).is_none());
    let skipped = x11_keymap::type_text(&conn, root, "ǅ").unwrap();
    assert_eq!(skipped, 0);
    let mut pressed = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match conn.poll_for_event().unwrap() {
            Some(Event::KeyPress(press)) => pressed.push(press.detail),
            Some(_) => {}
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    assert_eq!(pressed, [spare]);
    // The spare keycode is empty again.
    let after = x11_keymap::KeyMap::read(&conn).unwrap();
    assert_eq!(after.spare(), Some(spare));
    let _ = window;
}
