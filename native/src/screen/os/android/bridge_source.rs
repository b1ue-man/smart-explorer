//! One session of the Android host (SCR-M8): the stage the pipeline asks
//! for each frame. H.264 comes encoded from the app's MediaCodec encoder,
//! which this stage configures per encoder epoch (`configure`), asks for
//! key frames and new bit rates; tile previews and snapshots are stills of
//! the accessibility service (at most one per 333 ms, C1 Android L2) that
//! this stage encodes as tiles. Until the app reports the session ready
//! (after the system dialog) the session waits like a portal dialog;
//! refused, locked or ended in the app, the session ends with its reason.
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::bridge_channels::{Channel, Phase, Picture};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenSource, SourceFrame, TilesScreenEncoder,
};
use crate::screen::core::host_stage::{
    HostStage, StageEncoder, StageError, StageFrame, StageRequest,
};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{downscale_bgra, BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_hello::ScreenErrorCode;

/// Shortest interval of two stills (NR N16).
pub(crate) const STILL_INTERVAL: Duration = Duration::from_millis(333);
/// Wait for a still of the accessibility service.
const STILL_TIMEOUT: Duration = Duration::from_secs(2);
/// Wait for a key frame (the first one starts MediaCodec).
const KEY_TIMEOUT: Duration = Duration::from_secs(10);
/// Wait for the app while it prepares or asks its user, per frame.
const START_POLL: Duration = Duration::from_millis(200);
/// A bit rate change smaller than this share is not sent to the app.
const BITRATE_STEP_PERCENT: u64 = 10;
/// Bytes of the size header of a still.
const STILL_HEADER: usize = 8;

/// The source and stage of one Android host session.
pub(crate) struct BridgeSource {
    channel: Arc<Channel>,
    size: FrameSize,
    epoch: Option<u64>,
    /// The epoch for which the app was asked to obtain MediaProjection.
    consent_epoch: Option<u64>,
    bitrate: u64,
    /// The display size the app reported last (`screen.hostReady`).
    reported: Option<FrameSize>,
    last_still: Option<Instant>,
    tiles: Option<(u64, TilesScreenEncoder)>,
}

impl BridgeSource {
    pub(crate) fn new(channel: Arc<Channel>, size: FrameSize) -> Self {
        Self {
            channel,
            size,
            epoch: None,
            consent_epoch: None,
            bitrate: 0,
            reported: None,
            last_still: None,
            tiles: None,
        }
    }

    fn synthetic(&self) -> bool {
        self.channel.start.source == "synthetic"
    }

    /// The still that answers `frame_id`, as BGRA.
    fn still(&mut self, frame_id: u64) -> io::Result<BgraImage> {
        if let Some(last) = self.last_still {
            let left = STILL_INTERVAL.saturating_sub(last.elapsed());
            std::thread::sleep(left);
        }
        self.channel
            .send(json!({"op": "still", "frameId": frame_id}));
        self.last_still = Some(Instant::now());
        let picture = self
            .channel
            .next_picture(STILL_TIMEOUT, Picture::still)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "no still from the app"))?;
        decode_still(&picture.data)
    }

    fn tiles_frame(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        // Stills need the accessibility service; without it the preview stays empty.
        if !super::bridge_channels::prereqs().accessibility_on() {
            return Ok(StageFrame::Paused(SessionState::NoDisplay));
        }
        let failed = |error| StageError::new(ScreenErrorCode::CaptureFailed, error);
        let image = self.still(request.frame_id).map_err(failed)?;
        let source_size = image.size();
        if source_size != self.size {
            // Rotated or resized: the pipeline starts a new picture group.
            self.size = source_size;
            return Ok(StageFrame::Unchanged { source_size });
        }
        let size = request.size;
        let image = if source_size == size {
            image
        } else {
            downscale_bgra(&image.view(), size).map_err(failed)?
        };
        let encoder_failed = |error| StageError::new(ScreenErrorCode::EncoderFailed, error);
        let mut tiles = match self.tiles.take() {
            Some((epoch, encoder)) if epoch == request.encoder_epoch => encoder,
            _ => TilesScreenEncoder::new(size).map_err(encoder_failed)?,
        };
        let frame = tiles.encode(&image, request.keyframe, request.target_bps);
        self.tiles = Some((request.encoder_epoch, tiles));
        let frame = frame.map_err(encoder_failed)?;
        if frame.records.is_empty() {
            return Ok(StageFrame::Unchanged { source_size });
        }
        Ok(StageFrame::Encoded {
            records: frame.records,
            keyframe: frame.keyframe,
            capture_ts_us: now_us(),
            encoder: StageEncoder {
                name: "tiles".into(),
                hardware: false,
            },
            source_size,
        })
    }

    fn h264_frame(
        &mut self,
        request: &StageRequest,
        encoder: StageEncoder,
    ) -> Result<StageFrame, StageError> {
        let source_size = self.size;
        let fresh = self.epoch != Some(request.encoder_epoch);
        if fresh {
            self.channel.clear_pictures();
            self.channel.send(json!({
                "op": "configure",
                "epoch": request.encoder_epoch & 0xFFFF,
                "codec": "h264",
                "profile": request.choice.profile.map(|profile| profile.key()),
                "width": request.size.width,
                "height": request.size.height,
                "fps": request.fps,
                "bitrate": request.target_bps,
            }));
            self.epoch = Some(request.encoder_epoch);
            self.bitrate = request.target_bps;
        } else {
            if request.keyframe {
                self.channel.send(json!({"op": "keyframe"}));
            }
            let step = self.bitrate * BITRATE_STEP_PERCENT / 100;
            if request.target_bps.abs_diff(self.bitrate) > step {
                self.channel
                    .send(json!({"op": "bitrate", "bps": request.target_bps}));
                self.bitrate = request.target_bps;
            }
        }
        let epoch = u32::try_from(request.encoder_epoch & 0xFFFF).unwrap_or(0);
        let want_key = fresh || request.keyframe;
        let timeout = if want_key {
            KEY_TIMEOUT
        } else {
            frame_wait(request.fps)
        };
        let accept = |picture: &Picture| {
            !picture.still() && picture.epoch() == epoch && (!want_key || picture.keyframe())
        };
        let picture = self
            .channel
            .next_picture(timeout, accept)
            .map_err(ended_error)?;
        match picture {
            Some(picture) => Ok(StageFrame::Encoded {
                keyframe: picture.keyframe(),
                records: vec![picture.data],
                capture_ts_us: now_us(),
                encoder,
                source_size,
            }),
            None if want_key => Err(StageError::new(
                ScreenErrorCode::EncoderFailed,
                io::Error::new(io::ErrorKind::TimedOut, "no key frame from MediaCodec"),
            )),
            None => Ok(StageFrame::Unchanged { source_size }),
        }
    }
}

