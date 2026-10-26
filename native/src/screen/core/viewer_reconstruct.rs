//! Atomic reconstruction of one v2 frame. No partial or mixed-base picture escapes.
#[cfg(test)]
#[path = "batches_reconstruction_task_tests.rs"]
mod batches_task_tests;

use super::decoded_frame::{DecodedFrame, DecoderFallback};
use super::frame_budget::FrameContract;
use super::frame_protocol::{self, FrameStamp, Operation};
use super::frame_regions::copy_rect;
use super::host_backend::ScreenDecoder;
use super::pixel_convert::{BgraImage, Rect};
use super::wire::invalid;
use super::wire_binary::{FrameRecord, FLAG_KEYFRAME};
use std::io;

pub(crate) struct HybridDecoder {
    video: Box<dyn ScreenDecoder>,
    contract: FrameContract,
    image: BgraImage,
    staging: Option<BgraImage>,
    stamp: Option<FrameStamp>,
    last: Option<FrameStamp>,
    keyframe: bool,
    order: u8,
    video_valid: bool,
    covered: Vec<bool>,
    operations: usize,
    input: Option<DecodedFrame>,
    decoded: Option<DecodedFrame>,
    /// Retained on transaction failure: the raw decoder may still hold that AU.
    video_input: Option<FrameStamp>,
}
impl HybridDecoder {
    pub(crate) fn new(
        video: Box<dyn ScreenDecoder>,
        size: super::pixel_convert::FrameSize,
        contract: FrameContract,
    ) -> io::Result<Self> {
        contract.validate()?;
        Ok(Self {
            video,
            contract,
            image: BgraImage::new_with_limits(size, contract.limits)?,
            staging: None,
            stamp: None,
            last: None,
            keyframe: false,
            order: 0,
            video_valid: false,
            covered: Vec::new(),
            operations: 0,
            input: None,
            decoded: None,
            video_input: None,
        })
    }
    fn cover(&mut self, rect: Rect) {
        let size = self.image.size();
        let columns = size.width.div_ceil(64);
        for y in rect.y / 64..=(rect.y + rect.height - 1) / 64 {
            for x in rect.x / 64..=(rect.x + rect.width - 1) / 64 {
                let left = x * 64;
                let top = y * 64;
                if rect.x <= left
                    && rect.y <= top
                    && rect.x + rect.width >= (left + 64).min(size.width)
                    && rect.y + rect.height >= (top + 64).min(size.height)
                {
                    self.covered[(y * columns + x) as usize] = true;
                }
            }
        }
    }
    fn apply(&mut self, record: &FrameRecord) -> io::Result<bool> {
        let budget = super::frame_size_caps::PacketBudget::for_contract(Some(self.contract))?;
        if record.payload.len() > budget.record_bytes {
            return Err(invalid("hybrid record exceeds negotiated working memory"));
        }
        let (stamp, op) = frame_protocol::decode(&record.payload, self.image.size())?;
        if stamp.frame_id != record.frame_id {
            return Err(invalid("hybrid inner/outer frame id mismatch"));
        }
        if matches!(op, Operation::Begin) {
            if self.stamp.is_some() || !record.has_more() {
                return Err(invalid("hybrid transaction interrupted"));
            }
            let key = record.is_keyframe();
            if key {
                if stamp.base_frame_id != 0
                    || self.last.is_some_and(|last| {
                        stamp.frame_id <= last.frame_id || stamp.generation < last.generation
                    })
                {
                    return Err(invalid(
                        "hybrid keyframe does not rebuild a fresh reference",
                    ));
                }
            } else if self.last.is_none_or(|last| {
                last.generation != stamp.generation
                    || last.frame_id != stamp.base_frame_id
                    || stamp.frame_id <= last.frame_id
            }) {
                return Err(invalid("hybrid frame references a missing base"));
            }
            self.contract.limits.check(self.image.size())?;
            self.staging = Some(if key {
                BgraImage::new_with_limits(self.image.size(), self.contract.limits)?
            } else {
                self.image.clone()
            });
            self.covered = vec![false; super::frame_regions::grid(self.image.size()).len()];
            self.stamp = Some(stamp);
            self.keyframe = key;
            self.order = 0;
            self.operations = 0;
            self.input = Some(DecodedFrame::from_record(record));
            return Ok(false);
        }
        if self.stamp != Some(stamp) || self.input != Some(DecodedFrame::from_record(record)) {
            return Err(invalid(
                "hybrid operation uses a different frame, capture time or base generation",
            ));
        }
        self.operations += 1;
        if self.operations > self.covered.len().saturating_mul(3).saturating_add(4) {
            return Err(invalid("hybrid transaction exceeds operation budget"));
        }
        match op {
            Operation::Video {
                keyframe,
                rects,
                data,
            } => {
                if self.order != 0 || (!self.video_valid && !keyframe) {
                    return Err(invalid("hybrid video has no decoder reference"));
                }
                if self.video_input.is_some_and(|previous| {
                    stamp.frame_id <= previous.frame_id || stamp.generation < previous.generation
                }) {
                    return Err(invalid("hybrid video frame stamp was reused or reordered"));
                }
                let unit = FrameRecord {
                    frame_id: record.frame_id,
                    capture_ts_us: record.capture_ts_us,
                    flags: if keyframe { FLAG_KEYFRAME } else { 0 },
                    payload: data,
                };
                self.video_input = Some(stamp);
                if !self.video.decode(&unit)? {
                    return Err(DecoderFallback::NeedsMoreInput.error());
                }
                // The current operation's complete FrameStamp is pinned above. A
                // delayed video output must not be composited into this transaction.
                if self.video.decoded_frame() != Some(DecodedFrame::from_record(&unit)) {
                    return Err(DecoderFallback::OutputIdentityUnavailable.error());
                }
                self.video_valid = true;
                for rect in rects {
                    let staging = self
                        .staging
                        .as_mut()
                        .ok_or_else(|| invalid("missing hybrid staging picture"))?;
                    copy_rect(self.video.picture(), rect, staging, rect)?;
                    self.cover(rect);
                }
                self.order = 1;
            }
            Operation::Moves(moves) => {
                if self.keyframe || self.order > 1 {
                    return Err(invalid("hybrid moves must precede dirty regions"));
                }
                let staging = self
                    .staging
                    .as_mut()
                    .ok_or_else(|| invalid("missing hybrid staging picture"))?;
                for moved in moves {
                    copy_rect(
                        &self.image,
                        Rect {
                            x: moved.source_x,
                            y: moved.source_y,
                            ..moved.destination
                        },
                        staging,
                        moved.destination,
                    )?;
                }
                self.order = 2;
            }
            Operation::Dirty { rect, data } => {
                let staging = self
                    .staging
                    .as_mut()
                    .ok_or_else(|| invalid("missing hybrid staging picture"))?;
                frame_protocol::apply_dirty(staging, rect, &data)?;
                self.cover(rect);
                self.order = 3;
            }
            Operation::Commit => {
                if record.has_more() || (self.keyframe && self.covered.iter().any(|c| !c)) {
                    return Err(invalid("hybrid reference is incomplete"));
                }
                self.image = self
                    .staging
                    .take()
                    .ok_or_else(|| invalid("missing hybrid staging picture"))?;
                self.decoded = self.input.take();
                self.last = Some(stamp);
                self.stamp = None;
                return Ok(true);
            }
            Operation::Begin => return Err(invalid("duplicate hybrid begin")),
        }
        if !record.has_more() {
            return Err(invalid("hybrid frame ended without commit"));
        }
        Ok(false)
    }
}
impl ScreenDecoder for HybridDecoder {
    fn name(&self) -> String {
        format!("{}+regions-v2", self.video.name())
    }
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        let result = self.apply(record);
        if result.is_err() {
            self.staging = None;
            self.stamp = None;
            self.last = None;
            self.video_valid = false;
            self.input = None;
        }
        result
    }
    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }
    fn picture(&self) -> &BgraImage {
        &self.image
    }
}
