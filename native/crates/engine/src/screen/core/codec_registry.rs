//! Codec register (SCR-M3, decision C2): which encoders and decoders this
//! process has, in their order of choice, and opening them with fallback.
//! The platform side hands its codecs in as factories (`codec_inventory`
//! under `os/`: Media Foundation on Windows, OpenH264 on Windows and Linux);
//! the lossless tile codec is built in and always the last entry, so every
//! pair of peers keeps it as the final fallback. An encoder opens with the
//! factories that offer the chosen codec and profile, first those with the
//! chosen hardware flag, then the others (a hardware MFT that refuses `cb`
//! falls back to the software MFT, C2). `se screen codec --json` shows the
//! register as `CodecReport`. A decoder opens under the viewer's choice
//! „Dekodierer“ (`negotiate::decoder_preference`, plan 2026-11-14
//! „Bildkette“ 4.10): only implementations with a stage of that kind.
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::host_backend::{ScreenDecoder, ScreenEncoder};
use super::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile, MAX_CODEC_OFFERS};
use super::pixel_convert::FrameSize;
use super::wire_binary::GopHeader;

#[path = "codec_registry_continue.rs"]
mod continuation;
#[path = "codec_registry_open.rs"]
mod open;

/// Text at every place that controls OpenH264 (Cisco condition 3).
pub const OPENH264_NOTICE: &str = "OpenH264 Video Codec provided by Cisco Systems, Inc.";
/// The only OpenH264 version this build loads (pins in `openh264_pins.rs`).
pub const OPENH264_VERSION: &str = "2.6.0";
/// Name of the tile codec's encoder and decoder.
pub const TILES_NAME: &str = "tiles";

/// One encoder implementation of the platform (an MFT, OpenH264).
pub trait EncoderFactory: Send + Sync {
    fn open_admitted(
        &self,
        _choice: &CodecChoice,
        _size: FrameSize,
        _fps: u32,
        _grant: super::capture_admission::CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "encoder_factory_has_no_capture_admission",
        ))
    }
    /// `open_admitted` with the encoder's rate control starting at `bps`,
    /// the rate the session gives it (plan 2026-11-17), instead of its own
    /// start; a factory without such a start opens as `open_admitted` and
    /// takes `bps` with its first picture.
    fn open_admitted_at(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: super::capture_admission::CaptureGrant,
        _bps: u64,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.open_admitted(choice, size, fps, grant)
    }
    /// Name for statistics and `se screen codec`.
    fn name(&self) -> String;
    /// What it encodes: codec, profile, hardware and largest frame.
    fn offers(&self) -> Vec<CodecOffer>;
    /// Settings the encoder refused and left at its default (plan
    /// 2026-11-14 „Bildkette“ B-21), as `se screen codec --json` shows them,
    /// e.g. `CODECAPI_AVEncVideoLTRBufferControl`; none by default.
    fn refused_settings(&self) -> Vec<String> {
        Vec::new()
    }
    /// Opens an encoder for `choice` at `size` with `fps` frames per second.
    fn open(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>>;
}

/// One decoder implementation of the platform.
pub trait DecoderFactory: Send + Sync {
    fn name(&self) -> String;
    fn offers(&self) -> Vec<CodecOffer>;
    /// Opens a decoder for the picture group of `header`.
    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>>;
    /// B-36: whether the decoders it opens for `codec` go on over a
    /// continuation (`ScreenDecoder::keeps_references`); the viewer
    /// announces `gop_continue` only then (`codec_registry_continue.rs`).
    fn keeps_references(&self, _codec: CodecKind) -> bool {
        false
    }
}

/// An opened encoder, its factory and whether it runs on hardware.
pub struct OpenedEncoder {
    pub encoder: Box<dyn ScreenEncoder>,
    pub factory: String,
    pub hardware: bool,
}

/// One row of `se screen codec`: an implementation and one codec it covers
/// (an implementation of several codecs has a row for each).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecEntry {
    pub codec: CodecKind,
    /// The wire profiles (`cb`, `high`); empty for `tiles` and for HEVC and
    /// AV1 in 4:2:0 only (contract of v0.5.196, unchanged).
    pub profiles: Vec<H264Profile>,
    pub name: String,
    pub hardware: bool,
    /// The profiles by their names in the standard (`cb`, `high`, `main`,
    /// `main_444`, …), BK6.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_names: Vec<String>,
    /// Settings the encoder refused (B-21), BK6.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused_settings: Vec<String>,
}

/// State of the OpenH264 library of this user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenH264State {
    /// Not downloaded.
    Absent,
    Enabled,
    /// Downloaded and switched off by the user.
    Disabled,
    /// Present but not loadable: the file does not match its pin.
    Failed,
}

/// `openh264` of `se screen codec --json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenH264Report {
    pub state: OpenH264State,
    pub version: String,
    pub notice: String,
    /// Why the state is `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl OpenH264Report {
    pub fn new(state: OpenH264State, reason: Option<String>) -> Self {
        Self {
            state,
            version: OPENH264_VERSION.into(),
            notice: OPENH264_NOTICE.into(),
            reason,
        }
    }
}

