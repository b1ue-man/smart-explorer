//! What the toolbar shows of a choice (plan 2026-11-17 P9; user: „ich weis
//! weder ob ich alle sehe noch ob was ich einstell auch angewendet wird“):
//! every list names the choice and how the running session stands with it,
//! from the session's facts (the codec of the running stream, the decoder
//! that opened, whether the Direct3D 11 presenter shows the picture), not
//! from the stored choice alone. A change is „wird angewendet“ until the
//! facts change or the session answered without changing them; then the
//! list names what runs. Pure decisions; the toolbar draws them.
use std::time::{Duration, Instant};

use super::super::screen_prefs::{CodecSetting, DecoderLibrary, DecoderPreference, DisplayChoice};
use crate::screen::core::negotiate::CodecKind;

/// How long the session may go on showing pictures with unchanged facts
/// before a change counts as answered: the host answers `SET_DECODERS` and
/// `SET_MODE` with a key frame, and a decoder switch waits for the key
/// frame the viewer asks for at once, both within one capture interval
/// plus one round trip (assumption, not measured; a slower answer shows
/// the old state and then the new one).
pub const ANSWER_WINDOW: Duration = Duration::from_secs(2);

/// How a choice stands in the running session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    /// No session facts yet.
    Unknown,
    /// The session runs as chosen; `Some` names what „Automatisch“ took.
    Applied(Option<String>),
    /// Sent; the session has not answered yet.
    Pending,
    /// The session runs with this instead.
    Other(String),
}

/// The facts of the running session the toolbar compares with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionFacts {
    /// Codec of the running stream.
    pub codec: Option<CodecKind>,
    /// Name of the decoder that opened, with its stage.
    pub decoder: Option<String>,
    /// The Direct3D 11 presenter shows the picture.
    pub direct_showing: bool,
    /// Pictures of the session so far.
    pub pictures: u64,
}

impl SessionFacts {
    /// The facts a change waits to see change (not the picture count).
    fn key(&self) -> (Option<CodecKind>, Option<String>, bool) {
        (self.codec, self.decoder.clone(), self.direct_showing)
    }
}

/// The choices of the toolbar that the session applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Codec,
    Decoder,
    Library,
    Display,
}

#[derive(Clone, Debug)]
struct Change {
    setting: Setting,
    key: (Option<CodecKind>, Option<String>, bool),
    pictures: u64,
    at: Instant,
}

/// The changes sent and not answered yet.
#[derive(Clone, Debug, Default)]
pub struct PendingChoices {
    changes: Vec<Change>,
}

impl PendingChoices {
    /// `setting` changed now, with the session at `facts`.
    pub fn start(&mut self, setting: Setting, facts: &SessionFacts, now: Instant) {
        self.changes.retain(|change| change.setting != setting);
        self.changes.push(Change {
            setting,
            key: facts.key(),
            pictures: facts.pictures,
            at: now,
        });
    }

    /// Whether `setting` still waits: its facts are unchanged and the
    /// session has not shown pictures for `ANSWER_WINDOW` since. An
    /// answered change is forgotten.
    pub fn pending(&mut self, setting: Setting, facts: &SessionFacts, now: Instant) -> bool {
        let key = facts.key();
        let waiting = |change: &Change| {
            let answered = facts.pictures > change.pictures
                && now.saturating_duration_since(change.at) >= ANSWER_WINDOW;
            change.key == key && !answered
        };
        self.changes
            .retain(|change| change.setting != setting || waiting(change));
        self.changes.iter().any(|change| change.setting == setting)
    }
}

/// The name of a running codec in the lists.
pub fn codec_text(codec: CodecKind) -> &'static str {
    match codec {
        CodecKind::H264 => "H.264",
        CodecKind::Hevc => "HEVC",
        CodecKind::Av1 => "AV1",
        CodecKind::Tiles => "Verlustfrei",
    }
}

