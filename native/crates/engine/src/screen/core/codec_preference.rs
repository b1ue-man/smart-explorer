//! What the user chooses of the codecs (plan 2026-11-14 „Bildkette“ 4.10):
//! „Codec: Automatisch / H.264 / HEVC / AV1 / Verlustfrei“ (`CodecSetting`,
//! it replaces the switch „Verlustfrei“ and keeps its meaning: the lossless
//! tile codec through `SET_MODE`), „Dekodierer: Automatisch / Hardware /
//! Software“ (`DecoderPreference`, default automatic with fallback on
//! failure), and the negotiation filter both map to (`CodecPreference`,
//! also `se screen bench --codec`).
//!
//! The decoder choice of this process lives in one place
//! (`set_decoder_preference`): the desktop viewer, the Android viewer and
//! `se screen view` each set it before they open decoders, and every
//! decoder factory of the register reads it when it opens.
//!
//! Plan 2026-11-17 P9 („Automatisch“ took software although hardware
//! decodes the stream in 0.4 ms instead of 10 ms): the register opens in
//! passes (`DecoderPreference::passes`). „Automatisch“ tries every hardware
//! decoder of every library first (FFmpeg D3D11VA/DXVA2, then Media
//! Foundation hardware) and only then any software decoder; before, the
//! first library's software stage came before the second library's
//! hardware. During a pass the factories read the pass as the choice
//! (`with_decoder_pass`, per thread: the decode thread opens).
use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

use super::super::sharp_mode;
use super::{CodecKind, CodecOffer};

/// `--codec` of `se screen bench` and the filter of the negotiation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CodecPreference {
    #[default]
    Auto,
    H264,
    Hevc,
    Av1,
    /// „Scharf (GPU)“ (P9 Zwischenmodus, `sharp_mode.rs`): H.264 and HEVC
    /// on the GPU encoder, never AV1; the tiles stay the last fallback.
    Sharp,
    Tiles,
}

impl CodecPreference {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "h264" => Some(Self::H264),
            "hevc" => Some(Self::Hevc),
            "av1" => Some(Self::Av1),
            sharp_mode::SHARP_KEY => Some(Self::Sharp),
            "tiles" => Some(Self::Tiles),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
            Self::Sharp => sharp_mode::SHARP_KEY,
            Self::Tiles => "tiles",
        }
    }

    /// The one video codec this preference names.
    pub fn codec(self) -> Option<CodecKind> {
        match self {
            Self::H264 => Some(CodecKind::H264),
            Self::Hevc => Some(CodecKind::Hevc),
            Self::Av1 => Some(CodecKind::Av1),
            Self::Auto | Self::Sharp | Self::Tiles => None,
        }
    }

    /// Whether a pairing of `codec` may come out of the negotiation: a named
    /// video codec excludes the others and the tiles (`se screen bench`).
    pub fn admits(self, codec: CodecKind) -> bool {
        match (self, codec) {
            (Self::Auto, _) => true,
            (Self::Sharp, kind) => sharp_mode::sharp_codec(kind) || kind == CodecKind::Tiles,
            (Self::Tiles, kind) => kind == CodecKind::Tiles,
            (named, kind) => named.codec() == Some(kind),
        }
    }

    /// The viewer's decoder offers for this choice: the named codec and the
    /// tiles (every peer's last fallback stays), only the tiles, or all.
    pub fn filter_decoders(self, offers: &[CodecOffer]) -> Vec<CodecOffer> {
        if self == Self::Sharp {
            return sharp_mode::sharp_decoders(offers);
        }
        offers
            .iter()
            .filter(|offer| offer.codec == CodecKind::Tiles || self.admits(offer.codec))
            .copied()
            .collect()
    }
}

/// „Codec“ of the viewer (4.10); stored, sent through IPC and the CLI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecSetting {
    #[default]
    Auto,
    H264,
    Hevc,
    Av1,
    /// „Scharf (GPU)“: `sharp_mode.rs`, sent as `SET_MODE` `sharpness`.
    Sharp,
    /// „Verlustfrei“: the tile codec through `SET_MODE` `lossless`.
    Lossless,
}

