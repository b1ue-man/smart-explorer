//! Requests and the switch between the group view and the full view
//! (SCR-M7, U25-3/10). The full view always opens its own session with the
//! full request (user rule of 2026-11-18): a preview's hello caps the frame
//! rate at the preview's (the host never goes above the hello's `max_fps`)
//! and carries no „Monitor“/„Auflösung“ wish, so a raised preview ran the
//! full view at one picture a second without the chosen display. Under
//! „Fragen“ the host asks its user for the new session as before. The full
//! view goes back to a preview of its tile over the same connection
//! (`SET_MODE` to `preview`) when the host allows viewing without asking
//! (`screen_task_grid_switch_tile_reuses_connection`).
use crate::daemon::StreamRequest;

use super::screen_prefs::codec_offer::{self, SessionChoice};
use super::screen_prefs::{DecoderLibrary, ViewerPrefs};
use super::screen_session::{Backend, ViewerSession};
use super::screen_target::ScreenTarget;
use crate::screen::core::decoder_health::GivenUp;
use crate::screen::core::negotiate::{CodecOffer, CodecSetting, DecoderPreference};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::set_decoders::SetDecoders;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::{
    ControlAction, ControlRequest, SetMode, StreamMode, ViewerFrame,
};
use crate::screen::core::wire_hello::SessionMode;
use crate::share::access_grant::CapabilityDecision;

/// Previews are asked for in multiples of this many pixels of the long edge
/// (the host scales to what fits; steps keep resizing a tile from asking
/// again at every pixel).
pub const PREVIEW_STEP: u32 = 160;
/// The long edge of a preview of the smallest tile (decision C6).
pub const PREVIEW_MIN_EDGE: u32 = 320;
/// „Aktualisierung“ of the group view at the start, in ms.
pub const DEFAULT_PREVIEW_INTERVAL_MS: u64 = 1000;
/// The shortest „Aktualisierung“ the group view offers, in ms: a preview's
/// hello allows its rate, so a later choice of it is not capped at the
/// opening's (the host caps at the hello's rate, Codex 11/13).
pub const FASTEST_PREVIEW_INTERVAL_MS: u64 = 500;

/// Frames per second of the full view: the refresh rate of the monitor the
/// viewer's window is on (plan 2026-11-14 „Bildkette“ B-01).
pub fn full_fps() -> u16 {
    super::screen_refresh::wanted_fps()
}

/// The long edge of a preview for a tile `width_points` wide.
pub fn preview_edge(width_points: f32, pixels_per_point: f32) -> u32 {
    let wanted = (width_points * pixels_per_point).ceil().max(1.0) as u32;
    (wanted.div_ceil(PREVIEW_STEP) * PREVIEW_STEP).max(PREVIEW_MIN_EDGE)
}

/// Frames per second the host is asked for to get a picture every
/// `interval_ms` at best; `SET_MODE` knows whole frames per second only.
pub fn preview_fps(interval_ms: u64) -> u16 {
    u16::try_from((1000 / interval_ms.max(1)).max(1)).unwrap_or(u16::MAX)
}

/// Shortest time between two shown previews. An interval longer than a
/// second cannot be asked of the host, so the viewer shows the newest
/// picture once the interval has passed (each one is still decoded: tile
/// frames build on each other); one that came within it waits for that
/// moment instead of being left out (`Shared::take_due_picture`, Codex 12/13).
pub fn preview_gap_ms(interval_ms: u64) -> u64 {
    if interval_ms > 1000 {
        interval_ms
    } else {
        0
    }
}

/// A preview with the stored choices (task tests; the group view names its
/// device, `preview_request_to`).
#[cfg(test)]
pub fn preview_request(offers: &[CodecOffer], edge: u32, interval_ms: u64) -> StreamRequest {
    preview_request_to(None, offers, edge, interval_ms)
}

/// A preview of `target` with what its host serves (`open_choice`).
pub fn preview_request_to(
    target: Option<&ScreenTarget>,
    offers: &[CodecOffer],
    edge: u32,
    interval_ms: u64,
) -> StreamRequest {
    let choice = open_choice(target, offers, false);
    let base = preview_base(offers, edge, interval_ms, &choice);
    announced(base, GivenUp::default(), choice.library)
}

/// A preview's request before `announced`, with `choice`'s codec and stage.
fn preview_base(
    offers: &[CodecOffer],
    edge: u32,
    interval_ms: u64,
    choice: &SessionChoice,
) -> StreamRequest {
    StreamRequest {
        expected_node: None,
        mode: SessionMode::Preview,
        source: None,
        decoders: offers.to_vec(),
        display_size: Some(FrameSize::new(edge, edge)),
        max_fps: preview_fps(interval_ms.min(FASTEST_PREVIEW_INTERVAL_MS)),
        frame_capabilities: None,
        max_rate_bps: super::screen_prefs::max_rate_bps(),
        codec: Some(choice.codec),
        decoder: Some(choice.decoder),
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        // A preview never changes the host's display (plan 2026-11-17 P5).
        virtual_display: None,
    }
}

