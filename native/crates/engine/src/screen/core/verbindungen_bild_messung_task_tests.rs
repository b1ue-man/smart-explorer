//! Measurements M6 and M7 of plan 2026-11-14 (C5, C7, P7): where the time of
//! one picture goes (station times) and how the picture path behaves under
//! loss. Every case prints one line
//! `VERBINDUNGEN-MESSUNG fall=<M> plattform=<..> variante=<..> profil=<..> <size>=<value> ...`
//! (`share/core/verbindungen_messung_support.rs`). All tests are `#[ignore]`
//! and run only in the part `verbindungen` of the suite `crates-task`, one
//! after another (`--test-threads=1`); the link of M7 is the netem rule the
//! suite sets on `lo` (`SE_SPEED_PROFILE`: `loss`, `loss1`).
//!
//! Two ways to measure, both with the test picture `synthetic`:
//!
//! - `local`: `bench::run_local`, encoder and decoder of this device, the
//!   stations `capture`, `encode` and `decode` (what `se screen bench --local`
//!   reports in `stages_ms`);
//! - `paar`: a device pair in one process. The test plays the host with the
//!   real `GopWriter`, the viewer with the real picture readers of
//!   `gop_stream.rs`, both over a real QUIC connection with the `Screen`
//!   profile, so all five stations are known: `capture`, `encode`, `send`
//!   (all records of a frame handed to the stream), `network` (last record
//!   handed over until it arrived) and `decode`. A session with a real host
//!   process is not used: its fixture is private to `share/core/node_screen_tests.rs`.
//!
//! H.264 is measured with Cisco's OpenH264 when `SE_SCREEN_TASK_OPENH264`
//! names the pinned file (the suite downloads it, as for the screen tests),
//! otherwise with the codecs the process has; a device without H.264 prints
//! `status=nicht_verfuegbar`.
//!
//! The display is not measured (no screen on the runner). Recovery under loss
//! (M7) is measured as the gaps between shown frames longer than two frame
//! intervals ("time without a new picture"), since the in-process host does
//! not run the production pacing that resets stale streams and asks for key
//! frames.
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use iroh::endpoint::VarInt;
use tokio::sync::mpsc;

use super::bench::stages::{self, StageMs, StageSamples};
use super::bench::{run_local, BenchOptions};
use super::codec_registry::CodecRegistry;
use super::gop_stream::{
    accept_picture_streams, now_us, picture_channel, GopWriter, PictureEvent, VIEWER_UNI_STREAMS,
};
use super::host_backend::ScreenDecoder;
use super::metrics::LatencySamples;
use super::negotiate::{negotiate, CodecKind, CodecPreference};
use super::pixel_convert::{BgraImage, FrameSize};
use super::rate_control::PathKind;
use super::synthetic::render;
use super::wire_binary::{GopHeader, StreamKind};
use super::wire_control::ViewerFrame;
use crate::screen::os::openh264_codec::tests::{
    openh264_registry, store_with_openh264, OPENH264_ENV,
};
use crate::screen::os::openh264_codec::TEST_LOCK;
use crate::share::route_diagnostics::verbindungen_messung_support::{
    close_pair, field, hold, line, one_decimal, run_case, Link, ProfileOverrides, TransportPurpose,
};

/// Length of one picture measurement.
const SECONDS: u32 = 5;
/// Length of the measurement under loss (more packets, more losses).
const LOSS_SECONDS: u32 = 10;
const FPS: u16 = 30;
const SIZES: [FrameSize; 3] = [
    FrameSize::new(1920, 1080),
    FrameSize::new(2560, 1440),
    FrameSize::new(3840, 2160),
];

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The codecs of a measurement and what keeps them usable.
struct Codecs {
    registry: CodecRegistry,
    _store: Option<tempfile::TempDir>,
    _lock: MutexGuard<'static, ()>,
}

