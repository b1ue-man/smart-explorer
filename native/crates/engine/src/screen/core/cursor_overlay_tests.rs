//! Acceptance of the pointer overlay (plan 2026-11-14, package P6): the
//! frames, the negotiation, and the state inside the host process, the
//! daemon and the viewer.
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::screen::core::wire_control::{
    decode_host_frame, decode_viewer_frame, encode_host_frame, encode_viewer_frame, HostFrame,
    ViewerFrame,
};

fn shape(id: u32) -> CursorShape {
    CursorShape {
        id,
        width: 2,
        height: 3,
        hot_x: 1,
        hot_y: -1,
        shrink: 1,
        rgba: (0..24).collect(),
    }
}

fn position(shape_id: u32, x: i32) -> CursorPosition {
    CursorPosition {
        shape_id,
        visible: true,
        x,
        y: -7,
        area: FrameSize::new(1920, 1080),
    }
}

fn server(overlay: Option<u16>) -> ScreenServerHello {
    let mut hello = ScreenServerHello::new(
        "c".repeat(32),
        "host-device".into(),
        "host-key".into(),
        "host-fingerprint".into(),
        "host-node".into(),
    );
    hello.cursor_overlay = overlay;
    hello
}

#[test]
fn verbindungen_task_zeiger_frames_round_trip_and_refuse_bad_payloads() {
    let request = encode_viewer_frame(&ViewerFrame::CursorOverlay(true)).unwrap();
    assert_eq!(request, (CURSOR_REQUEST, vec![1]));
    assert_eq!(
        decode_viewer_frame(CURSOR_REQUEST, &[0]).unwrap(),
        ViewerFrame::CursorOverlay(false)
    );
    assert!(decode_viewer_frame(CURSOR_REQUEST, &[2]).is_err());
    assert!(decode_viewer_frame(CURSOR_REQUEST, &[]).is_err());

    let (tag, payload) = encode_host_frame(&HostFrame::CursorShape(shape(5))).unwrap();
    assert_eq!(tag, CURSOR_SHAPE);
    assert_eq!(
        decode_host_frame(tag, &payload).unwrap(),
        HostFrame::CursorShape(shape(5))
    );
    let (tag, payload) = encode_host_frame(&HostFrame::CursorPosition(position(5, -3))).unwrap();
    assert_eq!(tag, CURSOR_POSITION);
    assert_eq!(payload.len(), 21);
    assert_eq!(
        decode_host_frame(tag, &payload).unwrap(),
        HostFrame::CursorPosition(position(5, -3))
    );

    // A shape whose pixels do not match its size, a position of an empty
    // area and one with a flag other than 0 or 1 are protocol errors.
    let mut short = shape(5).encode().unwrap();
    short.pop();
    assert!(CursorShape::decode(&short).is_err());
    let mut empty_area = position(5, 0).encode();
    empty_area[13..17].copy_from_slice(&0u32.to_be_bytes());
    assert!(CursorPosition::decode(&empty_area).is_err());
    let mut flag = position(5, 0).encode();
    flag[4] = 2;
    assert!(CursorPosition::decode(&flag).is_err());
    // The frames of the other direction are refused.
    assert!(decode_host_frame(CURSOR_REQUEST, &[1]).is_err());
    assert!(decode_viewer_frame(CURSOR_SHAPE, &shape(5).encode().unwrap()).is_err());
}

#[test]
fn verbindungen_task_zeiger_update_frames_of_the_host_process_round_trip() {
    for update in [
        CursorUpdate::Shape(shape(2)),
        CursorUpdate::Position(position(2, 9)),
    ] {
        assert_eq!(
            decode_update(&encode_update(&update).unwrap()).unwrap(),
            update
        );
    }
    assert!(decode_update(&[]).is_err());
    assert!(decode_update(&[9, 0, 0]).is_err());
}

