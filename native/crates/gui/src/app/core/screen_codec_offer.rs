//! What the viewer offers in „Codec“, „Dekodierer“, „Dekodier-Bibliothek“
//! and „Anzeige“ (user rule of 2026-11-24: „was du anzeigst MUSS
//! funktionieren. end to fucking end, KEINE EINZIGE ausnahme“). A codec is
//! usable when the host's encoders (`HELLO_OK` `encoders`/`encoders_v2`,
//! the `codecs` of `se screen probe`) pair with the decoders this device
//! names for it (`negotiate_chosen`, the host's own negotiation); every
//! other entry stays in its list, disabled, with its German reason. A
//! stored choice the peer cannot serve is never refused: the session runs
//! with „Automatisch“ in its place (`session_choice`), the stored choice
//! stays for peers that serve it. The host's encoders are learnt per device
//! from every `HELLO_OK` the viewer sees and from a probe (`learn`).
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::super::screen_session::Wake;
use super::super::screen_target::ScreenTarget;
use super::DisplayChoice;
use crate::daemon::StreamRequest;
use crate::screen::core::decoder_library::DecoderLibrary;
use crate::screen::core::negotiate::{
    negotiate_chosen, CodecKind, CodecOffer, CodecSetting, CodecTrust, DecoderPreference,
};
use crate::screen::core::wire_hello::{ScreenHelloOk, SessionMode};

/// How a list entry stands for this pair (reasons in German).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Usable,
    /// Depends on the host's encoders, which are not known yet.
    Unknown(String),
    Blocked(String),
}

impl Verdict {
    /// The reason, `None` for a usable entry.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Usable => None,
            Self::Unknown(text) | Self::Blocked(text) => Some(text),
        }
    }
}

/// This device: its register's offers (before „Codec“ narrows them) and choices.
#[derive(Clone, Copy, Debug)]
pub struct Viewer<'a> {
    pub decoders: &'a [CodecOffer],
    pub decoder: DecoderPreference,
    pub library: DecoderLibrary,
}

const UNKNOWN: &str = "die Kodierer dieses Gastgebers sind noch nicht bekannt";
/// The reason of a library the register found absent.
pub const ABSENT: &str = "auf diesem Gerät nicht vorhanden";

/// The decoders a request under `setting` names (`screen_pool::announced`).
pub fn named_decoders(setting: CodecSetting, viewer: &Viewer) -> Vec<CodecOffer> {
    let mut decoders = viewer.decoders.to_vec();
    let video = decoders.iter().any(|offer| offer.codec.is_video());
    if viewer.library != DecoderLibrary::Automatic && setting != CodecSetting::Lossless && video {
        decoders.retain(|offer| offer.codec.is_video());
    }
    StreamRequest {
        expected_node: None,
        mode: SessionMode::View,
        source: None,
        decoders,
        display_size: None,
        frame_capabilities: None,
        max_fps: 1,
        max_rate_bps: None,
        codec: Some(setting),
        decoder: Some(viewer.decoder),
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        virtual_display: None,
    }
    .chosen_decoders()
}

fn stage(decoder: DecoderPreference) -> String {
    match decoder {
        DecoderPreference::Automatic => String::new(),
        named => format!(" der Art „{}“", named.label()),
    }
}

/// Why this device names no decoder for `setting`.
fn viewer_missing(setting: CodecSetting, decoder: DecoderPreference) -> String {
    let codec = match setting {
        CodecSetting::Lossless | CodecSetting::Auto => String::new(),
        CodecSetting::Sharp => "H.264- oder HEVC-".into(),
        named => format!("{}-", named.label()),
    };
    format!(
        "dieses Gerät hat keinen {codec}Dekodierer{}",
        stage(decoder)
    )
}

/// The host's reason of `negotiate_chosen` in German.
fn host_reason(setting: CodecSetting, reason: &str, decoder: DecoderPreference) -> String {
    match reason {
        "the host has no encoder of this codec" => {
            format!("dieser Gastgeber hat keinen {}-Kodierer", setting.label())
        }
        "the host has no GPU encoder of H.264 or HEVC" => {
            "dieser Gastgeber hat keinen GPU-Kodierer für H.264 oder HEVC".into()
        }
        crate::screen::core::negotiate::EMPTY_OFFER => viewer_missing(setting, decoder),
        _ => format!(
            "kein Kodierer dieses Gastgebers passt zu einem Dekodierer{} dieses Geräts",
            stage(decoder)
        ),
    }
}

