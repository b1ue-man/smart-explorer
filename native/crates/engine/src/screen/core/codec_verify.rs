//! Run-time check of the codecs (plan 2026-11-17 „Windows-Bild“ P3, TODO
//! `VERB-WIN-KODIERER`): an encoder counts as checked once one round trip
//! of the test picture (`synthetic.rs`) through it and a local decoder
//! succeeded. Measured on 2026-11-17: hardware encoders that enumerate were
//! offered and chosen although their pictures never arrived intact or took
//! a second each.
//!
//! The round trip: 6 pictures at the session's size, a key frame asked for
//! at pictures 1 and 5, pictures 3 to 6 at 30 per second, 6 Mbit/s. The
//! encoder passes when every picture gives exactly one access unit in its
//! own call (`coded_pictures`), its key-frame flag agrees with the stream
//! (`video_keyframe`), every asked key frame is one, the first key frame
//! carries its parameter sets, and encoding takes at most 200 ms at the 95th
//! percentile (pictures 2 to 6: the first one includes the encoder's lazy
//! start, decision of this batch). A decoder passes when it gives picture n
//! in the call of picture n (no delay) and the barcode reads n. The whole
//! round trip has a hard limit of 3 s (`Checks::check`, its own thread).
//!
//! Attribution: what the stream shows is the encoder's (`EncodeFailed`,
//! `Late` for a call without output, `NotOneUnit`, `KeyframeMismatch`,
//! `MissingKeyframe`, `MissingParameterSets`, `Slow`, `Deadline`). A decoder
//! that answers late is the decoder's (`Late`, the next decoder is tried). A
//! picture that does not decode or reads another barcode is the encoder's
//! (`Garbled`) unless another decoder decodes the same stream intact; then
//! the first decoder is `Garbled`. Without any local decoder of the codec,
//! or when every decoder was late or did not open, the encoder is
//! `Structural` (stream and time checked). Decoders are tried software
//! first (the more common reference), those already failed not at all.
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::codec_registry::{DecoderFactory, EncoderFactory};
use super::codec_verdicts::{FailReason, Verdict};
use super::frame_reference::ReferenceDecoder;
use super::frame_size_caps::{coded_pictures, stream_parameter_sets, video_keyframe};
use super::host_backend::ScreenDecoder;
use super::negotiate::{CodecChoice, CodecKind};
use super::pixel_convert::{BgraImage, FrameSize};
use super::synthetic::{read_barcode, render, BarcodeRead};
use super::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};

/// Pictures of one round trip.
pub const FRAMES: u64 = 6;
/// The pictures a key frame is asked for.
pub const KEY_FRAMES: [u64; 2] = [1, 5];
/// From this picture on the pictures follow at `CHECK_FPS`.
pub const PACED_FROM: u64 = 3;
pub const CHECK_FPS: u32 = 30;
pub const CHECK_BPS: u64 = 6_000_000;
/// Longest encoding at the 95th percentile.
pub const MAX_ENCODE_P95: Duration = Duration::from_millis(200);
/// Hard limit of one round trip.
pub const DEADLINE: Duration = Duration::from_secs(3);

/// The clock of a check; the task tests give one that only moves on
/// request.
pub trait VerifyClock: Send + Sync {
    /// Time since the clock's start.
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

/// The monotonic clock of this process.
pub struct RealClock(Instant);

impl RealClock {
    pub fn new() -> Self {
        Self(Instant::now())
    }
}

impl Default for RealClock {
    fn default() -> Self {
        Self::new()
    }
}

impl VerifyClock for RealClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// One encoded picture of the round trip.
struct Unit {
    frame_id: u64,
    bytes: Vec<u8>,
    keyframe: bool,
}

/// What one round trip found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub encoder: Verdict,
    /// Index into the decoders handed in and the verdict of each decoder
    /// the round trip could judge.
    pub decoders: Vec<(usize, Verdict)>,
    pub detail: String,
}

/// Why no verdict came out (the encoder did not open, the size has no test
/// picture): the encoder stays unchecked.
pub type Unchecked = String;

fn failed(reason: FailReason, detail: impl Into<String>) -> Outcome {
    Outcome {
        encoder: Verdict::Failed(reason),
        decoders: Vec::new(),
        detail: detail.into(),
    }
}

/// p95 by nearest rank.
fn p95(samples: &[Duration]) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort();
    let rank = (sorted.len() * 95).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().unwrap_or_default()
}