#[test]
fn verbindungen_task_zeiger_negotiated_only_for_a_full_view_of_an_announcing_host() {
    let announcing = server(Some(CURSOR_OVERLAY_V1));
    assert!(negotiated(&announcing, SessionMode::View, false));
    assert!(negotiated(&announcing, SessionMode::Control, false));
    assert!(!negotiated(&announcing, SessionMode::Preview, false));
    assert!(!negotiated(&announcing, SessionMode::Probe, false));
    assert!(
        !negotiated(&announcing, SessionMode::View, true),
        "a metadata channel"
    );
    // An older host announces nothing: the viewer never sends the frame.
    assert!(!negotiated(&server(None), SessionMode::View, false));
    assert!(!negotiated(&server(Some(2)), SessionMode::View, false));
    // The host's own hello announces it, and an older hello still parses.
    assert_eq!(
        ScreenServerHello::new(
            "c".repeat(32),
            "d".into(),
            "k".into(),
            "f".into(),
            "n".into()
        )
        .cursor_overlay,
        Some(CURSOR_OVERLAY_V1)
    );
    let mut json = serde_json::to_value(server(None)).unwrap();
    assert!(json.get("cursor_overlay").is_none());
    json["cursor_overlay"] = serde_json::json!(1);
    let parsed: ScreenServerHello = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.cursor_overlay, Some(1));
}

#[test]
fn verbindungen_task_zeiger_a_large_pointer_is_thinned_out_to_fit_a_frame() {
    // 300 x 300 would be 360 000 bytes: thinned by 3 to 100 x 100 (40 000).
    let big =
        CursorShape::sampled(1, 300, 300, 150, 90, |x, y| [x as u8, y as u8, 0, 255]).unwrap();
    assert_eq!((big.width, big.height, big.shrink), (100, 100, 3));
    assert_eq!((big.hot_x, big.hot_y), (50, 30));
    assert!(big.encode().unwrap().len() < 64 * 1024);
    // The pixel in the middle of each 3 x 3 block is kept.
    assert_eq!(&big.rgba[0..4], &[1, 1, 0, 255]);
    // A small pointer is kept as it is; an empty or giant one has no shape.
    let small = CursorShape::sampled(2, 32, 32, 0, 0, |_, _| [0, 0, 0, 255]).unwrap();
    assert_eq!((small.width, small.shrink), (32, 1));
    assert!(CursorShape::sampled(3, 0, 8, 0, 0, |_, _| [0; 4]).is_none());
    assert!(CursorShape::sampled(4, 600, 8, 0, 0, |_, _| [0; 4]).is_none());
    // XFixes pixels `0xAARRGGBB` become premultiplied RGBA bytes.
    let argb = CursorShape::from_argb(5, 1, 1, 0, 0, &[0x80FF_4020]).unwrap();
    assert_eq!(argb.rgba, vec![0xFF, 0x40, 0x20, 0x80]);
    assert!(CursorShape::from_argb(6, 2, 2, 0, 0, &[0]).is_none());
}

#[test]
fn verbindungen_task_zeiger_the_host_process_sends_only_changes_and_only_when_switched_on() {
    let cursor = WorkerCursor::new();
    // Nobody asked: the overlay stays off and the pointer stays in the picture.
    cursor.set_supported();
    assert!(!cursor.separated());
    cursor.publish_shape(10, Some(shape(1)));
    cursor.publish_position(4, 5, FrameSize::new(100, 50), true);
    assert!(!cursor.has_pending(), "off: nothing is queued for sending");
    assert!(cursor.take().is_empty());

    // Switching on repeats shape and position for the viewer that starts.
    assert!(cursor.set_separate(true));
    let first = cursor.take();
    assert!(
        matches!(first.as_slice(), [CursorUpdate::Shape(s), CursorUpdate::Position(p)]
        if s.id == 1 && p.shape_id == 1 && p.visible && (p.x, p.y) == (4, 5)),
        "{first:?}"
    );
    assert!(cursor.take().is_empty());
    // The same position again is no change; another one is.
    cursor.publish_position(4, 5, FrameSize::new(100, 50), true);
    assert!(!cursor.has_pending());
    cursor.publish_position(6, 5, FrameSize::new(100, 50), true);
    assert!(matches!(cursor.take().as_slice(), [CursorUpdate::Position(p)] if p.x == 6));
    // A pointer that cannot be sent hides the overlay pointer.
    cursor.publish_shape(11, None);
    cursor.publish_position(6, 5, FrameSize::new(100, 50), true);
    assert!(matches!(cursor.take().as_slice(), [CursorUpdate::Position(p)] if !p.visible));
    assert_eq!(cursor.shape_serial(), Some(11));
    // Switched off, the platform hook hears it and nothing is sent.
    let heard = Arc::new(Mutex::new(Vec::new()));
    let seen = heard.clone();
    cursor.set_hook(Box::new(move |on| seen.lock().unwrap().push(on)));
    assert!(!cursor.set_separate(false));
    assert_eq!(*heard.lock().unwrap(), vec![false]);
    cursor.publish_position(9, 9, FrameSize::new(100, 50), true);
    assert!(cursor.take().is_empty());
}

