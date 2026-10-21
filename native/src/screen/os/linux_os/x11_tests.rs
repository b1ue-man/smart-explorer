//! Acceptance of the X11 path (SCR-M5): hash, pointer blending, ids and
//! `WM_CLASS` as units; capture, window lists and a capture measurement
//! against a real server (`linux-desktop: screen-x11`, Xvfb with `openbox`
//! or without a window manager, `SE_SCREEN_TASK_X11=1`).
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, PropMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::*;
use crate::screen::core::host_backend::{ScreenEncoder, TilesScreenEncoder};
use crate::screen::core::window_list::{ListSource, MonitorInfo};
use crate::screen::os::x11_cursor::{blend, CursorImage};
use crate::screen::os::{x11_monitors, x11_windows};

#[test]
fn x11_picture_hash_follows_fnv1a_over_words() {
    assert_eq!(picture_hash(&[]), 0xCBF2_9CE4_8422_2325);
    let word = 0x0102_0304_0506_0708u64;
    let expected = (0xCBF2_9CE4_8422_2325u64 ^ word).wrapping_mul(0x0000_0100_0000_01B3);
    assert_eq!(picture_hash(&word.to_le_bytes()), expected);
    // The tail is zero-padded to a word.
    assert_eq!(
        picture_hash(&[1, 2, 3]),
        picture_hash(&[1, 2, 3, 0, 0, 0, 0, 0])
    );
    assert_ne!(picture_hash(&[1, 2, 3]), picture_hash(&[1, 2, 4]));
}

#[test]
fn x11_pointer_is_blended_premultiplied_and_clipped() {
    let size = FrameSize::new(4, 4);
    let mut picture = vec![100u8; size.bgra_len()];
    // 2 x 1 cursor: an opaque red pixel and a half-transparent premultiplied
    // white one; its top-left corner is at (1, 1) on the root.
    let cursor = CursorImage {
        x: 1,
        y: 1,
        width: 2,
        height: 1,
        argb: vec![0xFFFF_0000, 0x8080_8080],
    };
    blend(&mut picture, size, 0, 0, &cursor);
    let at = |x: usize, y: usize| &picture[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
    assert_eq!(at(1, 1), &[0, 0, 255, 255]);
    // 0x80 + 100 * 127 / 255 = 177.8 -> 178
    assert_eq!(at(2, 1), &[178, 178, 178, 255]);
    assert_eq!(at(0, 0), &[100, 100, 100, 100]);
    // An area whose corner is at (2, 0): the cursor starts at -1, -1 -> its
    // first column is outside, nothing is drawn above the area.
    let mut moved = vec![100u8; size.bgra_len()];
    blend(&mut moved, size, 2, 0, &cursor);
    assert_eq!(&moved[0..4], &[100, 100, 100, 100]);
    // A cursor wholly outside draws nothing.
    let mut outside = vec![7u8; size.bgra_len()];
    blend(&mut outside, size, 100, 100, &cursor);
    assert!(outside.iter().all(|byte| *byte == 7));
}

#[test]
fn x11_ids_and_classes() {
    assert_eq!(x11_monitors::monitor_id("HDMI-A-1"), "HDMI-A-1");
    assert_eq!(x11_monitors::monitor_id("DP 1\u{e4}"), "DP_1_");
    assert_eq!(x11_monitors::monitor_id(""), "monitor");
    assert_eq!(x11_monitors::monitor_id(&"x".repeat(100)).len(), 64);
    assert_eq!(x11_windows::parse_window_id("12345"), Some(12345));
    assert_eq!(x11_windows::parse_window_id("0x10"), None);
    assert_eq!(x11_windows::window_id(77), "77");
    assert_eq!(
        x11_windows::app_of_wm_class(b"navigator\0Firefox\0"),
        "Firefox"
    );
    assert_eq!(x11_windows::app_of_wm_class(b"xterm\0\0"), "xterm");
    assert_eq!(x11_windows::app_of_wm_class(b""), "");
    let monitors = vec![
        MonitorInfo {
            id: "A".into(),
            name: "A".into(),
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            primary: true,
        },
        MonitorInfo {
            id: "B".into(),
            name: "B".into(),
            x: 100,
            y: 0,
            width: 100,
            height: 100,
            primary: false,
        },
    ];
    assert_eq!(x11_windows::monitor_at(&monitors, 99, 50), Some("A".into()));
    assert_eq!(
        x11_windows::monitor_at(&monitors, 100, 50),
        Some("B".into())
    );
    assert_eq!(x11_windows::monitor_at(&monitors, 200, 50), None);
}

fn x11_stage() {
    assert!(
        std::env::var("SE_SCREEN_TASK_X11").is_ok_and(|value| value == "1"),
        "SE_SCREEN_TASK_X11=1 is not set"
    );
}

/// A window of one color with a title and class, mapped on the root.
fn show_window(
    conn: &RustConnection,
    root: Window,
    (x, y, width, height): (i16, i16, u16, u16),
    color: u32,
    title: &str,
) -> Window {
    let window = conn.generate_id().unwrap();
    let aux = CreateWindowAux::new().background_pixel(color);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        x,
        y,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &aux,
    )
    .unwrap();
    let utf8 = conn
        .intern_atom(false, b"UTF8_STRING")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let net_name = conn
        .intern_atom(false, b"_NET_WM_NAME")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let net_pid = conn
        .intern_atom(false, b"_NET_WM_PID")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let name = title.as_bytes();
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        name,
    )
    .unwrap();
    conn.change_property8(PropMode::REPLACE, window, net_name, utf8, name)
        .unwrap();
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"setask\0SeTask\0",
    )
    .unwrap();
    conn.change_property32(
        PropMode::REPLACE,
        window,
        net_pid,
        AtomEnum::CARDINAL,
        &[std::process::id()],
    )
    .unwrap();
    conn.map_window(window).unwrap();
    conn.flush().unwrap();
    window
}

