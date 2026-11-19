//! One session of the Android host (SCR-M8): the stage the pipeline asks
//! for each frame. H.264, HEVC and AV1 come encoded from the app's MediaCodec encoder,
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
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_bytes::AdmittedBytes;
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_backend::{ScreenEncoder, ScreenSource, SourceFrame};
use crate::screen::core::host_stage::{
    HostStage, StageEncoder, StageError, StageFrame, StageRequest,
};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_hello::ScreenErrorCode;

#[path = "bridge_source_capture.rs"]
mod capture;
#[path = "bridge_source_input.rs"]
mod input;

pub use input::decode_still;

/// Shortest interval of two stills (NR N16).
pub const STILL_INTERVAL: Duration = Duration::from_millis(333);
/// Wait for a still of the accessibility service.
const STILL_TIMEOUT: Duration = Duration::from_secs(2);
/// Wait for a key frame (the first one starts MediaCodec).
const KEY_TIMEOUT: Duration = Duration::from_secs(2);
/// Wait for the app while it prepares or asks its user, per frame.
const START_POLL: Duration = Duration::from_millis(200);
/// A bit rate rise smaller than this share is not sent to the app (a fall always is).
const BITRATE_STEP_PERCENT: u64 = 10;
/// Bytes of the size header of a still.
const STILL_HEADER: usize = 8;

/// The source and stage of one Android host session.
pub struct BridgeSource {
    channel: Arc<Channel>,
    size: FrameSize,
    epoch: Option<u64>,
    /// The epoch for which the app was asked to obtain MediaProjection.
    consent_epoch: Option<u64>,
    bitrate: u64,
    /// The display size the app reported last (`screen.hostReady`).
    reported: Option<FrameSize>,
    last_still: Option<Instant>,
    tiles: Option<(u64, u64, Box<dyn ScreenEncoder>)>,
    hybrid: Option<(
        u64,
        u64,
        crate::screen::core::host_hybrid::CapturedExternalHybrid,
    )>,
    capture_active: bool,
    activity_request: u64,
    limits: FrameLimits,
    /// The viewer heard that the encoder's lowest rate is above the rate
    /// asked of it; cleared once it no longer is (one notice per arising).
    floor_noticed: bool,
    /// The bit rate the app's encoder refused (Codex 3/16); `None` while the
    /// last one sent runs.
    refused_bps: Option<u64>,
}

impl BridgeSource {
    pub fn new(channel: Arc<Channel>, size: FrameSize) -> Self {
        // Raw capture is not an encoder input. The shared SessionAdmission
        // retains the observed Java heap ceiling without dividing N by W/24.
        let limits = FrameLimits::RESOURCE;
        Self {
            channel,
            size,
            epoch: None,
            consent_epoch: None,
            bitrate: 0,
            reported: None,
            last_still: None,
            tiles: None,
            hybrid: None,
            capture_active: true,
            activity_request: 0,
            limits,
            floor_noticed: false,
            refused_bps: None,
        }
    }

    fn synthetic(&self) -> bool {
        self.channel.start.source == "synthetic"
    }
}

/// The notice of an encoder whose lowest bit rate `floor` is above the rate
/// `asked` of it (`HostStage::take_notice`).
pub(super) fn floor_notice(floor: u64, asked: u64) -> String {
    format!(
        "the encoder's lowest bit rate {floor} bit/s is above the bit rate {asked} bit/s \
         asked of it; it sends at its lowest rate"
    )
}

/// How long one delta picture may take: two frame intervals, at least 50 ms.
fn frame_wait(fps: u32) -> Duration {
    Duration::from_millis((2_000 / u64::from(fps.max(1))).max(50))
}

/// The error of a session the app ended, by its reason key.
fn ended_error(reason: String) -> StageError {
    let code = if reason.starts_with("encoder_failed:") {
        ScreenErrorCode::EncoderFailed
    } else {
        match reason.as_str() {
            "consent_denied" => ScreenErrorCode::ApprovalDenied,
            "host_stopped" | "host_shutdown" => ScreenErrorCode::HostShutdown,
            _ => ScreenErrorCode::CaptureFailed,
        }
    };
    StageError::new(code, io::Error::other(reason))
}