#[test]
fn verbindungen_task_zeiger_a_wish_before_the_capture_is_kept_until_it_can_separate() {
    let cursor = WorkerCursor::new();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let seen = heard.clone();
    cursor.set_hook(Box::new(move |on| seen.lock().unwrap().push(on)));
    // The viewer's wish arrives before the capture exists (Windows creates
    // its capture session with the first picture): the overlay is not on yet.
    assert!(!cursor.set_separate(true));
    assert!(!cursor.separated());
    assert_eq!(*heard.lock().unwrap(), vec![false]);
    // The capture starts and can separate the pointer: the wish takes effect.
    cursor.set_supported();
    assert!(cursor.separated());
    assert_eq!(*heard.lock().unwrap(), vec![false, true]);
    // A capture that starts again while the overlay is on switches nothing.
    cursor.set_supported();
    assert_eq!(heard.lock().unwrap().len(), 2);
    // A withdrawn wish stays withdrawn for the next capture.
    assert!(!cursor.set_separate(false));
    cursor.set_supported();
    assert!(!cursor.separated());
}

#[test]
fn verbindungen_task_zeiger_flush_writes_frames_the_daemon_reads_back() {
    let cursor = WorkerCursor::new();
    cursor.set_supported();
    cursor.publish_shape(1, Some(shape(3)));
    cursor.publish_position(1, 2, FrameSize::new(10, 10), true);
    assert!(cursor.set_separate(true));
    let mut wire = Vec::new();
    cursor.flush(&mut wire).unwrap();
    let feed = CursorFeed::default();
    let mut reader = io::Cursor::new(wire);
    while let Some((tag, payload)) =
        crate::screen::core::worker_wire::read_message(&mut reader).unwrap()
    {
        assert_eq!(tag, crate::screen::core::worker_wire::CURSOR);
        feed.publish(decode_update(&payload).unwrap());
    }
    let mut seen = CursorSeen::default();
    let polled = feed.poll(&mut seen);
    assert!(matches!(
        polled.as_slice(),
        [CursorUpdate::Shape(_), CursorUpdate::Position(_)]
    ));
}

#[test]
fn verbindungen_task_zeiger_the_feed_gives_each_change_once_to_each_reader() {
    let feed = CursorFeed::default();
    let (mut first, mut second) = (CursorSeen::default(), CursorSeen::default());
    assert!(feed.poll(&mut first).is_empty());
    feed.publish(CursorUpdate::Shape(shape(1)));
    feed.publish(CursorUpdate::Position(position(1, 1)));
    feed.publish(CursorUpdate::Position(position(1, 2)));
    let polled = feed.poll(&mut first);
    assert!(
        matches!(polled.as_slice(), [CursorUpdate::Shape(_), CursorUpdate::Position(p)] if p.x == 2)
    );
    assert!(feed.poll(&mut first).is_empty(), "taken once");
    assert_eq!(
        feed.poll(&mut second).len(),
        2,
        "another reader has its own place"
    );
    // After a reset a reader gets nothing stale; the next publication arrives.
    feed.reset();
    assert!(feed.poll(&mut first).is_empty());
    feed.publish(CursorUpdate::Position(position(1, 3)));
    assert!(matches!(feed.poll(&mut first).as_slice(), [CursorUpdate::Position(p)] if p.x == 3));
}