impl CodecSetting {
    pub const ALL: [Self; 6] = [
        Self::Auto,
        Self::H264,
        Self::Hevc,
        Self::Av1,
        Self::Sharp,
        Self::Lossless,
    ];

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|setting| setting.key() == text)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
            Self::Sharp => sharp_mode::SHARP_KEY,
            Self::Lossless => "lossless",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatisch",
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
            Self::Av1 => "AV1",
            Self::Sharp => sharp_mode::SHARP_LABEL,
            Self::Lossless => "Verlustfrei",
        }
    }

    /// The setting of a viewer that stored only the switch „Verlustfrei“.
    pub fn from_lossless(lossless: bool) -> Self {
        if lossless {
            Self::Lossless
        } else {
            Self::Auto
        }
    }

    /// `SET_MODE` `lossless`: unchanged meaning of the old switch.
    pub fn lossless(self) -> bool {
        self == Self::Lossless
    }

    /// `SET_MODE` `sharpness`: „Scharf (GPU)“ (it replaces the switch
    /// „Schärfe vor Flüssigkeit“, which set the same bit).
    pub fn sharpness(self) -> bool {
        self == Self::Sharp
    }

    /// The negotiation filter: „Verlustfrei“ negotiates like „Automatisch“
    /// and switches to the tiles at run time, as the old switch did.
    pub fn preference(self) -> CodecPreference {
        match self {
            Self::Auto | Self::Lossless => CodecPreference::Auto,
            Self::H264 => CodecPreference::H264,
            Self::Hevc => CodecPreference::Hevc,
            Self::Av1 => CodecPreference::Av1,
            Self::Sharp => CodecPreference::Sharp,
        }
    }
}

/// „Dekodierer“ of the viewer (4.10, the stages of 4.10a).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecoderPreference {
    /// Hardware first, software when it fails (4.10a).
    #[default]
    #[serde(rename = "auto")]
    Automatic,
    Hardware,
    Software,
}

impl DecoderPreference {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::Hardware, Self::Software];

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.key() == text)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Hardware => "hardware",
            Self::Software => "software",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatisch",
            Self::Hardware => "Hardware",
            Self::Software => "Software",
        }
    }

    /// Whether a decoder stage that runs in `hardware` may be used.
    pub fn allows(self, hardware: bool) -> bool {
        match self {
            Self::Automatic => true,
            Self::Hardware => hardware,
            Self::Software => !hardware,
        }
    }

    /// The decoder offers of the hello under this choice; the tiles stay.
    /// A decoder that runs in both stages lists an offer for each.
    pub fn filter_offers(self, offers: &[CodecOffer]) -> Vec<CodecOffer> {
        offers
            .iter()
            .filter(|offer| offer.codec == CodecKind::Tiles || self.allows(offer.hardware))
            .copied()
            .collect()
    }

    /// The passes the register opens a decoder in, in order (P9):
    /// „Automatisch“ every hardware stage of every decoder before any
    /// software stage; „Hardware“ and „Software“ one pass of their kind.
    pub fn passes(self) -> &'static [DecoderPreference] {
        match self {
            Self::Automatic => &[Self::Hardware, Self::Software],
            Self::Hardware => &[Self::Hardware],
            Self::Software => &[Self::Software],
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Automatic => 0,
            Self::Hardware => 1,
            Self::Software => 2,
        }
    }
}

thread_local! {
    /// The pass the register runs on this thread (`with_decoder_pass`).
    static PASS: Cell<Option<DecoderPreference>> = const { Cell::new(None) };
}

/// Restores the pass that ran before, also when `open` panics.
struct PassGuard(Option<DecoderPreference>);

impl Drop for PassGuard {
    fn drop(&mut self) {
        PASS.with(|pass| pass.set(self.0));
    }
}

/// Runs `open` with `pass` as this thread's decoder choice: the factories
/// a pass of the register opens read it through `decoder_preference`.
pub fn with_decoder_pass<T>(pass: DecoderPreference, open: impl FnOnce() -> T) -> T {
    let _restore = PassGuard(PASS.with(|current| current.replace(Some(pass))));
    open()
}

static DECODER_PREFERENCE: AtomicU8 = AtomicU8::new(0);

/// Sets the decoder choice of this process (viewer start, a change).
pub fn set_decoder_preference(preference: DecoderPreference) {
    DECODER_PREFERENCE.store(preference.code(), Ordering::Release);
}

/// The decoder choice of this process; automatic until one is set. Inside
/// `with_decoder_pass` it is that pass.
pub fn decoder_preference() -> DecoderPreference {
    if let Some(pass) = PASS.with(Cell::get) {
        return pass;
    }
    match DECODER_PREFERENCE.load(Ordering::Acquire) {
        1 => DecoderPreference::Hardware,
        2 => DecoderPreference::Software,
        _ => DecoderPreference::Automatic,
    }
}

#[cfg(test)]
#[path = "p9_decoder_passes_task_tests.rs"]
mod p9_decoder_passes_task_tests;
