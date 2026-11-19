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
//!
//! Plan 2026-11-17 „Windows-Bild“ P3: a register with `Checks`
//! (`codec_verify.rs`) offers no encoder or decoder that failed its
//! run-time check, an HEVC or AV1 encoder only once it passed (H.264
//! unchecked as well: the gate at its opening checks it first), names the
//! verdict of every row (`CodecEntry::verdict`) and gives the host's trust
//! of the newer codecs to the negotiation (`codec_trust`, D6); AV1 never
//! keeps the first rank of „Automatisch“ (`keeps_first_rank`, plan P9).
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::codec_verdicts::{Role, VerdictView};
use super::codec_verify::Checks;
use super::host_backend::{ScreenDecoder, ScreenEncoder};
use super::negotiate::{
    CodecChoice, CodecKind, CodecOffer, CodecTrust, H264Profile, MAX_CODEC_OFFERS,
};
use super::pixel_convert::FrameSize;
use super::wire_binary::GopHeader;

#[path = "codec_registry_continue.rs"]
mod continuation;
#[path = "codec_registry_open.rs"]
mod open;
#[path = "codec_registry_verify.rs"]
mod verify;
pub use verify::VerifyLine;
#[path = "codec_registry_rank.rs"]
mod rank;
#[path = "codec_registry_entries.rs"]
mod rows;
pub use rank::{first_rank_trust, keeps_first_rank};
use rows::{entries, tiles_entry};

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
    /// The adapter it runs on as the verdicts name it
    /// (`vendor:device:revision@driver`, P3); empty for software and where
    /// the platform does not say.
    fn device(&self) -> String {
        String::new()
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
    /// The adapter of its hardware stage as the verdicts name it (P3).
    fn device(&self) -> String {
        String::new()
    }
    /// Whether it would open `header`'s stream only with stages it gave up
    /// in this viewer session (FFmpeg's last resort under a chosen
    /// „Dekodierer: Hardware“). The register tries it after every other
    /// factory of the pass (review 2026-11-19, Codex 4/11).
    fn retries_given_up(&self, _header: &GopHeader) -> bool {
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
    /// The run-time check (P3); absent for the tiles and without checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<VerdictView>,
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
    /// The run-time checks (P3); `None` offers everything, as before.
    checks: Option<Arc<Checks>>,
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

/// The codecs that keep a decoder without a failed check (`judged`: each
/// factory's offers and whether its check failed) of a kind `preference`
/// allows. Fix of 2026-11-18: a stored decoder verdict comes from the round
/// trip of this device's own encoder (`codec_verify.rs`), another stream;
/// where the chosen „Dekodierer“/„Dekodierer-Bibliothek“ leave no other
/// decoder of the codec, the failed one stays offered and the session's
/// guard judges it on the real stream, instead of the codec silently
/// leaving the hello (with only the tiles left under „Automatisch“).
fn sound_codecs(
    judged: &[Vec<(CodecOffer, bool)>],
    preference: super::negotiate::DecoderPreference,
) -> Vec<CodecKind> {
    let mut sound: Vec<CodecKind> = Vec::new();
    for (offer, failed) in judged.iter().flatten() {
        if !failed && preference.allows(offer.hardware) && !sound.contains(&offer.codec) {
            sound.push(offer.codec);
        }
    }
    sound
}

#[cfg(test)]
#[path = "codec_registry_tests.rs"]
mod tests;

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
        Self {
            encoders,
            decoders,
            checks: None,
        }
    }

    /// The same register with the run-time checks of P3.
    pub fn with_checks(mut self, checks: Arc<Checks>) -> Self {
        self.checks = Some(checks);
        self
    }

    pub fn checks(&self) -> Option<&Arc<Checks>> {
        self.checks.as_ref()
    }

    /// Whether `other` holds the same factories (the kept register of a
    /// process, P3 task tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn same_factories(&self, other: &Self) -> bool {
        self.encoders.len() == other.encoders.len()
            && self.decoders.len() == other.decoders.len()
            && self
                .encoders
                .iter()
                .zip(&other.encoders)
                .all(|(a, b)| Arc::ptr_eq(a, b))
            && self
                .decoders
                .iter()
                .zip(&other.decoders)
                .all(|(a, b)| Arc::ptr_eq(a, b))
    }

    /// The host's encoders for `HELLO_OK` and the negotiation.
    pub fn encoder_offers(&self) -> Vec<CodecOffer> {
        ordered_offers(
            self.encoders
                .iter()
                .map(|factory| self.offered(factory.as_ref())),
        )
    }

    /// P3: the offers of `factory` without those that failed their check;
    /// HEVC and AV1 only once checked.
    fn offered(&self, factory: &dyn EncoderFactory) -> Vec<CodecOffer> {
        let offers = factory.offers();
        let Some(checks) = &self.checks else {
            return offers;
        };
        let (name, device) = (factory.name(), factory.device());
        offers
            .into_iter()
            .filter(|offer| {
                if !offer.codec.is_video() {
                    return true;
                }
                let profile = Some(offer.codec.profile_name(offer.profile));
                let summary =
                    checks
                        .store()
                        .summary(&name, &device, offer.codec, profile, Role::Encoder);
                if summary.failed.is_some() && !summary.passes() {
                    return false;
                }
                summary.passes() || offer.codec == CodecKind::H264
            })
            .collect()
    }

    /// Whether `factory` failed its check as a decoder of `codec` (P3).
    fn decoder_failed(&self, factory: &dyn DecoderFactory, codec: CodecKind) -> bool {
        self.checks
            .as_ref()
            .is_some_and(|checks| checks.decoder_failed(factory, codec))
    }

    /// The viewer's decoders for its hello, without the decoders that failed
    /// their check (P3) while another decoder of their codec remains under
    /// the process's „Dekodierer“ (`sound_codecs`). A codec given up in a
    /// session stays out of that session only (`decoder_health::GivenUp`,
    /// fix of 2026-11-18: it was out of every later session of the process).
    pub fn decoder_offers(&self) -> Vec<CodecOffer> {
        let judged: Vec<Vec<(CodecOffer, bool)>> = self
            .decoders
            .iter()
            .map(|factory| {
                factory
                    .offers()
                    .into_iter()
                    .map(|offer| {
                        let failed = offer.codec.is_video()
                            && self.decoder_failed(factory.as_ref(), offer.codec);
                        (offer, failed)
                    })
                    .collect()
            })
            .collect();
        let sound = sound_codecs(&judged, super::negotiate::decoder_preference());
        ordered_offers(judged.into_iter().map(|offers| {
            offers
                .into_iter()
                .filter(|(offer, failed)| !failed || !sound.contains(&offer.codec))
                .map(|(offer, _)| offer.with_pixel_access(true))
                .collect()
        }))
    }

    /// D6: the newer codecs of which an encoder of this register passed
    /// its round trip with a local decoder, without AV1 (`first_rank_trust`);
    /// every codec without checks.
    pub fn codec_trust(&self) -> CodecTrust {
        let Some(checks) = &self.checks else {
            return CodecTrust::ALL;
        };
        let mut verified = Vec::new();
        for factory in &self.encoders {
            let (name, device) = (factory.name(), factory.device());
            for codec in [CodecKind::Hevc, CodecKind::Av1] {
                let summary = checks
                    .store()
                    .summary(&name, &device, codec, None, Role::Encoder);
                if summary.verified {
                    verified.push(codec);
                }
            }
        }
        first_rank_trust(verified)
    }

    /// The verdict of every row of `factory` (`name`, `device`) as `role`.
    fn judged(&self, mut rows: Vec<CodecEntry>, device: &str, role: Role) -> Vec<CodecEntry> {
        if let Some(checks) = &self.checks {
            for row in rows.iter_mut().filter(|row| row.codec.is_video()) {
                let summary = checks
                    .store()
                    .summary(&row.name, device, row.codec, None, role);
                row.verdict = Some(VerdictView::of(summary));
            }
        }
        rows
    }

    /// The register as `se screen codec` shows it.
    pub fn report(&self, openh264: OpenH264Report) -> CodecReport {
        let encoders = self.encoders.iter().flat_map(|factory| {
            let rows = entries(
                factory.name(),
                &factory.offers(),
                factory.refused_settings(),
            );
            self.judged(rows, &factory.device(), Role::Encoder)
        });
        let decoders = self.decoders.iter().flat_map(|factory| {
            let rows = entries(factory.name(), &factory.offers(), Vec::new());
            self.judged(rows, &factory.device(), Role::Decoder)
        });
        CodecReport {
            encoders: encoders.chain(std::iter::once(tiles_entry())).collect(),
            decoders: decoders.chain(std::iter::once(tiles_entry())).collect(),
            openh264,
        }
    }
}
