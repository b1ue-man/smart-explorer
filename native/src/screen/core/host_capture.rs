//! Capture and encoding of the host pipeline (SCR-M2, SCR-M3, decisions C3,
//! C6), run on the blocking pool: one frame of the source scaled to the size
//! of the picture group and encoded, either here with the backend's encoder
//! (opened on first use) or in the host process when the source was opened
//! there (`ScreenSource::host_stage`); and `SNAPSHOT`, one lossless still
//! picture as tiles on its own picture stream of the kind `snapshot`.
use std::io;
use std::time::Duration;

use iroh::endpoint::Connection;

use super::{Pipeline, PipelineError};
use crate::screen::core::gop_stream::{now_us, GopWriter};
use crate::screen::core::host_backend::{
    CapturedEncodedFrame as EncodedFrame, ScreenEncoder, ScreenHostBackend, ScreenSource, SourceFrame,
};
use crate::screen::core::host_stage::{StageEncoder, StageFrame, StageRequest};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pacing::STALE_STREAM_US;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::frame_budget::{FrameContract, FrameLimits};
use crate::screen::core::frame_size_caps::{PacketBudget, validate_encoded_budget};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_binary::{GopHeader, StreamKind};

/// Source and encoder, moved to the blocking pool for each frame.
pub(super) struct Stage {
    pub(super) source: Box<dyn ScreenSource>,
    pub(super) encoder: Option<Box<dyn ScreenEncoder>>,
    pub(super) encoder_book: Option<usize>,
    pub(super) memory: super::super::frame_memory::MemoryHold,
}

/// A detached/aborted blocking task drops its result before the Stage lease.
pub(super) struct StageOutput<T> { pub(super) result:T, pub(super) stage:Stage }
struct Snapshot { records:crate::screen::core::capture_records::CapturedRecords, size:FrameSize, capture_ts_us:u64, contract:Option<FrameContract>, family_layout_revision:Option<u64> }
impl Snapshot {
    fn header(&self,gop_id:u64)->GopHeader {
        GopHeader {family_layout_revision:self.family_layout_revision,gop_id,frame_contract:self.contract,
            kind:StreamKind::Snapshot,codec:CodecKind::Tiles,profile:None,size:self.size}
    }
}

/// What capture and encoding of one frame gave.
pub(super) enum Captured {
    Encoded {
        frame: EncodedFrame,
        capture_ts_us: u64,
        encoder: StageEncoder,
    },
    Unchanged,
    Paused(SessionState),
}

impl Pipeline {
    /// `SNAPSHOT`: one lossless key frame of the current source, at most
    /// `max` large, on a picture stream of the kind `snapshot`.
    pub(crate) async fn snapshot(
        &mut self,
        connection: &Connection,
        max: Option<FrameSize>,
    ) -> Result<(), PipelineError> {
        self.snapshot_epoch(connection,max,self.work.epoch()).await
    }
    pub(super) async fn snapshot_epoch(&mut self,connection:&Connection,max:Option<FrameSize>,work_epoch:u64)->Result<(),PipelineError> {
        let work=self.work.clone();
        if !work.current(work_epoch) {return Err(PipelineError::capture(io::Error::from(io::ErrorKind::Interrupted)));}
        // Snapshot consumes a newer GOP id: retire live state now so its next
        // frame is a fresh keyframe and old presentation receipts stay invalid.
        self.reset_stream();
        let mut stage = self
            .stage
            .take()
            .ok_or_else(|| PipelineError::capture(io::Error::other("no screen source")))?;
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;
        let contract=self.frame_contract;
        let limit = max.unwrap_or_else(||contract.map_or(FrameSize::MAX,|c|c.limits.max_size));
        let joined = tokio::task::spawn_blocking(move || {
            let result = still_tiles(&mut stage, frame_id, limit, contract);
            StageOutput {result,stage}
        })
        .await;
        let StageOutput {stage,result:shot} =
            joined.map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(stage);
        if !work.current(work_epoch) {self.reset_stream();return Err(PipelineError::capture(io::Error::from(io::ErrorKind::Interrupted)));}
        self.sync_source_reference();
        let shot = shot?;
        let header = shot.header(self.next_gop_id);
        let Snapshot {records,capture_ts_us,..}=shot;
        self.next_gop_id += 1;
        let stale = Duration::from_micros(STALE_STREAM_US);
        let opened=tokio::select! {biased;
            _=work.cancelled(work_epoch)=>return Err(PipelineError::capture(io::Error::from(io::ErrorKind::Interrupted))),
            result=tokio::time::timeout(stale,GopWriter::open(connection,header))=>result,
        };
        let mut gop = opened
            .map_err(|error| PipelineError::capture(io::Error::other(error)))?
            .map_err(PipelineError::capture)?;
        let written=tokio::select! {biased;
            _=work.cancelled(work_epoch)=>Err(io::Error::from(io::ErrorKind::Interrupted)),
            result=gop.write_frame_admitted(frame_id,capture_ts_us,true,records)=>result,
        };
        if !work.current(work_epoch) {gop.reset();return Err(PipelineError::capture(io::Error::from(io::ErrorKind::Interrupted)));}
        match written {
            Ok(_)=>{gop.finish();Ok(())},
            Err(error)=>{gop.reset();Err(PipelineError::capture(error))},
        }
    }
}

