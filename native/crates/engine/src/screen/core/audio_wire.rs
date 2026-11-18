//! The host's sound in a screen session (plan 2026-11-17 P9 „Audio“), its
//! negotiation and its packets. Model: Moonlight/Sunshine and Parsec send
//! the host's system output as Opus over an unreliable channel of the
//! session, beside the picture, in short frames (Sunshine: 5 ms packets,
//! 48 kHz stereo, CELT low delay), so a lost packet costs 5 ms of sound
//! and never waits for a retransmission.
//!
//! Negotiation (additive, an older peer keeps its behaviour):
//! - the viewer names `audio: 1` in `CLIENT_HELLO` when it can play the
//!   host's sound; an older host ignores the field;
//! - a host that can capture its output answers `audio` in `HELLO_OK`
//!   (`AudioAccept`: codec, rate, channels, frame length); an older host
//!   never does, and the viewer then never sends `AUDIO`;
//! - `AUDIO` (viewer to host, tag 53, one byte: 1 on, 0 off) switches the
//!   sound; a session starts without it, the viewer's „Ton“ sends the
//!   first `AUDIO` after `HELLO_OK`.
//!
//! Packets travel as QUIC datagrams of the session's connection (no other
//! screen code uses datagrams): kind `0xA5`, codec `u8`, sequence `u32`,
//! host clock in µs of the frame's last sample `u64`, all big-endian, then
//! the payload (one Opus packet or 5 ms of PCM16). Between the host's
//! capture process and the daemon a packet travels on the process's
//! standard output with a `u16` big-endian length in front.
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use super::wire::{invalid, BeReader};
use super::wire_hello::SessionMode;

/// Version of the sound contract both hellos name.
pub const AUDIO_V1: u16 = 1;
/// Viewer to host: 1 byte, 1 sends the host's sound, 0 stops it.
pub const AUDIO: u8 = 53;
/// Sample rate of every packet (Opus' native rate).
pub const SAMPLE_RATE: u32 = 48_000;
/// Channels of every packet: interleaved left, right.
pub const CHANNELS: u16 = 2;
/// Samples per channel of one frame: 5 ms at 48 kHz.
pub const FRAME_SAMPLES: usize = 240;
/// Length of one frame in µs.
pub const FRAME_US: u32 = 5_000;
/// First byte of an audio datagram.
pub const DATAGRAM_KIND: u8 = 0xA5;
/// Bytes before the payload of a packet.
pub const PACKET_HEAD: usize = 14;
/// Largest payload: 5 ms of PCM16 stereo (960 bytes) and every Opus packet
/// of a 5 ms frame up to 1.2 Mbit/s fit; below the QUIC datagram size of a
/// 1280-byte path MTU.
pub const MAX_PAYLOAD: usize = 1_000;
/// Largest packet.
pub const MAX_PACKET: usize = PACKET_HEAD + MAX_PAYLOAD;

/// How the payload of a packet is coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    /// Opus, CELT only (restricted low delay), one packet per frame.
    Opus,
    /// 16-bit little-endian PCM, interleaved, one frame per packet.
    Pcm16,
    /// A codec of a newer host; never played.
    #[serde(other)]
    Other,
}

impl AudioCodec {
    fn byte(self) -> u8 {
        match self {
            Self::Opus => 1,
            Self::Pcm16 => 2,
            Self::Other => 0,
        }
    }

    fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Opus,
            2 => Self::Pcm16,
            _ => Self::Other,
        }
    }
}

/// The host's answer in `HELLO_OK`: it sends its sound in this format once
/// the viewer asks with `AUDIO`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioAccept {
    pub version: u16,
    pub codec: AudioCodec,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_us: u32,
}

impl AudioAccept {
    /// The format of this build: Opus, 48 kHz stereo, 5 ms frames.
    pub fn opus() -> Self {
        Self {
            version: AUDIO_V1,
            codec: AudioCodec::Opus,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            frame_us: FRAME_US,
        }
    }

    /// Whether this build plays the format the host names.
    pub fn playable(&self) -> bool {
        self.version == AUDIO_V1
            && self.codec != AudioCodec::Other
            && self.sample_rate == SAMPLE_RATE
            && self.channels == CHANNELS
            && self.frame_us == FRAME_US
    }
}

/// The host's decision for the viewer's `audio` offer: sound only for a
/// session that shows the full picture (`view`, `control`), never for a
/// probe, a tile preview or a metadata channel, and only when this host can
/// capture its output.
pub fn answer(
    offer: Option<u16>,
    mode: SessionMode,
    metadata_only: bool,
    capture_available: bool,
) -> Option<AudioAccept> {
    let streams = matches!(mode, SessionMode::View | SessionMode::Control);
    (offer == Some(AUDIO_V1) && streams && !metadata_only && capture_available)
        .then(AudioAccept::opus)
}

/// The viewer's offer in `CLIENT_HELLO` for a session of `mode`.
pub fn offer(mode: SessionMode) -> Option<u16> {
    matches!(mode, SessionMode::View | SessionMode::Control).then_some(AUDIO_V1)
}

/// One frame of sound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioPacket {
    pub codec: AudioCodec,
    pub sequence: u32,
    /// Host clock (µs since its capture started) of the frame's last sample.
    pub host_us: u64,
    pub payload: Vec<u8>,
}

impl AudioPacket {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(invalid("audio payload exceeds its limit"));
        }
        let mut bytes = Vec::with_capacity(PACKET_HEAD + self.payload.len());
        bytes.push(DATAGRAM_KIND);
        bytes.push(self.codec.byte());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.host_us.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_PACKET {
            return Err(invalid("audio packet exceeds its limit"));
        }
        let mut reader = BeReader::new(bytes);
        if reader.u8()? != DATAGRAM_KIND {
            return Err(invalid("not an audio packet"));
        }
        let codec = AudioCodec::from_byte(reader.u8()?);
        let sequence = reader.u32()?;
        let host_us = reader.u64()?;
        let payload = reader.bytes(reader.remaining())?.to_vec();
        Ok(Self {
            codec,
            sequence,
            host_us,
            payload,
        })
    }

    /// Whether a datagram is a sound packet (its first byte).
    pub fn is_audio(bytes: &[u8]) -> bool {
        bytes.first() == Some(&DATAGRAM_KIND)
    }
}

/// Capture process to daemon: one packet with its length in front.
pub fn write_piped(writer: &mut impl Write, packet: &[u8]) -> io::Result<()> {
    if packet.len() > MAX_PACKET {
        return Err(invalid("audio packet exceeds its limit"));
    }
    let length = u16::try_from(packet.len()).map_err(|_| invalid("audio packet too long"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(packet)?;
    writer.flush()
}

/// Reads one packet of the capture process; `None` at a clean end.
pub fn read_piped(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut head = [0u8; 2];
    match reader.read_exact(&mut head) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = usize::from(u16::from_be_bytes(head));
    if length > MAX_PACKET {
        return Err(invalid("audio packet exceeds its limit"));
    }
    let mut packet = vec![0u8; length];
    reader.read_exact(&mut packet)?;
    Ok(Some(packet))
}

#[cfg(test)]
#[path = "audio_wire_tests.rs"]
mod tests;