/// How long one delta picture may take: two frame intervals, at least 50 ms.
fn frame_wait(fps: u32) -> Duration {
    Duration::from_millis((2_000 / u64::from(fps.max(1))).max(50))
}

/// The error of a session the app ended, by its reason key.
fn ended_error(reason: String) -> StageError {
    let code = match reason.as_str() {
        "consent_denied" => ScreenErrorCode::ApprovalDenied,
        "host_stopped" | "host_shutdown" => ScreenErrorCode::HostShutdown,
        _ => ScreenErrorCode::CaptureFailed,
    };
    StageError::new(code, io::Error::other(reason))
}

/// A still of the app: width and height (u32 little endian), RGBA rows.
pub(crate) fn decode_still(data: &[u8]) -> io::Result<BgraImage> {
    let invalid = |text: &str| io::Error::new(io::ErrorKind::InvalidData, text.to_string());
    let header = data
        .get(..STILL_HEADER)
        .ok_or_else(|| invalid("still without header"))?;
    let word = |at: usize| {
        u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    let size = FrameSize::new(word(0), word(4));
    size.check_frame()?;
    let pixels = &data[STILL_HEADER..];
    if pixels.len() != size.bgra_len() {
        return Err(invalid("still size does not match its pixels"));
    }
    let mut bgra = pixels.to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 0xFF;
    }
    BgraImage::from_vec(size, bgra)
}

impl ScreenSource for BridgeSource {
    fn size(&self) -> FrameSize {
        self.size
    }

    /// A still (snapshot, the pipeline's own path): needs the accessibility
    /// service; MediaProjection pictures never leave the encoder unencoded.
    fn capture(&mut self, frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        self.still(frame_id).map(SourceFrame::Frame)
    }

    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        Some(self)
    }
}

impl HostStage for BridgeSource {
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        let phase = self.channel.wait_started(START_POLL);
        let (width, height, encoder, projection) = match phase {
            Phase::Ended(reason) => return Err(ended_error(reason)),
            Phase::Consent => return Ok(StageFrame::Paused(SessionState::WaitingForPortal)),
            Phase::Ready {
                width,
                height,
                encoder,
                hardware,
                projection,
            } => (
                width,
                height,
                StageEncoder {
                    name: encoder,
                    hardware,
                },
                projection,
            ),
        };
        let reported = FrameSize::new(width, height);
        if width > 0 && height > 0 && self.reported != Some(reported) {
            self.reported = Some(reported);
            self.size = reported;
        }
        match request.choice.codec {
            CodecKind::Tiles => self.tiles_frame(request),
            // A preview turned full stream: the app asks its user first.
            CodecKind::H264 if !projection && !self.synthetic() => {
                if self.consent_epoch != Some(request.encoder_epoch) {
                    self.channel.send(json!({"op": "consent"}));
                    self.consent_epoch = Some(request.encoder_epoch);
                }
                Ok(StageFrame::Paused(SessionState::WaitingForPortal))
            }
            CodecKind::H264 => self.h264_frame(request, encoder),
        }
    }
}

impl Drop for BridgeSource {
    fn drop(&mut self) {
        super::bridge_channels::close(self.channel.id, "viewer_closed");
    }
}

/// The epoch bits of `flags` for `epoch` (the app's side, tests).
#[cfg(test)]
pub(crate) fn epoch_flags(epoch: u64) -> u32 {
    u32::try_from(epoch & 0xFFFF).unwrap_or(0) << super::bridge_channels::EPOCH_SHIFT
}