/// One still picture of the stage's source as tile records within `limit`.
fn still_tiles(
    stage: &mut Stage,
    frame_id: u64,
    limit: FrameSize,
    contract:Option<FrameContract>,
) -> Result<Snapshot, PipelineError> {
    let contract=contract.map(|mut contract| {contract.hybrid_regions=false;contract});
    if let Some(contract)=contract {contract.validate().map_err(PipelineError::capture)?;}
    let limits=contract.map_or(FrameLimits::LEGACY,|c|c.limits)
        .intersect(stage.source.frame_limits_for_contract(contract));
    let capture_ts_us = now_us();
    let image = match stage
        .source
        .capture_snapshot(frame_id, contract)
        .map_err(PipelineError::capture)?
    {
        SourceFrame::Frame(image) => image,
        SourceFrame::Unchanged | SourceFrame::Paused(_) => {
            let error = io::Error::other("the source has no picture to save");
            return Err(PipelineError::capture(error));
        }
    };
    let size = limits.fit(image.size().fit_within(limit));
    limits.check(size).map_err(PipelineError::capture)?;
    let image = if image.size() == size {
        image
    } else {
        image.downscale(size,limits,&image.grant(),crate::screen::core::capture_admission::CaptureOwner::ScaledPixels).map_err(PipelineError::capture)?
    };
    let budget=PacketBudget::for_contract(contract).map_err(PipelineError::encoder)?;
    let mut encoder=crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(size,limits,image.grant()).map_err(PipelineError::encoder)?;
    // The tile API's minimum is one full tile; a smaller image still emits
    // only its actual pixels. Validate actual records against the contract.
    let request=StageRequest {frame_id,frame_contract:contract,visible_rects:None,keyframe:true,target_bps:1,size,
        choice:crate::screen::core::negotiate::CodecChoice {codec:CodecKind::Tiles,profile:None,
            encoder_hardware:false,decoder_hardware:false,max_size:size,max_pixels:size.pixels(),max_macroblocks_per_second:None},
        encoder_epoch:frame_id,fps:1};
    let records=encoder.encode_captured(&image,&request).map_err(PipelineError::encoder)?.records;
    if records.iter().any(|record|record.len()>budget.record_bytes) {
        return Err(PipelineError::encoder(io::Error::other("snapshot record exceeds accepted budget")));
    }
    validate_encoded_budget(contract,CodecKind::Tiles,&records).map_err(PipelineError::encoder)?;
    Ok(Snapshot {records,size,capture_ts_us,contract,family_layout_revision:stage.source.family_layout_revision()})
}