/// The register's offers now (P9: „Dekodierer-Bibliothek“ at run time)
/// without the codecs `given_up` of one session (fix of 2026-11-18: a give-up
/// lasts for its session only; before, it took the codec out of every later
/// request of the process), and B-36: `gop_continue` only when every
/// decoder that may open for the request's offers keeps its references, and
/// per codec the offers whose decoders do (`screen_decode_local`); a preview
/// raised to the full view keeps its session, so both ask.
fn announced(
    mut request: StreamRequest,
    given_up: GivenUp,
    library: DecoderLibrary,
) -> StreamRequest {
    let offers = super::screen_decode_local::current_offers(&request.decoders);
    request.decoders = given_up.filter(&offers);
    // A chosen „Dekodier-Bibliothek“ offers no tiles beside its video (the
    // tiles decode in neither library), so the host never falls back to them
    // (Codex 6/13). Safe since the hello and `SET_DECODERS` name the choice
    // (`codec_choice`): the host no longer reads it from the missing tiles.
    // A library without any video is refused before (`library_refusal`);
    // `library` is „Automatisch“ where the host serves the library's video
    // with none of its encoders (`codec_offer::session_choice`).
    let lossless = request.codec == Some(super::screen_prefs::CodecSetting::Lossless);
    let video = request.decoders.iter().any(|offer| offer.codec.is_video());
    if library != super::screen_prefs::DecoderLibrary::Automatic && !lossless && video {
        request.decoders.retain(|offer| offer.codec.is_video());
    }
    request.gop_continue = super::screen_decode_local::announces(&request);
    request.gop_continue_decoders = super::screen_decode_local::continuing(&request);
    // Task tests with fake decoders write no log file.
    if let Some(line) = tiles_only_line(&request, given_up).filter(|_| !cfg!(test)) {
        crate::screen::os::host_worker_client::host_log::viewer_note(&line);
    }
    request
}

/// The viewer-log line of a request that offers the host no video codec
/// (fix of 2026-11-18): only the tiles under „Automatisch“, nothing under a
/// named codec; `None` when a video codec is offered or „Verlustfrei“ chose
/// the tiles.
pub fn tiles_only_line(request: &StreamRequest, given_up: GivenUp) -> Option<String> {
    let codec = request.codec.unwrap_or_default();
    let chosen = request.chosen_decoders();
    if codec == CodecSetting::Lossless || chosen.iter().any(|offer| offer.codec.is_video()) {
        return None;
    }
    let mut register: Vec<String> = Vec::new();
    for offer in &request.decoders {
        let stage = if offer.hardware { "hw" } else { "sw" };
        let text = format!("{}/{stage}", offer.codec.key());
        if !register.contains(&text) {
            register.push(text);
        }
    }
    let library = crate::screen::core::decoder_library::decoder_library();
    let missing = crate::screen::core::decoder_library::chosen_missing()
        .map(|library| format!("; Bibliothek „{}“ fehlt", library.label()))
        .unwrap_or_default();
    Some(format!(
        "nur Kacheln angeboten: kein Video-Codec unter Codec „{}“, Dekodierer „{}“, \
         Dekodier-Bibliothek „{}“{missing}; Angebote des Registers: [{}]; in dieser Sitzung \
         aufgegeben: [{}]",
        codec.label(),
        request.decoder.unwrap_or_default().label(),
        library.label(),
        register.join(", "),
        given_up.keys().join(", ")
    ))
}

/// `SET_DECODERS` with the viewer's current „Codec“ and „Dekodierer“ for
/// `offers` (P1): the decoders and the announcement a new session's hello
/// would carry.
/// The request `decoders_frame` is made of: its `decoders` are the
/// register's offers before „Codec“ and „Dekodierer“ narrow them.
/// The full view's session choice (`view_choice`) narrows them.
pub fn announced_request(offers: &[CodecOffer]) -> StreamRequest {
    announced_without(offers, GivenUp::default())
}

fn announced_without(offers: &[CodecOffer], given_up: GivenUp) -> StreamRequest {
    let choice = view_choice(offers, false);
    let base = preview_base(
        offers,
        PREVIEW_MIN_EDGE,
        DEFAULT_PREVIEW_INTERVAL_MS,
        &choice,
    );
    announced(base, given_up, choice.library)
}

pub fn decoders_frame(offers: &[CodecOffer]) -> SetDecoders {
    decoders_frame_without(offers, GivenUp::default())
}

