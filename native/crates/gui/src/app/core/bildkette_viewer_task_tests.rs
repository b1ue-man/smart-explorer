//! Desktop viewer of the picture chain (plan 2026-11-14 „Bildkette“ V1):
//! picture hand-over without a copy, the decoder over a continuation
//! (B-36), the broken chain and the decoder switch without a new session
//! (the announcement: `bildkette_viewer_announce_task_tests.rs`). A probe
//! decoder stands in for the codecs.
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::app::screen_modules::screen_decode_local::{empty_planes, KEPT};
use crate::app::screen_modules::screen_decode_thread::DecodeState;
use crate::app::screen_modules::screen_session::{DecoderOpener, PictureData, Shared};
use crate::app::screen_modules::screen_viewer_fake::hello;
use crate::app::screen_modules::screen_wgpu_yuv::YuvFrame;
use crate::daemon::StreamEvent;
use crate::screen::core::decoded_frame::DecodedFrame;
use crate::screen::core::host_backend::{Chroma, ScreenDecoder, YuvPicture};
use crate::screen::core::negotiate::{CodecKind, H264Profile};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize, YuvMatrix};
use crate::screen::core::wire_binary::{
    FrameRecord, GopHeader, StreamKind, FLAG_CONTINUE, FLAG_KEYFRAME, FLAG_MORE,
};
use crate::screen::core::wire_control::{KeyframeReason, ViewerFrame};
use crate::share::access_grant::CapabilityDecision;

const SIZE: FrameSize = FrameSize::new(4, 2);
/// A record payload the probe refuses to decode.
const DAMAGED: [u8; 1] = [0xEE];

/// What the probe decoders of one test did.
#[derive(Default)]
struct Log {
    opened: AtomicUsize,
    /// Plane storage made (not reused).
    allocations: AtomicUsize,
    /// Frame ids handed to any decoder, with the decoder's number.
    decoded: Mutex<Vec<(usize, u64)>>,
    /// Address of the luma plane of the newest output.
    last_luma: AtomicUsize,
}

/// A video decoder that hands its planes over and reads into returned ones.
struct Probe {
    number: usize,
    keeps: bool,
    log: Arc<Log>,
    out: Option<YuvPicture>,
    spare: Option<YuvPicture>,
    decoded: Option<DecodedFrame>,
    image: BgraImage,
}

impl ScreenDecoder for Probe {
    fn name(&self) -> String {
        format!("probe-{}", self.number)
    }
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        if record.payload == DAMAGED {
            return Err(io::Error::other("damaged record"));
        }
        self.log
            .decoded
            .lock()
            .unwrap()
            .push((self.number, record.frame_id));
        if record.has_more() {
            return Ok(false);
        }
        let log = self.log.clone();
        let mut out = self
            .spare
            .take()
            .or_else(|| self.out.take())
            .unwrap_or_else(|| {
                log.allocations.fetch_add(1, Ordering::SeqCst);
                YuvPicture {
                    y: Vec::with_capacity(8),
                    y_stride: 4,
                    uv: Vec::with_capacity(4),
                    uv_stride: 4,
                    chroma: Chroma::Nv12,
                    matrix: YuvMatrix::Bt709,
                }
            });
        out.y.clear();
        out.y.resize(8, record.frame_id as u8);
        out.uv.clear();
        out.uv.resize(4, 128);
        log.last_luma
            .store(out.y.as_ptr() as usize, Ordering::SeqCst);
        self.out = Some(out);
        self.decoded = Some(DecodedFrame::from_record(record));
        Ok(true)
    }
    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }
    fn picture(&self) -> &BgraImage {
        &self.image
    }
    fn yuv_picture(&self) -> Option<YuvPicture> {
        self.out.clone()
    }
    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.out.take()
    }
    fn recycle_yuv(&mut self, planes: YuvPicture) {
        self.spare = Some(planes);
    }
    fn keeps_references(&self) -> bool {
        self.keeps
    }
}

/// A decode thread with probe decoders and the frames it sends.
struct Rig {
    state: DecodeState,
    shared: Arc<Shared>,
    log: Arc<Log>,
    sent: Arc<Mutex<Vec<ViewerFrame>>>,
}

