//! Codec negotiation (decision C2 „Aushandlung“, plan 2026-11-14 „Bildkette“
//! B-30 to B-32). The host lists its encoders `{codec, profile, hardware,
//! max_width, max_height}`, the viewer its decoders `{codec, profile,
//! hardware}`. The ranking is hardware/hardware, hardware/software,
//! software/hardware, software/software; within the hardware/hardware tier
//! AV1 before HEVC before H.264, within every other tier H.264 before HEVC
//! before AV1 (B-30: H.264 before a software AV1); within one codec H.264
//! `high` before `cb`, HEVC and AV1 4:2:0 before 4:4:4; the lossless tile
//! codec always last. The tile codec is part of protocol version 1 and lives
//! in this platform-neutral core, so every pair of peers has it as the last
//! choice even when a list omits it.
//!
//! HEVC and AV1 travel only in the v2 lists of the hellos (`wire_hello`
//! `codecs_v2`, B-31): a peer of v0.5.196 fails on an unknown `CodecKind`.
//! Their `profile` reuses `H264Profile`: absent is the 4:2:0 profile (HEVC
//! Main, AV1 Main), `high` the 4:4:4 profile (HEVC Main 4:4:4 of the range
//! extensions, AV1 High, B-32); `cb` is never valid for them.
use serde::{Deserialize, Serialize};

use super::pixel_convert::FrameSize;

#[path = "codec_preference.rs"]
mod preference;
pub use preference::{
    decoder_preference, set_decoder_preference, with_decoder_pass, CodecPreference, CodecSetting,
    DecoderPreference,
};
// D6 of plan 2026-11-17 „Windows-Bild“ P3: the B-30 rank of the newer
// codecs only for a checked host encoder.
#[path = "negotiate_trust.rs"]
mod trust;
pub use trust::CodecTrust;

/// Most offers one list of a hello may carry: codec × profile × hardware
/// gives 4 H.264 entries plus `tiles` in version 1; 32 leaves room for later
/// codecs and several devices per kind. The legacy and the v2 list each
/// hold at most this many, so a negotiation pairs at most
/// `MAX_COMBINED_OFFERS` of each side (4096 candidates).
pub const MAX_CODEC_OFFERS: usize = 32;
pub const MAX_COMBINED_OFFERS: usize = 2 * MAX_CODEC_OFFERS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecKind {
    H264,
    Tiles,
    /// ITU-T H.265; only after `codecs_v2` (B-31).
    Hevc,
    /// AOMedia Video 1; only after `codecs_v2` (B-31).
    Av1,
}

impl CodecKind {
    /// Code in the head of a picture stream; 3 was reserved for AV1 since
    /// C2 L5, HEVC takes 4.
    pub fn wire_code(self) -> u8 {
        match self {
            Self::H264 => 1,
            Self::Tiles => 2,
            Self::Av1 => 3,
            Self::Hevc => 4,
        }
    }

    pub fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::H264),
            2 => Some(Self::Tiles),
            3 => Some(Self::Av1),
            4 => Some(Self::Hevc),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Tiles => "tiles",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
        }
    }

    /// A video codec (motion-compensated, one record per picture).
    pub fn is_video(self) -> bool {
        self != Self::Tiles
    }

    /// Known to a peer of v0.5.196: only these may enter the legacy lists.
    pub fn is_legacy(self) -> bool {
        matches!(self, Self::H264 | Self::Tiles)
    }

    /// Whether `profile` is a valid profile of this codec on the wire.
    pub fn profile_valid(self, profile: Option<H264Profile>) -> bool {
        match self {
            Self::H264 => profile.is_some(),
            Self::Tiles => profile.is_none(),
            Self::Hevc | Self::Av1 => matches!(profile, None | Some(H264Profile::High)),
        }
    }

    /// Whether a stream of this codec in `profile` has 4:4:4 chroma (B-32).
    pub fn chroma_444(self, profile: Option<H264Profile>) -> bool {
        matches!(self, Self::Hevc | Self::Av1) && profile == Some(H264Profile::High)
    }

    /// The standard's name of `profile` for this codec (`se screen codec`).
    pub fn profile_name(self, profile: Option<H264Profile>) -> &'static str {
        match (self, profile) {
            (Self::H264, Some(profile)) => profile.key(),
            (Self::Hevc, Some(H264Profile::High)) => "main_444",
            (Self::Av1, Some(H264Profile::High)) => "high",
            (Self::Hevc | Self::Av1, _) => "main",
            _ => "",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum H264Profile {
    /// Constrained Baseline.
    #[serde(rename = "cb")]
    ConstrainedBaseline,
    /// High without B frames, i.e. Constrained High (the host never sends B
    /// frames).
    #[serde(rename = "high")]
    High,
}

impl H264Profile {
    pub fn wire_code(self) -> u8 {
        match self {
            Self::ConstrainedBaseline => 1,
            Self::High => 2,
        }
    }

    pub fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::ConstrainedBaseline),
            2 => Some(Self::High),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::ConstrainedBaseline => "cb",
            Self::High => "high",
        }
    }

    /// Whether a decoder of this profile decodes a stream of `stream`: a
    /// Constrained Baseline stream also conforms to Main and High.
    pub fn decodes(self, stream: Self) -> bool {
        self == stream || self == Self::High
    }
}

