//! Fakes of the run-time codec check (plan 2026-11-17 „Windows-Bild“ P3)
//! for task tests without a GPU: an „H.264“ or „HEVC“ stream whose slices
//! carry the records of the lossless tile codec (`TilesScreenEncoder`,
//! `TilesScreenDecoder`), parameter sets on key frames, encoder faults on
//! request, decoders that answer late or broken, and a clock that only moves
//! when a fake says so.
use std::io;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::super::codec_registry::{DecoderFactory, EncoderFactory};
use super::super::decoded_frame::DecodedFrame;
use super::super::host_backend::{
    EncodedFrame, ScreenDecoder, ScreenEncoder, TilesScreenDecoder, TilesScreenEncoder,
};
use super::super::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile};
use super::super::pixel_convert::{BgraImage, FrameSize};
use super::super::synthetic::render;
use super::super::wire::invalid;
use super::super::wire_binary::{FrameRecord, GopHeader, FLAG_KEYFRAME, FLAG_MORE};
use super::VerifyClock;

/// The size of the task tests' round trips.
pub const SIZE: FrameSize = FrameSize::new(256, 144);

/// A clock that moves only on `advance` and `sleep`.
#[derive(Default)]
pub struct FakeClock {
    micros: AtomicU64,
}

impl FakeClock {
    pub fn advance(&self, by: Duration) {
        let micros = u64::try_from(by.as_micros()).unwrap_or(u64::MAX);
        self.micros.fetch_add(micros, Ordering::SeqCst);
    }
}

impl VerifyClock for FakeClock {
    fn now(&self) -> Duration {
        Duration::from_micros(self.micros.load(Ordering::SeqCst))
    }

    fn sleep(&self, duration: Duration) {
        self.advance(duration);
    }
}

/// What a fake encoder does wrong.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fault {
    #[default]
    None,
    /// Encodes another picture than it was given.
    Garbage,
    /// Each picture takes 250 ms on the fake clock.
    Slow,
    /// The first picture comes out of the second call.
    Late,
    /// A key frame asked for later than the first is none.
    IgnoresKeyframe,
    /// Key frames without parameter sets.
    NoParameterSets,
}

/// NAL types of the fake stream: parameter sets, key-frame slice, slice.
fn kinds(codec: CodecKind) -> (&'static [u8], u8, u8) {
    match codec {
        CodecKind::Hevc => (&[32, 33, 34], 19, 1),
        _ => (&[7, 8], 5, 1),
    }
}

fn header(codec: CodecKind, kind: u8) -> Vec<u8> {
    match codec {
        CodecKind::Hevc => vec![kind << 1, 1],
        _ => vec![0x60 | kind],
    }
}

/// Emulation prevention (H.264 7.4.1): no start code inside a unit.
fn escape(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + payload.len() / 64 + 1);
    let mut zeros = 0;
    for &byte in payload {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &byte in payload {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// The access unit of the tile `records`: parameter sets first when `sets`,
/// one slice per record (`first_mb_in_slice` / `first_slice_segment` 1 for
/// the first only), each payload escaped and closed by a stop byte.
pub fn wrap(codec: CodecKind, records: &[Vec<u8>], idr: bool, sets: bool) -> Vec<u8> {
    let (parameters, key, slice) = kinds(codec);
    let mut out = Vec::new();
    if sets {
        for kind in parameters {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&header(codec, *kind));
            out.push(0x42);
        }
    }
    for (index, record) in records.iter().enumerate() {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&header(codec, if idr { key } else { slice }));
        out.push(if index == 0 { 0x80 } else { 0x40 });
        out.extend_from_slice(&escape(record));
        out.push(0x80);
    }
    out
}

/// The tile records of the slices of `data`.
fn slices(codec: CodecKind, data: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let (_, key, slice) = kinds(codec);
    let size = header(codec, 0).len();
    let mut found = Vec::new();
    for unit in split_units(data) {
        let kind = match codec {
            CodecKind::Hevc => unit.first().map(|byte| (byte >> 1) & 0x3f),
            _ => unit.first().map(|byte| byte & 31),
        };
        if kind != Some(key) && kind != Some(slice) {
            continue;
        }
        let body = unit
            .get(size + 1..unit.len().saturating_sub(1))
            .ok_or_else(|| invalid("fake slice too short"))?;
        found.push(unescape(body));
    }
    Ok(found)
}

/// The units between four-byte start codes (the fake writes only those).
fn split_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 4 <= data.len() {
        if data[at..at + 4] == [0, 0, 0, 1] {
            starts.push(at + 4);
            at += 4;
        } else {
            at += 1;
        }
    }
    let mut units = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(data.len(), |next| next - 4);
        units.push(&data[*start..end]);
    }
    units
}

/// A fake video encoder around the tile encoder.
pub struct FakeEncoder {
    inner: TilesScreenEncoder,
    codec: CodecKind,
    fault: Fault,
    clock: Arc<FakeClock>,
    frames: u64,
    held: Vec<u8>,
}

impl ScreenEncoder for FakeEncoder {
    fn name(&self) -> String {
        "fake".into()
    }

    fn size(&self) -> FrameSize {
        self.inner.size()
    }

