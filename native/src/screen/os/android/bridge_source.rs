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
    ScreenEncoder, ScreenSource, SourceFrame,
};
use crate::screen::core::host_stage::{
    HostStage, StageEncoder, StageError, StageFrame, StageRequest,
};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::capture_admission::{CaptureGrant,CaptureOwner};
use crate::screen::core::capture_image::{CapturedImage,CaptureImageBuilder};
use crate::screen::core::capture_bytes::AdmittedBytes;
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_hello::ScreenErrorCode;

/// Shortest interval of two stills (NR N16).
pub(crate) const STILL_INTERVAL: Duration = Duration::from_millis(333);
/// Wait for a still of the accessibility service.
const STILL_TIMEOUT: Duration = Duration::from_secs(2);
/// Wait for a key frame (the first one starts MediaCodec).
const KEY_TIMEOUT: Duration = Duration::from_secs(2);
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
    tiles: Option<(u64,u64,Box<dyn ScreenEncoder>)>,
    hybrid: Option<(u64,u64,crate::screen::core::host_hybrid::CapturedExternalHybrid)>,
    capture_active: bool,
    activity_request: u64,
    limits:FrameLimits,
}

impl BridgeSource {
    pub(crate) fn new(channel: Arc<Channel>, size: FrameSize) -> Self {
        // Raw capture is not an encoder input. The shared SessionAdmission
        // retains the observed Java heap ceiling without dividing N by W/24.
        let limits=FrameLimits::RESOURCE;
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
            capture_active: true, activity_request:0,limits,
        }
    }

    fn synthetic(&self) -> bool {
        self.channel.start.source == "synthetic"
    }

    /// The still that answers `frame_id`, as BGRA.
    fn still(&mut self, frame_id: u64,limits:FrameLimits) -> io::Result<CapturedImage> {
        if self.synthetic() {
            let grant=self.channel.capture.synthetic_snapshot(self.size)?;
            let mut image=CaptureImageBuilder::new(self.size,FrameLimits::RESOURCE,&grant,CaptureOwner::SourcePixels)?;
            crate::screen::core::synthetic::render(frame_id,&mut image)?;
            let image=image.freeze();let target=limits.fit(image.size());
            return image.downscale(target,limits,&grant,CaptureOwner::ScaledPixels);
        }
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
        let grant=picture.data.grant()?;
        let image=decode_still(picture.data.as_slice(),&grant,
            if matches!(self.channel.phase(),Phase::Ready {projection:true,..}) {CaptureOwner::Readback} else {CaptureOwner::SourcePixels})?;
        let target=limits.fit(image.size());
        if target==image.size() {limits.check(target)?;Ok(image)}
        else {image.downscale(target,limits,&grant,CaptureOwner::ScaledPixels)}
    }

    fn tiles_frame(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        let projected = matches!(self.channel.phase(),Phase::Ready { projection:true,.. });
        if projected && self.epoch != Some(request.encoder_epoch) {
            self.channel.clear_pictures();
            self.channel.send(json!({"op":"configure","codec":"tiles","hybrid":true,
                "epoch":request.encoder_epoch & 0xFFFF,"width":request.size.width,"height":request.size.height,
                "fps":request.fps,"bitrate":request.target_bps,"profile":"high"}));
            self.epoch=Some(request.encoder_epoch);
        }
        // Accessibility remains the preview/snapshot fallback when projection is absent.
        if !projected && !self.synthetic() && !super::bridge_channels::prereqs().accessibility_on() {
            return Ok(StageFrame::Paused(SessionState::NoDisplay));
        }
        let failed = |error| StageError::new(ScreenErrorCode::CaptureFailed, error);
        // Read native geometry before the single output downscale. Treating a
        // legacy-sized intermediate as source geometry would repeatedly report
        // a resize against the app's unchanged native display dimensions.
        let image=self.still(request.frame_id,FrameLimits::RESOURCE).map_err(failed)?;
        let source_size = if projected {self.size} else {image.size()};
        if !projected && source_size != self.size {
            // Rotated or resized: the pipeline starts a new picture group.
            self.size = source_size;
            return Ok(StageFrame::Unchanged { source_size });
        }
        let size = request.size;
        let image = if image.size() == size {
            image
        } else {
            image.downscale(size,request.frame_contract.map_or(FrameLimits::LEGACY,|c|c.limits),&image.grant(),CaptureOwner::ScaledPixels).map_err(failed)?
        };
        let encoder_failed = |error| StageError::new(ScreenErrorCode::EncoderFailed, error);
        let generation=image.generation().value();
        let mut tiles = match self.tiles.take() {
            Some((epoch, previous, encoder)) if epoch == request.encoder_epoch && previous==generation => encoder,
            _ => {
                let grant=image.grant();
                let encoder: Box<dyn ScreenEncoder> = Box::new(crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(size,request.frame_contract.map_or(FrameLimits::LEGACY,|c|c.limits),grant.clone()).map_err(encoder_failed)?);
                crate::screen::core::host_hybrid::wrap_captured(encoder,request,grant).map_err(encoder_failed)?
            },
        };
        let frame = tiles.encode_captured(&image, request);
        self.tiles = Some((request.encoder_epoch,generation,tiles));
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
        let hybrid = request.frame_contract.is_some_and(|c|c.hybrid_regions) && !self.synthetic();
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
                "hybrid": hybrid,
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
        if hybrid { self.channel.send(json!({"op":"capture","frameId":request.frame_id})); }
        let epoch = u32::try_from(request.encoder_epoch & 0xFFFF).unwrap_or(0);
        let want_key = fresh || request.keyframe;
        let timeout = if want_key || hybrid {
            KEY_TIMEOUT
        } else {
            frame_wait(request.fps)
        };
        let accept = |picture: &Picture| {
            !picture.still() && picture.epoch() == epoch && (!want_key || picture.keyframe())
                && (!hybrid || (picture.captured_video() && picture.data.as_slice().get(8..16).is_some_and(|bytes|
                    u64::from_le_bytes(bytes.try_into().unwrap_or([0;8])) == request.frame_id)))
        };
        let picture = self
            .channel
            .next_picture(timeout, accept)
            .map_err(ended_error)?;
        // Kotlin admits the first paired capture only after publishing the actual encoder.
        let encoder=match self.channel.phase() {
            Phase::Ready {width,height,encoder,hardware,..}=>{
                let reported=FrameSize::new(width,height);
                if width>0 && height>0 && reported!=source_size {
                    self.size=reported;self.reported=Some(reported);
                    return Ok(StageFrame::Unchanged {source_size:reported});
                }
                StageEncoder {name:encoder,hardware}
            },
            Phase::Ended(reason)=>return Err(ended_error(reason)),
            _=>encoder,
        };
        match picture {
            Some(picture) if hybrid && picture.flags & 0x10 != 0 => {
                let fail=|error| StageError::new(ScreenErrorCode::EncoderFailed,error);
                let state=self.hybrid.as_mut().ok_or_else(||fail(io::Error::other("unchanged Android capture without base")))?;
                let frame=state.2.repeat_external(request).map_err(fail)?;
                if frame.records.is_empty() { Ok(StageFrame::Unchanged {source_size}) }
                else { Ok(StageFrame::Encoded {records:frame.records,keyframe:frame.keyframe,capture_ts_us:now_us(),encoder,source_size}) }
            }
            Some(picture) if hybrid => {
                let fail = |error| StageError::new(ScreenErrorCode::EncoderFailed,error);
                let data = picture.data.as_slice();
                let grant=picture.data.grant().map_err(fail)?;
                let size = request.size;
                let limits=request.frame_contract.map_or(FrameLimits::LEGACY,|c|c.limits);
                let raw_len=limits.check(size).map_err(fail)?;
                let end = 20usize.checked_add(raw_len).ok_or_else(||fail(io::Error::other("Android frame size overflow")))?;
                let head = data.get(..20).ok_or_else(||fail(io::Error::other("Android paired frame without header")))?;
                let width = u32::from_le_bytes(head[0..4].try_into().map_err(|_|fail(io::Error::other("Android frame width")))?);
                let height = u32::from_le_bytes(head[4..8].try_into().map_err(|_|fail(io::Error::other("Android frame height")))?);
                let video_len = u32::from_le_bytes(head[16..20].try_into().map_err(|_|fail(io::Error::other("Android frame video length")))?) as usize;
                if FrameSize::new(width,height) != size || video_len > crate::screen::core::wire_binary::MAX_RECORD_BYTES
                    || end.checked_add(video_len) != Some(data.len()) { return Err(fail(io::Error::other("Android raw/video pair size mismatch"))); }
                let mut pixels=AdmittedBytes::copy(&grant,CaptureOwner::Readback,&data[20..end]).map_err(fail)?;
                for pixel in pixels.as_mut_slice().chunks_exact_mut(4) { pixel.swap(0,2); pixel[3]=255; }
                let image=CaptureImageBuilder::from_bytes(size,limits,&grant,pixels).map_err(fail)?.freeze();
                let generation=grant.generation().value();
                let fresh = self.hybrid.as_ref().is_none_or(|(epoch,previous,_)|*epoch != request.encoder_epoch || *previous!=generation);
                if fresh { self.hybrid = Some((request.encoder_epoch,generation,crate::screen::core::host_hybrid::CapturedExternalHybrid::new(request,grant.clone()).map_err(fail)?)); }
                let mut stamped = request.clone(); stamped.keyframe |= picture.keyframe();
                let state = self.hybrid.as_mut().ok_or_else(||fail(io::Error::other("missing Android hybrid encoder")))?;
                let records=if video_len==0 {CapturedRecords::empty(&grant)} else {
                    AdmittedBytes::copy(&grant,CaptureOwner::CodecOutput,&data[end..]).and_then(CapturedRecords::one)
                }.map_err(fail)?;
                let frame = state.2.encode_external(&image,&stamped,crate::screen::core::host_backend::CapturedEncodedFrame {
                    records,keyframe:picture.keyframe(),
                }).map_err(fail)?;
                if frame.records.is_empty() { return Ok(StageFrame::Unchanged {source_size}); }
                Ok(StageFrame::Encoded { records:frame.records,keyframe:frame.keyframe,capture_ts_us:now_us(),encoder,source_size })
            }
            Some(picture) => {let keyframe=picture.keyframe();Ok(StageFrame::Encoded {
                keyframe,
                records: CapturedRecords::one(picture.data).map_err(|e|StageError::new(ScreenErrorCode::EncoderFailed,e))?,
                capture_ts_us: now_us(),
                encoder,
                source_size,
            })},
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
    let code = if reason.starts_with("encoder_failed:") { ScreenErrorCode::EncoderFailed } else { match reason.as_str() {
        "consent_denied" => ScreenErrorCode::ApprovalDenied,
        "host_stopped" | "host_shutdown" => ScreenErrorCode::HostShutdown,
        _ => ScreenErrorCode::CaptureFailed,
    }};
    StageError::new(code, io::Error::other(reason))
}

/// A still of the app: width and height (u32 little endian), RGBA rows.
pub(crate) fn decode_still(data: &[u8],grant:&CaptureGrant,owner:CaptureOwner) -> io::Result<CapturedImage> {
    let invalid = |text: &str| io::Error::new(io::ErrorKind::InvalidData, text.to_string());
    let header = data
        .get(..STILL_HEADER)
        .ok_or_else(|| invalid("still without header"))?;
    let word = |at: usize| {
        u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    let size = FrameSize::new(word(0), word(4));
    let len=FrameLimits::RESOURCE.check(size)?;
    let pixels = &data[STILL_HEADER..];
    if pixels.len() != len {
        return Err(invalid("still size does not match its pixels"));
    }
    let mut bgra = AdmittedBytes::copy(grant,owner,pixels)?;
    for pixel in bgra.as_mut_slice().chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 0xFF;
    }
    Ok(CaptureImageBuilder::from_bytes(size,FrameLimits::RESOURCE,grant,bgra)?.freeze())
}

impl ScreenSource for BridgeSource {
    fn capture_grant(&self)->Option<CaptureGrant> {self.channel.capture.current()}
    fn frame_limits(&self)->FrameLimits {self.limits}
    fn frame_limits_for_contract(&self,contract:Option<crate::screen::core::frame_budget::FrameContract>)->FrameLimits {
        if contract.is_some() {self.limits} else {FrameLimits::LEGACY}
    }
    fn set_capture_active(&mut self,active:bool) -> io::Result<()> {
        if matches!(self.channel.phase(),Phase::Ended(_)) {return Err(io::Error::new(io::ErrorKind::NotConnected,"Android capture source ended"));}
        self.capture_active=false; self.epoch=None; self.hybrid=None; self.tiles=None;
        self.activity_request=self.activity_request.checked_add(1).ok_or_else(||io::Error::other("capture activity request exhausted"))?;
        let request=self.activity_request;
        self.channel.send(json!({"op":"capture_active","active":active,"request":request}));
        let reply=self.channel.next_picture(Duration::from_secs(10),|picture| picture.flags & 0x20 !=0 && picture.data.len()==9
            && picture.data.as_slice()[0]==u8::from(active) && picture.data.as_slice().get(1..9).is_some_and(|bytes|u64::from_le_bytes(bytes.try_into().unwrap_or([0;8]))==request))
            .map_err(io::Error::other)?;
        if reply.is_none() {return Err(io::Error::new(io::ErrorKind::TimedOut,"Android capture pause/resume was not acknowledged"));}
        self.capture_active=active; Ok(())
    }
    fn supports_frame_regions(&self) -> bool { !self.synthetic() }
    fn size(&self) -> FrameSize {
        self.size
    }

    /// A still (snapshot, the pipeline's own path): needs the accessibility
    /// service; MediaProjection pictures never leave the encoder unencoded.
    fn capture(&mut self, frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        self.limits.validate()?;
        if !self.capture_active {return Ok(SourceFrame::Unchanged);}
        self.still(frame_id,FrameLimits::LEGACY).map(SourceFrame::Frame)
    }

    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        Some(self)
    }
}

impl HostStage for BridgeSource {
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        self.limits.validate().map_err(|e|StageError::new(ScreenErrorCode::CaptureFailed,e))?;
        self.frame_limits_for_contract(request.frame_contract).check(request.size).map_err(|e|StageError::new(ScreenErrorCode::CaptureFailed,e))?;
        if !self.capture_active {return Ok(StageFrame::Unchanged {source_size:self.size});}
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
