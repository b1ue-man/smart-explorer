//! Acceptance of the window list and still picture calls (SCR-M4) with a
//! scripted session: the events the relay delivers, the frames sent to the
//! host, the limits on waiting, and the texts of refusals. The host's own
//! answers are tested between two real nodes (`node_screen_still_tests`).
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use super::*;
use crate::screen::core::input_model::InputCapabilities;
use crate::screen::core::tiles_codec::{TileEncoder, MAX_TILES_RECORD_BYTES};
use crate::screen::core::window_list::{ListSource, MonitorInfo, WindowEntry};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, FLAG_KEYFRAME, FLAG_MORE};
use crate::screen::core::wire_hello::{
    ApprovalKind, HostPlatform, ScreenAuthorization, ScreenErrorCode,
};
use crate::share::access_grant::CapabilityDecision;

/// Events in the order the relay delivers them; `None` is a wait that ends
/// without an event, and so is the end of the script.
struct Script {
    events: VecDeque<Option<StreamEvent>>,
    sent: RefCell<Vec<String>>,
    closed: Cell<bool>,
}

fn script(events: Vec<Option<StreamEvent>>) -> Script {
    Script {
        events: events.into(),
        sent: RefCell::new(Vec::new()),
        closed: Cell::new(false),
    }
}

impl StillSession for Script {
    fn next(&mut self, _wait: Option<Duration>) -> io::Result<Option<StreamEvent>> {
        Ok(self.events.pop_front().unwrap_or(None))
    }

    fn send(&self, frame: &ViewerFrame) -> io::Result<()> {
        let name = match frame {
            ViewerFrame::ListWindows => "list_windows".to_string(),
            ViewerFrame::Snapshot(request) => format!("snapshot:{:?}", request.max_size),
            _ => "other".to_string(),
        };
        self.sent.borrow_mut().push(name);
        Ok(())
    }

    fn close(&self) {
        self.closed.set(true);
    }
}

fn hello(monitors: Vec<MonitorInfo>) -> ScreenHelloOk {
    ScreenHelloOk {
        family_version: None,
        icon_version: None,
        frame_contract: None,
        authorization: ScreenAuthorization {
            view: CapabilityDecision::Allow,
            control: CapabilityDecision::Off,
        },
        platform: HostPlatform::Windows,
        encoders: vec![CodecOffer::tiles()],
        input: InputCapabilities::default(),
        monitors,
        limits: Vec::new(),
        approval: ApprovalKind::NotRequired,
    }
}

fn monitor(id: &str, primary: bool) -> MonitorInfo {
    MonitorInfo {
        id: id.into(),
        name: format!("Bildschirm {id}"),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        primary,
    }
}

fn accepted(monitors: Vec<MonitorInfo>) -> Option<StreamEvent> {
    Some(StreamEvent::Accepted {
        hello: hello(monitors),
        path: "direct".into(),
    })
}

fn listed() -> WindowList {
    WindowList {
        windows: vec![WindowEntry {
            id: "4242".into(),
            title: "Editor".into(),
            app: "notepad".into(),
            pid: Some(7),
            monitor: Some("DISPLAY1".into()),
            minimized: false,
            focused: true,
        }],
        apps: Vec::new(),
        list_source: ListSource::EnumWindows,
        limits: vec!["secure_desktop_paused".into()],
    }
}

fn windows(
    events: Vec<Option<StreamEvent>>,
    accept_within: Option<Duration>,
) -> (Result<WindowList, String>, Script, Vec<Capability>) {
    let mut session = script(events);
    let mut heard = Vec::new();
    let result = windows_over(
        &mut session,
        accept_within,
        Duration::from_secs(30),
        &mut |capability| heard.push(capability),
    );
    (result, session, heard)
}

#[test]
fn screen_task_ipc_still_client_lists_windows_and_ends_the_session() {
    let events = vec![
        Some(StreamEvent::Pending(Capability::ScreenView)),
        accepted(vec![monitor("DISPLAY1", true)]),
        Some(StreamEvent::Clock(12)),
        Some(StreamEvent::Host(HostFrame::Windows(listed()))),
    ];
    let (result, session, heard) = windows(events, None);
    assert_eq!(result.unwrap(), listed());
    assert_eq!(heard, [Capability::ScreenView]);
    assert_eq!(*session.sent.borrow(), ["list_windows"]);
    assert!(session.closed.get());
}

