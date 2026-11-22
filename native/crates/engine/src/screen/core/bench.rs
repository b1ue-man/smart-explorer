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
//! block arrived identical (always so for the lossless `tiles`). `stages_ms`
//! gives p50 and p95 per station of the picture path (`bench_stages.rs`);
//! with `--source` the host captures that source instead of the test
//! picture (no barcode, no PSNR), and a host that announces
//! `frame_timing_v1` adds its own stations (`bench_host.rs`, BK0). Plan
//! 2026-11-17 „Windows-Bild“ P3: the barcode is checked against the
//! decoder's own identity of its picture, late pictures, decode errors and
//! decoders that did not open are counted, and the report keeps its
//! counters when nothing arrived intact (`failure`).
use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::codec_registry::CodecRegistry;
use super::decoded_frame::DecodedFrame;
use super::gop_stream::{now_us, path_of, PictureEvent};
use super::host_backend::ScreenDecoder;
use super::negotiate::{negotiate, CodecKind, CodecOffer, CodecPreference};
use super::pixel_convert::{BgraImage, FrameSize};
use super::rate_control::honour::HonourMeter;
use super::rate_control::model::start_table_bps;
use super::session_state::{KeyframeRequester, SessionState};
use super::synthetic::{render, MIN_SYNTHETIC_SIZE};
use super::viewer_client::{ViewerError, ViewerSession};
use super::wire_binary::{
    FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME, FLAG_MORE, RECORD_HEADER_BYTES,
};
use super::wire_control::{HostFrame, KeyframeReason, KeyframeRequest, ViewerFrame};

#[path = "bench_stages.rs"]
pub mod stages;
pub use stages::{StageMs, StageSamples};
#[path = "bench_measure.rs"]
mod measure;
use measure::Measure;
#[path = "bench_host.rs"]
mod host;
pub use host::{HOST_TIMING_REPORTED, HOST_TIMING_UNSUPPORTED};

pub const DEFAULT_SIZE: FrameSize = FrameSize::new(1280, 720);
/// Plan 2026-11-17 P9: the highest rate a viewer may ask for, not 30 (the
/// measurement of v0.5.198 showed `wanted_fps 30` with a 50 fps encoder);
/// the host's cadence then follows its source and the measured encode
/// capacity, the hardware being the limit.
pub const DEFAULT_FPS: u16 = super::pacing::MAX_FPS as u16;
pub const DEFAULT_SECONDS: u32 = 10;
/// Longest measurement: one hour.
pub const MAX_SECONDS: u32 = 3600;
/// `--codec h264` on a device without an H.264 codec.
const NO_H264: &str = "this device has no H.264 codec; `se screen codec openh264 enable` loads \
     OpenH264, or use --codec tiles";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BenchOptions {
    pub codec: CodecPreference,
    pub size: FrameSize,
    pub fps: u16,
    pub seconds: u32,
}

