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
    /// Whether the stream's byte budget admits a snapshot now (review
    /// 2026-11-19, Codex 3/8): a snapshot is a picture of the stream like
    /// every other (`stream_admit`); the actor keeps it queued until then.
    pub fn snapshot_admitted(&self) -> bool {
        // Codex 7/12: a snapshot is always tiles (`Snapshot::header`), so
        // the tiles' budget rule holds for it also in a video session.
        self.rate.stream_admit(now_us(), false)
    }

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
        // The snapshot's newer GOP id makes the viewer take the live group
        // for stale: retire live state so its next frame is a fresh key
        // frame. Review 2026-11-19 (Codex 2/13, 3/8, 4/14): only once the
        // snapshot was captured and its stream is open; a failed one keeps
        // the live group (frame ids only rise, `FrameSequence::accept`).
        self.reset_stream();
        // Review 2026-11-19 (Codex 13, 6/9): the snapshot's bytes count
        // against the stream's byte budget as every picture's
        // (`rate_tiles.rs`), of a write cut off what it handed on.
        // Round 10: as a live picture (`host_frame_send.rs`).
        self.rate.shared_budget().control_turn().await;
        // Codex 10/9: the picture holds the line from before it reads its
        // credit until its bytes are booked.
        let picture_write = self.rate.picture_write();
        let started_us = now_us();
        // Codex 7/12: the snapshot's records are tiles whatever the live
        // stream's codec is; their pace and booking follow the tiles' rule.
        let video = false;
        // Codex 4/5, 4/14: paced within the stream's byte budget like a live
        // picture (`host_frame_send.rs`), and bounded in time: a viewer that
        // takes no more ends the snapshot, not the session's pictures.
        let pace = self.rate.stream_pace(started_us, video);
        let (rate, credit) = pace.unwrap_or((u64::MAX, 0));
        gop.pace_next_frame(
            rate,
            credit,
            Some(self.work.ceiling_cell()),
            self.rate.user_ceiling_bps(),
            self.rate.control_bps(),
            self.rate.sustained_bps(),
        );
        gop.pace_absorbs(self.rate.shared_budget());
        // Round 7 P4: as a live picture (`host_frame_send.rs`): a stall ends
        // the write piece by piece, the whole write's limit is a safety net.
        let budget = crate::screen::core::gop_stream::WRITE_SAFETY_LIMIT;
        let writing = crate::screen::core::gop_stream::write_with_route(
            connection,
            budget,
            gop.write_frame_admitted(frame_id, capture_ts_us, true, records),
        );
        let written = tokio::select! {biased;
            _=work.cancelled(work_epoch)=>Err(io::Error::from(io::ErrorKind::Interrupted)),
            result=writing=>result,
        };
        // Its stream's header as the writer wrote it (`host_frame_send.rs`).
        let header_bytes = gop.header_bytes();
        let booked = written
            .as_ref()
            .map_or_else(|_| gop.handed_bytes(), |bytes| *bytes)
            + header_bytes;
        self.rate.on_stream_sent(started_us, booked, video);
        drop(picture_write);
        // The statistics' sent rate counts the snapshot (Gemini 3/6).
        self.second_bytes += booked;
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
