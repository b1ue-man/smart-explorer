//! Productive hybrid records retain every admitted byte owner.
use super::*;
/// Productive hybrid wrapper; mutable previous pixels and record copies never
/// escape the capture grant. The existing raw codec wrapper remains for benches.
pub(crate) fn wrap_captured(
    video: Box<dyn ScreenEncoder>,
    request: &StageRequest,
    grant: super::super::capture_admission::CaptureGrant,
) -> io::Result<Box<dyn ScreenEncoder>> {
    let Some(contract) = request
        .frame_contract
        .filter(|contract| contract.hybrid_regions)
    else {
        return Ok(video);
    };
    let count =
        request.size.width.div_ceil(64) as usize * request.size.height.div_ceil(64) as usize;
    let pending_lease = grant.claim(
        super::super::capture_admission::CaptureOwner::Metadata,
        count as u64,
    )?;
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(count)
        .map_err(|_| io::Error::other("hybrid pending allocation failed"))?;
    pending.resize(count, false);
    Ok(Box::new(CapturedHybrid {
        video,
        previous: None,
        pending,
        _pending_lease: pending_lease,
        grant,
        contract,
        generation: generation(request.encoder_epoch)?,
        last_frame: 0,
        tiles: request.choice.codec == super::super::negotiate::CodecKind::Tiles,
        visible: None,
    }))
}
struct CapturedHybrid {
    video: Box<dyn ScreenEncoder>,
    previous: Option<super::super::capture_image::CapturedImage>,
    pending: Vec<bool>,
    _pending_lease: super::super::capture_admission::CaptureSlice,
    grant: super::super::capture_admission::CaptureGrant,
    contract: FrameContract,
    generation: u64,
    last_frame: u64,
    tiles: bool,
    visible: Option<CapturedVisibility>,
}
struct CapturedVisibility {
    rects: Vec<super::super::visibility::VisibleRect>,
    _lease: super::super::capture_admission::CaptureSlice,
}
impl std::ops::Deref for CapturedVisibility {
    type Target = [super::super::visibility::VisibleRect];
    fn deref(&self) -> &Self::Target {
        &self.rects
    }
}
impl CapturedHybrid {
    fn record(
        &self,
        stamp: FrameStamp,
        operation: Operation,
        capacity: u64,
    ) -> io::Result<super::super::capture_bytes::AdmittedBytes> {
        super::super::capture_bytes::AdmittedBytes::from_allocator(
            &self.grant,
            super::super::capture_admission::CaptureOwner::CodecOutput,
            capacity,
            || frame_protocol::encode(stamp, operation),
        )
    }
    fn captured(
        &mut self,
        image: &super::super::capture_image::CapturedImage,
        request: &StageRequest,
    ) -> io::Result<super::super::host_backend::CapturedEncodedFrame> {
        self.captured_external(image, request, None)
    }
    fn captured_external(
        &mut self,
        image: &super::super::capture_image::CapturedImage,
        request: &StageRequest,
        external: Option<super::super::host_backend::CapturedEncodedFrame>,
    ) -> io::Result<super::super::host_backend::CapturedEncodedFrame> {
        let result = self.captured_impl(image, request, external);
        if result.is_err() {
            self.previous = None;
            self.pending.fill(false);
            self.visible = None;
        }
        result
    }
    fn captured_impl(
        &mut self,
        image: &super::super::capture_image::CapturedImage,
        request: &StageRequest,
        external: Option<super::super::host_backend::CapturedEncodedFrame>,
    ) -> io::Result<super::super::host_backend::CapturedEncodedFrame> {
        use super::super::capture_admission::CaptureOwner as O;
        use super::super::capture_records::{CaptureRecordBuilder, CapturedRecords};
        self.grant
            .check_identity(image.source(), image.generation())?;
        self.contract.limits.check(image.size())?;
        if image.size() != self.video.size() || request.frame_id <= self.last_frame {
            return Err(invalid("hybrid capture reordered or resized"));
        }
        let count = self.pending.len();
        let metadata_bytes = (count as u64)
            .checked_mul(
                (2 * std::mem::size_of::<Rect>()
                    + 1
                    + 2 * std::mem::size_of::<super::super::frame_regions::MoveRect>())
                    as u64,
            )
            .ok_or_else(|| invalid("hybrid temporary metadata overflow"))?;
        let _metadata = self.grant.claim(O::Metadata, metadata_bytes)?;
        let mut rects = Vec::new();
        rects
            .try_reserve_exact(count)
            .map_err(|_| io::Error::other("hybrid grid allocation failed"))?;
        for y in (0..image.size().height).step_by(64) {
            for x in (0..image.size().width).step_by(64) {
                rects.push(Rect {
                    x,
                    y,
                    width: 64.min(image.size().width - x),
                    height: 64.min(image.size().height - y),
                });
            }
        }
        let key = request.keyframe || self.previous.is_none();
        let regions = image
            .image()
            .regions()
            .filter(|regions| regions.validate(image.size()).is_ok());
        let mut changed = Vec::new();
        changed
            .try_reserve_exact(count)
            .map_err(|_| io::Error::other("hybrid change allocation failed"))?;
        changed.extend(rects.iter().map(|rect| {
            key || (regions.is_none_or(|regions| {
                regions.dirty.iter().any(|dirty| intersects(*rect, *dirty))
                    || regions
                        .moves
                        .iter()
                        .any(|moved| intersects(*rect, moved.destination))
            }) && self
                .previous
                .as_ref()
                .is_none_or(|previous| !same(previous.image(), image.image(), *rect)))
        }));
        self.visible = None;
        if let Some(rects) = &request.visible_rects {
            let lease = self.grant.claim(
                O::Metadata,
                (rects.len() * std::mem::size_of::<super::super::visibility::VisibleRect>()) as u64,
            )?;
            let mut admitted = Vec::new();
            admitted
                .try_reserve_exact(rects.len())
                .map_err(|_| io::Error::other("hybrid visibility allocation failed"))?;
            admitted.extend_from_slice(rects);
            self.visible = Some(CapturedVisibility {
                rects: admitted,
                _lease: lease,
            });
        }
        let external = external.filter(|frame| !frame.records.is_empty());
        let has_external = external.is_some();
        if !has_external && !key && !changed.iter().any(|changed| *changed) && !self.needs_refresh()
        {
            return Ok(super::super::host_backend::CapturedEncodedFrame {
                records: CapturedRecords::empty(&self.grant)?,
                keyframe: false,
            });
        }
        let packet =
            super::super::frame_size_caps::PacketBudget::for_contract(Some(self.contract))?;
        let stamp = FrameStamp {
            generation: self.generation,
            frame_id: request.frame_id,
            base_frame_id: if key { 0 } else { self.last_frame },
        };
        let mut records = CaptureRecordBuilder::new(
            &self.grant,
            packet.encoded_bytes as u64,
            packet.records as u64,
        )?;
        records.push(self.record(stamp, Operation::Begin, 25)?)?;
        if !self.tiles && (has_external || changed.iter().any(|changed| *changed)) {
            let mut video_rects = Vec::new();
            video_rects
                .try_reserve_exact(count)
                .map_err(|_| io::Error::other("hybrid video grid allocation failed"))?;
            video_rects.extend(
                rects
                    .iter()
                    .zip(&changed)
                    .filter_map(|(rect, changed)| (key || *changed).then_some(*rect)),
            );
            if has_external && video_rects.is_empty() {
                let first = rects
                    .first()
                    .copied()
                    .ok_or_else(|| invalid("hybrid grid is empty"))?;
                video_rects.push(first);
                self.pending[0] = true;
            }
            let mut encoded = match external {
                Some(frame) => frame,
                None if key == request.keyframe => self.video.encode_captured(image, request)?,
                None => {
                    let forced = StageRequest {
                        frame_id: request.frame_id,
                        frame_contract: request.frame_contract,
                        visible_rects: None,
                        keyframe: key,
                        target_bps: request.target_bps,
                        size: request.size,
                        choice: request.choice,
                        encoder_epoch: request.encoder_epoch,
                        fps: request.fps,
                    };
                    self.video.encode_captured(image, &forced)?
                }
            };
            if encoded.records.len() != 1 || (key && !encoded.keyframe) {
                return Err(invalid("hybrid video is not one complete access unit"));
            }
            let bytes = encoded
                .records
                .pop()?
                .ok_or_else(|| invalid("hybrid video is empty"))?;
            let capacity = bytes
                .len()
                .checked_add(30)
                .and_then(|n| n.checked_add(video_rects.len() * 16))
                .filter(|n| *n <= packet.record_bytes)
                .ok_or_else(|| invalid("hybrid video exceeds record budget"))?;
            let (data, _input_lease) = bytes.into_parts();
            records.push(self.record(
                stamp,
                Operation::Video {
                    keyframe: encoded.keyframe,
                    rects: video_rects,
                    data,
                },
                capacity as u64,
            )?)?;
        }
        if !key && self.visible.is_none() {
            if let (Some(regions), Some(previous)) = (regions, self.previous.as_ref()) {
                let mut moves = Vec::new();
                moves
                    .try_reserve_exact(count * 2)
                    .map_err(|_| io::Error::other("hybrid move allocation failed"))?;
                for moved in &regions.moves {
                    let source = Rect {
                        x: moved.source_x,
                        y: moved.source_y,
                        ..moved.destination
                    };
                    if !rects
                        .iter()
                        .enumerate()
                        .any(|(index, rect)| self.pending[index] && intersects(*rect, source))
                        && matches_move(previous.image(), image.image(), *moved)
                    {
                        moves.push(*moved);
                    }
                }
                let capacity = moves.len().checked_mul(24).and_then(|n| n.checked_add(29));
                if !moves.is_empty() && capacity.is_some_and(|n| n <= packet.record_bytes) {
                    records.push(self.record(
                        stamp,
                        Operation::Moves(moves),
                        capacity.unwrap_or(0) as u64,
                    )?)?;
                }
            }
        }
        for (index, rect) in rects.into_iter().enumerate() {
            if key
                || (visible(rect, self.visible.as_deref())
                    && ((self.tiles && changed[index]) || (!changed[index] && self.pending[index])))
            {
                let scratch_bytes =
                    64 * 64 * 4 + lz4_flex::block::get_maximum_output_size(64 * 64 * 4) + 4;
                let _scratch = self.grant.claim(O::FilterScratch, scratch_bytes as u64)?;
                let operation = frame_protocol::lossless(image.image(), rect)?;
                let capacity = match &operation {
                    Operation::Dirty { data, .. } => data.len() + 41,
                    _ => 0,
                };
                if capacity > packet.record_bytes {
                    return Err(invalid("hybrid lossless record exceeds contract"));
                }
                records.push(self.record(stamp, operation, capacity as u64)?)?;
                self.pending[index] = false;
            } else if changed[index] {
                self.pending[index] = true;
            }
        }
        let mut previous = super::super::capture_image::CaptureImageBuilder::new(
            image.size(),
            self.contract.limits,
            &self.grant,
            O::CodecInput,
        )?;
        previous.copy_from(&image.view())?;
        self.previous = Some(previous.freeze());
        if records.len() == 1 {
            return Ok(super::super::host_backend::CapturedEncodedFrame {
                records: CapturedRecords::empty(&self.grant)?,
                keyframe: false,
            });
        }
        records.push(self.record(stamp, Operation::Commit, 25)?)?;
        self.last_frame = request.frame_id;
        Ok(super::super::host_backend::CapturedEncodedFrame {
            records: records.freeze(),
            keyframe: key,
        })
    }
}

impl ScreenEncoder for CapturedHybrid {
    fn name(&self) -> String {
        self.video.name()
    }
    fn size(&self) -> FrameSize {
        self.video.size()
    }
    fn needs_refresh(&self) -> bool {
        let size = self.size();
        self.pending.iter().enumerate().any(|(index, pending)| {
            *pending
                && visible(
                    super::super::tiles_codec::tile_rect(
                        index as u32 % size.width.div_ceil(64),
                        index as u32 / size.width.div_ceil(64),
                        size,
                    ),
                    self.visible.as_deref(),
                )
        })
    }
    fn encode_captured(
        &mut self,
        image: &super::super::capture_image::CapturedImage,
        request: &StageRequest,
    ) -> io::Result<super::super::host_backend::CapturedEncodedFrame> {
        self.captured(image, request)
    }
    fn encode(&mut self, _image: &BgraImage, _key: bool, _bps: u64) -> io::Result<EncodedFrame> {
        Err(invalid("captured hybrid requires admitted input"))
    }
}

#[path = "captured_external_hybrid.rs"]
mod external;
pub(crate) use external::CapturedExternalHybrid;
