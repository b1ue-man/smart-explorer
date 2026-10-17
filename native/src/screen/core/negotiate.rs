//! Codec negotiation (decision C2 „Aushandlung“). The host lists its encoders
//! `{codec, profile, hardware, max_width, max_height}`, the viewer its
//! decoders `{codec, profile, hardware}`. The ranking is hardware/hardware,
//! hardware/software, software/hardware, software/software; within one tier
//! `high` before `cb`; the lossless tile codec always last. The tile codec is
//! part of protocol version 1 and lives in this platform-neutral core, so
//! every pair of peers has it as the last choice even when a list omits it.
use serde::{Deserialize, Serialize};

use super::pixel_convert::FrameSize;

/// Most offers one side may list: codec × profile × hardware gives 4 H.264
/// entries plus `tiles` in version 1; 32 leaves room for later codecs and
/// several devices per kind, and bounds the pairing to 1024 candidates.
pub(crate) const MAX_CODEC_OFFERS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodecKind {
    H264,
    Tiles,
}

impl CodecKind {
    /// Code in the head of a picture stream; 3 stays reserved for AV1 (C2 L5).
    pub(crate) fn wire_code(self) -> u8 {
        match self {
            Self::H264 => 1,
            Self::Tiles => 2,
        }
    }

    pub(crate) fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::H264),
            2 => Some(Self::Tiles),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Tiles => "tiles",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum H264Profile {
    /// Constrained Baseline.
    #[serde(rename = "cb")]
    ConstrainedBaseline,
    /// High without B frames, i.e. Constrained High (the host never sends B
    /// frames).
    #[serde(rename = "high")]
    High,
}

impl H264Profile {
    pub(crate) fn wire_code(self) -> u8 {
        match self {
            Self::ConstrainedBaseline => 1,
            Self::High => 2,
        }
    }

    pub(crate) fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::ConstrainedBaseline),
            2 => Some(Self::High),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::ConstrainedBaseline => "cb",
            Self::High => "high",
        }
    }

    /// Whether a decoder of this profile decodes a stream of `stream`: a
    /// Constrained Baseline stream also conforms to Main and High.
    pub(crate) fn decodes(self, stream: Self) -> bool {
        self == stream || self == Self::High
    }
}

/// One encoder of the host or one decoder of the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct CodecOffer {
    pub codec: CodecKind,
    /// Required for H.264, absent for `tiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<H264Profile>,
    #[serde(default)]
    pub hardware: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
}

impl CodecOffer {
    pub(crate) fn h264(profile: H264Profile, hardware: bool) -> Self {
        Self {
            codec: CodecKind::H264,
            profile: Some(profile),
            hardware,
            max_width: None,
            max_height: None,
        }
    }

    pub(crate) fn tiles() -> Self {
        Self {
            codec: CodecKind::Tiles,
            profile: None,
            hardware: false,
            max_width: None,
            max_height: None,
        }
    }

    pub(crate) fn with_max(self, size: FrameSize) -> Self {
        Self {
            max_width: Some(size.width),
            max_height: Some(size.height),
            ..self
        }
    }

    /// The largest frame of this offer within the protocol limit (C2).
    pub(crate) fn max_size(&self) -> FrameSize {
        let side = |value: Option<u32>, limit: u32| value.unwrap_or(limit).clamp(1, limit);
        FrameSize::new(
            side(self.max_width, FrameSize::MAX.width),
            side(self.max_height, FrameSize::MAX.height),
        )
    }
}

/// `--codec auto|h264|tiles` and the menu „Kodierung“; „Verlustfrei“ and the
/// group preview use `Tiles`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum CodecPreference {
    #[default]
    Auto,
    H264,
    Tiles,
}

impl CodecPreference {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "h264" => Some(Self::H264),
            "tiles" => Some(Self::Tiles),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::H264 => "h264",
            Self::Tiles => "tiles",
        }
    }
}

/// One usable pairing; the first of `negotiate` is used, the others are the
/// fallbacks when an encoder refuses at run time (C2: a hardware MFT that
/// refuses `cb`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CodecChoice {
    pub codec: CodecKind,
    pub profile: Option<H264Profile>,
    pub encoder_hardware: bool,
    pub decoder_hardware: bool,
    pub max_size: FrameSize,
}

fn smaller(a: FrameSize, b: FrameSize) -> FrameSize {
    FrameSize::new(a.width.min(b.width), a.height.min(b.height))
}

fn tier(encoder_hardware: bool, decoder_hardware: bool) -> u8 {
    match (encoder_hardware, decoder_hardware) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    }
}

/// All usable pairings in rank order; empty only when `preference` is `H264`
/// and no H.264 pairing exists. Offers beyond `MAX_CODEC_OFFERS` are ignored.
pub(crate) fn negotiate(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    preference: CodecPreference,
) -> Vec<CodecChoice> {
    let encoders = &encoders[..encoders.len().min(MAX_CODEC_OFFERS)];
    let decoders = &decoders[..decoders.len().min(MAX_CODEC_OFFERS)];
    let mut choices: Vec<CodecChoice> = Vec::new();
    if preference != CodecPreference::Tiles {
        let mut ranked = Vec::new();
        for (encoder_index, encoder) in encoders.iter().enumerate() {
            let (CodecKind::H264, Some(stream)) = (encoder.codec, encoder.profile) else {
                continue;
            };
            for (decoder_index, decoder) in decoders.iter().enumerate() {
                let (CodecKind::H264, Some(decodes)) = (decoder.codec, decoder.profile) else {
                    continue;
                };
                if !decodes.decodes(stream) {
                    continue;
                }
                let profile_rank = u8::from(stream != H264Profile::High);
                let choice = CodecChoice {
                    codec: CodecKind::H264,
                    profile: Some(stream),
                    encoder_hardware: encoder.hardware,
                    decoder_hardware: decoder.hardware,
                    max_size: smaller(encoder.max_size(), decoder.max_size()),
                };
                let rank = (tier(encoder.hardware, decoder.hardware), profile_rank);
                ranked.push((rank, encoder_index, decoder_index, choice));
            }
        }
        ranked.sort_by_key(|(rank, encoder_index, decoder_index, _)| {
            (*rank, *encoder_index, *decoder_index)
        });
        for (.., choice) in ranked {
            if !choices.contains(&choice) {
                choices.push(choice);
            }
        }
    }
    if preference != CodecPreference::H264 {
        let tile_limit = |offers: &[CodecOffer]| {
            offers
                .iter()
                .filter(|offer| offer.codec == CodecKind::Tiles)
                .map(CodecOffer::max_size)
                .fold(FrameSize::MAX, smaller)
        };
        choices.push(CodecChoice {
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: smaller(tile_limit(encoders), tile_limit(decoders)),
        });
    }
    choices
}

#[cfg(test)]
#[path = "negotiate_tests.rs"]
mod tests;
