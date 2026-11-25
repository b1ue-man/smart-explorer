//! Settings of the desktop viewer (SCR-M7, U25-6) in
//! `<app data>/screen/viewer.json`: „Niedrigste Latenz“, the fit, „Höchste
//! Datenrate“ (plan 2026-11-14 „Bildkette“ B-09), „Codec“ and „Dekodierer“
//! (BK6: „Codec: Verlustfrei“ replaces the switch „Verlustfrei“ with its
//! meaning and takes over an older file's `lossless`), „Dekodierer-
//! Bibliothek“, „Anzeige“, „Monitor“ and „Auflösung“ (plan 2026-11-17 P5;
//! never chosen: „Automatisch“, following the window), „Übertragung“ (user
//! 2026-11-25, `screen_delivery.rs`). Read at the start (a
//! missing or unreadable file means the defaults, without a message); the
//! setters (`set_max_rate_bps`, `set_codec_choice`, also `se screen view`)
//! store at once, `ViewerPrefs::save` writes only what this process changed.
//! Full screen is not stored. What the lists offer: `codec_offer`.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

pub use crate::screen::core::decoder_library::DecoderLibrary;
pub use crate::screen::core::negotiate::{CodecSetting, DecoderPreference};
use crate::screen::core::pixel_convert::FrameSize;
pub use crate::screen::core::virtual_display::{VirtualDisplayChoice, VirtualDisplayWish};

const SCHEMA: u32 = 1;

/// How the picture is laid into the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// As large as the window allows, with the aspect ratio kept.
    #[default]
    Fit,
    /// One picture pixel per screen pixel.
    Actual,
}

/// „Anzeige“ (plan 2026-11-17 P8): Direct3D 11 for FFmpeg's D3D11VA pictures
/// on Windows, wgpu/glow for every other picture and wherever Direct3D 11
/// does not open (Automatic); „wgpu“ keeps every picture on that path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayChoice {
    #[default]
    #[serde(rename = "auto")]
    Automatic,
    Direct3d11,
    Wgpu,
}

impl DisplayChoice {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::Direct3d11, Self::Wgpu];

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.key() == text)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Direct3d11 => "direct3d11",
            Self::Wgpu => "wgpu",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatisch",
            Self::Direct3d11 => "Direct3D 11",
            Self::Wgpu => "wgpu",
        }
    }

    /// Whether GPU pictures may go through the Direct3D 11 presenter.
    pub fn direct(self) -> bool {
        self != Self::Wgpu
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerPrefs {
    #[serde(default = "schema")]
    pub schema: u32,
    /// `AutoNoVsync` at the next start of the viewer (tearing possible).
    #[serde(default)]
    pub low_latency: bool,
    /// „Schärfe vor Flüssigkeit“ of older viewers; follows „Codec: Scharf
    /// (GPU)“ (P9), read as that codec when no codec is stored.
    #[serde(default)]
    pub sharpness: bool,
    /// „Verlustfrei“ (the tile codec).
    #[serde(default)]
    pub lossless: bool,
    #[serde(default)]
    pub fit: Fit,
    /// „Höchste Datenrate“ in bit/s; absent is „Automatisch“.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rate_bps: Option<u64>,
    /// „Codec“ (plan 2026-11-14 „Bildkette“ 4.10, BK6); absent: a file of an
    /// older viewer, whose switch `lossless` it takes over (`codec_setting`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<CodecSetting>,
    /// „Dekodierer“ (4.10, BK6); absent is „Automatisch“.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder: Option<DecoderPreference>,
    /// „Dekodierer-Bibliothek“ (P8); absent is „Automatisch“.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder_library: Option<DecoderLibrary>,
    /// „Anzeige“ (P8); absent is „Automatisch“.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayChoice>,
    /// „Monitor“ and „Auflösung“ (P5); absent: „Automatisch“ (the window).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<VirtualDisplayChoice>,
    /// „Ton“ (plan 2026-11-17 P9, `screen_audio.rs`); absent is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    /// „Übertragung“ (user 2026-11-25, `screen_delivery.rs`); absent is
    /// „Echtzeit (wie UDP)“. An older viewer reads the file without it and
    /// leaves it out when it writes the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<crate::screen::core::delivery::Delivery>,
}

fn schema() -> u32 {
    SCHEMA
}

impl Default for ViewerPrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            low_latency: false,
            sharpness: false,
            lossless: false,
            fit: Fit::Fit,
            max_rate_bps: None,
            codec: None,
            decoder: None,
            decoder_library: None,
            display: None,
            virtual_display: None,
            audio: None,
            delivery: None,
        }
    }
}

/// `<app data>/screen/viewer.json`.
pub fn prefs_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("viewer.json")
}