    fn encode(&mut self, frame: &BgraImage, keyframe: bool, bps: u64) -> io::Result<EncodedFrame> {
        self.frames += 1;
        if self.fault == Fault::Slow {
            self.clock.advance(Duration::from_millis(250));
        }
        let idr = self.frames == 1 || (keyframe && self.fault != Fault::IgnoresKeyframe);
        let garbage;
        let picture = if self.fault == Fault::Garbage {
            let mut other = frame.clone();
            render(self.frames + 1000, &mut other)?;
            garbage = other;
            &garbage
        } else {
            frame
        };
        let tiles = self.inner.encode(picture, idr, bps)?;
        let sets = idr && self.fault != Fault::NoParameterSets;
        let unit = wrap(self.codec, &tiles.records, idr, sets);
        if self.fault == Fault::Late && self.frames == 1 {
            self.held = unit;
            return Ok(EncodedFrame {
                records: Vec::new(),
                keyframe: false,
            });
        }
        let mut bytes = std::mem::take(&mut self.held);
        bytes.extend_from_slice(&unit);
        Ok(EncodedFrame {
            records: vec![bytes],
            keyframe: idr,
        })
    }
}

/// A factory of fake encoders that counts its openings.
pub struct FakeEncoderFactory {
    pub name: String,
    pub codec: CodecKind,
    pub hardware: bool,
    pub fault: Fault,
    pub clock: Arc<FakeClock>,
    pub opened: AtomicUsize,
}

impl EncoderFactory for FakeEncoderFactory {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        let offer = match self.codec {
            CodecKind::H264 => CodecOffer::h264(H264Profile::High, self.hardware),
            codec => CodecOffer::video(codec, false, self.hardware),
        };
        vec![offer.with_max(FrameSize::MAX)]
    }

    fn device(&self) -> String {
        "10de:0001:a1@1.2.3.4".into()
    }

    fn open(&self, _: &CodecChoice, size: FrameSize, _: u32) -> io::Result<Box<dyn ScreenEncoder>> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeEncoder {
            inner: TilesScreenEncoder::new(size)?,
            codec: self.codec,
            fault: self.fault,
            clock: self.clock.clone(),
            frames: 0,
            held: Vec::new(),
        }))
    }
}

/// A fake encoder factory of `codec` named `name`.
pub fn encoder(
    name: &str,
    codec: CodecKind,
    hardware: bool,
    fault: Fault,
    clock: &Arc<FakeClock>,
) -> Arc<FakeEncoderFactory> {
    Arc::new(FakeEncoderFactory {
        name: name.into(),
        codec,
        hardware,
        fault,
        clock: clock.clone(),
        opened: AtomicUsize::new(0),
    })
}

/// How a fake decoder answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Answer {
    #[default]
    Intact,
    /// Gives each picture one call late.
    Late,
    /// Gives a black picture.
    Broken,
}

/// A fake video decoder around the tile decoder.
pub struct FakeDecoder {
    inner: TilesScreenDecoder,
    codec: CodecKind,
    answer: Answer,
    held: Option<(DecodedFrame, BgraImage)>,
    out: Option<(DecodedFrame, BgraImage)>,
}

impl ScreenDecoder for FakeDecoder {
    fn name(&self) -> String {
        "fake-decoder".into()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        let parts = slices(self.codec, &record.payload)?;
        let count = parts.len();
        let mut complete = false;
        for (index, payload) in parts.into_iter().enumerate() {
            let mut flags = record.flags & FLAG_KEYFRAME;
            if index + 1 < count {
                flags |= FLAG_MORE;
            }
            let tile = FrameRecord {
                frame_id: record.frame_id,
                capture_ts_us: record.capture_ts_us,
                flags,
                payload,
            };
            complete = self.inner.decode(&tile)?;
        }
        let frame = match self.inner.decoded_frame() {
            Some(frame) if complete => frame,
            _ => return Ok(false),
        };
        let picture = match self.answer {
            Answer::Broken => BgraImage::new(self.inner.picture().size())?,
            _ => self.inner.picture().clone(),
        };
        if self.answer != Answer::Late {
            self.out = Some((frame, picture));
            return Ok(true);
        }
        match self.held.replace((frame, picture)) {
            Some(previous) => {
                self.out = Some(previous);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.out.as_ref().map(|(frame, _)| *frame)
    }

    fn picture(&self) -> &BgraImage {
        match &self.out {
            Some((_, picture)) => picture,
            None => self.inner.picture(),
        }
    }
}

/// A factory of fake decoders.
pub struct FakeDecoderFactory {
    pub name: String,
    pub codec: CodecKind,
    pub hardware: bool,
    pub answer: Answer,
}

impl DecoderFactory for FakeDecoderFactory {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        let offer = match self.codec {
            CodecKind::H264 => CodecOffer::h264(H264Profile::High, self.hardware),
            codec => CodecOffer::video(codec, false, self.hardware),
        };
        vec![offer.with_max(FrameSize::MAX)]
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        Ok(Box::new(FakeDecoder {
            inner: TilesScreenDecoder::new(header.size)?,
            codec: self.codec,
            answer: self.answer,
            held: None,
            out: None,
        }))
    }
}

/// A fake decoder factory of `codec` named `name`.
pub fn decoder(
    name: &str,
    codec: CodecKind,
    hardware: bool,
    answer: Answer,
) -> Arc<dyn DecoderFactory> {
    Arc::new(FakeDecoderFactory {
        name: name.into(),
        codec,
        hardware,
        answer,
    })
}

/// The pairing a round trip of `codec` checks.
pub fn choice(codec: CodecKind, hardware: bool) -> CodecChoice {
    CodecChoice {
        codec,
        profile: (codec == CodecKind::H264).then_some(H264Profile::High),
        encoder_hardware: hardware,
        decoder_hardware: false,
        max_size: FrameSize::MAX,
        max_pixels: FrameSize::MAX.pixels(),
        max_macroblocks_per_second: None,
    }
}