/// How „Codec“ `setting` stands between a host with the encoders `host`
/// (`None`: not known) and `viewer`.
pub fn codec_verdict(
    setting: CodecSetting,
    host: Option<&[CodecOffer]>,
    viewer: &Viewer,
) -> Verdict {
    // „Verlustfrei“ always works (2026-11-24, user: „was fällt dir ein die
    // Nutzereinstellung zu verweigern“): every host and viewer has the
    // tiles, and „Dekodierer“ and „Dekodier-Bibliothek“ choose among video
    // decoders only (`StreamRequest::chosen_decoders`).
    if setting == CodecSetting::Lossless {
        return Verdict::Usable;
    }
    // `screen_session::library_refusal_for`: a chosen library offers video.
    if viewer.library != DecoderLibrary::Automatic {
        let library = viewer.library.label();
        if !viewer.decoders.iter().any(|offer| offer.codec.is_video()) {
            return Verdict::Blocked(format!(
                "die Dekodier-Bibliothek „{library}“ bietet keinen Video-Dekodierer"
            ));
        }
    }
    let decoders = named_decoders(setting, viewer);
    if decoders.is_empty() {
        return Verdict::Blocked(viewer_missing(setting, viewer.decoder));
    }
    let preference = setting.preference();
    let tiles = preference.admits(CodecKind::Tiles)
        && decoders.iter().any(|offer| offer.codec == CodecKind::Tiles);
    let Some(host) = host else {
        return match tiles {
            true => Verdict::Usable,
            false => Verdict::Unknown(UNKNOWN.into()),
        };
    };
    match negotiate_chosen(host, &decoders, preference, None, CodecTrust::ALL) {
        Ok(choices) if !choices.is_empty() => Verdict::Usable,
        Ok(_) => Verdict::Blocked(host_reason(setting, "", viewer.decoder)),
        Err(error) => Verdict::Blocked(host_reason(setting, &error.reason, viewer.decoder)),
    }
}

/// The facts „Anzeige“ depends on.
#[derive(Clone, Copy, Debug)]
pub struct DisplayFacts<'a> {
    /// The window's renderer (`screen_present::window_renderer_label`).
    pub window: &'a str,
    /// The Direct3D 11 display's last error.
    pub direct_error: Option<&'a str>,
    pub library: DecoderLibrary,
    pub decoder: DecoderPreference,
    pub codec: CodecSetting,
    /// The running decoder with its stage (`SessionInfo::decoder`).
    pub running: Option<&'a str>,
}

/// How „Anzeige“ `choice` stands: Direct3D 11 shows FFmpeg's D3D11VA
/// pictures only, wgpu only in a window that draws with wgpu
/// (`screen_toolbar_state::display_state_in`).
pub fn display_verdict(choice: DisplayChoice, facts: &DisplayFacts) -> Verdict {
    let blocked = match choice {
        DisplayChoice::Automatic => None,
        DisplayChoice::Wgpu => (facts.window != "wgpu").then(|| {
            format!(
                "das Fenster zeichnet mit {}; wgpu zeigt kein Bild",
                facts.window
            )
        }),
        DisplayChoice::Direct3d11 => {
            if let Some(error) = facts.direct_error {
                Some(format!("Direct3D 11 nicht verfügbar: {error}"))
            } else if facts.library == DecoderLibrary::MediaFoundation {
                Some("zeigt nur D3D11VA-Bilder von FFmpeg (Bibliothek „Media Foundation“)".into())
            } else if facts.decoder == DecoderPreference::Software {
                Some("zeigt nur Hardware-Bilder (D3D11VA; Dekodierer „Software“)".into())
            } else if facts.codec == CodecSetting::Lossless {
                Some("zeigt keine Kacheln (Codec „Verlustfrei“)".into())
            } else {
                facts
                    .running
                    .filter(|name| !name.contains("[D3D11VA]"))
                    .map(|name| format!("das laufende Bild ist keine D3D11VA-Textur ({name})"))
            }
        }
    };
    blocked.map_or(Verdict::Usable, Verdict::Blocked)
}

/// What one session runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionChoice {
    pub codec: CodecSetting,
    pub decoder: DecoderPreference,
    /// The library whose narrowing the offers follow; „Automatisch“ keeps
    /// the tiles beside a chosen library's decoders.
    pub library: DecoderLibrary,
    /// One German sentence per stored choice this session does not use.
    pub notes: Vec<String>,
}

