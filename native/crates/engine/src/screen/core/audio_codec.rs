//! Coding of the host's sound (plan 2026-11-17 P9 „Audio“): Opus through
//! `opus-rs`, a pure-Rust port of libopus 1.6 (no C toolchain, so every
//! target of this workspace builds it), in the restricted-low-delay
//! application (CELT only, no SILK look-ahead) with 5 ms frames, as
//! Sunshine sends to Moonlight; PCM16 as the uncompressed alternative of
//! the packet format. A frame is 240 samples per channel, interleaved
//! stereo at 48 kHz.
use std::io;

use opus_rs::{Application, OpusDecoder, OpusEncoder};

use super::audio_convert::f32_to_pcm16;
use super::audio_wire::{AudioCodec, CHANNELS, FRAME_SAMPLES, MAX_PAYLOAD, SAMPLE_RATE};

/// Opus bit rate: 128 kbit/s stereo is transparent for CELT at 5 ms frames
/// in listening tests of the Opus authors (RFC 6716, 2.1.1: music at
/// 64-128 kbit/s stereo); Sunshine's stereo default is 96 kbit/s.
pub const OPUS_BITRATE: i32 = 128_000;
/// Opus complexity: 0 to 10; the encoder of a 5 ms frame stays far below a
/// millisecond at this setting on desktop processors (assumption, not
/// measured on this build).
pub const OPUS_COMPLEXITY: i32 = 7;
/// Samples of one interleaved frame.
pub const FRAME_VALUES: usize = FRAME_SAMPLES * CHANNELS as usize;

fn codec_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// The encoder of the capturing host.
pub enum AudioEncoder {
    Opus(Box<OpusEncoder>),
    Pcm16,
}

impl AudioEncoder {
    pub fn new(codec: AudioCodec) -> io::Result<Self> {
        match codec {
            AudioCodec::Opus => {
                let channels = usize::from(CHANNELS);
                let rate = i32::try_from(SAMPLE_RATE).map_err(|_| codec_error("rate"))?;
                let mut encoder = OpusEncoder::new(rate, channels, Application::RestrictedLowDelay)
                    .map_err(codec_error)?;
                encoder.bitrate_bps = OPUS_BITRATE;
                encoder.complexity = OPUS_COMPLEXITY;
                encoder.use_cbr = false;
                Ok(Self::Opus(Box::new(encoder)))
            }
            AudioCodec::Pcm16 => Ok(Self::Pcm16),
            AudioCodec::Other => Err(codec_error("unknown audio codec")),
        }
    }

    pub fn codec(&self) -> AudioCodec {
        match self {
            Self::Opus(_) => AudioCodec::Opus,
            Self::Pcm16 => AudioCodec::Pcm16,
        }
    }

    /// Codes one frame (`FRAME_VALUES` interleaved samples) into `out`.
    pub fn encode(&mut self, frame: &[f32], out: &mut Vec<u8>) -> io::Result<()> {
        if frame.len() != FRAME_VALUES {
            return Err(codec_error("audio frame has the wrong length"));
        }
        out.clear();
        match self {
            Self::Opus(encoder) => {
                out.resize(MAX_PAYLOAD, 0);
                let written = encoder
                    .encode(frame, FRAME_SAMPLES, out)
                    .map_err(codec_error)?;
                out.truncate(written.min(MAX_PAYLOAD));
            }
            Self::Pcm16 => {
                for sample in frame {
                    out.extend_from_slice(&f32_to_pcm16(*sample).to_le_bytes());
                }
            }
        }
        Ok(())
    }
}

/// The decoder of the viewer, one per codec of the packets it gets.
pub enum AudioDecoder {
    Opus(Box<OpusDecoder>),
    Pcm16,
}

impl AudioDecoder {
    pub fn new(codec: AudioCodec) -> io::Result<Self> {
        match codec {
            AudioCodec::Opus => {
                let rate = i32::try_from(SAMPLE_RATE).map_err(|_| codec_error("rate"))?;
                let decoder = OpusDecoder::new(rate, usize::from(CHANNELS)).map_err(codec_error)?;
                Ok(Self::Opus(Box::new(decoder)))
            }
            AudioCodec::Pcm16 => Ok(Self::Pcm16),
            AudioCodec::Other => Err(codec_error("unknown audio codec")),
        }
    }

    pub fn codec(&self) -> AudioCodec {
        match self {
            Self::Opus(_) => AudioCodec::Opus,
            Self::Pcm16 => AudioCodec::Pcm16,
        }
    }

    /// Decodes one frame into `out` (`FRAME_VALUES` samples); `None` is a
    /// lost frame: Opus conceals it, PCM16 plays silence.
    pub fn decode(&mut self, packet: Option<&[u8]>, out: &mut [f32]) -> io::Result<()> {
        if out.len() != FRAME_VALUES {
            return Err(codec_error("audio frame has the wrong length"));
        }
        match self {
            Self::Opus(decoder) => {
                let input = packet.unwrap_or(&[]);
                let samples = decoder
                    .decode(input, FRAME_SAMPLES, out)
                    .map_err(codec_error)?;
                if samples < FRAME_SAMPLES {
                    out[samples * usize::from(CHANNELS)..].fill(0.0);
                }
            }
            Self::Pcm16 => match packet {
                Some(bytes) if bytes.len() == FRAME_VALUES * 2 => {
                    for (sample, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
                        *sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0;
                    }
                }
                _ => out.fill(0.0),
            },
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "audio_codec_tests.rs"]
mod tests;
