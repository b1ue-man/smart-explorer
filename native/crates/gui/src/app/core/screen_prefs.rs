//! Settings of the desktop viewer (SCR-M7, U25-6): „Niedrigste Latenz“,
//! „Schärfe vor Flüssigkeit“, „Verlustfrei“, the fit (Einpassen/1:1) and
//! „Höchste Datenrate“ (plan 2026-11-14 „Bildkette“ B-09) in
//! `<app data>/screen/viewer.json`. Read at the start (a missing or
//! unreadable file means the defaults, without a message), written
//! atomically at every change by the user. Full screen is not stored.
//!
//! „Höchste Datenrate“ is changed by the toolbar and by `se screen view
//! --max-rate` directly (`set_max_rate_bps`); the value this process last
//! set or read is the one every later `save` writes, so a save of the other
//! settings never takes an older value back. „Codec“ and „Dekodierer“ (BK6)
//! work the same way (`set_codec_choice`, `se screen view --codec
//! --decoder`); „Codec: Verlustfrei“ replaces the switch „Verlustfrei“ with
//! its meaning, and a stored `lossless` of an older viewer becomes it.
//! „Monitor“ and „Auflösung“ (plan 2026-11-17 P5, `se screen view
//! --virtual-display --display-size`) as well (`set_virtual_display_choice`);
//! never chosen means „Automatisch“, following the window.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerPrefs {
    #[serde(default = "schema")]
    pub schema: u32,
    /// `AutoNoVsync` at the next start of the viewer (tearing possible).
    #[serde(default)]
    pub low_latency: bool,
    /// „Schärfe vor Flüssigkeit“.
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
    /// „Monitor“ and „Auflösung“ (P5); absent is „Automatisch“ following
    /// the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<VirtualDisplayChoice>,
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
            virtual_display: None,
        }
    }
}

/// `<app data>/screen/viewer.json`.
pub fn prefs_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("viewer.json")
}

/// „Höchste Datenrate“ of this process: unread (`None`) or the value.
static MAX_RATE: Mutex<Option<Option<u64>>> = Mutex::new(None);

/// The stored „Höchste Datenrate“ (`None`: automatic), read once.
pub fn max_rate_bps() -> Option<u64> {
    let mut known = MAX_RATE.lock().unwrap_or_else(PoisonError::into_inner);
    *known.get_or_insert_with(|| ViewerPrefs::load_from(&prefs_path()).max_rate_bps)
}

/// Stores „Höchste Datenrate“ (`None`: automatic) at once.
pub fn set_max_rate_bps(rate: Option<u64>) -> io::Result<()> {
    *MAX_RATE.lock().unwrap_or_else(PoisonError::into_inner) = Some(rate);
    let mut prefs = ViewerPrefs::load_from(&prefs_path());
    prefs.max_rate_bps = rate;
    prefs.save_to(&prefs_path())
}

/// „Codec“ and „Dekodierer“ of this process: unread (`None`) or the values.
type CodecChoice = (Option<CodecSetting>, Option<DecoderPreference>);
static CODEC: Mutex<Option<CodecChoice>> = Mutex::new(None);