/// With `SE_SCREEN_TASK_OPENH264` (the pinned file the suite downloads):
/// Cisco's OpenH264 and the tiles, installed into a temporary store as the
/// screen tests do. Without it: the codecs this process has (Media Foundation
/// on Windows; OpenH264 only where a store holds it), and a device without an
/// H.264 codec prints `status=nicht_verfuegbar` for H.264.
fn measurement_codecs() -> Codecs {
    let lock = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    if std::env::var_os(OPENH264_ENV).is_some() {
        let (dir, store) = store_with_openh264();
        return Codecs {
            registry: openh264_registry(&store),
            _store: Some(dir),
            _lock: lock,
        };
    }
    Codecs {
        registry: crate::screen::os::codec_inventory::registry(),
        _store: None,
        _lock: lock,
    }
}

/// What one run of a device pair measured.
struct PairRun {
    codec: &'static str,
    frames_sent: u64,
    frames_shown: u64,
    stages: BTreeMap<String, StageMs>,
    latency_p50_ms: f64,
    latency_p95_ms: f64,
    latency_max_ms: f64,
    /// Time between two shown frames that took longer than two frame
    /// intervals, in milliseconds.
    gaps_ms: Vec<f64>,
}

/// Whether the registry has an encoder and a decoder for `preference`.
fn available(codecs: &CodecRegistry, preference: CodecPreference) -> bool {
    !negotiate(
        &codecs.encoder_offers(),
        &codecs.decoder_offers(),
        preference,
    )
    .is_empty()
}

