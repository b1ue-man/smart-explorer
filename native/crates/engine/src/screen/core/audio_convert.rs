//! Sample conversion of the host's sound (plan 2026-11-17 P9 „Audio“): the
//! capture delivers the format of the output device (Windows: its mix
//! format, often 32-bit float at 44.1 or 48 kHz, two to eight channels);
//! the packets carry 48 kHz stereo. Channels mix down with the ITU-R
//! BS.775 coefficients (centre and surround at -3 dB, LFE left out), the
//! rate is converted by linear interpolation (a resampler of the device's
//! own quality is the device's AUTOCONVERTPCM path, which the capture asks
//! for first), and the frame assembler cuts 5 ms frames with the time of
//! their last sample.
use std::io;

use super::audio_wire::{FRAME_SAMPLES, SAMPLE_RATE};

/// -3 dB.
const SURROUND: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Most channels a device format may name (WAVEFORMATEXTENSIBLE, 7.1.4
/// and more fit).
pub const MAX_CHANNELS: u16 = 32;

/// Converts interleaved device samples to interleaved 48 kHz stereo.
pub struct ToStereo48 {
    rate: u32,
    channels: u16,
    /// Rate ratio in → out.
    step: f64,
    /// Position of the next output sample, in input frames after `last`.
    phase: f64,
    last: [f32; 2],
    mixed: Vec<f32>,
}

impl ToStereo48 {
    pub fn new(rate: u32, channels: u16) -> io::Result<Self> {
        if rate == 0 || channels == 0 || channels > MAX_CHANNELS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audio format has no rate or an unsupported channel count",
            ));
        }
        Ok(Self {
            rate,
            channels,
            step: f64::from(rate) / f64::from(SAMPLE_RATE),
            phase: 0.0,
            last: [0.0; 2],
            mixed: Vec::new(),
        })
    }

    /// Appends the converted samples of `input` to `out`; a trailing
    /// partial frame of `input` is ignored.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.mixed.clear();
        let channels = usize::from(self.channels);
        for frame in input.chunks_exact(channels) {
            let [left, right] = downmix(frame);
            self.mixed.push(left);
            self.mixed.push(right);
        }
        if self.rate == SAMPLE_RATE {
            out.extend_from_slice(&self.mixed);
            return;
        }
        let frames = self.mixed.len() / 2;
        // Position 0 is `last`, position k + 1 the k-th new frame.
        let at = |mixed: &[f32], last: [f32; 2], index: usize| -> [f32; 2] {
            if index == 0 {
                last
            } else {
                [mixed[2 * (index - 1)], mixed[2 * (index - 1) + 1]]
            }
        };
        let mut phase = self.phase;
        loop {
            let base = phase.floor();
            // `base` is a small non-negative whole number.
            let index = base as usize;
            if index + 1 > frames {
                break;
            }
            let fraction = (phase - base) as f32;
            let a = at(&self.mixed, self.last, index);
            let b = at(&self.mixed, self.last, index + 1);
            out.push(a[0] + (b[0] - a[0]) * fraction);
            out.push(a[1] + (b[1] - a[1]) * fraction);
            phase += self.step;
        }
        if frames > 0 {
            self.last = at(&self.mixed, self.last, frames);
            phase -= frames as f64;
        }
        self.phase = phase.max(0.0);
    }
}

/// One device frame mixed to left and right.
fn downmix(frame: &[f32]) -> [f32; 2] {
    match frame {
        [mono] => [*mono, *mono],
        [left, right] => [*left, *right],
        [left, right, rest @ ..] => {
            // WAVEFORMATEXTENSIBLE order: FL FR FC LFE BL BR (FLC FRC BC) SL SR.
            let get = |index: usize| rest.get(index - 2).copied().unwrap_or(0.0);
            let centre = get(2) * SURROUND;
            let (back_left, back_right) = (get(4) * SURROUND, get(5) * SURROUND);
            let (side_left, side_right) = if frame.len() >= 8 {
                (
                    get(frame.len() - 2) * SURROUND,
                    get(frame.len() - 1) * SURROUND,
                )
            } else {
                (0.0, 0.0)
            };
            [
                (left + centre + back_left + side_left).clamp(-1.0, 1.0),
                (right + centre + back_right + side_right).clamp(-1.0, 1.0),
            ]
        }
        [] => [0.0, 0.0],
    }
}

/// 16-bit samples to float (`sample / 32768`, the Opus convention).
pub fn pcm16_to_f32(input: &[i16], out: &mut Vec<f32>) {
    out.extend(input.iter().map(|sample| f32::from(*sample) / 32_768.0));
}

/// Float samples to 16 bit, clamped.
pub fn f32_to_pcm16(sample: f32) -> i16 {
    // Clamped to the i16 range before the cast.
    (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16
}

/// Cuts 48 kHz stereo into 5 ms frames stamped with the clock of their
/// last sample.
#[derive(Default)]
pub struct FrameAssembler {
    pending: Vec<f32>,
    /// Clock (µs) when the last pushed sample was captured.
    last_us: u64,
}

impl FrameAssembler {
    /// Samples whose last one was captured at `captured_us`.
    pub fn push(&mut self, stereo: &[f32], captured_us: u64) {
        self.pending.extend_from_slice(stereo);
        self.last_us = captured_us;
    }

    /// The next whole frame and the clock of its last sample.
    pub fn pop(&mut self) -> Option<(Vec<f32>, u64)> {
        let length = FRAME_SAMPLES * 2;
        if self.pending.len() < length {
            return None;
        }
        let frame: Vec<f32> = self.pending.drain(..length).collect();
        let after = (self.pending.len() / 2) as u64;
        let behind_us = after * 1_000_000 / u64::from(SAMPLE_RATE);
        Some((frame, self.last_us.saturating_sub(behind_us)))
    }
}

#[cfg(test)]
#[path = "audio_convert_tests.rs"]
mod tests;