/// Changes the stored settings under the lock all processes share.
pub fn update_stored(change: impl FnOnce(&mut ViewerPrefs)) -> io::Result<()> {
    store::update(&prefs_path(), change)
}

/// The settings this process loaded at its start, or saved last.
static LOADED: Mutex<Option<ViewerPrefs>> = Mutex::new(None);

/// „Höchste Datenrate“ of this process: unread (`None`) or the value.
static MAX_RATE: Mutex<Option<Option<u64>>> = Mutex::new(None);

/// The stored „Höchste Datenrate“ (`None`: automatic), read once.
pub fn max_rate_bps() -> Option<u64> {
    let mut known = MAX_RATE.lock().unwrap_or_else(PoisonError::into_inner);
    *known.get_or_insert_with(|| current().max_rate_bps)
}

/// Stores „Höchste Datenrate“ (`None`: automatic) at once.
pub fn set_max_rate_bps(rate: Option<u64>) -> io::Result<()> {
    *MAX_RATE.lock().unwrap_or_else(PoisonError::into_inner) = Some(rate);
    store::update(&prefs_path(), |prefs| prefs.max_rate_bps = rate)
}

/// „Codec“ and „Dekodierer“ of this process: unread (`None`) or the values.
type CodecChoice = (Option<CodecSetting>, Option<DecoderPreference>);
static CODEC: Mutex<Option<CodecChoice>> = Mutex::new(None);

fn codec_choice() -> CodecChoice {
    let mut known = CODEC.lock().unwrap_or_else(PoisonError::into_inner);
    *known.get_or_insert_with(|| {
        let prefs = current();
        (Some(prefs.codec_setting()), prefs.decoder)
    })
}

/// The stored „Codec“ (an older file's `lossless` taken over).
pub fn codec_setting() -> CodecSetting {
    codec_choice().0.unwrap_or_default()
}

/// The stored „Dekodierer“.
pub fn decoder_choice() -> DecoderPreference {
    codec_choice().1.unwrap_or_default()
}

/// Stores „Codec“ and „Dekodierer“ at once (`None` keeps that value, the
/// file's own for the file, Codex 8/12) and makes the decoder choice the
/// one of this process's decoders.
pub fn set_codec_choice(
    codec: Option<CodecSetting>,
    decoder: Option<DecoderPreference>,
) -> io::Result<()> {
    let (old_codec, old_decoder) = codec_choice();
    let chosen = (codec.or(old_codec), decoder.or(old_decoder));
    *CODEC.lock().unwrap_or_else(PoisonError::into_inner) = Some(chosen);
    crate::screen::core::negotiate::set_decoder_preference(chosen.1.unwrap_or_default());
    store::update(&prefs_path(), |prefs| {
        let stored_decoder = prefs.decoder;
        prefs.apply_codec((codec, decoder.or(stored_decoder)));
    })
}

/// „Dekodierer-Bibliothek“ and „Anzeige“ (P8): unread (`None`) or the values.
type OutputChoice = (Option<DecoderLibrary>, Option<DisplayChoice>);
static OUTPUT: Mutex<Option<OutputChoice>> = Mutex::new(None);

fn output_choice() -> OutputChoice {
    let mut known = OUTPUT.lock().unwrap_or_else(PoisonError::into_inner);
    *known.get_or_insert_with(|| {
        let prefs = current();
        (prefs.decoder_library, prefs.display)
    })
}

/// „Dekodierer-Bibliothek“ in force (`codec_offer::effective_library`).
pub fn decoder_library_choice() -> DecoderLibrary {
    codec_offer::effective_library(stored_output().0)
}

/// „Anzeige“ in force (`codec_offer::effective_display`).
pub fn display_choice() -> DisplayChoice {
    codec_offer::effective_display(stored_output().1)
}

/// The stored „Dekodierer-Bibliothek“ and „Anzeige“.
pub fn stored_output() -> (DecoderLibrary, DisplayChoice) {
    let (library, display) = output_choice();
    (library.unwrap_or_default(), display.unwrap_or_default())
}

/// Stores „Dekodierer-Bibliothek“ and „Anzeige“ at once (`None` keeps that
/// value, the file's own for the file, Codex 8/12) and makes the library
/// the one of this process's decoders.
pub fn set_output_choice(
    library: Option<DecoderLibrary>,
    display: Option<DisplayChoice>,
) -> io::Result<()> {
    let (old_library, old_display) = output_choice();
    let chosen = (library.or(old_library), display.or(old_display));
    *OUTPUT.lock().unwrap_or_else(PoisonError::into_inner) = Some(chosen);
    crate::screen::core::decoder_library::set_decoder_library(chosen.0.unwrap_or_default());
    store::update(&prefs_path(), |prefs| {
        prefs.decoder_library = library.or(prefs.decoder_library);
        prefs.display = display.or(prefs.display);
    })
}

