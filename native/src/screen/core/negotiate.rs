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
    pub pixel_access: Option<bool>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub max_macroblocks:Option<u64>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub max_macroblocks_per_second:Option<u64>,
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
            pixel_access: None,max_macroblocks:None,max_macroblocks_per_second:None,
            max_width: None,
            max_height: None,
        }
    }

    pub(crate) fn tiles() -> Self {
        Self {
            codec: CodecKind::Tiles,
            profile: None,
            hardware: false,
            pixel_access: Some(true),max_macroblocks:None,max_macroblocks_per_second:None,
            max_width: None,
            max_height: None,
        }
    }

    pub(crate) fn with_pixel_access(self, confirmed: bool) -> Self {Self {pixel_access:Some(confirmed),..self}}

    pub(crate) fn with_max(self, size: FrameSize) -> Self {
        Self {
            max_width: Some(size.width),
            max_height: Some(size.height),
            ..self
        }
    }

    pub(crate) fn pixel_limit(&self,contract:Option<super::frame_budget::FrameContract>)->u64 {
        let budget=contract.map_or(super::pixel_convert::MAX_FRAME_PIXELS,|c|c.limits.max_pixels);
        self.max_macroblocks.map_or_else(||if self.codec==CodecKind::Tiles {budget} else {budget.min(super::pixel_convert::MAX_FRAME_PIXELS)},
            |blocks|blocks.saturating_mul(256).min(budget))
    }
    pub(crate) fn supports_size_rate(&self,size:FrameSize,fps:u32,contract:Option<super::frame_budget::FrameContract>)->bool {
        let max=contract.map_or_else(||self.max_size(),|c|self.max_size_for_contract(c));
        let blocks=super::frame_size_caps::macroblocks(size);
        size.width>0 && size.height>0 && size.width<=max.width && size.height<=max.height && size.pixels()<=self.pixel_limit(contract)
            && self.max_macroblocks.is_none_or(|n|n>0 && blocks<=n)
            && (fps==0 || self.max_macroblocks_per_second.is_none_or(|n|n>0 && blocks.checked_mul(u64::from(fps)).is_some_and(|rate|rate<=n)))
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

impl CodecOffer {
    pub(crate) fn max_size_for_contract(&self, contract: super::frame_budget::FrameContract) -> FrameSize {
        let legacy=self.max_size();
        let explicit=match (self.max_width,self.max_height) {
            (Some(width),Some(height)) if width>0 && height>0 => FrameSize::new(width,height),
            _ if self.codec==CodecKind::Tiles => contract.limits.max_size,
            _ => legacy,
        };
        let evidenced=if self.codec==CodecKind::H264 && self.max_macroblocks.is_none() {smaller(explicit,legacy)} else {explicit};
        smaller(evidenced,contract.limits.max_size)
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
    pub max_pixels:u64,
    pub max_macroblocks_per_second:Option<u64>,
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
pub(crate) fn negotiate(encoders:&[CodecOffer],decoders:&[CodecOffer],preference:CodecPreference)->Vec<CodecChoice> {
    negotiate_for_contract(encoders,decoders,preference,None)
}
pub(crate) fn negotiate_for_contract(encoders:&[CodecOffer],decoders:&[CodecOffer],preference:CodecPreference,
    contract:Option<super::frame_budget::FrameContract>)->Vec<CodecChoice> {
    if contract.is_some_and(|c|c.validate().is_err()) {return Vec::new();}
    let encoders=&encoders[..encoders.len().min(MAX_CODEC_OFFERS)];
    let decoders=&decoders[..decoders.len().min(MAX_CODEC_OFFERS)];
    let dimensions=|offer:&CodecOffer|contract.map_or_else(||offer.max_size(),|c|offer.max_size_for_contract(c));
    let mut choices=Vec::new();
    if preference!=CodecPreference::Tiles {
        let mut ranked=Vec::new();
        for (ei,encoder) in encoders.iter().enumerate() {
            let (CodecKind::H264,Some(profile))=(encoder.codec,encoder.profile) else {continue;};
            for (di,decoder) in decoders.iter().enumerate() {
                if decoder.codec!=CodecKind::H264 || !decoder.profile.is_some_and(|p|p.decodes(profile))
                    || (contract.is_some_and(|c|c.hybrid_regions) && decoder.pixel_access!=Some(true)) {continue;}
                let max_pixels=encoder.pixel_limit(contract).min(decoder.pixel_limit(contract));
                let max_macroblocks_per_second=match (encoder.max_macroblocks_per_second,decoder.max_macroblocks_per_second) {
                    (Some(a),Some(b))=>Some(a.min(b)),(a,b)=>a.or(b),
                };
                if max_pixels==0 || max_macroblocks_per_second==Some(0) {continue;}
                let choice=CodecChoice {codec:CodecKind::H264,profile:Some(profile),encoder_hardware:encoder.hardware,
                    decoder_hardware:decoder.hardware,max_size:smaller(dimensions(encoder),dimensions(decoder)),max_pixels,max_macroblocks_per_second};
                ranked.push(((tier(encoder.hardware,decoder.hardware),u8::from(profile!=H264Profile::High)),ei,di,choice));
            }
        }
        ranked.sort_by_key(|(rank,ei,di,choice)|(*rank,std::cmp::Reverse(choice.max_pixels.min(choice.max_size.pixels())),*ei,*di));
        for (_,_,_,choice) in ranked {if !choices.contains(&choice) {choices.push(choice);}}
    }
    if preference!=CodecPreference::H264 {
        let initial=contract.map_or(FrameSize::MAX,|c|c.limits.max_size);
        let limit=|offers:&[CodecOffer]|offers.iter().filter(|offer|offer.codec==CodecKind::Tiles).map(dimensions).fold(initial,smaller);
        choices.push(CodecChoice {codec:CodecKind::Tiles,profile:None,encoder_hardware:false,decoder_hardware:false,
            max_size:smaller(limit(encoders),limit(decoders)),max_pixels:contract.map_or(super::pixel_convert::MAX_FRAME_PIXELS,|c|c.limits.max_pixels),max_macroblocks_per_second:None});
    }
    choices
}

#[cfg(test)]
#[path = "negotiate_tests.rs"]
mod tests;