/// `decoders_frame` without the codecs one session gave up.
pub fn decoders_frame_without(offers: &[CodecOffer], given_up: GivenUp) -> SetDecoders {
    announced_without(offers, given_up).decoders_frame()
}

pub fn full_request(
    offers: &[CodecOffer],
    source: Option<SourceId>,
    display: FrameSize,
    control: bool,
) -> StreamRequest {
    // „Codec“ and „Dekodierer“ (plan 2026-11-14 „Bildkette“ 4.10, BK6) as the
    // full view's host serves them (user rule 2026-11-24, `view_choice`).
    let choice = view_choice(offers, true);
    let request = StreamRequest {
        expected_node: None,
        mode: if control {
            SessionMode::Control
        } else {
            SessionMode::View
        },
        source,
        decoders: offers.to_vec(),
        display_size: Some(display),
        max_fps: full_fps(),
        frame_capabilities: None,
        max_rate_bps: super::screen_prefs::max_rate_bps(),
        codec: Some(choice.codec),
        decoder: Some(choice.decoder),
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        // „Monitor“ and „Auflösung“ (plan 2026-11-17 P5).
        virtual_display: Some(super::screen_prefs::virtual_display_choice()),
    };
    announced(request, GivenUp::default(), choice.library)
}

/// Longest wait of the full view's opening for its host's encoders
/// (assumption: a probe over a working path answers within it).
pub const OPEN_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Devices whose sessions run with „Dekodierer: Automatisch“ in place of the
/// stored stage (`SessionChoice::decoder`).
static RELAXED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// What a session to `target` runs with (`codec_offer::session_choice`):
/// the stored choices unless its host cannot serve them. The host's
/// encoders come from an earlier `HELLO_OK` or a probe (`learn`); `wait`
/// lets the full view's opening wait for a running probe at most
/// `OPEN_WAIT` (after it the stored choice goes out unchecked). Without a
/// target the stored choices. The register opens every decoder stage while
/// one device's sessions need „Automatisch“ in place of the stored one.
pub fn open_choice(
    target: Option<&ScreenTarget>,
    offers: &[CodecOffer],
    wait: bool,
) -> SessionChoice {
    let stored = super::screen_prefs::codec_setting();
    let decoder = super::screen_prefs::decoder_choice();
    let library = super::screen_prefs::decoder_library_choice();
    let Some(target) = target.filter(|_| codec_offer::enabled()) else {
        return SessionChoice {
            codec: stored,
            decoder,
            library,
            notes: Vec::new(),
        };
    };
    let decoders = super::screen_decode_local::current_offers(offers);
    let viewer = codec_offer::Viewer {
        decoders: &decoders,
        decoder,
        library,
    };
    let mut host = codec_offer::host_encoders(target);
    if host.is_none() && codec_offer::host_matters(stored, &viewer) {
        let running = codec_offer::learn(target, None);
        if running && wait {
            host = codec_offer::wait_host(target, OPEN_WAIT);
        }
    }
    let choice = codec_offer::session_choice(stored, host.as_deref(), &viewer);
    let key = codec_offer::peer_key(target);
    let mut relaxed = RELAXED.lock().unwrap_or_else(|e| e.into_inner());
    relaxed.retain(|k| *k != key);
    if choice.decoder != decoder {
        relaxed.push(key);
    }
    let process = match relaxed.is_empty() {
        true => decoder,
        false => DecoderPreference::Automatic,
    };
    crate::screen::core::negotiate::set_decoder_preference(process);
    crate::screen::core::decoder_library::set_decoder_library(library);
    choice
}

/// `open_choice` for the full view's device (`codec_offer::view`).
pub fn view_choice(offers: &[CodecOffer], wait: bool) -> SessionChoice {
    open_choice(codec_offer::view().as_ref(), offers, wait)
}

/// Whether the preview of `target` waits for its host's encoders: its
/// session depends on them and a probe runs (`wake` follows its answer).
pub fn host_pending(
    target: &ScreenTarget,
    offers: &[CodecOffer],
    wake: super::screen_session::Wake,
) -> bool {
    if !codec_offer::enabled() || codec_offer::host_encoders(target).is_some() {
        return false;
    }
    let decoders = super::screen_decode_local::current_offers(offers);
    let viewer = codec_offer::Viewer {
        decoders: &decoders,
        decoder: super::screen_prefs::decoder_choice(),
        library: super::screen_prefs::decoder_library_choice(),
    };
    codec_offer::host_matters(super::screen_prefs::codec_setting(), &viewer)
        && codec_offer::learn(target, Some(wake))
}

/// The codec the full view's session runs for: `stored` as `view_choice`
/// keeps it (a caller's own settings stay theirs).
fn view_codec(stored: CodecSetting) -> CodecSetting {
    match stored == super::screen_prefs::codec_setting() {
        true => view_choice(&[], false).codec,
        false => stored,
    }
}

