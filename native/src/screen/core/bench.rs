//! `se screen bench` (decision C9): measures a picture path with the test
//! picture `synthetic`, either over a session with another device or only
//! encoder and decoder on this one (`--local`). Encoders and decoders come
//! from the codec register of this process (SCR-M3: H.264 through Media
//! Foundation or OpenH264, the lossless tiles always). Each shown frame's barcode
//! must carry its frame id (else it counts as corrupt); latency is display
//! (decode done) minus capture, corrected by the clock offset of
//! `PING`/`PONG`; a broken picture stream counts as a disruption until the
//! next good frame (recovery time). `static_psnr_db` compares the static
//! text block with the reference picture; JSON `null` means infinite, the
//! block arrived identical (always so for the lossless `tiles`).
use std::io;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::codec_registry::CodecRegistry;
use super::gop_stream::{now_us, path_of, PictureEvent};
use super::host_backend::ScreenDecoder;
use super::metrics::{psnr_db, ClockSync, LatencySamples, RateMeter, RecoveryTracker};
use super::negotiate::{negotiate, CodecKind, CodecOffer, CodecPreference};
use super::pixel_convert::{downscale_bgra, BgraImage, FrameSize, Rect};
use super::rate_control::PathKind;
use super::session_state::{KeyframeRequester, SessionState};
use super::synthetic::{read_barcode, render, static_block, BarcodeRead, MIN_SYNTHETIC_SIZE};
use super::viewer_client::{ViewerError, ViewerSession};
use super::wire_binary::{
    FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME, FLAG_MORE, RECORD_HEADER_BYTES,
};
use super::wire_control::{HostFrame, KeyframeReason, KeyframeRequest, ViewerFrame};

pub(crate) const DEFAULT_SIZE: FrameSize = FrameSize::new(1280, 720);
pub(crate) const DEFAULT_FPS: u16 = 30;
pub(crate) const DEFAULT_SECONDS: u32 = 10;
/// Longest measurement: one hour.
pub(crate) const MAX_SECONDS: u32 = 3600;
/// Interval of the PSNR samples (a reference picture costs a render).
const PSNR_EVERY: Duration = Duration::from_secs(1);
/// `--codec h264` on a device without an H.264 codec.
const NO_H264: &str = "this device has no H.264 codec; `se screen codec openh264 enable` loads \
     OpenH264, or use --codec tiles";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BenchOptions {
    pub codec: CodecPreference,
    pub size: FrameSize,
    pub fps: u16,
    pub seconds: u32,
}