fn codec_choice() -> CodecChoice {
    let mut known = CODEC.lock().unwrap_or_else(PoisonError::into_inner);
    *known.get_or_insert_with(|| {
        let prefs = ViewerPrefs::load_from(&prefs_path());
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

/// Stores „Codec“ and „Dekodierer“ at once (`None` keeps that value) and
/// makes the decoder choice the one of this process's decoders.
pub fn set_codec_choice(
    codec: Option<CodecSetting>,
    decoder: Option<DecoderPreference>,
) -> io::Result<()> {
    let (old_codec, old_decoder) = codec_choice();
    let chosen = (codec.or(old_codec), decoder.or(old_decoder));
    *CODEC.lock().unwrap_or_else(PoisonError::into_inner) = Some(chosen);
    crate::screen::core::negotiate::set_decoder_preference(chosen.1.unwrap_or_default());
    let mut prefs = ViewerPrefs::load_from(&prefs_path());
    prefs.apply_codec(chosen);
    prefs.save_to(&prefs_path())
}

/// „Monitor“ and „Auflösung“ of this process: unread (`None`) or the value.
static VIRTUAL_DISPLAY: Mutex<Option<Option<VirtualDisplayChoice>>> = Mutex::new(None);

/// The stored „Monitor“ and „Auflösung“; never chosen is „Automatisch“
/// following the window (`VirtualDisplayChoice::default`).
pub fn virtual_display_choice() -> VirtualDisplayChoice {
    let mut known = VIRTUAL_DISPLAY
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let stored =
        *known.get_or_insert_with(|| ViewerPrefs::load_from(&prefs_path()).virtual_display);
    stored.unwrap_or_default()
}

/// Stores „Monitor“ (`None` keeps it) and „Auflösung“ (`None` keeps it,
/// `Some(None)` follows the window) at once.
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
    let mut prefs = ViewerPrefs::load_from(&prefs_path());
    prefs.virtual_display = Some(chosen);
    prefs.save_to(&prefs_path())
}

impl ViewerPrefs {
    /// „Codec“; a file of a viewer before BK6 has only `lossless`.
    pub fn codec_setting(&self) -> CodecSetting {
        self.codec
            .unwrap_or_else(|| CodecSetting::from_lossless(self.lossless))
    }

    /// Writes „Codec“ and „Dekodierer“; `lossless` follows „Verlustfrei“, so
    /// an older viewer reading the file keeps its switch (4.10).
    fn apply_codec(&mut self, (codec, decoder): CodecChoice) {
        if let Some(codec) = codec {
            self.codec = Some(codec);
            self.lossless = codec.lossless();
        }
        self.decoder = decoder;
    }

    /// The stored settings, or the defaults.
    pub fn load() -> Self {
        Self::load_from(&prefs_path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
    }

    /// Stores the settings; „Höchste Datenrate“ is the process's current value.
    pub fn save(&self) -> io::Result<()> {
        let mut stored = self.clone();
        stored.max_rate_bps = max_rate_bps();
        // Like the rate: the codec choice this process knows wins (BK6).
        if let Some(known) = *CODEC.lock().unwrap_or_else(PoisonError::into_inner) {
            stored.apply_codec(known);
        }
        // And „Monitor“ and „Auflösung“ (P5).
        if let Some(known) = *VIRTUAL_DISPLAY
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            stored.virtual_display = known;
        }
        stored.save_to(&prefs_path())
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        crate::support_dirs::write_private_atomic(path, &bytes)
    }
}

#[cfg(test)]
mod bildkette_codec_tests {
    use super::*;
    use crate::screen::core::negotiate::CodecPreference;

    /// 4.10: „Codec“ replaces the switch „Verlustfrei“ without changing its
    /// meaning: a stored `lossless` of an older viewer becomes „Verlustfrei“
    /// (still `SET_MODE` `lossless`, negotiation automatic), and every new
    /// choice writes `lossless` along for an older viewer reading the file.
    #[test]
    fn bildkette_task_codec_lossless_switch_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("viewer.json");
        std::fs::write(&path, br#"{"schema":1,"lossless":true}"#).unwrap();
        let older = ViewerPrefs::load_from(&path);
        assert_eq!(older.codec, None);
        assert_eq!(older.codec_setting(), CodecSetting::Lossless);
        assert_eq!(older.codec_setting().preference(), CodecPreference::Auto);
        let display = crate::screen::core::pixel_convert::FrameSize::new(1920, 1080);
        let mode = super::super::screen_pool::full_set_mode(None, display, &older);
        assert!(mode.lossless, "„Verlustfrei“ still asks for the tiles");
        assert_eq!(ViewerPrefs::default().codec_setting(), CodecSetting::Auto);

        let mut chosen = older.clone();
        chosen.apply_codec((Some(CodecSetting::Hevc), Some(DecoderPreference::Software)));
        assert!(!chosen.lossless);
        chosen.save_to(&path).unwrap();
        let stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["codec"], "hevc");
        assert_eq!(stored["decoder"], "software");
        assert_eq!(stored["lossless"], false);
        assert_eq!(ViewerPrefs::load_from(&path), chosen);
        let mode = super::super::screen_pool::full_set_mode(None, display, &chosen);
        assert!(!mode.lossless);

        chosen.apply_codec((
            Some(CodecSetting::Lossless),
            Some(DecoderPreference::Automatic),
        ));
        assert!(chosen.lossless, "an older viewer reads its switch again");
        assert_eq!(chosen.codec_setting(), CodecSetting::Lossless);
    }
}