/// One frame as `request` says: in the host process when the source runs
/// there, else here with the backend's encoder, opened on first use.
pub(super) fn produce(
    stage: &mut Stage,
    backend: &dyn ScreenHostBackend,
    request: &StageRequest,
) -> Result<Captured, PipelineError> {
    match stage.source.prepare_capture(request) {
        Ok(Some(state)) => return Ok(Captured::Paused(state)),
        Ok(None) => {},
        Err(error) if error.kind() == io::ErrorKind::WouldBlock =>
            return Ok(Captured::Paused(SessionState::Starting)),
        Err(error) => return Err(PipelineError::capture(error)),
    }
    if request.size.width==0 || request.size.height==0 {return Ok(Captured::Unchanged);}
    if let Some(host) = stage.source.host_stage() {
        return Ok(match host.produce(request)? {
            StageFrame::Encoded {
                records,
                keyframe,
                capture_ts_us,
                encoder,
                ..
            } => {
                super::super::frame_size_caps::validate_encoded_budget(request.frame_contract,request.choice.codec,&records)
                    .map_err(PipelineError::encoder)?;
                Captured::Encoded {
                frame: EncodedFrame { records, keyframe },
                capture_ts_us,
                encoder,
            }},
            StageFrame::Unchanged { .. } => Captured::Unchanged,
            StageFrame::Paused(state) => Captured::Paused(state),
        });
    }
    capture_and_encode(stage, backend, request)
}

/// Captures the picture of the request's frame, scales it to the request's
/// size and encodes it with the stage's encoder.
fn capture_and_encode(
    stage: &mut Stage,
    backend:&dyn ScreenHostBackend,
    request: &StageRequest,
) -> Result<Captured, PipelineError> {
    let capture_ts_us = now_us();
    let generation=stage.source.reference_generation();
    let captured = stage
        .source
        .capture(request.frame_id, request.keyframe || stage.encoder.as_ref().is_some_and(|e|e.needs_refresh()));
    if stage.source.reference_generation()!=generation {
        stage.encoder=None;
        return match captured {
            Ok(SourceFrame::Paused(state))=>Ok(Captured::Paused(state)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(Captured::Unchanged),
            Err(error)=>Err(PipelineError::capture(error)),
            _=>Ok(Captured::Unchanged),
        };
    }
    let captured = match captured {
        Ok(frame) => frame,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(Captured::Unchanged),
        Err(error) => return Err(PipelineError::capture(error)),
    };
    let image = match captured {
        SourceFrame::Frame(image) => image,
        SourceFrame::Unchanged => return Ok(Captured::Unchanged),
        SourceFrame::Paused(state) => return Ok(Captured::Paused(state)),
    };
    if image.size().width<request.size.width || image.size().height<request.size.height { return Ok(Captured::Unchanged); }
    let limits = request.frame_contract.map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| c.limits)
        .intersect(stage.source.frame_limits_for_contract(request.frame_contract));
    limits.check(request.size).map_err(PipelineError::capture)?;
    let image = if image.size() == request.size {
        image
    } else {
        match image.downscale(request.size, limits, &image.grant(),
            crate::screen::core::capture_admission::CaptureOwner::ScaledPixels) {
            Ok(image) => image,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(Captured::Unchanged),
            Err(error) => return Err(PipelineError::capture(error)),
        }
    };
    let book = image.grant().book_identity();
    if stage.encoder_book != Some(book) {stage.encoder = None;}
    if stage.encoder.is_none() {
        match backend.open_encoder_for_stage_admitted(request, image.grant()) {
            Ok(encoder) => {stage.encoder = Some(encoder); stage.encoder_book = Some(book);}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(Captured::Unchanged),
            Err(error) => return Err(PipelineError::encoder(error)),
        }
    }
    let encoder = stage
        .encoder
        .as_mut()
        .ok_or_else(|| PipelineError::encoder(io::Error::other("no screen encoder")))?;
    let frame = match encoder.encode_captured(&image, request) {
        Ok(frame) => frame,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(Captured::Unchanged),
        Err(error) => return Err(PipelineError::encoder(error)),
    };
    super::super::frame_size_caps::validate_encoded_budget(request.frame_contract,request.choice.codec,&frame.records)
        .map_err(PipelineError::encoder)?;
    let encoder = StageEncoder {
        name: encoder.name(),
        hardware: request.choice.encoder_hardware,
    };
    Ok(Captured::Encoded {
        frame,
        capture_ts_us,
        encoder,
    })
}

#[cfg(test)]
#[path="batches_snapshot_task_tests.rs"]
mod tests;