/// `<name>: <why> – diese Sitzung: Automatisch`.
pub fn replaced_text(name: &str, why: &str) -> String {
    format!("{name}: {why} – diese Sitzung: Automatisch")
}

/// The choices a session with a host of `host` encoders runs with: the
/// stored ones while they work, else „Automatisch“ in their place.
pub fn session_choice(
    stored: CodecSetting,
    host: Option<&[CodecOffer]>,
    viewer: &Viewer,
) -> SessionChoice {
    let mut current = *viewer;
    let mut choice = SessionChoice {
        codec: stored,
        decoder: viewer.decoder,
        library: viewer.library,
        notes: Vec::new(),
    };
    if let Verdict::Blocked(why) = codec_verdict(stored, host, &current) {
        if stored != CodecSetting::Auto {
            choice.notes.push(replaced_text(stored.label(), &why));
            choice.codec = CodecSetting::Auto;
        }
    }
    if current.decoder != DecoderPreference::Automatic {
        if let Verdict::Blocked(why) = codec_verdict(choice.codec, host, &current) {
            let name = format!("Dekodierer „{}“", current.decoder.label());
            choice.notes.push(replaced_text(&name, &why));
            current.decoder = DecoderPreference::Automatic;
            choice.decoder = DecoderPreference::Automatic;
        }
    }
    if current.library != DecoderLibrary::Automatic {
        if let Verdict::Blocked(why) = codec_verdict(choice.codec, host, &current) {
            let name = format!("Dekodier-Bibliothek „{}“", current.library.label());
            choice.notes.push(replaced_text(&name, &why));
            choice.library = DecoderLibrary::Automatic;
        }
    }
    choice
}

/// Whether what a session of `stored` runs with depends on the host's
/// encoders (it stays unknown without them).
pub fn host_matters(stored: CodecSetting, viewer: &Viewer) -> bool {
    let choice = session_choice(stored, None, viewer);
    let with = Viewer {
        decoder: choice.decoder,
        library: choice.library,
        ..*viewer
    };
    matches!(
        codec_verdict(choice.codec, None, &with),
        Verdict::Unknown(_)
    )
}

/// A failed probe of a host is tried again after this long.
const RETRY: Duration = Duration::from_secs(10);

/// What this process knows of a host's encoders.
enum Known {
    Learning,
    Encoders(Vec<CodecOffer>),
    Failed(Instant),
}

/// The hosts' encoders, the full view's device, the libraries the register
/// found absent while chosen and the full view's running decoder.
struct Store {
    hosts: Vec<(String, Known)>,
    view: Option<ScreenTarget>,
    absent: Vec<DecoderLibrary>,
    running: Option<String>,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    hosts: Vec::new(),
    view: None,
    absent: Vec::new(),
    running: None,
});
static CHANGED: Condvar = Condvar::new();

fn store() -> MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Off in the task tests: they run fake hosts in parallel, and what one
/// noted would decide for another (as `screen_pool::tiles_only_line`).
pub fn enabled() -> bool {
    !cfg!(test)
}

/// The key of a device (`ScreenTarget::same_device`).
pub fn peer_key(target: &ScreenTarget) -> String {
    let device = target.device_id.as_deref().unwrap_or("");
    format!(
        "{:?}\u{1f}{}\u{1f}{device}",
        target.relation, target.relation_id
    )
}

fn note(key: &str, known: Known) {
    if !enabled() {
        return;
    }
    let mut store = store();
    match store.hosts.iter_mut().find(|(k, _)| k == key) {
        Some((_, slot)) => *slot = known,
        None => store.hosts.push((key.to_string(), known)),
    }
    CHANGED.notify_all();
}

/// The encoders `hello` names for the device `target`.
pub fn note_hello(target: &ScreenTarget, hello: &ScreenHelloOk) {
    let encoders = hello.all_encoders();
    if host_encoders(target).as_ref() != Some(&encoders) {
        note(&peer_key(target), Known::Encoders(encoders));
    }
}

/// The encoders of `target` when known.
pub fn host_encoders(target: &ScreenTarget) -> Option<Vec<CodecOffer>> {
    let key = peer_key(target);
    store().hosts.iter().find_map(|(k, known)| match known {
        Known::Encoders(encoders) if *k == key => Some(encoders.clone()),
        _ => None,
    })
}

