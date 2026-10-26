//! Acceptance of the desktop viewer (SCR-M7, U25-14): the switch between
//! group and full view over one connection, the tile under „Fragen“, the
//! picture groups of the decode thread and the release of the controls. The
//! host is a stand-in (`screen_viewer_fake`), so nothing needs a network.
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui;

use super::screen_decode_thread::DecodeState;
use super::screen_devices::{Devices, GridScope};
use super::screen_input_map::{Geometry, InputMapper};
use super::screen_pool::{full_request, to_tile, to_view};
use super::screen_prefs::ViewerPrefs;
use super::screen_session::{DecoderOpener, PictureData, Shared, ViewerSession};
use super::screen_tile_face::Face;
use super::screen_tile_grid::ScreenTileGrid;
use super::screen_toolbar_ui::ToolbarAction;
use super::screen_viewer_fake::{
    hello, open, open_preview, profiles, set_modes, target, wait_for, Fake, DISPLAY,
};
use super::screen_viewer_view::{Common, ViewState};
use crate::daemon::StreamEvent;
use crate::screen::core::host_backend::{
    ScreenDecoder, ScreenEncoder, TilesScreenDecoder, TilesScreenEncoder,
};
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::synthetic::render;
use crate::screen::core::wire_binary::{
    FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME, FLAG_MORE,
};
use crate::screen::core::wire_control::{ControlAction, KeyframeReason, StreamMode, ViewerFrame};
use crate::screen::core::wire_hello::SessionMode;
use crate::share::access_grant::CapabilityDecision;
use crate::share::ShareStatus;

#[test]
fn screen_task_grid_switch_reuses_connection() {
    // Allowed without asking: the tile's connection becomes the full view
    // and goes back to its tile; the host accepts it once.
    let fake = Fake::new(CapabilityDecision::Allow);
    let backend = fake.backend();
    let preview = open_preview(&fake, &backend);
    fake.accept(0);
    wait_for("the host's accept", || preview.info().accepted == 1);
    let view = to_view(&backend, Some(preview), &target(), &open());
    assert_eq!(fake.opens(), 1, "the full view opened no second connection");
    assert_eq!(view.mode, SessionMode::View);
    wait_for("SET_MODE full", || set_modes(&fake) == [StreamMode::Full]);
    assert_eq!(view.info().accepted, 1);
    assert!(!fake.closed(0));
    let tile = to_tile(view, 320, 1000).expect("the connection stays for the tile");
    assert_eq!(tile.mode, SessionMode::Preview);
    wait_for("SET_MODE preview", || {
        set_modes(&fake) == [StreamMode::Full, StreamMode::Preview]
    });
    assert_eq!(fake.opens(), 1);

    // Under „Fragen“ the preview has no picture: the full view is a new
    // connection, for which the host asks; the preview ends.
    let ask = Fake::new(CapabilityDecision::Ask);
    let backend = ask.backend();
    let preview = open_preview(&ask, &backend);
    ask.accept(0);
    wait_for("the host's accept", || preview.info().accepted == 1);
    let view = to_view(&backend, Some(preview), &target(), &open());
    wait_for("the second connection", || ask.opens() == 2);
    assert_eq!(view.mode, SessionMode::View);
    wait_for("the end of the preview", || ask.closed(0));
    assert!(set_modes(&ask).is_empty(), "no SET_MODE under Ask");
    let requests = ask.inner.requests.lock().unwrap().clone();
    assert_eq!(requests[0].mode, SessionMode::Preview);
    assert_eq!(requests[1].mode, SessionMode::View);
    assert_eq!(requests[1].display_size, Some(DISPLAY));
    // Back to the group the full view ends instead of becoming a preview.
    ask.accept(1);
    wait_for("the second accept", || view.info().accepted == 1);
    assert!(to_tile(view, 320, 1000).is_none());
    wait_for("the end of the full view", || ask.closed(1));

    // A wish for control is a session of its own (only the mode `control`
    // may take it), whatever the tile had.
    let again = Fake::new(CapabilityDecision::Allow);
    let backend = again.backend();
    let preview = open_preview(&again, &backend);
    again.accept(0);
    wait_for("the host's accept", || preview.info().accepted == 1);
    let mut wish = target();
    wish.control = true;
    let view = to_view(&backend, Some(preview), &wish, &open());
    wait_for("the control session", || again.opens() == 2);
    assert_eq!(view.mode, SessionMode::Control);
    assert_eq!(
        full_request(&backend.offers, None, DISPLAY, true).mode,
        SessionMode::Control
    );
}

fn frame(ctx: &egui::Context, grid: &mut ScreenTileGrid) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            grid.show(ui);
        });
    });
}