#[test]
fn screen_task_ipc_still_client_names_refusals_by_code() {
    let end = |code: &str| {
        Some(StreamEvent::End(RelayEnd {
            code: code.into(),
            detail: None,
        }))
    };
    let (result, session, _) = windows(vec![end("approval_denied")], None);
    assert_eq!(result.unwrap_err(), "approval_denied: Am Gerät abgelehnt");
    assert!(session.sent.borrow().is_empty());
    let error = ScreenWireError::new(ScreenErrorCode::CaptureFailed, Some("kaputt".into()));
    let (result, _, _) = windows(
        vec![
            accepted(Vec::new()),
            Some(StreamEvent::Host(HostFrame::Error(error))),
        ],
        None,
    );
    assert_eq!(result.unwrap_err(), "capture_failed: kaputt");
    let (result, _, _) = windows(vec![accepted(Vec::new()), end("viewer_closed")], None);
    assert_eq!(result.unwrap_err(), "viewer_closed");
}

#[test]
fn screen_task_ipc_still_client_gives_up_waiting_for_the_user() {
    let events = vec![Some(StreamEvent::Pending(Capability::ScreenView)), None];
    let (result, session, heard) = windows(events, Some(Duration::from_secs(5)));
    let error = result.unwrap_err();
    assert!(error.starts_with("approval_pending: "), "{error}");
    assert!(error.contains("5 s"), "{error}");
    assert_eq!(heard.len(), 1);
    assert!(session.sent.borrow().is_empty());
    assert!(session.closed.get(), "the other device must see the end");
}

#[test]
fn screen_task_ipc_still_client_ends_a_silent_host_after_the_read_limit() {
    let (result, session, _) = windows(vec![accepted(Vec::new()), None], None);
    let error = result.unwrap_err();
    assert!(error.starts_with("capture_failed: "), "{error}");
    assert!(error.contains("30 s"), "{error}");
    assert!(session.closed.get());
}

fn picture() -> BgraImage {
    let size = FrameSize::new(150, 90);
    let mut image = BgraImage::new(size).unwrap();
    for y in 0..size.height {
        for (x, pixel) in image.row_mut(y).chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[x as u8, y as u8, (x as u32 + y) as u8, 255]);
        }
    }
    image
}

fn records(image: &BgraImage, kind: StreamKind, gop_id: u64) -> Vec<Option<StreamEvent>> {
    let payloads = TileEncoder::new(image.size())
        .unwrap()
        .encode(&image.view(), true, MAX_TILES_RECORD_BYTES)
        .unwrap();
    let count = payloads.len();
    let header = GopHeader {
            family_layout_revision: None,
        frame_contract: None,
        gop_id,
        kind,
        codec: crate::screen::core::negotiate::CodecKind::Tiles,
        profile: None,
        size: image.size(),
    };
    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            let more = index + 1 < count;
            Some(StreamEvent::Record {
                header,
                record: FrameRecord {
                    frame_id: 1,
                    capture_ts_us: 0,
                    flags: FLAG_KEYFRAME | if more { FLAG_MORE } else { 0 },
                    payload,
                },
            })
        })
        .collect()
}

fn snapshot(
    events: Vec<Option<StreamEvent>>,
    source: Option<SourceId>,
) -> (Result<StillImage, String>, Script) {
    let mut session = script(events);
    let result = snapshot_over(
        &mut session,
        source,
        Some(FrameSize::new(1568, 1568)),
        None,
        Duration::from_secs(30),
        &mut |_| {},
    );
    (result, session)
}

#[test]
fn screen_task_ipc_still_client_assembles_the_snapshot_stream() {
    let image = picture();
    let mut events = vec![accepted(vec![
        monitor("DISPLAY2", false),
        monitor("DISPLAY1", true),
    ])];
    // The video stream of the session runs beside the still picture and
    // is not part of it.
    events.extend(records(&image, StreamKind::Video, 7));
    events.extend(records(&image, StreamKind::Snapshot, 8));
    let (result, session) = snapshot(events, None);
    let still = result.unwrap();
    assert_eq!(still.image, image);
    assert_eq!(still.source, "monitor:DISPLAY1");
    assert_eq!(
        *session.sent.borrow(),
        ["snapshot:Some(FrameSize { width: 1568, height: 1568 })"]
    );
    assert!(session.closed.get());
    // A named source is reported as named.
    let mut events = vec![accepted(Vec::new())];
    events.extend(records(&image, StreamKind::Snapshot, 3));
    let (result, _) = snapshot(events, Some(SourceId::Window("55".into())));
    assert_eq!(result.unwrap().source, "window:55");
}

#[test]
fn screen_task_ipc_still_client_reports_a_paused_source() {
    let error = ScreenWireError::new(ScreenErrorCode::CaptureFailed, Some("paused".into()));
    let events = vec![
        accepted(Vec::new()),
        Some(StreamEvent::Host(HostFrame::Error(error))),
    ];
    let (result, _) = snapshot(events, None);
    assert_eq!(result.err().unwrap(), "capture_failed: paused");
}
