//! Rank D6 of plan 2026-11-17 „Windows-Bild“ P3: within the
//! hardware/hardware tier AV1 before HEVC before H.264 (B-30) only for the
//! codecs whose host encoder passed the run-time check with a local decoder
//! (`Verified`, `codec_verify.rs`); a newer codec without that proof follows
//! H.264 (HEVC before AV1, as in the other tiers). The host negotiates, so
//! only the host's trust counts; nothing travels on the wire. A host that
//! checks nothing (an older register, the task tests) trusts every codec and
//! ranks as before (`CodecTrust::ALL`), except AV1: plan 2026-11-17 P9
//! (AV1 under steady motion piled up and stalled, user measurement) ranks
//! AV1 last among the video codecs (4) in every case, so „Automatisch“
//! never starts with AV1; a viewer that names AV1 still gets it.
use super::CodecKind;

/// The codecs whose host encoder may keep its B-30 rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CodecTrust {
    bits: u8,
}

fn bit(codec: CodecKind) -> u8 {
    match codec {
        CodecKind::H264 => 1,
        CodecKind::Hevc => 2,
        CodecKind::Av1 => 4,
        CodecKind::Tiles => 8,
    }
}

impl Default for CodecTrust {
    fn default() -> Self {
        Self::ALL
    }
}

impl CodecTrust {
    /// Every codec: a host that does not check its encoders.
    pub const ALL: Self = Self { bits: u8::MAX };
    /// H.264 and the tiles only (they never lose their rank).
    pub const NONE: Self = Self { bits: 0 };

    /// `self` with `codec` trusted.
    pub fn with(self, codec: CodecKind) -> Self {
        Self {
            bits: self.bits | bit(codec),
        }
    }

    /// Whether the newer `codec` keeps its B-30 rank (H.264 and the tiles
    /// always).
    pub fn trusts(self, codec: CodecKind) -> bool {
        matches!(codec, CodecKind::H264 | CodecKind::Tiles) || self.bits & bit(codec) != 0
    }

    /// The order of `codec` within a tier: B-30 (the newer codecs first only
    /// when both sides run in hardware) with D6 (and only when trusted).
    pub fn rank(self, codec: CodecKind, both_hardware: bool) -> u8 {
        if !both_hardware {
            return match codec {
                CodecKind::H264 => 0,
                CodecKind::Hevc => 1,
                CodecKind::Av1 => 2,
                CodecKind::Tiles => 5,
            };
        }
        match codec {
            // P9: AV1 never first, trusted or not.
            CodecKind::Hevc if self.trusts(codec) => 1,
            CodecKind::H264 => 2,
            CodecKind::Hevc => 3,
            CodecKind::Av1 => 4,
            CodecKind::Tiles => 5,
        }
    }
}