/// Runs frames until the first tile shows something other than `Probing`.
fn settle(grid: &mut ScreenTileGrid) -> Face {
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        frame(&ctx, grid);
        let face = grid.faces().first().cloned();
        if let Some(face) = face.filter(|face| *face != Face::Probing) {
            return face;
        }
        assert!(Instant::now() < deadline, "the tile never left the probe");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn screen_task_viewer_ask_tile_has_no_preview() {
    // „Fragen“: the tile offers „Ansehen anfragen“ and opens no session.
    let fake = Fake::new(CapabilityDecision::Ask);
    let devices = Devices::fixed(profiles(ShareStatus::Available, Some("device-c1")));
    let mut grid = ScreenTileGrid::new(GridScope::AllDevices, fake.backend(), devices.clone());
    assert_eq!(settle(&mut grid), Face::Ask);
    let ctx = egui::Context::default();
    for _ in 0..5 {
        frame(&ctx, &mut grid);
    }
    assert_eq!(fake.opens(), 0);
    assert_eq!(fake.inner.probes.load(Ordering::SeqCst), 1, "one probe");

    // „Aus“: nothing to show, no session either.
    let off = Fake::new(CapabilityDecision::Off);
    let mut grid = ScreenTileGrid::new(GridScope::AllDevices, off.backend(), devices.clone());
    assert_eq!(settle(&mut grid), Face::NotShared);
    assert_eq!(off.opens(), 0);

    // „Erlauben“: the tile streams a preview, one picture per interval.
    let allow = Fake::new(CapabilityDecision::Allow);
    let mut grid = ScreenTileGrid::new(GridScope::AllDevices, allow.backend(), devices);
    assert!(matches!(settle(&mut grid), Face::Waiting(_)));
    // The tile opens its session in the frame after the probe answered, so
    // frames keep running while the test waits.
    wait_for("the preview session", || {
        frame(&ctx, &mut grid);
        allow.opens() == 1
    });
    let request = allow.inner.requests.lock().unwrap()[0].clone();
    assert_eq!(request.mode, SessionMode::Preview);
    assert_eq!(request.max_fps, 1);

    // A device that is not online is not connected to.
    let gone = Fake::new(CapabilityDecision::Allow);
    let offline = Devices::fixed(profiles(ShareStatus::Offline, Some("device-c1")));
    let mut grid = ScreenTileGrid::new(GridScope::AllDevices, gone.backend(), offline);
    assert_eq!(settle(&mut grid), Face::Offline);
    assert_eq!(gone.opens() + gone.inner.probes.load(Ordering::SeqCst), 0);
}

fn tile_header(gop_id: u64, size: FrameSize) -> GopHeader {
    GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id,
        kind: StreamKind::Video,
        codec: CodecKind::Tiles,
        profile: None,
        size,
    }
}

/// The records of one tile frame.
fn tile_records(frame_id: u64, image: &BgraImage, keyframe: bool) -> Vec<FrameRecord> {
    let mut encoder = TilesScreenEncoder::new(image.size()).unwrap();
    let encoded = encoder.encode(image, keyframe, 0).unwrap();
    let last = encoded.records.len().saturating_sub(1);
    encoded
        .records
        .into_iter()
        .enumerate()
        .map(|(index, payload)| FrameRecord {
            frame_id,
            capture_ts_us: 0,
            flags: if keyframe { FLAG_KEYFRAME } else { 0 }
                | if index < last { FLAG_MORE } else { 0 },
            payload,
        })
        .collect()
}

fn keyframe_requests(frames: &Mutex<Vec<ViewerFrame>>) -> Vec<(KeyframeReason, Option<u64>)> {
    frames
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter_map(|frame| match frame {
            ViewerFrame::KeyframeRequest(request) => Some((request.reason, request.gop_id)),
            _ => None,
        })
        .collect()
}

