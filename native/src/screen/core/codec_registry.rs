//! Codec register (SCR-M3, decision C2): which encoders and decoders this
//! process has, in their order of choice, and opening them with fallback.
//! The platform side hands its codecs in as factories (`codec_inventory`
//! under `os/`: Media Foundation on Windows, OpenH264 on Windows and Linux);
//! the lossless tile codec is built in and always the last entry, so every
//! pair of peers keeps it as the final fallback. An encoder opens with the
//! factories that offer the chosen codec and profile, first those with the
//! chosen hardware flag, then the others (a hardware MFT that refuses `cb`
//! falls back to the software MFT, C2). `se screen codec --json` shows the
//! register as `CodecReport`.
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::host_backend::{ScreenDecoder, ScreenEncoder, TilesScreenDecoder, TilesScreenEncoder};
use super::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile, MAX_CODEC_OFFERS};
use super::pixel_convert::FrameSize;
use super::wire_binary::GopHeader;

/// Text at every place that controls OpenH264 (Cisco condition 3).
pub(crate) const OPENH264_NOTICE: &str = "OpenH264 Video Codec provided by Cisco Systems, Inc.";
/// The only OpenH264 version this build loads (pins in `openh264_pins.rs`).
pub(crate) const OPENH264_VERSION: &str = "2.6.0";
/// Name of the tile codec's encoder and decoder.
pub(crate) const TILES_NAME: &str = "tiles";

/// One encoder implementation of the platform (an MFT, OpenH264).
pub(crate) trait EncoderFactory: Send + Sync {
    /// Name for statistics and `se screen codec`.
    fn name(&self) -> String;
    /// What it encodes: codec, profile, hardware and largest frame.
    fn offers(&self) -> Vec<CodecOffer>;
    /// Opens an encoder for `choice` at `size` with `fps` frames per second.
    fn open(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>>;
}

/// One decoder implementation of the platform.
pub(crate) trait DecoderFactory: Send + Sync {
    fn name(&self) -> String;
    fn offers(&self) -> Vec<CodecOffer>;
    /// Opens a decoder for the picture group of `header`.
    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>>;
}

/// An opened encoder, its factory and whether it runs on hardware.
pub(crate) struct OpenedEncoder {
    pub encoder: Box<dyn ScreenEncoder>,
    pub factory: String,
    pub hardware: bool,
}

/// One row of `se screen codec`: an implementation and what it covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CodecEntry {
    pub codec: CodecKind,
    /// The H.264 profiles; empty for `tiles`.
    pub profiles: Vec<H264Profile>,
    pub name: String,
    pub hardware: bool,
}

