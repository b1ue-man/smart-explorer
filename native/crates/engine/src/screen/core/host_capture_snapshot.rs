//! Lossless tile `SNAPSHOT` stream production, split from `host_capture.rs`.
use std::io;
use std::time::Duration;

use iroh::endpoint::Connection;

use super::{Stage, StageOutput};
use crate::screen::core::frame_budget::{FrameContract, FrameLimits};
use crate::screen::core::frame_size_caps::{validate_encoded_budget, PacketBudget};
use crate::screen::core::gop_stream::{now_us, GopWriter};
use crate::screen::core::host_backend::{ScreenEncoder, SourceFrame};
use crate::screen::core::host_pipeline::{Pipeline, PipelineError};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pacing::STALE_STREAM_US;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::{GopHeader, StreamKind};

pub(super) struct Snapshot {
    pub(super) records: crate::screen::core::capture_records::CapturedRecords,
    pub(super) size: FrameSize,
    pub(super) capture_ts_us: u64,
    pub(super) contract: Option<FrameContract>,
    pub(super) family_layout_revision: Option<u64>,
}

impl Snapshot {
    pub(super) fn header(&self, gop_id: u64) -> GopHeader {
        GopHeader {
            family_layout_revision: self.family_layout_revision,
            gop_id,
            frame_contract: self.contract,
            kind: StreamKind::Snapshot,
            codec: CodecKind::Tiles,
            profile: None,
            size: self.size,
        }
    }
}

impl Pipeline {
    /// `SNAPSHOT`: one lossless key frame of the current source, at most
    /// `max` large, on a picture stream of the kind `snapshot`.
    pub async fn snapshot(
        &mut self,
        connection: &Connection,
        max: Option<FrameSize>,
    ) -> Result<(), PipelineError> {
        self.snapshot_epoch(connection, max, self.work.epoch())
            .await
    }

    pub(in crate::screen::core::host_pipeline) async fn snapshot_epoch(
        &mut self,
        connection: &Connection,
        max: Option<FrameSize>,
        work_epoch: u64,
    ) -> Result<(), PipelineError> {
        let work = self.work.clone();
        if !work.current(work_epoch) {
            return Err(PipelineError::capture(io::Error::from(
                io::ErrorKind::Interrupted,
            )));
        }
        // Snapshot consumes a newer GOP id: retire live state now so its next
        // frame is a fresh keyframe and old presentation receipts stay invalid.
        self.reset_stream();
        let mut stage = self
            .stage
            .take()
            .ok_or_else(|| PipelineError::capture(io::Error::other("no screen source")))?;
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;
        let contract = self.frame_contract;
        let local = self.output_limits().map_err(PipelineError::capture)?;
        let limit = max.unwrap_or(local.max_size);
        let joined = tokio::task::spawn_blocking(move || {
            let result = still_tiles_admitted(&mut stage, frame_id, limit, contract, local);
            StageOutput { result, stage }
        })
        .await;
        let StageOutput {
            stage,
            result: shot,
        } = joined.map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(stage);
        if !work.current(work_epoch) {
            self.reset_stream();
            return Err(PipelineError::capture(io::Error::from(
                io::ErrorKind::Interrupted,
            )));
        }
        self.sync_source_reference();
        let shot = shot?;
        let header = shot.header(self.next_gop_id);
        let Snapshot {
            records,
            capture_ts_us,
            ..
        } = shot;
        self.next_gop_id += 1;
        let stale = Duration::from_micros(STALE_STREAM_US);
        let opened = tokio::select! {biased;
            _=work.cancelled(work_epoch)=>return Err(PipelineError::capture(io::Error::from(io::ErrorKind::Interrupted))),
            result=tokio::time::timeout(stale,GopWriter::open(connection,header))=>result,
        };
        let mut gop = opened
            .map_err(|error| PipelineError::capture(io::Error::other(error)))?
            .map_err(PipelineError::capture)?;
        // Review 2026-11-19 (Codex 13): the snapshot's bytes count against
        // the stream's byte budget as every picture's (`rate_tiles.rs`); a
        // cut-off write counts whole.
        let planned = crate::screen::core::wire_binary::RECORD_HEADER_BYTES as u64
            * records.len() as u64
            + records.iter().map(|r| r.len() as u64).sum::<u64>();
        let started_us = now_us();
        let written = tokio::select! {biased;
            _=work.cancelled(work_epoch)=>Err(io::Error::from(io::ErrorKind::Interrupted)),
            result=gop.write_frame_admitted(frame_id,capture_ts_us,true,records)=>result,
        };
        let video = self.choice.codec.is_video();
        let booked = written.as_ref().map_or(planned, |bytes| *bytes);
        self.rate.on_stream_sent(started_us, booked, video);
        if !work.current(work_epoch) {
            gop.reset();
            return Err(PipelineError::capture(io::Error::from(
                io::ErrorKind::Interrupted,
            )));
        }
        match written {
            Ok(_) => {
                gop.finish();
                Ok(())
            }
            Err(error) => {
                gop.reset();
                Err(PipelineError::capture(error))
            }
        }
    }
}

/// One still picture of the stage's source as tile records within `limit`.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn still_tiles(
    stage: &mut Stage,
    frame_id: u64,
    limit: FrameSize,
    contract: Option<FrameContract>,
) -> Result<Snapshot, PipelineError> {
    still_tiles_admitted(
        stage,
        frame_id,
        limit,
        contract,
        contract.map_or(FrameLimits::LEGACY, |c| c.limits),
    )
}

fn still_tiles_admitted(
    stage: &mut Stage,
    frame_id: u64,
    limit: FrameSize,
    contract: Option<FrameContract>,
    local: FrameLimits,
) -> Result<Snapshot, PipelineError> {
    let contract = contract.map(|mut contract| {
        contract.hybrid_regions = false;
        contract
    });
    if let Some(contract) = contract {
        contract.validate().map_err(PipelineError::capture)?;
    }
    let limits = contract
        .map_or(local, |c| c.limits)
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
        image
            .downscale(
                size,
                limits,
                &image.grant(),
                crate::screen::core::capture_admission::CaptureOwner::ScaledPixels,
            )
            .map_err(PipelineError::capture)?
    };
    let budget =
        PacketBudget::for_receive(contract, Some(limits)).map_err(PipelineError::encoder)?;
    let mut encoder = crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(
        size,
        limits,
        image.grant(),
    )
    .map_err(PipelineError::encoder)?;
    // The tile API's minimum is one full tile; a smaller image still emits
    // only its actual pixels. Validate actual records against the contract.
    let request = StageRequest {
        frame_id,
        frame_contract: contract,
        visible_rects: None,
        keyframe: true,
        target_bps: 1,
        size,
        choice: crate::screen::core::negotiate::CodecChoice {
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: size,
            max_pixels: size.pixels(),
            max_macroblocks_per_second: None,
        },
        encoder_epoch: frame_id,
        fps: 1,
    };
    let records = encoder
        .encode_captured(&image, &request)
        .map_err(PipelineError::encoder)?
        .records;
    if records
        .iter()
        .any(|record| record.len() > budget.record_bytes)
    {
        return Err(PipelineError::encoder(io::Error::other(
            "snapshot record exceeds accepted budget",
        )));
    }
    validate_encoded_budget(contract, CodecKind::Tiles, &records)
        .map_err(PipelineError::encoder)?;
    Ok(Snapshot {
        records,
        size,
        capture_ts_us,
        contract,
        family_layout_revision: stage.source.family_layout_revision(),
    })
}
