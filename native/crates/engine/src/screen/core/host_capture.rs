//! Capture and encoding of the host pipeline (SCR-M2, SCR-M3, decisions C3,
//! C6), run on the blocking pool: one frame of the source scaled to the size
//! of the picture group and encoded, either here with the backend's encoder
//! (opened on first use) or in the host process when the source was opened
//! there (`ScreenSource::host_stage`); and `SNAPSHOT`, one lossless still
//! picture as tiles on its own picture stream of the kind `snapshot`.
use std::io;

use super::PipelineError;
use crate::screen::core::capture_image::CapturedImage;
use crate::screen::core::codec_registry::{OpenedEncoder, TILES_NAME};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_backend::{
    CapturedEncodedFrame as EncodedFrame, ScreenHostBackend, ScreenSource, SourceFrame,
};
use crate::screen::core::host_stage::{StageEncoder, StageFrame, StageRequest};
use crate::screen::core::session_state::SessionState;

#[path = "host_capture_snapshot.rs"]
mod snapshot;
#[cfg(any(test, feature = "test-support"))]
use snapshot::still_tiles;

/// Source and encoder, moved to the blocking pool for each frame.
pub(in crate::screen) struct Stage {
    pub(in crate::screen) source: Box<dyn ScreenSource>,
    pub(in crate::screen) encoder: Option<OpenedEncoder>,
    pub(in crate::screen) encoder_book: Option<usize>,
    pub(in crate::screen) refused: Vec<String>,
    pub(in crate::screen) memory: super::super::frame_memory::MemoryHold,
    /// The pipeline's station meter (BK0).
    pub(in crate::screen) chain: super::super::chain_meter::SharedChain,
}