#[test]
fn screen_task_viewer_decode_gop_switch() {
    let size = FrameSize::new(256, 128);
    let mut image = BgraImage::new(size).unwrap();
    render(1, &mut image).unwrap();
    let opened = Arc::new(AtomicUsize::new(0));
    let counted = opened.clone();
    let decoder: DecoderOpener = Arc::new(move |header: &GopHeader| {
        counted.fetch_add(1, Ordering::SeqCst);
        Ok::<Box<dyn ScreenDecoder>, io::Error>(Box::new(TilesScreenDecoder::new(header.size)?))
    });
    let sent = Arc::new(Mutex::new(Vec::new()));
    let log = sent.clone();
    let send = Arc::new(move |frame: &ViewerFrame| {
        log.lock().unwrap().push(frame.clone());
        Ok::<(), io::Error>(())
    });
    let shared = Shared::new(false, 0);
    let mut state = DecodeState::new(shared.clone(), decoder, send, false);
    let feed = |state: &mut DecodeState, gop_id, records: Vec<FrameRecord>| {
        for record in records {
            let header = tile_header(gop_id, size);
            state.on_event(StreamEvent::Record { header, record });
        }
    };

    // The first group opens a decoder and shows its picture.
    feed(&mut state, 1, tile_records(1, &image, true));
    assert_eq!(opened.load(Ordering::SeqCst), 1);
    let picture = shared.take_picture().expect("a picture of group 1");
    assert_eq!(picture.size, size);
    assert!(matches!(picture.data, PictureData::Bgra(_)));
    assert!(shared.take_picture().is_none(), "only the newest is kept");

    // A newer group drops the old one and opens its own decoder; a late
    // record of the old group is ignored.
    feed(&mut state, 2, tile_records(2, &image, true));
    assert_eq!(opened.load(Ordering::SeqCst), 2);
    assert!(shared.take_picture().is_some());
    feed(&mut state, 1, tile_records(3, &image, false));
    assert_eq!(
        opened.load(Ordering::SeqCst),
        2,
        "no decoder for a stale group"
    );
    assert!(shared.take_picture().is_none());

    // A record that cannot be decoded asks for one key frame; more broken
    // records and a report of the broken group ask for no second one.
    let garbage = |flags| FrameRecord {
        frame_id: 9,
        capture_ts_us: 0,
        flags,
        payload: vec![1, 2, 3],
    };
    feed(&mut state, 3, vec![garbage(0), garbage(0)]);
    state.on_event(StreamEvent::Broken { gop_id: 3 });
    assert_eq!(
        keyframe_requests(&sent),
        [(KeyframeReason::DecodeError, Some(3))],
        "only one key frame is open"
    );
    assert!(shared.take_picture().is_none());

    // The key frame ends the damage.
    feed(&mut state, 3, tile_records(10, &image, true));
    assert!(shared.take_picture().is_some());
    assert_eq!(keyframe_requests(&sent).len(), 1);

    // The end of the session is its last event.
    let end = StreamEvent::End(serde_json::from_str(r#"{"code":"host_stopped"}"#).unwrap());
    assert!(state.on_event(end));
    let info = shared.info();
    assert_eq!(info.state, SessionState::Ended);
    assert_eq!(
        info.end.as_ref().map(|end| end.text.as_str()),
        Some("Das Gerät hat die Ansicht beendet")
    );
}

#[test]
fn screen_task_viewer_releases_all_on_disable() {
    let fake = Fake::new(CapabilityDecision::Allow);
    let backend = fake.backend();
    let mut wish = target();
    wish.control = true;
    let request = full_request(&backend.offers, None, DISPLAY, true);
    let session = ViewerSession::open(&backend, wish.clone(), request, true, false, 0);
    wait_for("the connection", || fake.opens() == 1);
    fake.accept(0);
    // Accepted in the mode `control`, the viewer asks for the control.
    wait_for("the request for control", || {
        fake.frames().iter().any(|frame| {
            matches!(
                frame,
                ViewerFrame::ControlRequest(request) if request.action == ControlAction::Take
            )
        })
    });
    let common = Common {
        backend,
        devices: Devices::fixed(crate::share::ShareProfiles::default()),
        prefs: ViewerPrefs::default(),
        gpu: false,
        stats_on: false,
        low_latency_changed: false,
    };
    let mut state = ViewState::with_session(wish, session, DISPLAY, false);
    // The user holds Ctrl+Z and the left button; the frames went out already.
    let geometry = Geometry {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 360.0)),
        size: FrameSize::new(1280, 720),
    };
    let mods = egui::Modifiers {
        ctrl: true,
        ..egui::Modifiers::NONE
    };
    let events = [
        egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: mods,
        },
        egui::Event::PointerButton {
            pos: egui::pos2(100.0, 100.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: mods,
        },
    ];
    let mapper: &mut InputMapper = state
        .widget
        .mapper_mut(hello(CapabilityDecision::Allow).input);
    mapper.process(&events, mods, &geometry);
    while mapper.take_batch().is_some() {}

    // „Steuern“ off: everything that is down goes up, then the control is
    // given back, in this order.
    let mut common = common;
    state.apply(&ToolbarAction::Control(false), &mut common, DISPLAY);
    wait_for("the release", || {
        fake.frames().iter().any(|frame| {
            matches!(
                frame,
                ViewerFrame::ControlRequest(request) if request.action == ControlAction::Release
            )
        })
    });
    let frames = fake.frames();
    let release = frames.iter().position(|frame| {
        matches!(
            frame,
            ViewerFrame::ControlRequest(request) if request.action == ControlAction::Release
        )
    });
    let lifts = frames
        .iter()
        .position(|frame| matches!(frame, ViewerFrame::Input(_)));
    assert!(
        lifts < release,
        "the lifts go before the release: {frames:?}"
    );
    let Some(ViewerFrame::Input(events)) = lifts.map(|index| frames[index].clone()) else {
        panic!("no lifts sent");
    };
    let down: Vec<bool> = events
        .iter()
        .map(|event| match event {
            InputEvent::Key { down, .. } | InputEvent::Button { down, .. } => *down,
            _ => true,
        })
        .collect();
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(down.iter().all(|down| !down), "only lifts: {events:?}");
    assert!(!state.control_on);
}