// The production opener binds capabilities to its concrete renderer and lease.

/// `SET_MODE` to the full view with the viewer's settings.
pub fn full_set_mode(source: Option<SourceId>, display: FrameSize, prefs: &ViewerPrefs) -> SetMode {
    SetMode {
        mode: StreamMode::Full,
        source,
        size: Some(display),
        fps: Some(full_fps()),
        // „Scharf (GPU)“ sets the bit the switch „Schärfe vor Flüssigkeit“
        // set (P9); „Codec: Verlustfrei“ keeps the old switch's meaning
        // (4.10); both as the session runs them (`view_codec`).
        sharpness: view_codec(prefs.codec_setting()).sharpness(),
        lossless: view_codec(prefs.codec_setting()).lossless(),
    }
}

/// `SET_MODE` back to a preview.
/// The rate of `interval_ms` alone, size and codec unchanged: a new rate
/// keeps the encoder (host `same_but_rate`). A preview opens at the host's
/// own rate (one picture a second) until it hears another.
pub fn preview_rate_mode(interval_ms: u64, codec: CodecSetting) -> SetMode {
    SetMode {
        size: None,
        ..preview_set_mode_for(PREVIEW_MIN_EDGE, interval_ms, codec)
    }
}

/// `SET_MODE` back to a preview of a session that runs for `codec`
/// (`Shared::codec_choice`: what its host serves of the stored choice).
pub fn preview_set_mode_for(edge: u32, interval_ms: u64, codec: CodecSetting) -> SetMode {
    SetMode {
        mode: StreamMode::Preview,
        source: None,
        size: Some(FrameSize::new(edge, edge)),
        fps: Some(preview_fps(interval_ms)),
        // „Scharf (GPU)“ and „Codec: Verlustfrei“ stay in the preview too
        // (fix 2026-11-19; round 12, B3): the host opened the preview with
        // the choice (`set_codec_preference`), so a `SET_MODE` without it
        // replaced the encoder right after the opening and left the choice;
        // before, a view lowered to its tile left it as well.
        sharpness: codec.sharpness(),
        lossless: codec.lossless(),
    }
}

/// How the full view opens.
pub struct ViewOpen {
    pub display: FrameSize,
    /// The draw code takes planes (GPU path).
    pub prefer_yuv: bool,
}

fn view_allowed(session: &ViewerSession) -> bool {
    let info = session.info();
    !info.is_ended()
        && info
            .hello
            .as_ref()
            .is_some_and(|hello| hello.authorization.view == CapabilityDecision::Allow)
}

/// Group → view: the tile's preview session ends and the full view opens
/// a session of its own in the mode `view` (or `control`) with the full
/// request: the full frame rate and the „Monitor“/„Auflösung“ wish, which a
/// preview's hello does not carry (module head).
pub fn to_view(
    backend: &Backend,
    tile: Option<ViewerSession>,
    target: &ScreenTarget,
    open: &ViewOpen,
) -> ViewerSession {
    // The tile's hello names the host's encoders for the full request.
    codec_offer::set_view(target);
    if let Some(session) = tile {
        if let Some(hello) = session.info().hello.as_ref() {
            codec_offer::note_hello(target, hello);
        }
        session.close();
    }
    ViewerSession::open(
        backend,
        target.clone(),
        full_request(
            &backend.offers,
            target.source.clone(),
            open.display,
            target.control,
        ),
        target.control,
        open.prefer_yuv,
        0,
    )
}

/// View → group: the full session stays connected as the preview of its
/// tile when the host allows viewing without asking; otherwise it ends.
pub fn to_tile(mut session: ViewerSession, edge: u32, interval_ms: u64) -> Option<ViewerSession> {
    if !view_allowed(&session) {
        session.close();
        return None;
    }
    if session.mode == SessionMode::Control {
        let release = ControlRequest {
            action: ControlAction::Release,
        };
        session.send(ViewerFrame::ControlRequest(release));
    }
    // A tile draws no pointer overlay: the host's pointer goes back into
    // the picture (P6).
    session.send(ViewerFrame::CursorOverlay(false));
    // A tile plays no sound: the host stops sending it (Gemini 3/23); the
    // view it is raised to again asks with the setting (`screen_audio::follow`).
    let sound = super::screen_audio::negotiated(&session.info());
    if sound {
        session.send(ViewerFrame::Audio(false));
    }
    let codec = session.shared.codec_choice();
    session.send(ViewerFrame::SetMode(preview_set_mode_for(
        edge,
        interval_ms,
        codec,
    )));
    session.set_prefer_yuv(false);
    session.set_present_gap_ms(preview_gap_ms(interval_ms));
    session.mode = SessionMode::Preview;
    Some(session)
}