/// The session sends a pointer change on when it arrives, not at its next
/// picture tick: the feed wakes its reader for every publication, also for
/// one that came before the reader waited.
#[tokio::test]
async fn verbindungen_task_zeiger_a_publication_wakes_the_reader_of_the_feed() {
    let feed = Arc::new(CursorFeed::default());
    let quiet = tokio::time::timeout(Duration::from_millis(50), feed.changed()).await;
    assert!(
        quiet.is_err(),
        "nothing published: the reader keeps waiting"
    );
    // The reader thread of the host process publishes from outside the runtime.
    let publisher = feed.clone();
    std::thread::spawn(move || {
        publisher.publish(CursorUpdate::Shape(shape(1)));
        publisher.publish(CursorUpdate::Position(position(1, 1)));
    })
    .join()
    .expect("publisher thread");
    tokio::time::timeout(Duration::from_secs(10), feed.changed())
        .await
        .expect("the publication wakes the reader");
    let mut seen = CursorSeen::default();
    assert_eq!(
        feed.poll(&mut seen).len(),
        2,
        "shape and position are there"
    );
    let again = tokio::time::timeout(Duration::from_millis(50), feed.changed()).await;
    assert!(again.is_err(), "one wake for what was published");
}

#[test]
fn verbindungen_task_zeiger_the_view_shows_only_a_visible_pointer_of_the_held_shape() {
    let mut view = CursorView::default();
    assert!(view.visible().is_none());
    view.set_position(position(1, 0));
    assert!(view.visible().is_none(), "no shape yet");
    view.set_shape(shape(1));
    let (held, at) = view.visible().expect("shape and position belong together");
    assert_eq!((held.id, at.x), (1, 0));
    view.set_position(position(2, 0));
    assert!(view.visible().is_none(), "a position of another shape");
    let mut hidden = position(1, 0);
    hidden.visible = false;
    view.set_position(hidden);
    assert!(view.visible().is_none());
    // The host's "nothing to draw" (after a request, after a change of the
    // source) travels as a position frame and hides the held pointer.
    view.set_position(position(1, 0));
    assert!(view.visible().is_some());
    let (tag, payload) =
        encode_host_frame(&HostFrame::CursorPosition(CursorPosition::hidden())).unwrap();
    let HostFrame::CursorPosition(nothing) = decode_host_frame(tag, &payload).unwrap() else {
        panic!("a position frame");
    };
    assert_eq!(nothing, CursorPosition::hidden());
    view.set_position(nothing);
    assert!(view.visible().is_none());
}

/// A text check that runs on every platform (as `windows_static_tests.rs`
/// does for the other Windows sources): the Windows capture session draws
/// the pointer only while the overlay is off, the overlay's switch reaches
/// that session, and a Windows without that switch never offers the overlay.
#[test]
fn verbindungen_task_zeiger_windows_capture_stops_drawing_the_pointer_with_the_overlay() {
    let session = include_str!("../os/windows/capture_retirement.rs");
    assert!(
        session.contains("super::cursor_overlay::attach(&session);"),
        "every capture session is handed to the overlay"
    );
    assert!(
        !session.contains("SetIsCursorCaptureEnabled("),
        "no fixed pointer capture is left beside the overlay's switch"
    );
    let overlay = include_str!("../os/windows/cursor_overlay.rs");
    let starts = overlay
        .find("SetIsCursorCaptureEnabled(!overlay.separated())")
        .expect("a new session starts with the pointer the overlay leaves to the capture");
    let offers = overlay
        .find("overlay.set_supported();")
        .expect("the capture offers the overlay");
    assert!(
        starts < offers,
        "the overlay is offered only after the session took the pointer switch"
    );
    assert!(
        overlay.contains("SetIsCursorCaptureEnabled(!on)"),
        "switching the overlay switches the pointer capture of the session"
    );
    assert!(
        overlay.contains("GetCursorInfo"),
        "shape and position come from the system"
    );
    // The DXGI path is not touched (it has no pointer in its pictures).
    assert!(!include_str!("../os/windows/capture_dxgi.rs").contains("cursor_overlay"));
}