fn micros_since(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// A detached/aborted blocking task drops its result before the Stage lease.
pub(super) struct StageOutput<T> {
    pub(super) result: T,
    pub(super) stage: Stage,
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

/// One frame as `request` says: in the host process when the source runs
/// there, else here with the backend's encoder, opened on first use.
/// `recover` (B-36): the frame continues a reset stream and may reference
/// only that confirmed frame or older; an encoder that cannot gives a key
/// frame of a new encoder instead, as after any reset.
pub(super) fn produce(
    stage: &mut Stage,
    backend: &dyn ScreenHostBackend,
    request: &StageRequest,
    recover: Option<u64>,
) -> Result<Captured, PipelineError> {
    match stage.source.prepare_capture(request) {
        Ok(Some(state)) => return Ok(Captured::Paused(state)),
        Ok(None) => {}
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return Ok(Captured::Paused(SessionState::Starting))
        }
        Err(error) => return Err(PipelineError::capture(error)),
    }
    if request.size.width == 0 || request.size.height == 0 {
        return Ok(Captured::Unchanged);
    }
    if let Some(host) = stage.source.host_stage() {
        if let Some(last_good) = recover {
            host.recover_references(last_good);
        }
        let produced = host.produce(request);
        // The station times and facts the host process reported (BK0).
        host.drain_chain(&mut super::super::chain_meter::lock(&stage.chain));
        return Ok(match produced? {
            StageFrame::Encoded {
                records,
                keyframe,
                capture_ts_us,
                encoder,
                ..
            } => {
                super::super::chain_meter::lock(&stage.chain)
                    .on_capture_ts(capture_ts_us, request.fps);
                super::super::frame_size_caps::validate_encoded_budget(
                    request.frame_contract,
                    request.choice.codec,
                    &records,
                )
                .map_err(PipelineError::encoder)?;
                Captured::Encoded {
                    frame: EncodedFrame { records, keyframe },
                    capture_ts_us,
                    encoder,
                }
            }
            StageFrame::Unchanged { .. } => Captured::Unchanged,
            StageFrame::Paused(state) => Captured::Paused(state),
        });
    }
    capture_and_encode(stage, backend, request, recover)
}

/// Captures the picture of the request's frame, scales it to the request's
/// size and encodes it with the stage's encoder.
fn capture_and_encode(
    stage: &mut Stage,
    backend: &dyn ScreenHostBackend,
    request: &StageRequest,
    recover: Option<u64>,
) -> Result<Captured, PipelineError> {
    let capture_ts_us = now_us();
    let capture_started = std::time::Instant::now();
    let generation = stage.source.reference_generation();
    // A continuation needs a picture even when nothing changed: the lost
    // frames may have carried the last change.
    let captured = stage.source.capture(
        request.frame_id,
        request.keyframe
            || recover.is_some()
            || stage
                .encoder
                .as_ref()
                .is_some_and(|opened| opened.encoder.needs_refresh()),
    );
    if stage.source.reference_generation() != generation {
        stage.encoder = None;
        return match captured {
            Ok(SourceFrame::Paused(state)) => Ok(Captured::Paused(state)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(Captured::Unchanged),
            Err(error) => Err(PipelineError::capture(error)),
            _ => Ok(Captured::Unchanged),
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
    if image.size().width < request.size.width || image.size().height < request.size.height {
        return Ok(Captured::Unchanged);
    }
    let capture_us = micros_since(capture_started);
    let encode_started = std::time::Instant::now();
    let limits = request
        .frame_contract
        .map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| {
            c.limits
        })
        .intersect(
            stage
                .source
                .frame_limits_for_contract(request.frame_contract),
        );
    limits.check(request.size).map_err(PipelineError::capture)?;
    let image = if image.size() == request.size {
        image
    } else {
        match image.downscale(
            request.size,
            limits,
            &image.grant(),
            crate::screen::core::capture_admission::CaptureOwner::ScaledPixels,
        ) {
            Ok(image) => image,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(Captured::Unchanged)
            }
            Err(error) => return Err(PipelineError::capture(error)),
        }
    };
    // B-36: the running encoder references the confirmed picture; one that
    // cannot answers with a key frame of its own, as after any reset
    // (`Pipeline::reset_stream`, 2026-11-18). Review 2026-11-19 (Gemini 2):
    // it gave way to a new encoder, for every encoder without reference
    // invalidation (the trait's default) at every continued reset.
    let mut request = std::borrow::Cow::Borrowed(request);
    let mut refused_recovery = false;
    if let Some(last_good) = recover {
        let continued = stage.encoder.as_mut().is_some_and(|opened| {
            opened.encoder.recover_references(last_good)
                == super::super::ref_recovery::Recovery::Continued
        });
        if !continued {
            refused_recovery = true;
            request.to_mut().keyframe = true;
        }
    }
    let (frame, encoder, convert_us) =
        match encode_stage_frame(stage, backend, &image, request.clone()) {
            Ok(encoded) => encoded,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(Captured::Unchanged)
            }
            Err(error) => return Err(PipelineError::encoder(error)),
        };
    // Review 2026-11-19 (V4): an encoder that cannot continue and still
    // answers without a key frame references its newest picture, not the
    // confirmed one; sent as the continuation it would break the viewer's
    // picture. It gives way to a new encoder, whose first picture is a key
    // frame (as `forced_key` in the host process); this picture is skipped.
    if refused_recovery
        && !super::super::first_picture::starts_group(
            request.choice.codec,
            frame.keyframe,
            frame.records.iter(),
        )
    {
        stage.encoder = None;
        return Ok(Captured::Unchanged);
    }
    super::super::frame_size_caps::validate_encoded_budget(
        request.frame_contract,
        request.choice.codec,
        &frame.records,
    )
    .map_err(PipelineError::encoder)?;
    {
        use super::super::chain_meter::{lock, Station};
        let mut chain = lock(&stage.chain);
        chain.record_us(Station::Capture, capture_us);
        let total_us = micros_since(encode_started);
        let encode_us = super::super::convert_clock::encode_without(total_us, convert_us);
        chain.record_us(Station::Encode, encode_us);
        if let Some(convert_us) = convert_us {
            chain.record_us(Station::Convert, convert_us);
        }
        chain.on_capture_ts(capture_ts_us, request.fps);
        // This path encodes CPU pixels only; the capture of this process
        // names its path where this process captures itself (r1).
        chain.set_facts(crate::screen::os::capture_facts().merged(
            super::super::frame_timing::CaptureFacts {
                on_gpu: Some(false),
                ..Default::default()
            },
        ));
    }
    Ok(Captured::Encoded {
        frame,
        capture_ts_us,
        encoder,
    })
}

/// Encodes `image` on the daemon's in-process stage with bounded rank
/// fallback (W4, B3): each failing factory goes onto `stage.refused` once
/// and forces a key frame on its replacement.
pub(in crate::screen) fn encode_stage_frame(
    stage: &mut Stage,
    backend: &dyn ScreenHostBackend,
    image: &CapturedImage,
    mut request: std::borrow::Cow<'_, StageRequest>,
) -> io::Result<(EncodedFrame, StageEncoder, Option<u64>)> {
    let book = image.grant().book_identity();
    if stage.encoder_book != Some(book)
        || stage
            .encoder
            .as_ref()
            .is_some_and(|opened| opened.encoder.size() != request.size)
    {
        stage.encoder = None;
    }
    // Counted only after a failure (plan 2026-11-17 P3).
    let mut max_attempts: Option<usize> = None;
    let mut attempt = 0;
    loop {
        attempt += 1;
        if stage.encoder.is_none() {
            let opened = backend.open_stage_encoder(&request, &stage.refused, image.grant())?;
            stage.encoder = Some(opened);
            stage.encoder_book = Some(book);
        }
        let opened = stage
            .encoder
            .as_mut()
            .ok_or_else(|| io::Error::other("no screen encoder"))?;
        super::super::convert_clock::arm();
        let encoded = opened.encoder.encode_captured(image, &request);
        let convert_us = super::super::convert_clock::take();
        match encoded {
            Ok(frame) => {
                let info = StageEncoder {
                    name: opened.encoder.name(),
                    hardware: opened.hardware,
                };
                return Ok((frame, info, convert_us));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(error),
            Err(error) => {
                let factory = opened.factory.clone();
                stage.encoder = None;
                if factory == TILES_NAME {
                    return Err(error);
                }
                if !stage.refused.contains(&factory) {
                    stage.refused.push(factory);
                }
                let stages =
                    *max_attempts.get_or_insert_with(|| backend.encoder_stages(&request).max(1));
                if attempt >= stages {
                    return Err(error);
                }
                request.to_mut().keyframe = true;
            }
        }
    }
}

#[cfg(test)]
#[path = "batches_snapshot_task_tests.rs"]
mod tests;