/// Starts the probe of `target`'s encoders unless they are known, it runs
/// or it failed within `RETRY`; `wake` follows its answer. `true` while a
/// probe runs (its answer may still come).
pub fn learn(target: &ScreenTarget, wake: Option<Wake>) -> bool {
    let key = peer_key(target);
    match store()
        .hosts
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, k)| k)
    {
        _ if !enabled() => return false,
        Some(Known::Learning) => return true,
        Some(Known::Encoders(_)) => return false,
        Some(Known::Failed(at)) if at.elapsed() < RETRY => return false,
        _ => {}
    }
    let Ok(peer) = target.to_peer() else {
        note(&key, Known::Failed(Instant::now()));
        return false;
    };
    note(&key, Known::Learning);
    let named = key.clone();
    let spawned = std::thread::Builder::new()
        .name("screen-codec-probe".into())
        .spawn(move || {
            use crate::share::access_grant::CapabilityDecision;
            // A device that refuses the view names no encoder (`supported`
            // with empty `codecs`): that is no fact about its encoders.
            let known = match crate::daemon::screen_probe(peer) {
                Ok(report) if report.supported && report.view != CapabilityDecision::Off => {
                    Known::Encoders(report.codecs)
                }
                _ => Known::Failed(Instant::now()),
            };
            note(&named, known);
            if let Some(wake) = wake {
                wake();
            }
        });
    if spawned.is_err() {
        note(&key, Known::Failed(Instant::now()));
    }
    spawned.is_ok()
}

/// The encoders of `target`, waiting at most `max` for a running probe.
pub fn wait_host(target: &ScreenTarget, max: Duration) -> Option<Vec<CodecOffer>> {
    let key = peer_key(target);
    let deadline = Instant::now() + max;
    let mut store = store();
    loop {
        match store.hosts.iter().find(|(k, _)| *k == key).map(|(_, k)| k) {
            Some(Known::Encoders(encoders)) => return Some(encoders.clone()),
            Some(Known::Learning) => {}
            _ => return None,
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        store = CHANGED
            .wait_timeout(store, left)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// The full view's device; `None` before one opened.
pub fn view() -> Option<ScreenTarget> {
    store().view.clone()
}

/// The full view shows `target` from now on.
pub fn set_view(target: &ScreenTarget) {
    if enabled() && store().view.as_ref() != Some(target) {
        store().view = Some(target.clone());
    }
}

/// The device of a `--screen-view` start JSON is the full view's.
pub fn start_target(target_json: &str) {
    if let Ok(target) = ScreenTarget::parse(target_json) {
        set_view(&target);
    }
}

/// The full view's running decoder (for „Anzeige“).
pub fn note_running(decoder: Option<&str>) {
    if enabled() && store().running.as_deref() != decoder {
        store().running = decoder.map(str::to_string);
    }
}

/// The libraries the register found absent in this process.
pub fn absent_libraries() -> Vec<DecoderLibrary> {
    store().absent.clone()
}

/// The library in force: the stored one unless the register found it
/// absent in this process.
pub fn effective_library(stored: DecoderLibrary) -> DecoderLibrary {
    if !enabled() {
        return stored;
    }
    let mut store = store();
    if let Some(missing) = crate::screen::core::decoder_library::chosen_missing() {
        if !store.absent.contains(&missing) {
            store.absent.push(missing);
        }
    }
    match store.absent.contains(&stored) {
        true => DecoderLibrary::Automatic,
        false => stored,
    }
}

/// „Anzeige“ facts of this process now, for `with`.
pub fn with_display_facts<R>(with: impl FnOnce(&DisplayFacts) -> R) -> R {
    let running = store().running.clone();
    let window = super::super::screen_present::window_renderer_label();
    let error = super::super::screen_direct::last_error();
    with(&DisplayFacts {
        window,
        direct_error: error.as_deref(),
        library: super::decoder_library_choice(),
        decoder: super::decoder_choice(),
        codec: super::codec_setting(),
        running: running.as_deref(),
    })
}

/// „Anzeige“ in force: the stored one unless it shows no picture here.
pub fn effective_display(stored: DisplayChoice) -> DisplayChoice {
    if !enabled() || stored == DisplayChoice::Automatic {
        return stored;
    }
    match with_display_facts(|facts| display_verdict(stored, facts)) {
        Verdict::Usable => stored,
        _ => DisplayChoice::Automatic,
    }
}

#[cfg(test)]
#[path = "screen_codec_offer_tests.rs"]
mod tests;