impl ScreenSource for BridgeSource {
    /// Input bound to this session (`input::BridgeInputSink`).
    fn input_sink(&self) -> Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>> {
        Some(Arc::new(input::BridgeInputSink {
            channel: self.channel.clone(),
        }))
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.channel.capture.current()
    }
    fn frame_limits(&self) -> FrameLimits {
        self.limits
    }
    fn frame_limits_for_contract(
        &self,
        contract: Option<crate::screen::core::frame_budget::FrameContract>,
    ) -> FrameLimits {
        if contract.is_some() {
            self.limits
        } else {
            FrameLimits::LEGACY
        }
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        if matches!(self.channel.phase(), Phase::Ended(_)) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Android capture source ended",
            ));
        }
        self.capture_active = false;
        self.epoch = None;
        self.hybrid = None;
        self.tiles = None;
        self.activity_request = self
            .activity_request
            .checked_add(1)
            .ok_or_else(|| io::Error::other("capture activity request exhausted"))?;
        let request = self.activity_request;
        self.channel
            .send(json!({"op":"capture_active","active":active,"request":request}));
        let reply = self
            .channel
            .next_picture(Duration::from_secs(10), |picture| {
                picture.flags & 0x20 != 0
                    && picture.data.len() == 9
                    && picture.data.as_slice()[0] == u8::from(active)
                    && picture.data.as_slice().get(1..9).is_some_and(|bytes| {
                        u64::from_le_bytes(bytes.try_into().unwrap_or([0; 8])) == request
                    })
            })
            .map_err(io::Error::other)?;
        if reply.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Android capture pause/resume was not acknowledged",
            ));
        }
        self.capture_active = active;
        Ok(())
    }
    fn supports_frame_regions(&self) -> bool {
        !self.synthetic()
    }
    fn size(&self) -> FrameSize {
        self.size
    }

    /// A still (snapshot, the pipeline's own path): needs the accessibility
    /// service; MediaProjection pictures never leave the encoder unencoded.
    fn capture(&mut self, frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        self.limits.validate()?;
        if !self.capture_active {
            return Ok(SourceFrame::Unchanged);
        }
        self.still(frame_id, FrameLimits::LEGACY)
            .map(SourceFrame::Frame)
    }

    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        Some(self)
    }
}

impl HostStage for BridgeSource {
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        self.limits
            .validate()
            .map_err(|e| StageError::new(ScreenErrorCode::CaptureFailed, e))?;
        self.frame_limits_for_contract(request.frame_contract)
            .check(request.size)
            .map_err(|e| StageError::new(ScreenErrorCode::CaptureFailed, e))?;
        if !self.capture_active {
            return Ok(StageFrame::Unchanged {
                source_size: self.size,
            });
        }
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
            // HEVC and AV1 run in the app's MediaCodec encoder too: `configure`
            // names the codec (`bridge_source_capture.rs`), and the host offers
            // them only when the app reported a hardware encoder.
            CodecKind::H264 | CodecKind::Hevc | CodecKind::Av1
                if !projection && !self.synthetic() =>
            {
                if self.consent_epoch != Some(request.encoder_epoch) {
                    self.channel.send(json!({"op": "consent"}));
                    self.consent_epoch = Some(request.encoder_epoch);
                }
                Ok(StageFrame::Paused(SessionState::WaitingForPortal))
            }
            CodecKind::H264 | CodecKind::Hevc | CodecKind::Av1 => self.h264_frame(request, encoder),
        }
    }

    /// Review 2026-11-19: what the app's encoder could not do as asked while
    /// it keeps running, once when it arises: a parameter it refused (the
    /// app's sentence) or a lowest bit rate above the rate asked of it. The
    /// stage knows the rate it asked for (`self.bitrate`, the pipeline's
    /// target), not the viewer's „Höchste Datenrate“, so the sentence names
    /// that rate; the ceiling itself is held by the pipeline's byte budget.
    fn take_notice(&mut self) -> Option<String> {
        if let Some(text) = self.channel.take_notice() {
            return Some(text);
        }
        let floor = self
            .channel
            .rate_floor()
            .filter(|floor| self.bitrate > 0 && *floor > self.bitrate);
        let Some(floor) = floor else {
            self.floor_noticed = false;
            return None;
        };
        if self.floor_noticed {
            return None;
        }
        self.floor_noticed = true;
        Some(floor_notice(floor, self.bitrate))
    }

    /// r1: the capture path of the statistics (`HostStats::chain`):
    /// MediaProjection, the test picture for a synthetic source.
    fn drain_chain(&mut self, meter: &mut crate::screen::core::chain_meter::ChainMeter) {
        let mut facts = crate::screen::os::capture_facts();
        if self.synthetic() {
            facts.path = Some(crate::screen::core::frame_timing::CapturePath::TestPicture);
        }
        meter.set_facts(facts);
    }
}

impl Drop for BridgeSource {
    fn drop(&mut self) {
        super::bridge_channels::close(self.channel.id, "viewer_closed");
    }
}

/// The epoch bits of `flags` for `epoch` (the app's side, tests).
#[cfg(any(test, feature = "test-support"))]
pub fn epoch_flags(epoch: u64) -> u32 {
    u32::try_from(epoch & 0xFFFF).unwrap_or(0) << super::bridge_channels::EPOCH_SHIFT
}
