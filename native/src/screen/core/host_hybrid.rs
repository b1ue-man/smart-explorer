//! Video for changed regions, lossless repair for stable regions, one reference.
use super::frame_budget::FrameContract;
use super::frame_protocol::{self, FrameStamp, Operation};
use super::frame_regions::{grid, intersects, matches_move, same, visible};
use super::host_backend::{EncodedFrame, ScreenEncoder};
use super::host_stage::StageRequest;
use super::pixel_convert::{BgraImage, FrameSize, Rect};
use super::wire::invalid;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

static GENERATION: AtomicU64 = AtomicU64::new(1);
fn generation(epoch: u64) -> io::Result<u64> {
    GENERATION
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |previous| {
            previous.max(epoch).checked_add(1)
        })
        .map(|previous| previous.max(epoch) + 1)
        .map_err(|_| invalid("screen reference generation exhausted"))
}

pub(crate) fn wrap(
    encoder: Box<dyn ScreenEncoder>,
    request: &StageRequest,
) -> io::Result<Box<dyn ScreenEncoder>> {
    let Some(contract) = request.frame_contract.filter(|c| c.hybrid_regions) else {
        return Ok(encoder);
    };
    contract.validate()?;
    contract.limits.check(request.size)?;
    Ok(Box::new(HybridEncoder {
        video: encoder,
        contract,
        generation: generation(request.encoder_epoch)?,
        previous: None,
        pending: Vec::new(),
        last_frame: 0,
        tiles: request.choice.codec == super::negotiate::CodecKind::Tiles,
        visible_rects: request.visible_rects.clone(),
    }))
}
pub(crate) struct HybridEncoder {
    video: Box<dyn ScreenEncoder>,
    contract: FrameContract,
    generation: u64,
    previous: Option<BgraImage>,
    pending: Vec<bool>,
    last_frame: u64,
    tiles: bool,
    visible_rects: Option<Vec<super::visibility::VisibleRect>>,
}
impl ScreenEncoder for HybridEncoder {
    fn name(&self) -> String {
        self.video.name()
    }
    fn size(&self) -> FrameSize {
        self.video.size()
    }
    fn needs_refresh(&self) -> bool {
        grid(self.size())
            .iter()
            .zip(&self.pending)
            .any(|(r, p)| *p && visible(*r, self.visible_rects.as_deref()))
    }
    fn encode(&mut self, _frame: &BgraImage, _key: bool, _bps: u64) -> io::Result<EncodedFrame> {
        Err(invalid("hybrid encoder requires a stamped StageRequest"))
    }
    fn encode_frame(
        &mut self,
        image: &BgraImage,
        request: &StageRequest,
    ) -> io::Result<EncodedFrame> {
        self.encode_impl(image, request, None)
    }
}
impl HybridEncoder {
    pub(crate) fn for_external(request: &StageRequest) -> io::Result<Self> {
        let contract = request
            .frame_contract
            .ok_or_else(|| invalid("external hybrid frame has no negotiated contract"))?;
        Ok(Self {
            video: Box::new(super::host_backend::TilesScreenEncoder::new_with_limits(
                request.size,
                contract.limits,
            )?),
            contract,
            generation: generation(request.encoder_epoch)?,
            previous: None,
            pending: Vec::new(),
            last_frame: 0,
            tiles: false,
            visible_rects: request.visible_rects.clone(),
        })
    }
    pub(crate) fn repeat_external(&mut self, request: &StageRequest) -> io::Result<EncodedFrame> {
        self.visible_rects = request.visible_rects.clone();
        if !request.keyframe && !self.needs_refresh() {
            return Ok(EncodedFrame {
                records: Vec::new(),
                keyframe: false,
            });
        }
        let image = self
            .previous
            .clone()
            .ok_or_else(|| invalid("unchanged capture has no confirmed source pixels"))?;
        self.encode_impl(&image, request, None)
    }
    pub(crate) fn encode_external(
        &mut self,
        image: &BgraImage,
        request: &StageRequest,
        video: EncodedFrame,
    ) -> io::Result<EncodedFrame> {
        let video = (!video.records.is_empty()).then_some(video);
        self.encode_impl(image, request, video)
    }
    fn encode_impl(
        &mut self,
        image: &BgraImage,
        request: &StageRequest,
        external: Option<EncodedFrame>,
    ) -> io::Result<EncodedFrame> {
        self.visible_rects = request.visible_rects.clone();
        self.contract.limits.check(image.size())?;
        if image.size() != self.size() || request.frame_id <= self.last_frame {
            return Err(invalid("hybrid encoder frame reordered or resized"));
        }
        let key = request.keyframe || self.previous.is_none();
        let budget = super::frame_size_caps::PacketBudget::for_contract(Some(self.contract))?;
        let rects = grid(image.size());
        if self.pending.len() != rects.len() {
            self.pending = vec![false; rects.len()];
        }
        let regions = image
            .regions()
            .filter(|regions| regions.validate(image.size()).is_ok());
        let changed: Vec<bool> = rects
            .iter()
            .map(|r| {
                let touched = regions.is_none_or(|regions| {
                    regions.dirty.iter().any(|dirty| intersects(*r, *dirty))
                        || regions
                            .moves
                            .iter()
                            .any(|moved| intersects(*r, moved.destination))
                });
                key || (touched && self.previous.as_ref().is_none_or(|p| !same(p, image, *r)))
            })
            .collect();
        let external_video = external.is_some();
        if !external_video && !key && !changed.iter().any(|c| *c) && !self.needs_refresh() {
            return Ok(EncodedFrame {
                records: Vec::new(),
                keyframe: false,
            });
        }
        let stamp = FrameStamp {
            generation: self.generation,
            frame_id: request.frame_id,
            base_frame_id: if key { 0 } else { self.last_frame },
        };
        let mut records = vec![frame_protocol::encode(stamp, Operation::Begin)?];
        let mut video_rects: Vec<Rect> = rects
            .iter()
            .zip(&changed)
            .filter_map(|(r, c)| (key || *c).then_some(*r))
            .collect();
        if external_video && video_rects.is_empty() {
            video_rects.push(rects[0]);
            self.pending[0] = true;
        }
        if !self.tiles && !video_rects.is_empty() {
            let encoded = match external {
                Some(encoded) => encoded,
                None => self.video.encode(image, key, request.target_bps)?,
            };
            if encoded.records.len() != 1 || (key && !encoded.keyframe) {
                return Err(invalid(
                    "hybrid video encoder did not produce one complete reference access unit",
                ));
            }
            let data = encoded
                .records
                .into_iter()
                .next()
                .ok_or_else(|| invalid("empty hybrid video"))?;
            if data
                .len()
                .checked_add(30)
                .and_then(|n| n.checked_add(video_rects.len() * 16))
                .is_none_or(|n| n > budget.record_bytes)
            {
                return Err(invalid("hybrid video exceeds negotiated record memory"));
            }
            records.push(frame_protocol::encode(
                stamp,
                Operation::Video {
                    keyframe: encoded.keyframe,
                    rects: video_rects,
                    data,
                },
            )?);
        }
        // Moves read the previous composite, never the already modified destination.
        if !key && self.visible_rects.is_none() {
            if let (Some(regions), Some(previous)) = (image.regions(), self.previous.as_ref()) {
                regions.validate(image.size())?;
                let mut moves = Vec::new();
                for moved in &regions.moves {
                    let source = Rect {
                        x: moved.source_x,
                        y: moved.source_y,
                        ..moved.destination
                    };
                    let exact_source = !rects
                        .iter()
                        .enumerate()
                        .any(|(index, rect)| self.pending[index] && intersects(*rect, source));
                    if exact_source && matches_move(previous, image, *moved) {
                        moves.push(*moved);
                    }
                }
                // Moves are an optimization. Dirty/video regions already cover
                // the destinations if one move list cannot fit this contract.
                if !moves.is_empty()
                    && moves
                        .len()
                        .checked_mul(24)
                        .and_then(|n| n.checked_add(29))
                        .is_some_and(|n| n <= budget.record_bytes)
                {
                    records.push(frame_protocol::encode(stamp, Operation::Moves(moves))?);
                }
            }
        }
        for (index, rect) in rects.into_iter().enumerate() {
            // Keyframes establish exact pixels. Afterwards only newly stable tiles are repaired.
            let send = visible(rect, self.visible_rects.as_deref());
            if key
                || (send
                    && ((self.tiles && changed[index]) || (!changed[index] && self.pending[index])))
            {
                records.push(frame_protocol::encode(
                    stamp,
                    frame_protocol::lossless(image, rect)?,
                )?);
                self.pending[index] = false;
            } else if changed[index] {
                self.pending[index] = true;
            }
        }
        if records.len() == 1 {
            self.previous = Some(image.clone());
            return Ok(EncodedFrame {
                records: Vec::new(),
                keyframe: false,
            });
        }
        records.push(frame_protocol::encode(stamp, Operation::Commit)?);
        let total = records.iter().try_fold(0usize, |n, r| {
            n.checked_add(r.len()).and_then(|n| n.checked_add(21))
        });
        if records.len() > budget.records
            || total.is_none_or(|n| n > budget.encoded_bytes)
            || records.iter().any(|r| r.len() > budget.record_bytes)
        {
            return Err(invalid("hybrid transaction exceeds negotiated memory"));
        }
        self.previous = Some(image.clone());
        self.last_frame = request.frame_id;
        Ok(EncodedFrame {
            records,
            keyframe: key,
        })
    }
}

#[path = "captured_hybrid.rs"]
mod captured;
pub(crate) use captured::{wrap_captured, CapturedExternalHybrid};
