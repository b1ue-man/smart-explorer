//! R15c: a `WINDOWS` list has no entry count (formerly 4096); only the frame
//! size of protocol version 1 bounds it.
use super::*;
use crate::screen::core::wire::{MAX_WINDOWS_BYTES, WINDOWS};
use crate::screen::core::wire_control::{decode_host_frame, encode_host_frame, HostFrame};

fn app(index: usize) -> AppEntry {
    AppEntry {
        app_id: format!("org.example.app{index}"),
        name: format!("App {index}"),
        icon: None,
        pid: Some(1000),
    }
}

fn window(index: usize) -> WindowEntry {
    WindowEntry {
        id: format!("{index:x}"),
        title: format!("Fenster {index}"),
        app: "editor".into(),
        app_id: None,
        icon: None,
        pid: Some(7),
        monitor: Some("1".into()),
        minimized: false,
        focused: false,
    }
}

/// More than the former 4096 entries reach the viewer whole, as long as
/// the frame holds them.
#[test]
fn screen_task_window_list_keeps_more_than_4096_entries() {
    let list = WindowList {
        windows: (0..3000).map(window).collect(),
        apps: (0..3000).map(app).collect(),
        list_source: ListSource::UsageStats,
        limits: Vec::new(),
    };
    let bounded = list.clone().bounded();
    assert_eq!(bounded.windows.len(), 3000);
    assert_eq!(bounded.apps.len(), 3000);
    bounded.validate().unwrap();
    let (tag, payload) = encode_host_frame(&HostFrame::Windows(bounded)).unwrap();
    assert_eq!(tag, WINDOWS);
    assert!(payload.len() <= MAX_WINDOWS_BYTES);
    match decode_host_frame(tag, &payload).unwrap() {
        HostFrame::Windows(received) => assert_eq!(received, list),
        other => panic!("unexpected frame {other:?}"),
    }
}

/// A list larger than the frame is bounded by the frame's bytes, not by a
/// count; a foreign peer's oversized payload stays refused by the frame.
#[test]
fn screen_task_window_list_frame_bytes_bound_the_list() {
    let huge = WindowList {
        windows: Vec::new(),
        apps: (0..60_000).map(app).collect(),
        list_source: ListSource::SystemdUnits,
        limits: Vec::new(),
    };
    let bounded = huge.bounded();
    assert!(bounded.apps.len() > 4096, "kept {}", bounded.apps.len());
    assert!(bounded.apps.len() < 60_000);
    let (_, payload) = encode_host_frame(&HostFrame::Windows(bounded)).unwrap();
    assert!(payload.len() <= MAX_WINDOWS_BYTES);
}

/// R15c: titles, app names and limit notes arrive whole (formerly cut at
/// 128 and 64 bytes), and more than the former 16 limit notes pass.
#[test]
fn screen_task_window_list_keeps_whole_texts_and_all_limit_notes() {
    let long_window = WindowEntry {
        title: "Ü".repeat(400),
        app: "a".repeat(300),
        ..window(1)
    };
    let long_app = AppEntry {
        app_id: format!("org.example.{}", "x".repeat(300)),
        name: "Ä".repeat(300),
        icon: None,
        pid: None,
    };
    let limits: Vec<String> = (0..40)
        .map(|index| format!("note_{index}_{}", "n".repeat(100)))
        .collect();
    let list = WindowList {
        windows: vec![long_window],
        apps: vec![long_app],
        list_source: ListSource::Ewmh,
        limits,
    };
    let bounded = list.clone().bounded();
    assert_eq!(bounded, list);
    let (tag, payload) = encode_host_frame(&HostFrame::Windows(bounded)).unwrap();
    match decode_host_frame(tag, &payload).unwrap() {
        HostFrame::Windows(received) => assert_eq!(received, list),
        other => panic!("unexpected frame {other:?}"),
    }
}

/// The frame still holds: one entry larger than the frame is dropped whole
/// (not cut), and a peer's window id outside the source grammar is refused.
#[test]
fn screen_task_window_list_drops_oversized_entry_and_refuses_bad_ids() {
    let giant = WindowEntry {
        title: "t".repeat(MAX_WINDOWS_BYTES),
        ..window(2)
    };
    let bounded = WindowList {
        windows: vec![window(1), giant, window(3)],
        apps: vec![app(1)],
        list_source: ListSource::EnumWindows,
        limits: Vec::new(),
    }
    .bounded();
    assert_eq!(bounded.windows, vec![window(1)]);
    // Windows stop at the first one that does not fit; apps fit on their own.
    assert_eq!(bounded.apps, vec![app(1)]);
    let (_, payload) = encode_host_frame(&HostFrame::Windows(bounded)).unwrap();
    assert!(payload.len() <= MAX_WINDOWS_BYTES);

    let bad = br#"{"windows":[{"id":"has space","title":"","app":""}],"list_source":"ewmh"}"#;
    let error = decode_host_frame(WINDOWS, bad).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}
