//! Pictures of a window family's new layout that reach the viewer before its
//! layout state (round 12 rest, Codex 12/8). The host publishes the family
//! state after the picture operation that captured the new layout
//! (`host_session.rs`: `family_snapshot` after every completed tick), and the
//! records and the `FAMILY_SNAPSHOT` travel on different streams, so the
//! group the host opened for the new layout (its key frame first) may come
//! first. Dropping those records lost the key frame: the viewer only found
//! back over a decode error and one more key frame request, which cost one
//! more group. The viewer holds them here until the layout state arrives and
//! then decodes them in their order (desktop `screen_decode_layout.rs`,
//! Android `screen_view_layout.rs`).
//!
//! Only records of a layout newer than the one the viewer knows wait; records
//! of an older layout stay dropped as before. At most `MAX_HELD_FRAMES`
//! pictures wait: the decoder's own identity window
//! (`decoded_frame::MAX_PENDING_FRAMES`) takes no more pictures in flight, and
//! the host sends its family state after the first picture operation of the
//! new layout. Beyond it the held records are dropped as before and the
//! viewer asks for a key frame when the layout state comes.
use super::decoded_frame::MAX_PENDING_FRAMES;
use super::wire_binary::{FrameRecord, GopHeader};

/// Pictures (records without `FLAG_MORE`) held at most.
pub const MAX_HELD_FRAMES: usize = MAX_PENDING_FRAMES;

/// What the viewer does with a record of a family session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutFit {
    /// The record's layout is the one the viewer knows: decode it.
    Decode,
    /// A newer layout than the viewer knows (or none known yet): held.
    Hold,
    /// An older layout, or a group without a layout: dropped.
    Drop,
}

/// `header_layout`: the record's group layout; `known`: the layout of the
/// family state the viewer holds (`None` before the first one).
pub fn fit(header_layout: Option<u64>, known: Option<u64>) -> LayoutFit {
    match (header_layout, known) {
        (Some(layout), Some(known)) if layout == known => LayoutFit::Decode,
        (Some(layout), Some(known)) if layout > known => LayoutFit::Hold,
        (Some(_), None) => LayoutFit::Hold,
        _ => LayoutFit::Drop,
    }
}

/// The records of one newer layout, in their order.
#[derive(Default)]
pub struct LayoutHold {
    layout: Option<u64>,
    records: Vec<(GopHeader, FrameRecord)>,
    pictures: usize,
}

/// What `LayoutHold::hold` did.
#[derive(Debug, PartialEq, Eq)]
pub struct Held {
    /// Records of an older held layout this record replaced (its pictures
    /// are lost: count them).
    pub replaced: Vec<(GopHeader, FrameRecord)>,
    /// `false`: the hold was full; the record and everything held are
    /// dropped (returned in `replaced` with the record last).
    pub kept: bool,
}

impl LayoutHold {
    /// Holds `record` of the newer layout its header names.
    pub fn hold(&mut self, header: GopHeader, record: FrameRecord) -> Held {
        let layout = header.family_layout_revision;
        let mut replaced = Vec::new();
        if self.layout.is_some() && layout > self.layout {
            replaced = self.take();
        } else if self.layout.is_some() && layout < self.layout {
            // An older layout than the one waiting: superseded already.
            return Held {
                replaced: vec![(header, record)],
                kept: false,
            };
        }
        let picture = !record.has_more();
        if picture && self.pictures >= MAX_HELD_FRAMES {
            replaced.extend(self.take());
            replaced.push((header, record));
            return Held {
                replaced,
                kept: false,
            };
        }
        self.layout = layout;
        self.pictures += usize::from(picture);
        self.records.push((header, record));
        Held {
            replaced,
            kept: true,
        }
    }

    /// The layout state of `known` arrived: the held records of that layout
    /// in their order (to decode), and those of an older layout (dropped);
    /// records of a still newer layout keep waiting.
    pub fn release(
        &mut self,
        known: u64,
    ) -> (Vec<(GopHeader, FrameRecord)>, Vec<(GopHeader, FrameRecord)>) {
        match self.layout {
            Some(layout) if layout == known => (self.take(), Vec::new()),
            Some(layout) if layout < known => (Vec::new(), self.take()),
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// Everything held, dropped (a new host session).
    pub fn take(&mut self) -> Vec<(GopHeader, FrameRecord)> {
        self.layout = None;
        self.pictures = 0;
        std::mem::take(&mut self.records)
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::negotiate::CodecKind;
    use crate::screen::core::pixel_convert::FrameSize;
    use crate::screen::core::wire_binary::{StreamKind, FLAG_KEYFRAME, FLAG_MORE};

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

    fn record(frame_id: u64, flags: u8) -> FrameRecord {
        FrameRecord {
            frame_id,
            capture_ts_us: 0,
            flags,
            payload: vec![1],
        }
    }

    /// Round 12 rest (Codex 12/8): a newer layout's records wait for its
    /// state and come back in their order, the key frame first; an older
    /// layout's records are dropped; the hold keeps at most
    /// `MAX_HELD_FRAMES` pictures.
    #[test]
    fn screen_task_new_layout_records_wait_for_their_state() {
        assert_eq!(fit(Some(3), Some(3)), LayoutFit::Decode);
        assert_eq!(fit(Some(4), Some(3)), LayoutFit::Hold);
        assert_eq!(fit(Some(4), None), LayoutFit::Hold);
        assert_eq!(fit(Some(2), Some(3)), LayoutFit::Drop);
        assert_eq!(fit(None, Some(3)), LayoutFit::Drop);

        let mut hold = LayoutHold::default();
        assert!(
            hold.hold(header(8, 4), record(20, FLAG_KEYFRAME | FLAG_MORE))
                .kept
        );
        assert!(hold.hold(header(8, 4), record(20, 0)).kept);
        assert!(hold.hold(header(8, 4), record(21, 0)).kept);
        // The state of layout 3 changes nothing for layout 4.
        assert_eq!(hold.release(3), (Vec::new(), Vec::new()));
        let (decode, dropped) = hold.release(4);
        assert!(dropped.is_empty());
        let frames: Vec<(u64, u8)> = decode
            .iter()
            .map(|(_, record)| (record.frame_id, record.flags))
            .collect();
        assert_eq!(
            frames,
            vec![(20, FLAG_KEYFRAME | FLAG_MORE), (20, 0), (21, 0)]
        );
        assert!(hold.is_empty());

        // A newer layout replaces the held one; a later state of a newer
        // layout drops what waits.
        assert!(hold.hold(header(9, 5), record(30, FLAG_KEYFRAME)).kept);
        let held = hold.hold(header(10, 6), record(31, FLAG_KEYFRAME));
        assert!(held.kept);
        assert_eq!(held.replaced.len(), 1);
        let late = hold.hold(header(9, 5), record(32, 0));
        assert!(!late.kept);
        let (decode, dropped) = hold.release(7);
        assert!(decode.is_empty());
        assert_eq!(dropped.len(), 1);

        // The bound: one picture beyond it drops everything held.
        for frame in 0..MAX_HELD_FRAMES as u64 {
            assert!(hold.hold(header(11, 8), record(frame, 0)).kept);
        }
        let full = hold.hold(header(11, 8), record(99, 0));
        assert!(!full.kept);
        assert_eq!(full.replaced.len(), MAX_HELD_FRAMES + 1);
        assert!(hold.is_empty());
    }
}