fn rig(keeps: bool, announced: bool) -> Rig {
    let log = Arc::new(Log::default());
    let opener_log = log.clone();
    let decoder: DecoderOpener = Arc::new(move |_header: &GopHeader| {
        let number = opener_log.opened.fetch_add(1, Ordering::SeqCst) + 1;
        Ok::<Box<dyn ScreenDecoder>, io::Error>(Box::new(Probe {
            number,
            keeps,
            log: opener_log.clone(),
            out: None,
            spare: None,
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
    let shared = Shared::new(true, 0);
    shared.update(|info| info.hello = Some(hello(CapabilityDecision::Allow)));
    let mut state = DecodeState::new(shared.clone(), decoder, send, false);
    state.announce_continuation(announced);
    Rig {
        state,
        shared,
        log,
        sent,
    }
}

fn header(gop_id: u64) -> GopHeader {
    GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id,
        kind: StreamKind::Video,
        codec: CodecKind::H264,
        profile: Some(H264Profile::High),
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

impl Rig {
    fn feed(&mut self, gop_id: u64, record: FrameRecord) {
        let header = header(gop_id);
        self.state.on_event(StreamEvent::Record { header, record });
    }
    fn broken(&mut self, gop_id: u64) {
        self.state.on_event(StreamEvent::Broken { gop_id });
    }
    fn keyframe_requests(&self) -> Vec<(KeyframeReason, Option<u64>)> {
        let sent = self.sent.lock().unwrap();
        sent.iter()
            .filter_map(|frame| match frame {
                ViewerFrame::KeyframeRequest(request) => Some((request.reason, request.gop_id)),
                _ => None,
            })
            .collect()
    }
    /// The shown picture's group, frame and planes.
    fn shown(&self) -> Option<(u64, u64, YuvPicture)> {
        let picture = self.shared.take_picture()?;
        let PictureData::Yuv(planes) = picture.data else {
            panic!("the probe hands over planes");
        };
        Some((picture.presented.gop_id, picture.presented.frame_id, planes))
    }
    fn decoded(&self) -> Vec<(usize, u64)> {
        self.log.decoded.lock().unwrap().clone()
    }
}

#[test]
fn bildkette_task_viewer_picture_handed_over_without_copy() {
    let mut rig = rig(false, false);
    rig.feed(1, record(1, FLAG_KEYFRAME));
    let (_, frame, planes) = rig.shown().expect("the key frame is shown");
    assert_eq!(frame, 1);
    // The slot holds the decoder's own buffer, not a clone of it.
    assert_eq!(
        planes.y.as_ptr() as usize,
        rig.log.last_luma.load(Ordering::SeqCst)
    );
    let first = planes.y.as_ptr() as usize;
    // The draw code lets the picture go: its planes go back to the pool.
    let mut drawn = YuvFrame::new(SIZE, planes);
    drawn.pool = Some(rig.shared.local.pool.clone());
    drop(drawn);
    assert_eq!(rig.shared.local.pool.held(), (1, 0));
    for frame_id in 2..=6 {
        rig.feed(1, record(frame_id, 0));
        let (_, frame, planes) = rig.shown().expect("every picture is shown");
        assert_eq!(frame, frame_id);
        assert_eq!(planes.y, vec![frame_id as u8; 8]);
        // Read into the returned storage: same buffer, nothing allocated.
        assert_eq!(planes.y.as_ptr() as usize, first, "frame {frame_id}");
        let mut drawn = YuvFrame::new(SIZE, planes);
        drawn.pool = Some(rig.shared.local.pool.clone());
        drop(drawn);
    }
    assert_eq!(rig.log.allocations.load(Ordering::SeqCst), 1);
    // A picture nobody took is given back when a newer one replaces it.
    rig.feed(1, record(7, 0));
    rig.feed(1, record(8, 0));
    assert_eq!(rig.shared.local.pool.held().0, 1);
    assert_eq!(rig.shown().map(|(_, frame, _)| frame), Some(8));
    // The pool keeps at most `KEPT` pictures per kind.
    let pool = rig.shared.local.pool.clone();
    for _ in 0..KEPT + 3 {
        pool.give_yuv(empty_planes());
    }
    assert_eq!(pool.held().0, KEPT);
}

#[test]
fn bildkette_task_viewer_default_hand_over_copies_like_before() {
    // A decoder without its own hand-over (VA-API, dav1d, tiles' planes)
    // keeps the old behaviour: a copy of `yuv_picture`.
    struct Plain(Option<YuvPicture>, BgraImage);
    impl ScreenDecoder for Plain {
        fn name(&self) -> String {
            "plain".into()
        }
        fn decode(&mut self, _record: &FrameRecord) -> io::Result<bool> {
            Ok(false)
        }
        fn picture(&self) -> &BgraImage {
            &self.1
        }
        fn yuv_picture(&self) -> Option<YuvPicture> {
            self.0.clone()
        }
    }
    let planes = YuvPicture {
        y: vec![7; 8],
        y_stride: 4,
        uv: vec![128; 4],
        uv_stride: 4,
        chroma: Chroma::Nv12,
        matrix: YuvMatrix::Bt709,
    };
    let mut plain = Plain(Some(planes.clone()), BgraImage::new(SIZE).unwrap());
    let taken = plain.take_yuv_picture().unwrap();
    assert_eq!(taken, planes);
    assert_ne!(taken.y.as_ptr(), plain.0.as_ref().unwrap().y.as_ptr());
    assert_eq!(plain.take_picture(), BgraImage::new(SIZE).unwrap());
    assert!(!plain.keeps_references(), "no continuation by default");
}

#[test]
fn bildkette_task_viewer_decoder_stays_over_a_continuation() {
    let mut rig = rig(true, true);
    rig.feed(1, record(1, FLAG_KEYFRAME));
    rig.feed(1, record(2, 0));
    assert_eq!(rig.shown().map(|(_, frame, _)| frame), Some(2));
    // The host reset the stale stream: no key frame is asked for, the
    // decoder waits for the host's next stream.
    rig.broken(1);
    assert!(rig.keyframe_requests().is_empty());
    rig.feed(2, record(5, FLAG_CONTINUE));
    rig.feed(2, record(6, 0));
    assert_eq!(
        rig.log.opened.load(Ordering::SeqCst),
        1,
        "the decoder stayed"
    );
    assert_eq!(rig.decoded(), [(1, 1), (1, 2), (1, 5), (1, 6)]);
    let (gop, frame, _) = rig.shown().expect("the continuation is shown");
    assert_eq!((gop, frame), (2, 6));
    // A continuation without a broken stream before it (a new stream right
    // away) keeps the decoder as well.
    rig.feed(3, record(7, FLAG_CONTINUE));
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.shown().map(|(gop, frame, _)| (gop, frame)),
        Some((3, 7))
    );
    assert!(rig.keyframe_requests().is_empty());
    // A key frame stream still opens a new decoder.
    rig.feed(4, record(8, FLAG_KEYFRAME));
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 2);
}

#[test]
fn bildkette_task_viewer_broken_chain_asks_for_a_key_frame_and_shows_no_damage() {
    // A decode error breaks the chain: the continuation after it is not
    // decoded, nothing is shown, a key frame is asked for.
    let mut rig = rig(true, true);
    rig.feed(1, record(1, FLAG_KEYFRAME));
    assert!(rig.shown().is_some());
    rig.feed(
        1,
        FrameRecord {
            payload: DAMAGED.to_vec(),
            ..record(2, 0)
        },
    );
    assert_eq!(
        rig.keyframe_requests(),
        [(KeyframeReason::DecodeError, Some(1))]
    );
    rig.broken(1);
    rig.feed(2, record(3, FLAG_CONTINUE));
    rig.feed(2, record(4, 0));
    assert!(rig.shown().is_none(), "no picture of a broken chain");
    assert!(!rig.decoded().iter().any(|(_, frame)| *frame >= 3));
    // The key frame of the next stream opens a decoder and is shown.
    rig.feed(3, record(5, FLAG_KEYFRAME));
    assert_eq!(
        rig.shown().map(|(gop, frame, _)| (gop, frame)),
        Some((3, 5))
    );
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 2);

    // A stream that broke inside a frame leaves part of it in the decoder.
    let mut rig = rig_with_frames(true, true);
    rig.feed(1, record(2, FLAG_MORE));
    rig.broken(1);
    assert_eq!(rig.keyframe_requests(), [(KeyframeReason::Reset, Some(1))]);
    rig.feed(2, record(3, FLAG_CONTINUE));
    assert!(rig.shown().is_none());
    assert!(!rig.decoded().iter().any(|(_, frame)| *frame == 3));

    // A decoder that cannot keep its references skips the continuation
    // and asks for a key frame at once; the last picture stays in place.
    let mut rig = rig_with_frames(false, true);
    rig.broken(1);
    assert_eq!(rig.keyframe_requests(), [(KeyframeReason::Reset, Some(1))]);
    let mut rig = rig_with_frames(false, true);
    rig.feed(2, record(3, FLAG_CONTINUE));
    assert_eq!(rig.keyframe_requests(), [(KeyframeReason::Reset, Some(2))]);
    assert!(rig.shown().is_none());
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 1);

    // Without the announcement a broken stream asks for a key frame at
    // once, as before.
    let mut rig = rig_with_frames(true, false);
    rig.broken(1);
    assert_eq!(rig.keyframe_requests(), [(KeyframeReason::Reset, Some(1))]);

    // No new stream within `STALE_STREAM_US`: the next event asks after all.
    let mut rig = rig_with_frames(true, true);
    rig.broken(1);
    assert!(rig.keyframe_requests().is_empty());
    let (gop, _) = rig.state.chain.awaiting.expect("waits for the next stream");
    rig.state.chain.awaiting = Some((gop, Instant::now() - Duration::from_secs(2)));
    rig.state.on_event(StreamEvent::Clock(0));
    assert_eq!(rig.keyframe_requests(), [(KeyframeReason::Reset, Some(1))]);
    rig.feed(2, record(9, FLAG_CONTINUE));
    assert!(rig.shown().is_none(), "the decoder is gone");

    // A continuation header with changed stream parameters is refused.
    let mut rig = rig_with_frames(true, true);
    let mut changed = header(2);
    changed.size = FrameSize::new(8, 4);
    rig.state.on_event(StreamEvent::Record {
        header: changed,
        record: record(3, FLAG_CONTINUE),
    });
    assert_eq!(rig.keyframe_requests(), [(KeyframeReason::Reset, Some(2))]);
    assert!(rig.shown().is_none());
}

/// A rig that showed the key frame 1 of group 1 (taken out of the slot).
fn rig_with_frames(keeps: bool, announced: bool) -> Rig {
    let mut rig = rig(keeps, announced);
    rig.feed(1, record(1, FLAG_KEYFRAME));
    assert!(rig.shown().is_some());
    rig
}

#[test]
fn bildkette_task_viewer_decoder_switch_without_reopening() {
    let mut rig = rig_with_frames(true, true);
    // `ViewerSession::switch_decoder` took the new choice over.
    rig.shared.local.switches.fetch_add(1, Ordering::AcqRel);
    rig.feed(1, record(2, 0));
    assert_eq!(rig.keyframe_requests(), [(KeyframeReason::Reset, Some(1))]);
    // The old decoder decodes until the key frame arrives.
    assert_eq!(rig.shown().map(|(_, frame, _)| frame), Some(2));
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 1);
    // The broken chain of the switch accepts no continuation.
    rig.feed(2, record(3, FLAG_CONTINUE));
    assert!(rig.shown().is_none());
    // The key frame opens the decoder of the new choice.
    rig.feed(3, record(4, FLAG_KEYFRAME));
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 2);
    assert_eq!(rig.decoded().last(), Some(&(2, 4)));
    assert_eq!(rig.shown().map(|(_, frame, _)| frame), Some(4));
    // A key frame inside the running group opens it as well.
    rig.shared.local.switches.fetch_add(1, Ordering::AcqRel);
    rig.feed(3, record(5, FLAG_KEYFRAME));
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 3);
    rig.feed(3, record(6, 0));
    assert_eq!(rig.log.opened.load(Ordering::SeqCst), 3);
}