impl BenchOptions {
    /// Checks the values a user typed.
    pub fn validate(&self) -> Result<(), String> {
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
    /// the ones of the named codec for `h264`, `hevc` or `av1` (an error
    /// without any), only `tiles` for `tiles`.
    pub fn decoders(&self, codecs: &CodecRegistry) -> Result<Vec<CodecOffer>, String> {
        let offers = codecs.decoder_offers();
        match self.codec {
            CodecPreference::Auto => Ok(offers),
            CodecPreference::Tiles => Ok(vec![CodecOffer::tiles()]),
            // P9 „Scharf (GPU)“: H.264, HEVC and the tiles.
            CodecPreference::Sharp => Ok(CodecPreference::Sharp.filter_decoders(&offers)),
            named => {
                let kind = named.codec();
                let chosen: Vec<CodecOffer> = offers
                    .into_iter()
                    .filter(|offer| Some(offer.codec) == kind)
                    .collect();
                if chosen.is_empty() {
                    return Err(if kind == Some(CodecKind::H264) {
                        NO_H264.into()
                    } else {
                        format!("this device has no {} decoder", named.key())
                    });
                }
                Ok(chosen)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatencyMs {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitrateKbps {
    pub mean: f64,
    pub max: f64,
}

/// `se screen bench --json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
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
    /// Pictures that came out of a later decoder call than their own (P3).
    #[serde(default)]
    pub late_frames: u64,
    /// Records the decoder refused (P3).
    #[serde(default)]
    pub decode_errors: u64,
    /// Picture groups no decoder opened for (P3).
    #[serde(default)]
    pub decoder_open_failures: u64,
    /// Lowest PSNR of the static text block; `null` = infinite.
    pub static_psnr_db: Option<f64>,
    /// p50 and p95 in milliseconds per station of the picture path; left
    /// out of the JSON while empty (P7).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stages_ms: BTreeMap<String, StageMs>,
    /// The host's source with `--source` (BK0); absent for the test picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `reported` or `unsupported_by_host`: whether the host's stations are
    /// in `stages_ms` (BK0); absent with `--local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_timing: Option<String>,
    /// Stages and limits the host reported (`frame_timing::ChainReport`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_chain: Option<super::frame_timing::ChainReport>,
    /// Why the measurement has no result (no picture arrived intact); the
    /// counters stay in the report (P3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// The current picture group and its decoder.
type Decoding = Option<(GopHeader, Box<dyn ScreenDecoder>)>;

/// Measures a running session of the test picture for `options.seconds`
/// with the decoders of `codecs`.
pub async fn run(
    session: ViewerSession,
    options: BenchOptions,
    codecs: &CodecRegistry,
) -> Result<BenchReport, ViewerError> {
    run_source(session, options, codecs, None).await
}

/// The end a host named when it closed the connection
/// (`EndReason::from_close`), else `error`.
fn closed_by_host(connection: &iroh::endpoint::Connection, error: io::Error) -> ViewerError {
    match super::session_state::EndReason::from_close_detail(connection) {
        Some((reason, Some(detail))) => ViewerError::Refused(format!("{}: {detail}", reason.key())),
        Some((reason, None)) => ViewerError::Refused(reason.key().into()),
        None => ViewerError::Io(error),
    }
}

/// `run` for a session of the host's `source` (`--source`, BK0); `None`
/// is the test picture.
pub async fn run_source(
    mut session: ViewerSession,
    options: BenchOptions,
    codecs: &CodecRegistry,
    source: Option<String>,
) -> Result<BenchReport, ViewerError> {
    let mut measure = Measure::new(options.size);
    measure.real = source.is_some();
    let mut host = host::HostTiming::default();
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
                // The host's reason travels with the code (2026-11-18: it was dropped).
                Some(Ok(HostFrame::Error(error))) => return Err(ViewerError::refused_by(&error)),
                Some(Ok(HostFrame::Stats(stats))) => {
                    if let Some(frame) = host.on_stats(stats, now_us()) {
                        let _ = session.control.send(frame).await;
                    }
                }
                Some(Ok(_)) => {}
                // The host's reason when it closed the connection before its
                // last `STATE`, as the other viewers read it (round 10).
                Some(Err(error)) => return Err(closed_by_host(&session.connection, error)),
                None => {
                    let error = io::Error::other("the host ended the session");
                    return Err(closed_by_host(&session.connection, error));
                }
            },
            event = session.pictures.recv() => {
                let Some(event) = event else {
                    let error = io::Error::other("the picture streams ended");
                    return Err(closed_by_host(&session.connection, error));
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
    let mut report = measure.report(&encoder, path.key());
    host.finish(&measure.clock, &mut report);
    report.source = source;
    Ok(report)
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
    if !record.has_more() {
        // Capture to the last record's arrival; the host stations are not
        // separable on this side of the wire.
        if let Some(latency) = measure.clock.latency_us(record.capture_ts_us, received_us) {
            measure
                .stages
                .record_us(stages::CAPTURE_TO_ARRIVAL, latency);
        }
    }
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
            measure.open_failures += 1;
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
    let decode_started = Instant::now();
    match active.decode(&record) {
        Ok(true) => {
            measure.stages.record_since(stages::DECODE, decode_started);
            measure.decoder = active.name();
            let shown_us = now_us();
            // P3: the picture the decoder gave, which may be an older one.
            let frame = active
                .decoded_frame()
                .unwrap_or_else(|| DecodedFrame::from_record(&record));
            measure.shown(active.picture(), frame, record.frame_id, shown_us, false);
            None
        }
        Ok(false) => None,
        Err(_) => {
            *decoding = None;
            measure.decode_errors += 1;
            measure.recovery.on_disruption(now_us());
            Some(KeyframeReason::DecodeError)
        }
    }
}

/// `--local`: encoder and decoder of `codecs` on this device only, at the
/// wanted rate; the codec is the first of the negotiation with itself.
pub fn run_local(options: BenchOptions, codecs: &CodecRegistry) -> io::Result<BenchReport> {
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
    let size = if choice.codec.is_video() {
        size.even()
    } else {
        size
    };
    let mut measure = Measure::new(size);
    let fps = u32::from(options.fps.max(1));
    let opened = codecs.open_encoder(&choice, size, fps)?;
    let mut encoder = opened.encoder;
    let header = GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id: 1,
        kind: StreamKind::Video,
        codec: choice.codec,
        profile: choice.profile,
        size,
    };
    let mut decoder = codecs.open_decoder(&header)?;
    measure.decoder = decoder.name();
    // The start rate of the stream as a session gives it (plan 2026-11-17,
    // `rate_control::model`); `tiles` ignores it.
    let chroma_444 = choice.codec.chroma_444(choice.profile);
    let target_bps = start_table_bps(size, fps, choice.codec, chroma_444);
    let mut honour = HonourMeter::watching();
    let mut image = BgraImage::new(size)?;
    let interval = Duration::from_micros(1_000_000 / u64::from(fps));
    let end = Instant::now() + Duration::from_secs(u64::from(options.seconds));
    let mut frame_id = 0u64;
    let mut next = Instant::now();
    // A key frame first and after a decode error (P3: the run goes on).
    let mut key_next = true;
    measure.codec = Some(choice.codec);
    while Instant::now() < end {
        frame_id += 1;
        let capture_ts_us = now_us();
        let capture_started = Instant::now();
        render(frame_id, &mut image)?;
        measure
            .stages
            .record_since(stages::CAPTURE, capture_started);
        let encode_started = Instant::now();
        super::convert_clock::arm();
        let encoded = encoder.encode(&image, std::mem::take(&mut key_next), target_bps);
        let convert_us = super::convert_clock::take();
        let encoded = encoded?;
        let bytes: u64 = encoded.records.iter().map(|r| r.len() as u64).sum();
        honour.on_frame(bytes, encoded.keyframe, target_bps, target_bps, fps);
        let total_us = u64::try_from(encode_started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let encode_us = super::convert_clock::encode_without(total_us, convert_us);
        measure.stages.record_us(stages::ENCODE, encode_us);
        if let Some(convert_us) = convert_us {
            measure.stages.record_us(stages::CONVERT, convert_us);
        }
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
            let decode_started = Instant::now();
            let decoded = match decoder.decode(&record) {
                Ok(decoded) => decoded,
                Err(_) => {
                    // P3: counted; a new decoder from the next key frame.
                    measure.decode_errors += 1;
                    measure.recovery.on_disruption(now_us());
                    decoder = codecs
                        .open_decoder(&header)
                        .inspect_err(|_| measure.open_failures += 1)?;
                    key_next = true;
                    break;
                }
            };
            if decoded {
                measure.stages.record_since(stages::DECODE, decode_started);
                let frame = decoder
                    .decoded_frame()
                    .unwrap_or_else(|| DecodedFrame::from_record(&record));
                measure.shown(decoder.picture(), frame, record.frame_id, now_us(), true);
            }
        }
        measure.highest_frame = frame_id;
        next += interval;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let mut report = measure.report(&encoder.name(), "local");
    // Plan 2026-11-17: the rate the encoder was given and whether it kept it.
    if choice.codec.is_video() {
        report.host_chain = Some(super::frame_timing::ChainReport {
            encoder_rate_kbps: Some(target_bps / 1000),
            rate_honour: Some(honour.verdict().key().to_string()),
            ..Default::default()
        });
    }
    Ok(report)
}

#[cfg(test)]
#[path = "bildkette_bench_task_tests.rs"]
mod bildkette_task_tests;