impl BenchOptions {
    /// Checks the values a user typed.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !self.size.is_valid_frame()
            || self.size.width < MIN_SYNTHETIC_SIZE.width
            || self.size.height < MIN_SYNTHETIC_SIZE.height
        {
            return Err("--size must lie between 128x72 and 4096x2304".into());
        }
        if !(1..=240).contains(&self.fps) {
            return Err("--fps must lie between 1 and 240".into());
        }
        if !(1..=MAX_SECONDS).contains(&self.seconds) {
            return Err(format!("--seconds must lie between 1 and {MAX_SECONDS}"));
        }
        Ok(())
    }

    /// The decoders of `codecs` for the chosen codec: every one for `auto`,
    /// the H.264 ones for `h264` (an error without any), only `tiles` for
    /// `tiles`.
    pub(crate) fn decoders(&self, codecs: &CodecRegistry) -> Result<Vec<CodecOffer>, String> {
        let offers = codecs.decoder_offers();
        match self.codec {
            CodecPreference::Auto => Ok(offers),
            CodecPreference::Tiles => Ok(vec![CodecOffer::tiles()]),
            CodecPreference::H264 => {
                let h264: Vec<CodecOffer> = offers
                    .into_iter()
                    .filter(|offer| offer.codec == CodecKind::H264)
                    .collect();
                if h264.is_empty() {
                    return Err(NO_H264.into());
                }
                Ok(h264)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LatencyMs {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct BitrateKbps {
    pub mean: f64,
    pub max: f64,
}

/// `se screen bench --json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct BenchReport {
    pub codec: String,
    pub encoder: String,
    pub decoder: String,
    /// `direct`, `relay` or `local`.
    pub path: String,
    /// Frames the host encoded (the highest frame id seen).
    pub frames_sent: u64,
    /// Frames decoded with a valid barcode.
    pub frames_shown: u64,
    pub fps_mean: f64,
    pub fps_min: f64,
    pub latency_ms: LatencyMs,
    pub bitrate_kbps: BitrateKbps,
    pub keyframes: u64,
    /// Milliseconds from each disruption to the next good frame.
    pub recoveries: Vec<f64>,
    pub corrupt_frames: u64,
    /// Lowest PSNR of the static text block; `null` = infinite.
    pub static_psnr_db: Option<f64>,
}

/// The measurements of one bench run.
struct Measure {
    size: FrameSize,
    clock: ClockSync,
    latency: LatencySamples,
    shown: RateMeter,
    received: RateMeter,
    recovery: RecoveryTracker,
    highest_frame: u64,
    keyframes: u64,
    /// A key frame was shown since the last look (answers the open request).
    fresh_keyframe: bool,
    corrupt: u64,
    psnr: Option<f64>,
    psnr_at: Option<Instant>,
    codec: Option<CodecKind>,
    decoder: String,
}

impl Measure {
    fn new(size: FrameSize) -> Self {
        Self {
            size,
            clock: ClockSync::default(),
            latency: LatencySamples::default(),
            shown: RateMeter::default(),
            received: RateMeter::default(),
            recovery: RecoveryTracker::default(),
            highest_frame: 0,
            keyframes: 0,
            fresh_keyframe: false,
            corrupt: 0,
            psnr: None,
            psnr_at: None,
            codec: None,
            decoder: "tiles".into(),
        }
    }

    /// One decoded picture of `record`, shown at `shown_us` (viewer clock);
    /// `local` pictures share the host clock.
    fn shown(&mut self, picture: &BgraImage, record: &FrameRecord, shown_us: u64, local: bool) {
        match read_barcode(&picture.view(), self.size) {
            Ok(BarcodeRead::Valid(id)) if id == record.frame_id => {}
            _ => {
                self.corrupt += 1;
                self.recovery.on_disruption(shown_us);
                return;
            }
        }
        if record.is_keyframe() {
            self.keyframes += 1;
            self.fresh_keyframe = true;
        }
        self.recovery.on_good_frame(shown_us);
        self.shown.record(shown_us, 0);
        let latency = if local {
            Some(shown_us.saturating_sub(record.capture_ts_us))
        } else {
            self.clock.latency_us(record.capture_ts_us, shown_us)
        };
        if let Some(latency) = latency {
            self.latency.record(latency);
        }
        if self.psnr_at.is_none_or(|at| at.elapsed() >= PSNR_EVERY) {
            self.psnr_at = Some(Instant::now());
            if let Some(value) = static_psnr(picture, record.frame_id, self.size) {
                self.psnr = Some(self.psnr.map_or(value, |lowest| lowest.min(value)));
            }
        }
    }

    fn report(&self, encoder: &str, path: &str) -> io::Result<BenchReport> {
        let now = now_us();
        let shown = self.shown.summary(now);
        if shown.frames == 0 {
            return Err(io::Error::other(
                "no frame of the test picture arrived intact",
            ));
        }
        let bytes = self.received.summary(now);
        let latency = self.latency.summary();
        let milliseconds =
            |pick: fn(&super::metrics::LatencySummary) -> f64| latency.as_ref().map_or(0.0, pick);
        Ok(BenchReport {
            codec: self.codec.map_or("tiles", CodecKind::key).into(),
            encoder: encoder.into(),
            decoder: self.decoder.clone(),
            path: path.into(),
            frames_sent: self.highest_frame,
            frames_shown: shown.frames,
            fps_mean: shown.fps_mean,
            fps_min: shown.fps_min,
            latency_ms: LatencyMs {
                p50: milliseconds(|summary| summary.p50_ms),
                p95: milliseconds(|summary| summary.p95_ms),
                max: milliseconds(|summary| summary.max_ms),
            },
            bitrate_kbps: BitrateKbps {
                mean: bytes.kbps_mean,
                max: bytes.kbps_max,
            },
            keyframes: self.keyframes,
            recoveries: self.recovery.recoveries_ms(),
            corrupt_frames: self.corrupt,
            static_psnr_db: self.psnr.filter(|value| value.is_finite()),
        })
    }
}

/// PSNR of the static block of `picture` against the reference of
/// `frame_id` drawn at the source size `source` and scaled like `picture`.
fn static_psnr(picture: &BgraImage, frame_id: u64, source: FrameSize) -> Option<f64> {
    let mut reference = BgraImage::new(source).ok()?;
    render(frame_id, &mut reference).ok()?;
    let size = picture.size();
    let reference = if size == source {
        reference
    } else {
        downscale_bgra(&reference.view(), size).ok()?
    };
    let block = static_block(source);
    let scale = |value: u32, from: u32, to: u32| {
        (u64::from(value) * u64::from(to) / u64::from(from.max(1))) as u32
    };
    let rect = Rect {
        x: scale(block.x, source.width, size.width),
        y: scale(block.y, source.height, size.height),
        width: scale(block.width, source.width, size.width).max(1),
        height: scale(block.height, source.height, size.height).max(1),
    };
    psnr_db(&picture.view(), &reference.view(), rect).ok()
}

/// The current picture group and its decoder.
type Decoding = Option<(GopHeader, Box<dyn ScreenDecoder>)>;

/// Measures a running session of the test picture for `options.seconds`
/// with the decoders of `codecs`.
pub(crate) async fn run(
    mut session: ViewerSession,
    options: BenchOptions,
    codecs: &CodecRegistry,
) -> Result<BenchReport, ViewerError> {
    let mut measure = Measure::new(options.size);
    let mut decoding: Decoding = None;
    let mut requester = KeyframeRequester::default();
    let mut encoder = "unknown".to_string();
    let deadline = tokio::time::sleep(Duration::from_secs(u64::from(options.seconds)));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => break,
            frame = session.host.recv() => match frame {
                Some(Ok(HostFrame::Pong(pong))) => {
                    measure.clock.on_pong(&pong, now_us());
                }
                Some(Ok(HostFrame::State(state))) => {
                    if let Some(stream) = state.stream {
                        encoder = stream.encoder;
                    }
                    if state.state == SessionState::Ended {
                        let reason = state.reason.unwrap_or_else(|| "ended".into());
                        return Err(ViewerError::Refused(reason));
                    }
                }
                Some(Ok(HostFrame::Error(error))) => return Err(ViewerError::Refused(error.code)),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(ViewerError::Io(error)),
                None => return Err(ViewerError::Io(io::Error::other("the host ended the session"))),
            },
            event = session.pictures.recv() => {
                let Some(event) = event else {
                    return Err(ViewerError::Io(io::Error::other("the picture streams ended")));
                };
                let wanted = picture(&mut measure, &mut decoding, codecs, event);
                if std::mem::take(&mut measure.fresh_keyframe) {
                    requester.on_keyframe();
                }
                if let Some(reason) = wanted.filter(|_| requester.should_request(now_us())) {
                    let gop_id = decoding.as_ref().map(|(header, _)| header.gop_id);
                    let request = KeyframeRequest { reason, gop_id };
                    let _ = session.control.send(ViewerFrame::KeyframeRequest(request)).await;
                }
            }
        }
    }
    let (path, _) = path_of(&session.connection);
    session.close();
    measure
        .report(&encoder, path.key())
        .map_err(ViewerError::Io)
}

/// Applies one picture event; a key-frame request reason when the stream
/// broke or did not decode.
fn picture(
    measure: &mut Measure,
    decoding: &mut Decoding,
    codecs: &CodecRegistry,
    event: PictureEvent,
) -> Option<KeyframeReason> {
    let (header, record, received_us) = match event {
        PictureEvent::Record {
            header,
            record,
            received_us,
        } => (header, record, received_us),
        PictureEvent::Broken { gop_id } => {
            let current = decoding
                .as_ref()
                .is_some_and(|(header, _)| header.gop_id == gop_id);
            if current {
                measure.recovery.on_disruption(now_us());
            }
            return current.then_some(KeyframeReason::Reset);
        }
    };
    let bytes = (record.payload.len() + RECORD_HEADER_BYTES) as u64;
    measure.received.record(received_us, bytes);
    measure.highest_frame = measure.highest_frame.max(record.frame_id);
    let fresh = decoding
        .as_ref()
        .is_none_or(|(current, _)| current.gop_id < header.gop_id);
    if fresh {
        if !record.is_keyframe() {
            return Some(KeyframeReason::Join);
        }
        let Ok(next) = codecs.open_decoder(&header) else {
            // No decoder for this stream: nothing of it can be shown.
            *decoding = None;
            measure.recovery.on_disruption(now_us());
            return None;
        };
        *decoding = Some((header, next));
        measure.codec = Some(header.codec);
    }
    let (current, active) = decoding.as_mut()?;
    if current.gop_id != header.gop_id {
        return None;
    }
    match active.decode(&record) {
        Ok(true) => {
            measure.decoder = active.name();
            let shown_us = now_us();
            measure.shown(active.picture(), &record, shown_us, false);
            None
        }
        Ok(false) => None,
        Err(_) => {
            *decoding = None;
            measure.recovery.on_disruption(now_us());
            Some(KeyframeReason::DecodeError)
        }
    }
}

/// `--local`: encoder and decoder of `codecs` on this device only, at the
/// wanted rate; the codec is the first of the negotiation with itself.
pub(crate) fn run_local(options: BenchOptions, codecs: &CodecRegistry) -> io::Result<BenchReport> {
    let choices = negotiate(
        &codecs.encoder_offers(),
        &codecs.decoder_offers(),
        options.codec,
    );
    let choice = choices
        .first()
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, NO_H264))?;
    let size = options.size.fit_within(choice.max_size);
    let size = match choice.codec {
        CodecKind::H264 => size.even(),
        CodecKind::Tiles => size,
    };
    let mut measure = Measure::new(size);
    let fps = u32::from(options.fps.max(1));
    let opened = codecs.open_encoder(&choice, size, fps)?;
    let mut encoder = opened.encoder;
    let header = GopHeader {
        gop_id: 1,
        kind: StreamKind::Video,
        codec: choice.codec,
        profile: choice.profile,
        size,
    };
    let mut decoder = codecs.open_decoder(&header)?;
    measure.decoder = decoder.name();
    // The start rate of a direct path (C3); `tiles` ignores it.
    let target_bps = PathKind::Direct.start_bps();
    let mut image = BgraImage::new(size)?;
    let interval = Duration::from_micros(1_000_000 / u64::from(fps));
    let end = Instant::now() + Duration::from_secs(u64::from(options.seconds));
    let mut frame_id = 0u64;
    let mut next = Instant::now();
    measure.codec = Some(choice.codec);
    while Instant::now() < end {
        frame_id += 1;
        let capture_ts_us = now_us();
        render(frame_id, &mut image)?;
        let encoded = encoder.encode(&image, frame_id == 1, target_bps)?;
        let count = encoded.records.len();
        for (index, payload) in encoded.records.into_iter().enumerate() {
            let mut flags = if encoded.keyframe { FLAG_KEYFRAME } else { 0 };
            if index + 1 < count {
                flags |= FLAG_MORE;
            }
            let bytes = (payload.len() + RECORD_HEADER_BYTES) as u64;
            let record = FrameRecord {
                frame_id,
                capture_ts_us,
                flags,
                payload,
            };
            measure.received.record(now_us(), bytes);
            if decoder.decode(&record)? {
                measure.shown(decoder.picture(), &record, now_us(), true);
            }
        }
        measure.highest_frame = frame_id;
        next += interval;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    measure.report(&encoder.name(), "local")
}
