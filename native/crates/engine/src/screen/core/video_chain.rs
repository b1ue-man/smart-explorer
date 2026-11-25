//! Which complete picture the viewer hands to its decoder
//! (`video_transport.rs`): a picture references the ones before it in its
//! group, so after a picture of the chain is lost every following non-key
//! picture of that group is skipped until a key frame or a newer group (a
//! continuation or a key frame the host sends after `FRAME_LOSS`), and the
//! host hears of the break once. Each message names the picture of its
//! group sent before it (`prev_frame_id`), so a picture of which not one
//! packet arrived is noticed with the next one.
use super::FrameLoss;

/// What becomes of a complete picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// To the decoder, and acknowledged.
    Deliver,
    /// Not decodable: its chain broke before (already reported).
    Skip,
    /// The picture before it never came: the chain breaks here; report the
    /// loss and skip the picture.
    Lost(FrameLoss),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChainGate {
    /// The newest group handed on and its last picture.
    current: Option<(u64, u64)>,
    /// The newest group whose chain broke: its non-key pictures are skipped.
    broken: Option<u64>,
}

impl ChainGate {
    /// A picture of group `gop_id` was given up incomplete; the loss to
    /// report, once per chain and never for a group a newer one replaced.
    pub fn gave_up(&mut self, gop_id: u64, frame_id: u64) -> Option<FrameLoss> {
        if self.current.is_some_and(|(group, _)| gop_id < group)
            || self.broken.is_some_and(|broken| gop_id <= broken)
        {
            return None;
        }
        self.broken = Some(gop_id);
        Some(FrameLoss { gop_id, frame_id })
    }

    /// A complete picture; `prev_frame_id` is the picture of its group sent
    /// before it (0: the group's first).
    pub fn picture(
        &mut self,
        gop_id: u64,
        frame_id: u64,
        prev_frame_id: u64,
        keyframe: bool,
        continuation: bool,
    ) -> Verdict {
        if self.current.is_some_and(|(group, _)| gop_id < group) {
            return Verdict::Skip;
        }
        if keyframe {
            return self.take(gop_id, frame_id);
        }
        if self.broken.is_some_and(|broken| gop_id <= broken) {
            return Verdict::Skip;
        }
        let missing = match self.current {
            Some((group, last)) if group == gop_id => {
                (prev_frame_id != last).then_some(prev_frame_id)
            }
            // A new group starts with a continuation (its key frame came
            // above); any other first picture lost its predecessors.
            _ if prev_frame_id == 0 && continuation => None,
            _ if prev_frame_id == 0 => Some(frame_id),
            _ => Some(prev_frame_id),
        };
        match missing {
            None => self.take(gop_id, frame_id),
            Some(lost) => {
                self.broken = Some(gop_id);
                Verdict::Lost(FrameLoss {
                    gop_id,
                    frame_id: lost,
                })
            }
        }
    }

    fn take(&mut self, gop_id: u64, frame_id: u64) -> Verdict {
        self.current = Some((gop_id, frame_id));
        if self.broken.is_some_and(|broken| broken <= gop_id) {
            self.broken = None;
        }
        Verdict::Deliver
    }

    /// Whether the chain of `gop_id` is broken now.
    pub fn broken(&self, gop_id: u64) -> bool {
        self.broken.is_some_and(|broken| gop_id <= broken)
    }
}