/// Whether the decoder `name` runs on hardware, from its stage in the name
/// (`FFmpeg av1 [D3D11VA]`, `… [Hardware (D3D11VA)]`, `… [Software …]`).
pub fn decoder_hardware(name: &str) -> Option<bool> {
    if name.contains("[Software") {
        Some(false)
    } else if ["[D3D11VA]", "[DXVA2]", "[Hardware"]
        .iter()
        .any(|stage| name.contains(stage))
    {
        Some(true)
    } else {
        None
    }
}

/// The library of the decoder `name` (Windows): FFmpeg's names start with
/// „FFmpeg“, Media Foundation's carry its stage label.
pub fn decoder_library(name: &str) -> Option<DecoderLibrary> {
    if name.starts_with("FFmpeg ") {
        Some(DecoderLibrary::Ffmpeg)
    } else if name.contains("[Hardware (D3D11VA)]") || name.contains("(Media Foundation)]") {
        Some(DecoderLibrary::MediaFoundation)
    } else {
        None
    }
}

fn stage_text(hardware: bool) -> &'static str {
    if hardware {
        "Hardware"
    } else {
        "Software"
    }
}

/// The verdict from whether the facts match the choice.
fn verdict(matches: Option<bool>, pending: bool, running: Option<String>, auto: bool) -> Applied {
    match matches {
        None => Applied::Unknown,
        Some(true) => Applied::Applied(running.filter(|_| auto)),
        Some(false) if pending => Applied::Pending,
        Some(false) => running.map_or(Applied::Unknown, Applied::Other),
    }
}

/// „Codec“: the codec of the running stream.
pub fn codec_state(chosen: CodecSetting, facts: &SessionFacts, pending: bool) -> Applied {
    let Some(running) = facts.codec else {
        return Applied::Unknown;
    };
    let matches = match chosen {
        CodecSetting::Auto => running != CodecKind::Tiles,
        CodecSetting::H264 => running == CodecKind::H264,
        CodecSetting::Hevc => running == CodecKind::Hevc,
        CodecSetting::Av1 => running == CodecKind::Av1,
        CodecSetting::Sharp => matches!(running, CodecKind::H264 | CodecKind::Hevc),
        CodecSetting::Lossless => running == CodecKind::Tiles,
    };
    let auto = chosen == CodecSetting::Auto;
    verdict(
        Some(matches),
        pending,
        Some(codec_text(running).to_string()),
        auto,
    )
}

/// The running tiles against a decoder or library choice: the tiles have one
/// decoder of their own, so a named choice does not apply while they run
/// (user rule 2026-11-18: never „angewendet“ when it is not).
fn tiles_state(named: bool) -> Applied {
    if named {
        Applied::Other(TILES_RUNNING.into())
    } else {
        Applied::Applied(Some(TILES_RUNNING.into()))
    }
}

/// What runs instead of a decoder while the tiles stream.
pub const TILES_RUNNING: &str = "Kacheln";

/// „Dekodierer“: the stage of the decoder that opened.
pub fn decoder_state(chosen: DecoderPreference, facts: &SessionFacts, pending: bool) -> Applied {
    if facts.codec == Some(CodecKind::Tiles) {
        return tiles_state(chosen != DecoderPreference::Automatic);
    }
    let Some(hardware) = facts.decoder.as_deref().and_then(decoder_hardware) else {
        return Applied::Unknown;
    };
    let matches = chosen.allows(hardware);
    let auto = chosen == DecoderPreference::Automatic;
    verdict(
        Some(matches),
        pending,
        Some(stage_text(hardware).to_string()),
        auto,
    )
}

/// „Bibliothek“: the library of the decoder that opened.
pub fn library_state(chosen: DecoderLibrary, facts: &SessionFacts, pending: bool) -> Applied {
    // A chosen library this process does not have is named, not replaced.
    if let Some(missing) = crate::screen::core::decoder_library::chosen_missing() {
        return Applied::Other(format!("{} nicht vorhanden", missing.label()));
    }
    if facts.codec == Some(CodecKind::Tiles) {
        return tiles_state(chosen != DecoderLibrary::Automatic);
    }
    let Some(running) = facts.decoder.as_deref().and_then(decoder_library) else {
        return Applied::Unknown;
    };
    let matches = chosen == DecoderLibrary::Automatic || chosen == running;
    let auto = chosen == DecoderLibrary::Automatic;
    verdict(
        Some(matches),
        pending,
        Some(running.label().to_string()),
        auto,
    )
}