/// Nearest-rank percentile of `values`; 0 without values.
fn percentile(values: &[f64], share: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (share * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The host plays `seconds` of the test picture at `FPS` through a real
/// connection; the viewer decodes what arrives.
async fn pair_run(
    codecs: &CodecRegistry,
    preference: CodecPreference,
    size: FrameSize,
    seconds: u32,
) -> Result<PairRun, String> {
    let choices = negotiate(
        &codecs.encoder_offers(),
        &codecs.decoder_offers(),
        preference,
    );
    let choice = choices
        .first()
        .copied()
        .ok_or_else(|| "this device has no codec for the preference".to_owned())?;
    let size = size.fit_within(choice.max_size);
    let size = match choice.codec {
        CodecKind::H264 => size.even(),
        CodecKind::Tiles => size,
    };
    let fps = u32::from(FPS);
    let mut encoder = codecs
        .open_encoder(&choice, size, fps)
        .map_err(text)?
        .encoder;
    let header = GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id: 1,
        kind: StreamKind::Video,
        codec: choice.codec,
        profile: choice.profile,
        size,
    };
    let link = Link::new().await?;
    let (host_connection, viewer_connection) = link
        .connect(
            crate::screen::core::wire::SCREEN_ALPN,
            TransportPurpose::Screen,
        )
        .await?;
    viewer_connection.set_max_concurrent_uni_streams(VarInt::from_u32(VIEWER_UNI_STREAMS));
    let (events_tx, mut events_rx) = picture_channel();
    let (acks_tx, mut acks_rx) = mpsc::channel::<ViewerFrame>(64);
    let reader = tokio::spawn(accept_picture_streams(
        viewer_connection.clone(),
        events_tx,
        acks_tx,
    ));
    let drain = tokio::spawn(async move { while acks_rx.recv().await.is_some() {} });
    let sent_at: Arc<Mutex<HashMap<u64, u64>>> = Arc::default();
    let host_stages: Arc<Mutex<StageSamples>> = Arc::default();
    let kept = host_connection.clone();
    let host = {
        let (sent_at, host_stages) = (sent_at.clone(), host_stages.clone());
        let end = Instant::now() + Duration::from_secs(u64::from(seconds));
        tokio::spawn(async move {
            let mut writer = GopWriter::open(&host_connection, header).await?;
            let mut image = BgraImage::new(size)?;
            let interval = Duration::from_micros(1_000_000 / u64::from(fps));
            let target_bps = PathKind::Direct.start_bps();
            let mut next = Instant::now();
            let mut frame_id = 0u64;
            while Instant::now() < end {
                frame_id += 1;
                let capture_ts_us = now_us();
                let started = Instant::now();
                render(frame_id, &mut image)?;
                let captured = started.elapsed();
                let started = Instant::now();
                let encoded = encoder.encode(&image, frame_id == 1, target_bps)?;
                let encoding = started.elapsed();
                let started = Instant::now();
                writer
                    .write_frame(frame_id, capture_ts_us, encoded.keyframe, encoded.records)
                    .await?;
                let sending = started.elapsed();
                if let Ok(mut map) = sent_at.lock() {
                    map.insert(frame_id, now_us());
                }
                if let Ok(mut samples) = host_stages.lock() {
                    samples.record_duration(stages::CAPTURE, captured);
                    samples.record_duration(stages::ENCODE, encoding);
                    samples.record_duration(stages::SEND, sending);
                }
                next += interval;
                if let Some(wait) = next.checked_duration_since(Instant::now()) {
                    tokio::time::sleep(wait).await;
                }
            }
            writer.finish();
            Ok::<u64, std::io::Error>(frame_id)
        })
    };
    let mut decoding: Option<(GopHeader, Box<dyn ScreenDecoder>)> = None;
    let mut viewer_stages = StageSamples::default();
    let mut latency = LatencySamples::default();
    let mut arrivals: Vec<u64> = Vec::new();
    let mut shown = 0u64;
    let deadline = tokio::time::sleep(Duration::from_secs(u64::from(seconds) + 2));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => break,
            event = events_rx.recv() => {
                let Some(event) = event else { break };
                let PictureEvent::Record { header, record, received_us } = event else {
                    continue;
                };
                let fresh = decoding
                    .as_ref()
                    .is_none_or(|(current, _)| current.gop_id < header.gop_id);
                if fresh {
                    if !record.is_keyframe() {
                        continue;
                    }
                    let next = codecs.open_decoder(&header).map_err(text)?;
                    decoding = Some((header, next));
                }
                let Some((current, active)) = decoding.as_mut() else { continue };
                if current.gop_id != header.gop_id {
                    continue;
                }
                if !record.has_more() {
                    let sent = sent_at
                        .lock()
                        .ok()
                        .and_then(|map| map.get(&record.frame_id).copied());
                    if let Some(sent) = sent {
                        viewer_stages.record_us(stages::NETWORK, received_us.saturating_sub(sent));
                    }
                }
                let started = Instant::now();
                if active.decode(&record).map_err(text)? {
                    viewer_stages.record_since(stages::DECODE, started);
                    let now = now_us();
                    shown += 1;
                    latency.record(now.saturating_sub(record.capture_ts_us));
                    arrivals.push(now);
                }
            }
        }
    }
    drop(events_rx);
    reader.abort();
    drain.abort();
    let frames_sent = match tokio::time::timeout(Duration::from_secs(10), host).await {
        Ok(joined) => joined.map_err(text)?.map_err(text)?,
        Err(_) => return Err("the host loop does not end".to_owned()),
    };
    close_pair(&(kept, viewer_connection));
    link.close().await;
    let summary = latency
        .summary()
        .ok_or_else(|| "no picture arrived".to_owned())?;
    let mut all_stages = viewer_stages.summary();
    all_stages.extend(
        host_stages
            .lock()
            .map_err(|_| "locked".to_owned())?
            .summary(),
    );
    let limit = 2 * 1_000_000 / u64::from(fps);
    let gaps_ms = arrivals
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .filter(|gap| *gap > limit)
        .map(|gap| gap as f64 / 1000.0)
        .collect();
    Ok(PairRun {
        codec: choice.codec.key(),
        frames_sent,
        frames_shown: shown,
        stages: all_stages,
        latency_p50_ms: summary.p50_ms,
        latency_p95_ms: summary.p95_ms,
        latency_max_ms: summary.max_ms,
        gaps_ms,
    })
}

/// The fields `<station>_p50_ms` and `<station>_p95_ms` of `stages`.
fn stage_fields(stages: &BTreeMap<String, StageMs>) -> Vec<(String, String)> {
    stages
        .iter()
        .flat_map(|(name, stage)| {
            [
                field(&format!("{name}_p50_ms"), one_decimal(stage.p50)),
                field(&format!("{name}_p95_ms"), one_decimal(stage.p95)),
            ]
        })
        .collect()
}