/// Connects to the server of `DISPLAY`. The stage's fixture waits for the
/// socket of Xvfb, not for an accepted connection, and a server that has just
/// served a capture loop may reset one setup: the connection is tried again
/// for ten seconds before the check calls the server missing.
fn connect() -> (RustConnection, Window, usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let (conn, screen) = loop {
        match RustConnection::connect(None) {
            Ok(connected) => break connected,
            Err(error) if Instant::now() >= deadline => {
                panic!("an X server on DISPLAY: {error:?}")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let root = conn.setup().roots[screen].root;
    (conn, root, screen)
}

fn pixel(image: &BgraImage, x: usize, y: usize) -> [u8; 4] {
    let at = (y * image.size().width as usize + x) * 4;
    image.data()[at..at + 4].try_into().unwrap()
}

/// Captures until `done` accepts the picture.
fn capture_until(source: &mut X11Source, mut done: impl FnMut(&BgraImage) -> bool) -> BgraImage {
    let started = Instant::now();
    loop {
        if let SourceFrame::Frame(image) = source.capture(1, true).unwrap() {
            if done(&image) {
                return image;
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the picture never matched"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "needs an X server (Xvfb) on DISPLAY (SE_SCREEN_TASK_X11=1)"]
fn screen_task_x11_snapshot_matches() {
    x11_stage();
    let (conn, root, _) = connect();
    let window = show_window(
        &conn,
        root,
        (100, 100, 200, 150),
        0x00FF_8000,
        "se-snapshot",
    );
    // Color 0x00FF8000 is red 255, green 128, blue 0: BGRA 00 80 FF FF.
    let expected = [0x00, 0x80, 0xFF, 0xFF];
    let monitors = x11_monitors::list(&conn, root).unwrap();
    assert!(monitors.iter().any(|monitor| monitor.primary));
    let mut monitor = X11Source::open(&SourceId::Monitor(monitors[0].id.clone()))
        .ok()
        .unwrap();
    assert_eq!(
        monitor.size(),
        FrameSize::new(monitors[0].width, monitors[0].height)
    );
    let image = capture_until(&mut monitor, |image| pixel(image, 150, 150) == expected);
    assert_eq!(image.size(), monitor.size());
    // The window alone: every pixel has the color.
    let mut single = X11Source::open(&SourceId::Window(window.to_string()))
        .ok()
        .unwrap();
    assert_eq!(single.size(), FrameSize::new(200, 150));
    let shot = capture_until(&mut single, |image| pixel(image, 100, 75) == expected);
    assert!((0..150)
        .step_by(7)
        .all(|y| (0..200).step_by(7).all(|x| pixel(&shot, x, y) == expected)));
    // Unchanged without a key frame, the same picture with one.
    assert!(matches!(
        single.capture(2, false).unwrap(),
        SourceFrame::Frame(_) | SourceFrame::Unchanged
    ));
    assert!(matches!(
        single.capture(3, false).unwrap(),
        SourceFrame::Unchanged
    ));
    assert!(matches!(
        single.capture(4, true).unwrap(),
        SourceFrame::Frame(_)
    ));
    // A window that is gone: not found, the session ends with the text.
    conn.destroy_window(window).unwrap();
    conn.flush().unwrap();
    let error = loop {
        match single.capture(5, true) {
            Err(error) => break error,
            Ok(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(error.to_string(), "Das Fenster existiert nicht mehr");
}

#[test]
#[ignore = "needs Xvfb with openbox or without a window manager (SE_SCREEN_TASK_X11=1, SE_SCREEN_TASK_X11_WM=ewmh|none)"]
fn screen_task_x11_window_list_ewmh_and_querytree() {
    x11_stage();
    let manager = std::env::var("SE_SCREEN_TASK_X11_WM").expect("SE_SCREEN_TASK_X11_WM");
    let expected = match manager.as_str() {
        "ewmh" => ListSource::Ewmh,
        "none" => ListSource::X11QueryTree,
        other => panic!("SE_SCREEN_TASK_X11_WM must be ewmh or none, not {other}"),
    };
    let (conn, root, screen) = connect();
    let title = format!("se-task-window-{}", std::process::id());
    let window = show_window(&conn, root, (10, 10, 120, 90), 0x0000_FF00, &title);
    let started = Instant::now();
    let entry = loop {
        let list = x11_windows::list_on(&conn, root, screen).unwrap();
        if let Some(entry) = list.windows.iter().find(|entry| entry.title == title) {
            assert_eq!(list.list_source, expected);
            break entry.clone();
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the window is not listed"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(entry.id, window.to_string());
    assert_eq!(entry.app, "SeTask");
    assert!(!entry.minimized);
    if expected == ListSource::Ewmh {
        assert_eq!(entry.pid, Some(std::process::id()));
    }
    assert!(entry.monitor.is_some());
}

#[test]
#[ignore = "needs an X server (Xvfb) on DISPLAY (SE_SCREEN_TASK_X11=1)"]
fn screen_task_x11_bench_real_capture() {
    x11_stage();
    let (conn, root, _) = connect();
    let monitors = x11_monitors::list(&conn, root).unwrap();
    let mut source = X11Source::open(&SourceId::Monitor(monitors[0].id.clone()))
        .ok()
        .unwrap();
    let mut encoder = TilesScreenEncoder::new(source.size()).unwrap();
    let started = Instant::now();
    let mut frames = 0u32;
    while started.elapsed() < Duration::from_secs(3) {
        if let SourceFrame::Frame(image) = source.capture(u64::from(frames), true).unwrap() {
            encoder.encode(&image, true, 0).unwrap();
            frames += 1;
        }
    }
    let rate = f64::from(frames) / started.elapsed().as_secs_f64();
    println!(
        "x11 capture+tiles: {rate:.1} pictures/s at {:?}",
        source.size()
    );
    // Threshold of assumption A1 until the first remote measurement.
    assert!(rate >= 10.0, "{rate:.1} pictures/s");
}