/// State of the OpenH264 library of this user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OpenH264State {
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
pub(crate) struct OpenH264Report {
    pub state: OpenH264State,
    pub version: String,
    pub notice: String,
    /// Why the state is `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl OpenH264Report {
    pub(crate) fn new(state: OpenH264State, reason: Option<String>) -> Self {
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
pub(crate) struct CodecReport {
    pub encoders: Vec<CodecEntry>,
    pub decoders: Vec<CodecEntry>,
    pub openh264: OpenH264Report,
}

/// The codecs of this process.
#[derive(Clone, Default)]
pub(crate) struct CodecRegistry {
    encoders: Vec<Arc<dyn EncoderFactory>>,
    decoders: Vec<Arc<dyn DecoderFactory>>,
}

/// Whether `offer` encodes or decodes `choice`'s codec and profile.
fn offers_stream(offer: &CodecOffer, codec: CodecKind, profile: Option<H264Profile>) -> bool {
    offer.codec == codec && offer.profile == profile
}

/// Offers of `factories` in order, each once, then `tiles`; at most
/// `MAX_CODEC_OFFERS`, the tile codec always included.
fn ordered_offers(lists: impl Iterator<Item = Vec<CodecOffer>>) -> Vec<CodecOffer> {
    let mut offers: Vec<CodecOffer> = Vec::new();
    for offer in lists.flatten() {
        if offer.codec != CodecKind::Tiles && !offers.contains(&offer) {
            offers.push(offer);
        }
    }
    offers.truncate(MAX_CODEC_OFFERS - 1);
    offers.push(CodecOffer::tiles());
    offers
}

fn entry(name: String, offers: &[CodecOffer]) -> Option<CodecEntry> {
    let first = offers.first()?;
    let mut profiles: Vec<H264Profile> = Vec::new();
    for offer in offers.iter().filter(|offer| offer.codec == first.codec) {
        if let Some(profile) = offer.profile.filter(|profile| !profiles.contains(profile)) {
            profiles.push(profile);
        }
    }
    Some(CodecEntry {
        codec: first.codec,
        profiles,
        name,
        hardware: first.hardware,
    })
}

fn tiles_entry() -> CodecEntry {
    CodecEntry {
        codec: CodecKind::Tiles,
        profiles: Vec::new(),
        name: TILES_NAME.into(),
        hardware: false,
    }
}

impl CodecRegistry {
    /// Only the tile codec (task tests).
    #[cfg(test)]
    pub(crate) fn builtin() -> Self {
        Self::default()
    }

    /// The platform codecs in their order of choice; `tiles` follows them.
    pub(crate) fn new(
        encoders: Vec<Arc<dyn EncoderFactory>>,
        decoders: Vec<Arc<dyn DecoderFactory>>,
    ) -> Self {
        Self { encoders, decoders }
    }

    /// The host's encoders for `HELLO_OK` and the negotiation.
    pub(crate) fn encoder_offers(&self) -> Vec<CodecOffer> {
        ordered_offers(self.encoders.iter().map(|factory| factory.offers()))
    }

    /// The viewer's decoders for its hello.
    pub(crate) fn decoder_offers(&self) -> Vec<CodecOffer> {
        ordered_offers(self.decoders.iter().map(|factory| factory.offers()))
    }

    /// Opens an encoder for `choice` at `size`. H.264 tries every factory
    /// that offers the codec and profile within its largest frame, those with
    /// the chosen hardware flag first; the error names every refusal.
    pub(crate) fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<OpenedEncoder> {
        self.open_encoder_except(choice, size, fps, &[])
    }

    /// `open_encoder` without the factories named in `refused` (a host
    /// process skips an encoder that failed while encoding).
    pub(crate) fn open_encoder_except(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        refused: &[String],
    ) -> io::Result<OpenedEncoder> {
        if choice.codec == CodecKind::Tiles {
            return Ok(OpenedEncoder {
                encoder: Box::new(TilesScreenEncoder::new(size)?),
                factory: TILES_NAME.into(),
                hardware: false,
            });
        }
        let mut candidates: Vec<(bool, &Arc<dyn EncoderFactory>)> = Vec::new();
        for factory in &self.encoders {
            if refused.contains(&factory.name()) {
                continue;
            }
            let fitting = factory.offers().into_iter().find(|offer| {
                offers_stream(offer, choice.codec, choice.profile)
                    && size.width <= offer.max_size().width
                    && size.height <= offer.max_size().height
            });
            if let Some(offer) = fitting {
                candidates.push((offer.hardware, factory));
            }
        }
        candidates.sort_by_key(|(hardware, _)| *hardware != choice.encoder_hardware);
        let mut refusals: Vec<String> = Vec::new();
        for (hardware, factory) in candidates {
            match factory.open(choice, size, fps) {
                Ok(encoder) => {
                    let factory = factory.name();
                    return Ok(OpenedEncoder {
                        encoder,
                        factory,
                        hardware,
                    });
                }
                Err(error) => refusals.push(format!("{}: {error}", factory.name())),
            }
        }
        let codec = choice.codec.key();
        let profile = choice.profile.map_or("", H264Profile::key);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            if refusals.is_empty() {
                format!(
                    "no {codec} {profile} encoder for {}x{}",
                    size.width, size.height
                )
            } else {
                format!(
                    "no {codec} {profile} encoder opened: {}",
                    refusals.join("; ")
                )
            },
        ))
    }

    /// Opens a decoder for the picture group of `header`: `tiles` directly,
    /// H.264 with the first factory whose profile decodes the stream's.
    pub(crate) fn open_decoder(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        if header.codec == CodecKind::Tiles {
            return Ok(Box::new(TilesScreenDecoder::new(header.size)?));
        }
        let mut refusals: Vec<String> = Vec::new();
        for factory in &self.decoders {
            let decodes = factory.offers().iter().any(|offer| {
                offer.codec == header.codec
                    && match (offer.profile, header.profile) {
                        (Some(decoder), Some(stream)) => decoder.decodes(stream),
                        _ => false,
                    }
                    && header.size.width <= offer.max_size().width
                    && header.size.height <= offer.max_size().height
            });
            if !decodes {
                continue;
            }
            match factory.open(header) {
                Ok(decoder) => return Ok(decoder),
                Err(error) => refusals.push(format!("{}: {error}", factory.name())),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            if refusals.is_empty() {
                format!("no decoder for {}", header.codec.key())
            } else {
                format!("no decoder opened: {}", refusals.join("; "))
            },
        ))
    }

    /// The register as `se screen codec` shows it.
    pub(crate) fn report(&self, openh264: OpenH264Report) -> CodecReport {
        let encoders = self
            .encoders
            .iter()
            .filter_map(|factory| entry(factory.name(), &factory.offers()));
        let decoders = self
            .decoders
            .iter()
            .filter_map(|factory| entry(factory.name(), &factory.offers()));
        CodecReport {
            encoders: encoders.chain(std::iter::once(tiles_entry())).collect(),
            decoders: decoders.chain(std::iter::once(tiles_entry())).collect(),
            openh264,
        }
    }
}
