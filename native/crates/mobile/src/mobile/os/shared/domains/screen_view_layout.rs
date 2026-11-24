//! A window family's new layout on the Android viewer (round 12 rest,
//! Codex 12/8): the records of a group the host opened for a layout newer
//! than the family state the session holds wait in `layout_hold` until that
//! state arrives and then go to the sink in their order, the key frame
//! first; before, they were dropped and the session asked for one more key
//! frame (one more group). Records of an older layout stay dropped
//! (`family_layout_hold.rs`).
use std::sync::atomic::Ordering;

use super::{lock, on_sink_output, ViewSession};
use crate::screen::core::family_layout_hold::{fit, LayoutFit};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};
use crate::screen::os::viewer_sink::ViewerSink;

/// One record of the session (its group id already the viewer's own).
pub(super) fn record(
    session: &ViewSession,
    sink: &mut ViewerSink,
    encoder: &(String, bool),
    header: GopHeader,
    record: FrameRecord,
) {
    // `None`: no family; `Some(None)`: a family without a state yet.
    let known = lock(&session.family).as_ref().map(|family| {
        family
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.layout_revision)
    });
    if let Some(known) = known {
        if header.gop_id < session.latest_gop.load(Ordering::SeqCst) {
            return;
        }
        match fit(header.family_layout_revision, known) {
            LayoutFit::Decode => {}
            LayoutFit::Drop => return,
            LayoutFit::Hold => {
                // What a newer layout or the bound pushes out is dropped, as
                // every record of a superseded layout.
                let _ = lock(&session.layout_hold).hold(header, record);
                return;
            }
        }
        if let Some(layout) = known {
            *lock(&session.family_picture) = Some((header.gop_id, layout));
        }
    }
    session
        .latest_gop
        .fetch_max(header.gop_id, Ordering::SeqCst);
    for output in sink.record(&header, &record, now_us()) {
        on_sink_output(session, encoder, output);
    }
}

/// A family state arrived: the records held for its layout go to the sink
/// now, in their order; those of an older layout are dropped. `true` when
/// records went.
pub(super) fn release(
    session: &ViewSession,
    sink: &mut ViewerSink,
    encoder: &(String, bool),
) -> bool {
    let known = lock(&session.family)
        .as_ref()
        .and_then(|family| family.snapshot.as_ref())
        .map(|snapshot| snapshot.layout_revision);
    let Some(known) = known else {
        return false;
    };
    let (decode, _dropped) = lock(&session.layout_hold).release(known);
    let released = !decode.is_empty();
    for (header, held) in decode {
        record(session, sink, encoder, header, held);
    }
    released
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::super::{lock, open_for_tests};
    use crate::remote_window::{FamilySnapshot, WindowIdentity, FAMILY_VERSION};
    use crate::screen::core::family_viewer::FamilyViewer;
    use crate::screen::core::negotiate::CodecKind;
    use crate::screen::core::pixel_convert::FrameSize;
    use crate::screen::core::session_state::KeyframeRequester;
    use crate::screen::core::window_list::SourceId;
    use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};
    use crate::screen::core::wire_hello::SessionMode;
    use crate::screen::os::viewer_sink::{PullQueue, ViewerSink};
    use std::sync::atomic::Ordering;

    const SESSION: &str = "0123456789abcdef0123456789abcdef";

    fn snapshot(layout: u64) -> FamilySnapshot {
        FamilySnapshot {
            version: FAMILY_VERSION,
            session: SESSION.into(),
            revision: layout,
            layout_revision: layout,
            root: WindowIdentity {
                session: SESSION.into(),
                generation: 1,
            },
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
            size: FrameSize::new(4, 2),
        }
    }

    fn keyframe(frame_id: u64) -> FrameRecord {
        FrameRecord {
            frame_id,
            capture_ts_us: 0,
            flags: FLAG_KEYFRAME,
            payload: vec![1],
        }
    }

    /// Round 12 rest (Codex 12/8): a group of a newer layout waits on the
    /// Android viewer until its state arrives and then reaches the sink; a
    /// group of an older layout never does.
    #[test]
    fn screen_task_new_layout_pictures_wait_for_their_state_on_android() {
        let session = open_for_tests(SessionMode::View, None);
        let source = SourceId::Window(format!("@family.{SESSION}"));
        let mut family = FamilyViewer::for_source(&source).expect("a family source");
        family.snapshot = Some(snapshot(3));
        *lock(&session.family) = Some(family);
        let keyframes = Arc::new(Mutex::new(KeyframeRequester::default()));
        let mut sink = ViewerSink::new(Arc::new(PullQueue::default()), keyframes);
        let encoder = (String::new(), false);
        super::record(&session, &mut sink, &encoder, header(1, 3), keyframe(1));
        assert_eq!(session.latest_gop.load(Ordering::SeqCst), 1);
        // Layout 4 before its state: held, not dropped.
        super::record(&session, &mut sink, &encoder, header(2, 4), keyframe(2));
        assert_eq!(session.latest_gop.load(Ordering::SeqCst), 1);
        // An older layout: dropped.
        super::record(&session, &mut sink, &encoder, header(3, 2), keyframe(3));
        assert_eq!(session.latest_gop.load(Ordering::SeqCst), 1);
        assert!(!super::release(&session, &mut sink, &encoder));
        if let Some(family) = lock(&session.family).as_mut() {
            family.snapshot = Some(snapshot(4));
        }
        assert!(super::release(&session, &mut sink, &encoder));
        assert_eq!(session.latest_gop.load(Ordering::SeqCst), 2);
        assert_eq!(*lock(&session.family_picture), Some((2, 4)));
        session.close();
    }
}