/// One encoder of the host or one decoder of the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CodecOffer {
    pub codec: CodecKind,
    /// Required for H.264, absent for `tiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<H264Profile>,
    #[serde(default)]
    pub hardware: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_access: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_macroblocks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_macroblocks_per_second: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
}

impl CodecOffer {
    pub fn h264(profile: H264Profile, hardware: bool) -> Self {
        Self {
            codec: CodecKind::H264,
            profile: Some(profile),
            hardware,
            pixel_access: None,
            max_macroblocks: None,
            max_macroblocks_per_second: None,
            max_width: None,
            max_height: None,
        }
    }

    /// An HEVC or AV1 offer: 4:2:0 (Main), or 4:4:4 (B-32) with `chroma_444`.
    pub fn video(codec: CodecKind, chroma_444: bool, hardware: bool) -> Self {
        Self {
            profile: chroma_444.then_some(H264Profile::High),
            codec,
            ..Self::h264(H264Profile::High, hardware)
        }
    }

    /// Whether a decoder of this offer decodes a stream of `codec` in
    /// `profile`: H.264 by `H264Profile::decodes`, HEVC and AV1 4:4:4
    /// decoders also 4:2:0.
    pub fn decodes(&self, codec: CodecKind, profile: Option<H264Profile>) -> bool {
        self.codec == codec
            && match codec {
                CodecKind::H264 => self.profile.zip(profile).is_some_and(|(d, s)| d.decodes(s)),
                CodecKind::Tiles => true,
                CodecKind::Hevc | CodecKind::Av1 => {
                    codec.profile_valid(self.profile)
                        && codec.profile_valid(profile)
                        && (self.profile == profile || self.profile == Some(H264Profile::High))
                }
            }
    }

    pub fn tiles() -> Self {
        Self {
            codec: CodecKind::Tiles,
            profile: None,
            hardware: false,
            pixel_access: Some(true),
            max_macroblocks: None,
            max_macroblocks_per_second: None,
            max_width: None,
            max_height: None,
        }
    }

    pub fn with_pixel_access(self, confirmed: bool) -> Self {
        Self {
            pixel_access: Some(confirmed),
            ..self
        }
    }

    pub fn with_max(self, size: FrameSize) -> Self {
        Self {
            max_width: Some(size.width),
            max_height: Some(size.height),
            ..self
        }
    }

    pub fn pixel_limit(&self, contract: Option<super::frame_budget::FrameContract>) -> u64 {
        let budget = contract.map_or(super::pixel_convert::MAX_FRAME_PIXELS, |c| {
            c.limits.max_pixels
        });
        self.max_macroblocks.map_or_else(
            || {
                if self.codec == CodecKind::Tiles {
                    budget
                } else {
                    budget.min(super::pixel_convert::MAX_FRAME_PIXELS)
                }
            },
            |blocks| blocks.saturating_mul(256).min(budget),
        )
    }
    pub fn supports_size_rate(
        &self,
        size: FrameSize,
        fps: u32,
        contract: Option<super::frame_budget::FrameContract>,
    ) -> bool {
        let max = contract.map_or_else(|| self.max_size(), |c| self.max_size_for_contract(c));
        let blocks = super::frame_size_caps::macroblocks(size);
        size.width > 0
            && size.height > 0
            && size.width <= max.width
            && size.height <= max.height
            && size.pixels() <= self.pixel_limit(contract)
            && self.max_macroblocks.is_none_or(|n| n > 0 && blocks <= n)
            && (fps == 0
                || self.max_macroblocks_per_second.is_none_or(|n| {
                    n > 0
                        && blocks
                            .checked_mul(u64::from(fps))
                            .is_some_and(|rate| rate <= n)
                }))
    }
    /// The largest frame of this offer within the protocol limit (C2).
    pub fn max_size(&self) -> FrameSize {
        let side = |value: Option<u32>, limit: u32| value.unwrap_or(limit).clamp(1, limit);
        FrameSize::new(
            side(self.max_width, FrameSize::MAX.width),
            side(self.max_height, FrameSize::MAX.height),
        )
    }
}

impl CodecOffer {
    pub fn max_size_for_contract(&self, contract: super::frame_budget::FrameContract) -> FrameSize {
        let legacy = self.max_size();
        let explicit = match (self.max_width, self.max_height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => FrameSize::new(width, height),
            _ if self.codec == CodecKind::Tiles => contract.limits.max_size,
            _ => legacy,
        };
        let evidenced = if self.codec.is_video() && self.max_macroblocks.is_none() {
            smaller(explicit, legacy)
        } else {
            explicit
        };
        smaller(evidenced, contract.limits.max_size)
    }
}

/// One usable pairing; the first of `negotiate` is used, the others are the
/// fallbacks when an encoder refuses at run time (C2: a hardware MFT that
/// refuses `cb`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CodecChoice {
    pub codec: CodecKind,
    pub profile: Option<H264Profile>,
    pub encoder_hardware: bool,
    pub decoder_hardware: bool,
    pub max_size: FrameSize,
    pub max_pixels: u64,
    pub max_macroblocks_per_second: Option<u64>,
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