/// Encodes the test picture; the stream and time checks of the encoder.
fn encode_all(
    factory: &dyn EncoderFactory,
    choice: &CodecChoice,
    size: FrameSize,
    clock: &dyn VerifyClock,
) -> Result<Result<Vec<Unit>, Outcome>, Unchecked> {
    let mut image = BgraImage::new(size).map_err(|error| error.to_string())?;
    render(1, &mut image).map_err(|error| error.to_string())?;
    let mut encoder = factory
        .open(choice, size, CHECK_FPS)
        .map_err(|error| format!("{} does not open: {error}", factory.name()))?;
    let codec = choice.codec;
    let interval = Duration::from_micros(1_000_000 / u64::from(CHECK_FPS));
    let (mut units, mut times) = (Vec::new(), Vec::new());
    let mut paced: Option<Duration> = None;
    let mut parameters = false;
    for frame_id in 1..=FRAMES {
        if frame_id >= PACED_FROM {
            let due = paced.get_or_insert_with(|| clock.now());
            let now = clock.now();
            if *due > now {
                clock.sleep(*due - now);
            }
            *due += interval;
        }
        render(frame_id, &mut image).map_err(|error| error.to_string())?;
        let asked = KEY_FRAMES.contains(&frame_id);
        let started = clock.now();
        let encoded = encoder.encode(&image, asked, CHECK_BPS);
        let took = clock.now().saturating_sub(started);
        let encoded = match encoded {
            Ok(encoded) => encoded,
            Err(error) => {
                let text = format!("picture {frame_id}: {error}");
                return Ok(Err(failed(FailReason::EncodeFailed, text)));
            }
        };
        if frame_id > 1 {
            times.push(took);
        }
        let bytes = encoded.records.concat();
        if bytes.is_empty() {
            let text = format!("picture {frame_id} came out of no call of its own");
            return Ok(Err(failed(FailReason::Late, text)));
        }
        match coded_pictures(codec, &bytes) {
            Ok(1) => {}
            Ok(count) => {
                let text = format!("picture {frame_id}: {count} pictures in one call");
                return Ok(Err(failed(FailReason::NotOneUnit, text)));
            }
            Err(error) => {
                let text = format!("picture {frame_id}: {error}");
                return Ok(Err(failed(FailReason::NotOneUnit, text)));
            }
        }
        let stream = codec != CodecKind::Tiles && video_keyframe(codec, &bytes);
        if codec != CodecKind::Tiles && encoded.keyframe != stream {
            let text = format!(
                "picture {frame_id}: flag {} but the stream says {stream}",
                encoded.keyframe
            );
            return Ok(Err(failed(FailReason::KeyframeMismatch, text)));
        }
        if asked && codec != CodecKind::Tiles && !stream {
            let text = format!("picture {frame_id}: no key frame on request");
            return Ok(Err(failed(FailReason::MissingKeyframe, text)));
        }
        if stream && !parameters {
            if stream_parameter_sets(codec, &bytes).is_none() {
                let text = format!("picture {frame_id}: first key frame without parameter sets");
                return Ok(Err(failed(FailReason::MissingParameterSets, text)));
            }
            parameters = true;
        }
        units.push(Unit {
            frame_id,
            bytes,
            keyframe: encoded.keyframe || (codec == CodecKind::Tiles && frame_id == 1),
        });
    }
    let slowest = p95(&times);
    if slowest > MAX_ENCODE_P95 {
        let text = format!("encoding p95 {} ms", slowest.as_millis());
        return Ok(Err(failed(FailReason::Slow, text)));
    }
    Ok(Ok(units))
}

/// How one decoder took the round trip's stream.
enum Decoded {
    Intact,
    /// A picture came out of a later call (or of none).
    Late(String),
    /// A picture did not decode or reads another barcode.
    Broken(String),
    Unopened(String),
}