/// `se screen codec --json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecReport {
    pub encoders: Vec<CodecEntry>,
    pub decoders: Vec<CodecEntry>,
    pub openh264: OpenH264Report,
}

/// The codecs of this process.
#[derive(Clone, Default)]
pub struct CodecRegistry {
    encoders: Vec<Arc<dyn EncoderFactory>>,
    decoders: Vec<Arc<dyn DecoderFactory>>,
}

/// Whether `offer` encodes `codec` and `profile`: `Some(true)` for the
/// exact profile, `Some(false)` for a Baseline H.264 stream every `High`
/// decoder accepts (`OpenH264` behind `High` encoders, W4).
pub(super) fn encodes_stream(
    offer: &CodecOffer,
    codec: CodecKind,
    profile: Option<H264Profile>,
) -> Option<bool> {
    if offer.codec != codec {
        return None;
    }
    if offer.profile == profile {
        return Some(true);
    }
    (codec == CodecKind::H264
        && profile
            .zip(offer.profile)
            .is_some_and(|(wanted, offered)| wanted.decodes(offered)))
    .then_some(false)
}

/// Offers of `factories` in order, each once, then `tiles`. H.264 and the
/// tiles fill the legacy list of a hello (at most `MAX_CODEC_OFFERS`, the
/// tile codec always included), HEVC and AV1 its v2 list (at most
/// `MAX_CODEC_OFFERS` more, plan 2026-11-14 „Bildkette“ B-31).
fn ordered_offers(lists: impl Iterator<Item = Vec<CodecOffer>>) -> Vec<CodecOffer> {
    let mut legacy: Vec<CodecOffer> = Vec::new();
    let mut newer: Vec<CodecOffer> = Vec::new();
    for offer in lists.flatten() {
        let list = if offer.codec.is_legacy() {
            &mut legacy
        } else {
            &mut newer
        };
        if offer.codec != CodecKind::Tiles && !list.contains(&offer) {
            list.push(offer);
        }
    }
    legacy.truncate(MAX_CODEC_OFFERS - 1);
    legacy.push(CodecOffer::tiles());
    newer.truncate(MAX_CODEC_OFFERS);
    legacy.extend(newer);
    legacy
}

/// The rows of one implementation: one per codec it offers, in the order
/// of its offers.
fn entries(name: String, offers: &[CodecOffer], refused: Vec<String>) -> Vec<CodecEntry> {
    let mut rows: Vec<CodecEntry> = Vec::new();
    for offer in offers {
        let index = match rows.iter().position(|row| row.codec == offer.codec) {
            Some(index) => index,
            None => {
                rows.push(CodecEntry {
                    codec: offer.codec,
                    profiles: Vec::new(),
                    name: name.clone(),
                    hardware: false,
                    profile_names: Vec::new(),
                    refused_settings: refused.clone(),
                });
                rows.len() - 1
            }
        };
        let row = &mut rows[index];
        row.hardware |= offer.hardware;
        if let Some(profile) = offer
            .profile
            .filter(|profile| !row.profiles.contains(profile))
        {
            row.profiles.push(profile);
        }
        let named = offer.codec.profile_name(offer.profile).to_string();
        if !named.is_empty() && !row.profile_names.contains(&named) {
            row.profile_names.push(named);
        }
    }
    rows
}

fn tiles_entry() -> CodecEntry {
    CodecEntry {
        codec: CodecKind::Tiles,
        profiles: Vec::new(),
        name: TILES_NAME.into(),
        hardware: false,
        profile_names: Vec::new(),
        refused_settings: Vec::new(),
    }
}

impl CodecRegistry {
    /// Only the tile codec (task tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn builtin() -> Self {
        Self::default()
    }

    /// The platform codecs in their order of choice; `tiles` follows them.
    pub fn new(
        encoders: Vec<Arc<dyn EncoderFactory>>,
        decoders: Vec<Arc<dyn DecoderFactory>>,
    ) -> Self {
        Self { encoders, decoders }
    }

    /// The host's encoders for `HELLO_OK` and the negotiation.
    pub fn encoder_offers(&self) -> Vec<CodecOffer> {
        ordered_offers(self.encoders.iter().map(|factory| factory.offers()))
    }

    /// The viewer's decoders for its hello, without the codecs that gave no
    /// picture in this process (`decoder_health`).
    pub fn decoder_offers(&self) -> Vec<CodecOffer> {
        super::decoder_health::usable_offers(&ordered_offers(self.decoders.iter().map(|factory| {
            factory
                .offers()
                .into_iter()
                .map(|offer| offer.with_pixel_access(true))
                .collect()
        })))
    }

    /// The register as `se screen codec` shows it.
    pub fn report(&self, openh264: OpenH264Report) -> CodecReport {
        let encoders = self.encoders.iter().flat_map(|factory| {
            entries(
                factory.name(),
                &factory.offers(),
                factory.refused_settings(),
            )
        });
        let decoders = self
            .decoders
            .iter()
            .flat_map(|factory| entries(factory.name(), &factory.offers(), Vec::new()));
        CodecReport {
            encoders: encoders.chain(std::iter::once(tiles_entry())).collect(),
            decoders: decoders.chain(std::iter::once(tiles_entry())).collect(),
            openh264,
        }
    }
}