/// The 4:4:4 pairing of `choice`'s codec and stages among `choices` (B-32):
/// the host switches to it under „Schärfe vor Flüssigkeit“.
pub fn sharp_alternative(choices: &[CodecChoice], choice: &CodecChoice) -> Option<CodecChoice> {
    choices.iter().copied().find(|other| {
        other.codec == choice.codec
            && other.codec.chroma_444(other.profile)
            && !choice.codec.chroma_444(choice.profile)
            && other.encoder_hardware == choice.encoder_hardware
            && other.decoder_hardware == choice.decoder_hardware
    })
}

/// All usable pairings in rank order; empty only when `preference` names
/// one video codec and no pairing of it exists. Offers beyond
/// `MAX_COMBINED_OFFERS` are ignored.
pub fn negotiate(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    preference: CodecPreference,
) -> Vec<CodecChoice> {
    negotiate_for_contract(encoders, decoders, preference, None)
}
pub fn negotiate_for_contract(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    preference: CodecPreference,
    contract: Option<super::frame_budget::FrameContract>,
) -> Vec<CodecChoice> {
    negotiate_trusted(encoders, decoders, preference, contract, CodecTrust::ALL)
}

/// `negotiate_for_contract` of a host whose `trust` names the newer codecs
/// with a checked encoder (D6, `negotiate_trust.rs`).
pub fn negotiate_trusted(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    preference: CodecPreference,
    contract: Option<super::frame_budget::FrameContract>,
    trust: CodecTrust,
) -> Vec<CodecChoice> {
    if contract.is_some_and(|c| c.validate().is_err()) {
        return Vec::new();
    }
    let encoders = &encoders[..encoders.len().min(MAX_COMBINED_OFFERS)];
    let decoders = &decoders[..decoders.len().min(MAX_COMBINED_OFFERS)];
    let dimensions = |offer: &CodecOffer| {
        contract.map_or_else(|| offer.max_size(), |c| offer.max_size_for_contract(c))
    };
    let mut choices = Vec::new();
    if preference != CodecPreference::Tiles {
        let mut ranked = Vec::new();
        for (ei, encoder) in encoders.iter().enumerate() {
            let (codec, profile) = (encoder.codec, encoder.profile);
            if !codec.is_video() || !codec.profile_valid(profile) || !preference.admits(codec) {
                continue;
            }
            for (di, decoder) in decoders.iter().enumerate() {
                if !decoder.decodes(codec, profile)
                    || (contract.is_some_and(|c| c.hybrid_regions)
                        && decoder.pixel_access != Some(true))
                {
                    continue;
                }
                let max_pixels = encoder
                    .pixel_limit(contract)
                    .min(decoder.pixel_limit(contract));
                let max_macroblocks_per_second = match (
                    encoder.max_macroblocks_per_second,
                    decoder.max_macroblocks_per_second,
                ) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                if max_pixels == 0 || max_macroblocks_per_second == Some(0) {
                    continue;
                }
                let choice = CodecChoice {
                    codec,
                    profile,
                    encoder_hardware: encoder.hardware,
                    decoder_hardware: decoder.hardware,
                    max_size: smaller(dimensions(encoder), dimensions(decoder)),
                    max_pixels,
                    max_macroblocks_per_second,
                };
                // H.264 `high` before `cb`; HEVC and AV1 4:2:0 before 4:4:4.
                let profile_rank = if codec == CodecKind::H264 {
                    u8::from(profile != Some(H264Profile::High))
                } else {
                    u8::from(codec.chroma_444(profile))
                };
                let both = encoder.hardware && decoder.hardware;
                ranked.push((
                    (
                        tier(encoder.hardware, decoder.hardware),
                        trust.rank(codec, both),
                        profile_rank,
                    ),
                    ei,
                    di,
                    choice,
                ));
            }
        }
        ranked.sort_by_key(|(rank, ei, di, choice)| {
            (
                *rank,
                std::cmp::Reverse(choice.max_pixels.min(choice.max_size.pixels())),
                *ei,
                *di,
            )
        });
        for (_, _, _, choice) in ranked {
            if !choices.contains(&choice) {
                choices.push(choice);
            }
        }
    }
    if preference.admits(CodecKind::Tiles) {
        let initial = contract.map_or(FrameSize::MAX, |c| c.limits.max_size);
        let limit = |offers: &[CodecOffer]| {
            offers
                .iter()
                .filter(|offer| offer.codec == CodecKind::Tiles)
                .map(dimensions)
                .fold(initial, smaller)
        };
        choices.push(CodecChoice {
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: smaller(limit(encoders), limit(decoders)),
            max_pixels: contract.map_or(super::pixel_convert::MAX_FRAME_PIXELS, |c| {
                c.limits.max_pixels
            }),
            max_macroblocks_per_second: None,
        });
    }
    choices
}

#[cfg(test)]
#[path = "negotiate_tests.rs"]
mod tests;