/// The fields of one M6 case: `size` measured `local` or as a device pair.
fn measure_size(
    codecs: &CodecRegistry,
    preference: CodecPreference,
    size: FrameSize,
    local: bool,
) -> Result<Vec<(String, String)>, String> {
    let mut fields = Vec::new();
    if local {
        let options = BenchOptions {
            codec: preference,
            size,
            fps: FPS,
            seconds: SECONDS,
        };
        let report = run_local(options, codecs).map_err(text)?;
        fields.push(field("codec", &report.codec));
        fields.push(field("bilder", report.frames_shown));
        fields.push(field("fps", one_decimal(report.fps_mean)));
        fields.extend(stage_fields(&report.stages_ms));
    } else {
        let run = run_case(pair_run(codecs, preference, size, SECONDS))?;
        fields.push(field("codec", run.codec));
        fields.push(field("gesendet", run.frames_sent));
        fields.push(field("bilder", run.frames_shown));
        fields.push(field("bild_p50_ms", one_decimal(run.latency_p50_ms)));
        fields.push(field("bild_p95_ms", one_decimal(run.latency_p95_ms)));
        fields.extend(stage_fields(&run.stages));
    }
    Ok(fields)
}

/// M6 for `codec` over all sizes, `local` or as a device pair. A size that
/// cannot be measured prints its line with `status=fehler` and fails the test
/// after the other sizes ran.
fn stations_case(codec: &str, preference: CodecPreference, local: bool) -> Result<(), String> {
    let _plan = hold(ProfileOverrides::default());
    let codecs = measurement_codecs();
    let mode = if local { "local" } else { "paar" };
    let mut failures = Vec::new();
    for size in SIZES {
        let variant = format!("{codec}_{}x{}_{mode}", size.width, size.height);
        let mut fields = Vec::new();
        if !available(&codecs.registry, preference) {
            fields.push(field("status", "nicht_verfuegbar"));
        } else {
            match measure_size(&codecs.registry, preference, size, local) {
                Ok(measured) => {
                    fields.push(field("status", "ok"));
                    fields.extend(measured);
                }
                Err(error) => {
                    fields.push(field("status", "fehler"));
                    failures.push(format!("{variant}: {error}"));
                }
            }
        }
        println!("{}", line("M6", &variant, fields));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m6_tiles_local() -> Result<(), String> {
    stations_case("tiles", CodecPreference::Tiles, true)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m6_tiles_paar() -> Result<(), String> {
    stations_case("tiles", CodecPreference::Tiles, false)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m6_h264_local() -> Result<(), String> {
    stations_case("h264", CodecPreference::H264, true)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m6_h264_paar() -> Result<(), String> {
    stations_case("h264", CodecPreference::H264, false)
}

/// M7: 1080p over the link the suite shaped (`loss`: 0.1 % loss, 100 ms round
/// trip; `loss1`: 1 % loss, 40 ms, 100 Mbit/s), as a device pair.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m7_bild_unter_verlust() -> Result<(), String> {
    let _plan = hold(ProfileOverrides::default());
    let codecs = measurement_codecs();
    let run = run_case(pair_run(
        &codecs.registry,
        CodecPreference::Auto,
        SIZES[0],
        LOSS_SECONDS,
    ))?;
    let interval_ms = 1000.0 / f64::from(FPS);
    println!(
        "{}",
        line(
            "M7",
            "paar_1080p",
            vec![
                field("codec", run.codec),
                field("gesendet", run.frames_sent),
                field("bilder", run.frames_shown),
                field("bild_p50_ms", one_decimal(run.latency_p50_ms)),
                field("bild_p95_ms", one_decimal(run.latency_p95_ms)),
                field("bild_max_ms", one_decimal(run.latency_max_ms)),
                field("bildintervall_ms", one_decimal(interval_ms)),
                field("luecken", run.gaps_ms.len()),
                field("luecke_p95_ms", one_decimal(percentile(&run.gaps_ms, 0.95))),
                field(
                    "luecke_max_ms",
                    one_decimal(run.gaps_ms.iter().copied().fold(0.0, f64::max))
                ),
            ]
        )
    );
    Ok(())
}
