//! Capture and encoding of the host pipeline (SCR-M2, decisions C3, C6),
//! run on the blocking pool: one frame of the source scaled to the size of
//! the picture group and encoded, and `SNAPSHOT`, one lossless still
//! picture as tiles on its own picture stream of the kind `snapshot`.
use std::io;
use std::time::Duration;

use iroh::endpoint::Connection;

use super::{Pipeline, PipelineError};
use crate::screen::core::gop_stream::{now_us, GopWriter};
use crate::screen::core::host_backend::{
    EncodedFrame, ScreenEncoder, ScreenSource, SourceFrame, TilesScreenEncoder,
};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pacing::STALE_STREAM_US;
use crate::screen::core::pixel_convert::{downscale_bgra, FrameSize};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_binary::{GopHeader, StreamKind};

/// Source and encoder, moved to the blocking pool for each frame.
pub(super) struct Stage {
    pub(super) source: Box<dyn ScreenSource>,
    pub(super) encoder: Option<Box<dyn ScreenEncoder>>,
}

/// What capture and encoding of one frame gave.
pub(super) enum Captured {
    /// The records and the capture time (µs).
    Encoded(EncodedFrame, u64),
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
        let mut stage = self
            .stage
            .take()
            .ok_or_else(|| PipelineError::capture(io::Error::other("no screen source")))?;
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;
        let limit = max.unwrap_or(FrameSize::MAX);
        let joined = tokio::task::spawn_blocking(move || {
            let shot = still_tiles(&mut stage, frame_id, limit);
            (stage, shot)
        })
        .await;
        let (stage, shot) =
            joined.map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(stage);
        let (records, size, capture_ts_us) = shot?;
        let header = GopHeader {
            gop_id: self.next_gop_id,
            kind: StreamKind::Snapshot,
            codec: CodecKind::Tiles,
            profile: None,
            size,
        };
        self.next_gop_id += 1;
        let stale = Duration::from_micros(STALE_STREAM_US);
        let mut gop = tokio::time::timeout(stale, GopWriter::open(connection, header))
            .await
            .map_err(|error| PipelineError::capture(io::Error::other(error)))?
            .map_err(PipelineError::capture)?;
        gop.write_frame(frame_id, capture_ts_us, true, records)
            .await
            .map_err(PipelineError::capture)?;
        gop.finish();
        Ok(())
    }
}

/// One still picture of the stage's source as tile records within `limit`.
fn still_tiles(
    stage: &mut Stage,
    frame_id: u64,
    limit: FrameSize,
) -> Result<(Vec<Vec<u8>>, FrameSize, u64), PipelineError> {
    let capture_ts_us = now_us();
    let image = match stage
        .source
        .capture(frame_id, true)
        .map_err(PipelineError::capture)?
    {
        SourceFrame::Frame(image) => image,
        SourceFrame::Unchanged | SourceFrame::Paused(_) => {
            let error = io::Error::other("the source has no picture to save");
            return Err(PipelineError::capture(error));
        }
    };
    let size = image.size().fit_within(limit);
    let image = if image.size() == size {
        image
    } else {
        downscale_bgra(&image.view(), size).map_err(PipelineError::capture)?
    };
    let mut encoder = TilesScreenEncoder::new(size).map_err(PipelineError::encoder)?;
    let frame = encoder
        .encode(&image, true, 0)
        .map_err(PipelineError::encoder)?;
    Ok((frame.records, size, capture_ts_us))
}

/// Captures the picture of `frame_id`, scales it to `size` and encodes it.
pub(super) fn capture_and_encode(
    stage: &mut Stage,
    size: FrameSize,
    frame_id: u64,
    keyframe: bool,
    target_bps: u64,
) -> Result<Captured, PipelineError> {
    let capture_ts_us = now_us();
    let captured = stage
        .source
        .capture(frame_id, keyframe)
        .map_err(PipelineError::capture)?;
    let image = match captured {
        SourceFrame::Frame(image) => image,
        SourceFrame::Unchanged => return Ok(Captured::Unchanged),
        SourceFrame::Paused(state) => return Ok(Captured::Paused(state)),
    };
    let image = if image.size() == size {
        image
    } else {
        downscale_bgra(&image.view(), size).map_err(PipelineError::capture)?
    };
    let encoder = stage
        .encoder
        .as_mut()
        .ok_or_else(|| PipelineError::encoder(io::Error::other("no screen encoder")))?;
    let frame = encoder
        .encode(&image, keyframe, target_bps)
        .map_err(PipelineError::encoder)?;
    Ok(Captured::Encoded(frame, capture_ts_us))
}