/// „Anzeige“: whether the Direct3D 11 presenter shows the picture. Only
/// FFmpeg's D3D11VA pictures can go that way; every other picture is drawn
/// by the window (wgpu, or glow when wgpu had no surface). The state names
/// the way the picture really takes and why (user rule 2026-11-18).
pub fn display_state(chosen: DisplayChoice, facts: &SessionFacts, pending: bool) -> Applied {
    let window = super::super::screen_present::window_renderer_label();
    let error = super::super::screen_direct::last_error();
    display_state_in(chosen, facts, pending, window, error.as_deref())
}

/// `display_state` with the window's renderer (`wgpu`, `OpenGL (glow)`)
/// and the Direct3D 11 display's last error.
pub fn display_state_in(
    chosen: DisplayChoice,
    facts: &SessionFacts,
    pending: bool,
    window: &str,
    direct_error: Option<&str>,
) -> Applied {
    if facts.pictures == 0 {
        return Applied::Unknown;
    }
    if facts.direct_showing {
        let running = Some(DisplayChoice::Direct3d11.label().to_string());
        let matches = chosen != DisplayChoice::Wgpu;
        return verdict(
            Some(matches),
            pending,
            running,
            chosen == DisplayChoice::Automatic,
        );
    }
    let d3d11_picture = facts
        .decoder
        .as_deref()
        .is_some_and(|name| name.contains("[D3D11VA]"));
    match chosen {
        DisplayChoice::Automatic => Applied::Applied(Some(window.to_string())),
        DisplayChoice::Wgpu if window == "wgpu" => Applied::Applied(None),
        DisplayChoice::Wgpu => Applied::Other(window.to_string()),
        DisplayChoice::Direct3d11 if pending => Applied::Pending,
        DisplayChoice::Direct3d11 => {
            let why = match (direct_error, d3d11_picture) {
                (Some(error), _) => format!("Direct3D 11 nicht verfügbar: {error}"),
                (None, true) => "Direct3D 11 zeigt das Bild noch nicht".to_string(),
                (None, false) => format!(
                    "Bild ist keine D3D11VA-Textur ({})",
                    facts.decoder.as_deref().unwrap_or("Kacheln")
                ),
            };
            Applied::Other(format!("{window} – {why}"))
        }
    }
}

/// Characters a closed list shows of a free text (a window title) at most:
/// a list keeps its whole width in the toolbar, so a long title would be
/// wider than a narrow window (P9).
pub const SHORT_TEXT: usize = 48;

/// `text` cut to `SHORT_TEXT` characters with „…“; the open list shows
/// the whole text.
pub fn short_text(text: &str) -> String {
    if text.chars().count() <= SHORT_TEXT {
        return text.to_string();
    }
    let kept: String = text.chars().take(SHORT_TEXT - 1).collect();
    format!("{kept}…")
}

/// The closed list's text: `Codec: Automatisch (AV1)`, `Codec: H.264 (wird
/// angewendet …)`, `Codec: H.264 → läuft: AV1`.
pub fn list_text(name: &str, chosen: &str, state: &Applied) -> String {
    match state {
        Applied::Unknown | Applied::Applied(None) => format!("{name}: {chosen}"),
        Applied::Applied(Some(running)) => format!("{name}: {chosen} ({running})"),
        Applied::Pending => format!("{name}: {chosen} (wird angewendet …)"),
        Applied::Other(running) => format!("{name}: {chosen} → läuft: {running}"),
    }
}

#[cfg(test)]
#[path = "p9_toolbar_state_task_tests.rs"]
mod p9_toolbar_state_task_tests;
