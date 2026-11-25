//! The settings one `se screen view` call opens its window with (Codex
//! 10/14): the call stores them in `viewer.json` and also hands them to its
//! own window in the start request (`choices` beside the target), so a
//! second call that stores other ones before this window read the file
//! never decides for it. Without them (a window the app opens itself) the
//! file decides as before.
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use super::{
    prefs_path, CodecSetting, DecoderLibrary, DecoderPreference, DisplayChoice, ViewerPrefs,
    VirtualDisplayChoice,
};

/// The values a window started by the CLI uses in place of the file's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartChoices {
    #[serde(default)]
    codec: Option<CodecSetting>,
    #[serde(default)]
    decoder: Option<DecoderPreference>,
    #[serde(default)]
    decoder_library: Option<DecoderLibrary>,
    #[serde(default)]
    display: Option<DisplayChoice>,
    /// `None`: automatic (the snapshot always names it).
    #[serde(default)]
    max_rate_bps: Option<u64>,
    #[serde(default)]
    virtual_display: Option<VirtualDisplayChoice>,
    #[serde(default)]
    audio: Option<bool>,
    /// „Übertragung“ (`screen_delivery.rs`).
    #[serde(default)]
    delivery: Option<crate::screen::core::delivery::Delivery>,
}

/// This process's start choices, when its start request named them.
static START: Mutex<Option<StartChoices>> = Mutex::new(None);

/// The calling process's settings now (`se screen view` after it stored
/// the given ones), as the `choices` of its window's start request.
pub fn start_choices() -> serde_json::Value {
    let choices = StartChoices {
        codec: Some(super::codec_setting()),
        decoder: Some(super::decoder_choice()),
        decoder_library: Some(super::decoder_library_choice()),
        display: Some(super::display_choice()),
        max_rate_bps: super::max_rate_bps(),
        virtual_display: Some(super::virtual_display_choice()),
        audio: super::super::screen_audio::stored(),
        delivery: super::super::screen_delivery::stored(),
    };
    serde_json::to_value(choices).unwrap_or(serde_json::Value::Null)
}

/// Takes the `choices` of the start request `target_json` for this process,
/// before any setting is read.
pub fn hold_start_choices(target_json: &str) {
    let choices = serde_json::from_str::<serde_json::Value>(target_json)
        .ok()
        .and_then(|target| target.get("choices").cloned())
        .and_then(|choices| serde_json::from_value::<StartChoices>(choices).ok());
    *START.lock().unwrap_or_else(PoisonError::into_inner) = choices;
}

/// The stored settings with this process's start choices over them: what
/// every setting of the process is first read from.
pub fn current() -> ViewerPrefs {
    let mut prefs = ViewerPrefs::load_from(&prefs_path());
    let start = START.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(start) = start {
        if start.codec.is_some() || start.decoder.is_some() {
            prefs.apply_codec((start.codec, start.decoder.or(prefs.decoder)));
        }
        prefs.decoder_library = start.decoder_library.or(prefs.decoder_library);
        prefs.display = start.display.or(prefs.display);
        prefs.max_rate_bps = start.max_rate_bps;
        prefs.virtual_display = start.virtual_display.or(prefs.virtual_display);
        prefs.audio = start.audio.or(prefs.audio);
        prefs.delivery = start.delivery.or(prefs.delivery);
    }
    prefs
}