/// „Monitor“ and „Auflösung“ of this process: unread (`None`) or the value.
static VIRTUAL_DISPLAY: Mutex<Option<Option<VirtualDisplayChoice>>> = Mutex::new(None);

/// The stored „Monitor“ and „Auflösung“; never chosen is „Automatisch“
/// following the window (`VirtualDisplayChoice::default`).
pub fn virtual_display_choice() -> VirtualDisplayChoice {
    let mut known = VIRTUAL_DISPLAY
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let stored = *known.get_or_insert_with(|| current().virtual_display);
    stored.unwrap_or_default()
}

/// Stores „Monitor“ (`None` keeps it) and „Auflösung“ (`None` keeps it,
/// `Some(None)` follows the window) at once; the file keeps its own value
/// of a part left out (Codex 8/12).
pub fn set_virtual_display_choice(
    wish: Option<VirtualDisplayWish>,
    size: Option<Option<FrameSize>>,
) -> io::Result<()> {
    let mut chosen = virtual_display_choice();
    if let Some(wish) = wish {
        chosen.wish = wish;
    }
    if let Some(size) = size {
        chosen.size = size;
    }
    chosen.validate()?;
    *VIRTUAL_DISPLAY
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(Some(chosen));
    store::update(&prefs_path(), |prefs| {
        let mut stored = prefs.virtual_display.unwrap_or_default();
        if let Some(wish) = wish {
            stored.wish = wish;
        }
        if let Some(size) = size {
            stored.size = size;
        }
        // A stored part left invalid with this one is not written.
        let valid = stored.validate().is_ok();
        prefs.virtual_display = Some(if valid { stored } else { chosen });
    })
}

impl ViewerPrefs {
    /// „Codec“; a file of a viewer before BK6 has only `lossless`.
    pub fn codec_setting(&self) -> CodecSetting {
        // P9: a stored switch „Schärfe vor Flüssigkeit“ is „Scharf (GPU)“.
        self.codec
            .unwrap_or_else(|| match (self.lossless, self.sharpness) {
                (false, true) => CodecSetting::Sharp,
                (lossless, _) => CodecSetting::from_lossless(lossless),
            })
    }

    /// Writes „Codec“ and „Dekodierer“; `lossless` follows „Verlustfrei“, so
    /// an older viewer reading the file keeps its switch (4.10).
    fn apply_codec(&mut self, (codec, decoder): CodecChoice) {
        if let Some(codec) = codec {
            self.codec = Some(codec);
            self.lossless = codec.lossless();
            self.sharpness = codec.sharpness();
        }
        self.decoder = decoder;
    }

    /// The stored settings, or the defaults; they are what this process
    /// started from (`save` writes only what it changed since).
    pub fn load() -> Self {
        let prefs = current();
        *LOADED.lock().unwrap_or_else(PoisonError::into_inner) = Some(prefs.clone());
        prefs
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
    }

    /// Stores the settings this process changed (user rule 2026-11-18):
    /// only the fields that differ from what it loaded go over the file, so
    /// a value `se screen view` or the MCP tool changed meanwhile stays; the
    /// rest is written by its own setter at once.
    pub fn save(&self) -> io::Result<()> {
        let mut loaded = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
        let base = loaded.clone().unwrap_or_default();
        self.save_changes_to(&base, &prefs_path())?;
        *loaded = Some(self.clone());
        Ok(())
    }

    /// Writes the fields of `self` that differ from `base` over the file.
    pub fn save_changes_to(&self, base: &Self, path: &Path) -> io::Result<()> {
        store::update(path, |stored| self.changes_onto(base, stored))
    }

    fn changes_onto(&self, base: &Self, stored: &mut Self) {
        macro_rules! changed {
            ($($field:ident),*) => {$(
                if self.$field != base.$field {
                    stored.$field = self.$field;
                }
            )*};
        }
        changed!(
            low_latency,
            sharpness,
            lossless,
            fit,
            max_rate_bps,
            codec,
            decoder,
            decoder_library,
            display,
            virtual_display,
            audio,
            delivery
        );
        stored.schema = SCHEMA;
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        crate::support_dirs::write_private_atomic(path, &bytes)
    }
}

#[cfg(test)]
#[path = "screen_prefs_codec_tests.rs"]
mod bildkette_codec_tests;

#[path = "screen_codec_offer.rs"]
pub mod codec_offer;
#[path = "screen_prefs_start.rs"]
mod start;
#[path = "screen_prefs_store.rs"]
mod store;
pub use start::{current, start_choices};

/// `start::hold_start_choices`; the target's device is the full view's.
pub fn hold_start_choices(target_json: &str) {
    start::hold_start_choices(target_json);
    codec_offer::start_target(target_json);
}
#[cfg(test)]
#[path = "wahlgilt_prefs_task_tests.rs"]
mod wahlgilt_prefs_task_tests;