fn decode_with(factory: &dyn DecoderFactory, header: &GopHeader, units: &[Unit]) -> Decoded {
    let raw = match factory.open(header) {
        Ok(raw) => raw,
        Err(error) => return Decoded::Unopened(error.to_string()),
    };
    let mut decoder = if header.codec.is_video() {
        ReferenceDecoder::new_for_codec(raw, header.codec)
    } else {
        ReferenceDecoder::new(raw)
    };
    for unit in units {
        let record = FrameRecord {
            frame_id: unit.frame_id,
            capture_ts_us: unit.frame_id,
            flags: if unit.keyframe { FLAG_KEYFRAME } else { 0 },
            payload: unit.bytes.clone(),
        };
        let id = unit.frame_id;
        match decoder.decode(&record) {
            Err(error) => return Decoded::Broken(format!("picture {id}: {error}")),
            Ok(false) => return Decoded::Late(format!("picture {id} came out of no call")),
            Ok(true) => {}
        }
        let out = decoder.decoded_frame().map(|frame| frame.frame_id);
        if out != Some(id) {
            return Decoded::Late(format!("picture {id} gave picture {out:?}"));
        }
        match read_barcode(&decoder.picture().view(), header.size) {
            Ok(BarcodeRead::Valid(read)) if read == id => {}
            read => return Decoded::Broken(format!("picture {id} reads {read:?}")),
        }
    }
    Decoded::Intact
}

/// Whether `factory` may serve as the reference of `choice` at `size`, and
/// whether it does so in software (tried first).
fn reference_of(
    factory: &dyn DecoderFactory,
    choice: &CodecChoice,
    size: FrameSize,
) -> Option<bool> {
    let offers = factory.offers();
    let fits: Vec<_> = offers
        .iter()
        .filter(|offer| {
            offer.decodes(choice.codec, choice.profile) && offer.supports_size_rate(size, 0, None)
        })
        .collect();
    (!fits.is_empty()).then(|| fits.iter().any(|offer| !offer.hardware))
}

/// One round trip of `encoder` against `decoders` (those `skip` marks at
/// their index not); `Err` leaves the encoder unchecked.
pub fn round_trip(
    encoder: &dyn EncoderFactory,
    decoders: &[Arc<dyn DecoderFactory>],
    skip: &[bool],
    choice: &CodecChoice,
    size: FrameSize,
    clock: &dyn VerifyClock,
) -> Result<Outcome, Unchecked> {
    let units = match encode_all(encoder, choice, size, clock)? {
        Ok(units) => units,
        Err(outcome) => return Ok(outcome),
    };
    let header = GopHeader {
        gop_id: 1,
        frame_contract: None,
        family_layout_revision: None,
        kind: StreamKind::Video,
        codec: choice.codec,
        profile: choice.profile,
        size,
    };
    let mut candidates: Vec<(bool, usize)> = decoders
        .iter()
        .enumerate()
        .filter(|(index, _)| !skip.get(*index).copied().unwrap_or(false))
        .filter_map(|(index, factory)| {
            reference_of(factory.as_ref(), choice, size).map(|software| (!software, index))
        })
        .collect();
    candidates.sort();
    let mut judged: Vec<(usize, Verdict)> = Vec::new();
    let mut broken: Vec<(usize, String)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (_, index) in candidates {
        let name = decoders[index].name();
        match decode_with(decoders[index].as_ref(), &header, &units) {
            Decoded::Intact => {
                judged.push((index, Verdict::Verified));
                for (index, _) in &broken {
                    judged.push((*index, Verdict::Failed(FailReason::Garbled)));
                }
                return Ok(Outcome {
                    encoder: Verdict::Verified,
                    decoders: judged,
                    detail: format!("intact with {}", decoders[index].name()),
                });
            }
            Decoded::Late(text) => {
                judged.push((index, Verdict::Failed(FailReason::Late)));
                notes.push(format!("{name}: {text}"));
            }
            Decoded::Broken(text) => broken.push((index, text)),
            Decoded::Unopened(text) => notes.push(format!("{name} does not open: {text}")),
        }
    }
    if let Some((index, text)) = broken.into_iter().next() {
        let detail = format!("{}: {text}", decoders[index].name());
        return Ok(Outcome {
            encoder: Verdict::Failed(FailReason::Garbled),
            decoders: judged,
            detail,
        });
    }
    notes.insert(0, "no local decoder took the stream".into());
    Ok(Outcome {
        encoder: Verdict::Structural,
        decoders: judged,
        detail: notes.join("; "),
    })
}

// The gate at the opening and the check on its own thread.
#[path = "codec_verify_gate.rs"]
mod gate;
pub use gate::Checks;

#[cfg(test)]
#[path = "bildkette_verify_task_tests.rs"]
mod bildkette_verify_task_tests;
#[cfg(test)]
#[path = "bildkette_verify_fakes.rs"]
pub(super) mod fakes;
