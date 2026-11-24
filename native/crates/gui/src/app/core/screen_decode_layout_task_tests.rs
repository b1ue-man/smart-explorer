//! Round 12 rest (Codex 12/8): the records of a window family's new layout
//! that reach the desktop viewer before its layout state wait for it and are
//! decoded then, the key frame first, without a decode error, a key frame
//! request or a second group.
use std::io;
use std::sync::{Arc, Mutex};

use crate::app::screen_modules::screen_decode_thread::DecodeState;
use crate::app::screen_modules::screen_session::{DecoderOpener, Shared};
use crate::app::screen_modules::screen_viewer_fake::hello;
use crate::daemon::StreamEvent;
use crate::remote_window::{FamilySnapshot, WindowIdentity, FAMILY_VERSION};
use crate::screen::core::decoded_frame::DecodedFrame;
use crate::screen::core::family_viewer::FamilyViewer;
use crate::screen::core::host_backend::ScreenDecoder;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};
use crate::screen::core::wire_control::ViewerFrame;
use crate::share::access_grant::CapabilityDecision;

const SIZE: FrameSize = FrameSize::new(4, 2);
const SESSION: &str = "0123456789abcdef0123456789abcdef";

/// A decoder that names every frame it decodes, with its group.
struct Named {
    group: u64,
    log: Arc<Mutex<Vec<(u64, u64)>>>,
    decoded: Option<DecodedFrame>,
    image: BgraImage,
}

impl ScreenDecoder for Named {
    fn name(&self) -> String {
        "named".into()
    }
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.log.lock().unwrap().push((self.group, record.frame_id));
        self.decoded = Some(DecodedFrame::from_record(record));
        Ok(true)
    }
    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }
    fn picture(&self) -> &BgraImage {
        &self.image
    }
}

fn snapshot(layout: u64) -> FamilySnapshot {
    let root = WindowIdentity {
        session: SESSION.into(),
        generation: 1,
    };
    FamilySnapshot {
        version: FAMILY_VERSION,
        session: SESSION.into(),
        revision: layout,
        layout_revision: layout,
        root,
        members: Vec::new(),
        limitations: Vec::new(),
    }
}

fn header(gop_id: u64, layout: u64) -> GopHeader {
    GopHeader {
        family_layout_revision: Some(layout),
        frame_contract: None,
        gop_id,
        kind: StreamKind::Video,
        codec: CodecKind::Tiles,
        profile: None,
        size: SIZE,
    }
}

fn record(frame_id: u64, flags: u8) -> FrameRecord {
    FrameRecord {
        frame_id,
        capture_ts_us: 0,
        flags,
        payload: vec![frame_id as u8],
    }
}

fn feed(state: &mut DecodeState, gop_id: u64, layout: u64, record: FrameRecord) {
    let header = header(gop_id, layout);
    state.on_event(StreamEvent::Record { header, record });
}

#[test]
fn screen_task_new_layout_pictures_wait_for_their_state_on_the_desktop() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let opened = log.clone();
    let decoder: DecoderOpener = Arc::new(move |header: &GopHeader| {
        Ok::<Box<dyn ScreenDecoder>, io::Error>(Box::new(Named {
            group: header.gop_id,
            log: opened.clone(),
            decoded: None,
            image: BgraImage::new(SIZE)?,
        }))
    });
    let sent = Arc::new(Mutex::new(Vec::new()));
    let frames = sent.clone();
    let send = Arc::new(move |frame: &ViewerFrame| {
        frames.lock().unwrap().push(frame.clone());
        Ok::<(), io::Error>(())
    });
    let shared = Shared::new(false, 0);
    let source = SourceId::Window(format!("@family.{SESSION}"));
    shared.update(|info| {
        info.hello = Some(hello(CapabilityDecision::Allow));
        let mut family = FamilyViewer::for_source(&source).expect("a family source");
        family.snapshot = Some(snapshot(3));
        info.family = Some(family);
    });
    let mut state = DecodeState::new(shared.clone(), decoder, send, false);
    feed(&mut state, 1, 3, record(1, FLAG_KEYFRAME));
    feed(&mut state, 1, 3, record(2, 0));
    // Group 2 carries layout 4, whose state has not arrived yet: it waits.
    feed(&mut state, 2, 4, record(3, FLAG_KEYFRAME));
    feed(&mut state, 2, 4, record(4, 0));
    // An older layout's record is dropped as before.
    feed(&mut state, 1, 2, record(5, 0));
    assert_eq!(*log.lock().unwrap(), vec![(1, 1), (1, 2)]);
    // The state of layout 4 arrives: the held group is decoded in order,
    // its key frame first.
    shared.update(|info| {
        if let Some(family) = info.family.as_mut() {
            family.snapshot = Some(snapshot(4));
        }
    });
    assert!(state.release_layout());
    feed(&mut state, 2, 4, record(6, 0));
    assert_eq!(
        *log.lock().unwrap(),
        vec![(1, 1), (1, 2), (2, 3), (2, 4), (2, 6)]
    );
    // No decode error and no key frame request: no further group.
    assert!(sent
        .lock()
        .unwrap()
        .iter()
        .all(|frame| !matches!(frame, ViewerFrame::KeyframeRequest(_))));
    assert!(shared.info().notice.is_none());
}
