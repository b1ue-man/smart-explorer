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

use super::screen_prefs::ViewerPrefs;
use super::screen_session::{Backend, ViewerSession};
use super::screen_target::ScreenTarget;
use crate::screen::core::decoder_health::GivenUp;
use crate::screen::core::negotiate::{CodecOffer, CodecSetting};
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
/// second cannot be asked of the host, so the viewer shows every n-th
/// picture itself (each one is still decoded: tile frames build on each
/// other).
pub fn preview_gap_ms(interval_ms: u64) -> u64 {
    if interval_ms > 1000 {
        interval_ms
    } else {
        0
    }
}

pub fn preview_request(offers: &[CodecOffer], edge: u32, interval_ms: u64) -> StreamRequest {
    announced(preview_base(offers, edge, interval_ms), GivenUp::default())
}

/// A preview's request before `announced`.
fn preview_base(offers: &[CodecOffer], edge: u32, interval_ms: u64) -> StreamRequest {
    StreamRequest {
        expected_node: None,
        mode: SessionMode::Preview,
        source: None,
        decoders: offers.to_vec(),
        display_size: Some(FrameSize::new(edge, edge)),
        max_fps: preview_fps(interval_ms.min(FASTEST_PREVIEW_INTERVAL_MS)),
        frame_capabilities: None,
        max_rate_bps: super::screen_prefs::max_rate_bps(),
        codec: Some(super::screen_prefs::codec_setting()),
        decoder: Some(super::screen_prefs::decoder_choice()),
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
fn announced(mut request: StreamRequest, given_up: GivenUp) -> StreamRequest {
    let offers = super::screen_decode_local::current_offers(&request.decoders);
    request.decoders = given_up.filter(&offers);
    // A chosen „Dekodier-Bibliothek“ offers no tiles beside its video (the
    // tiles decode in neither library), so the host never falls back to them
    // (Codex 6/13). Safe since the hello and `SET_DECODERS` name the choice
    // (`codec_choice`): the host no longer reads it from the missing tiles.
    // A library without any video is refused before (`library_refusal`).
    let library = super::screen_prefs::decoder_library_choice();
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
pub fn announced_request(offers: &[CodecOffer]) -> StreamRequest {
    let base = preview_base(offers, PREVIEW_MIN_EDGE, DEFAULT_PREVIEW_INTERVAL_MS);
    announced(base, GivenUp::default())
}

pub fn decoders_frame(offers: &[CodecOffer]) -> SetDecoders {
    decoders_frame_without(offers, GivenUp::default())
}

/// `decoders_frame` without the codecs one session gave up.
pub fn decoders_frame_without(offers: &[CodecOffer], given_up: GivenUp) -> SetDecoders {
    let base = preview_base(offers, PREVIEW_MIN_EDGE, DEFAULT_PREVIEW_INTERVAL_MS);
    announced(base, given_up).decoders_frame()
}

pub fn full_request(
    offers: &[CodecOffer],
    source: Option<SourceId>,
    display: FrameSize,
    control: bool,
) -> StreamRequest {
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
        // „Codec“ and „Dekodierer“ (plan 2026-11-14 „Bildkette“ 4.10, BK6).
        codec: Some(super::screen_prefs::codec_setting()),
        decoder: Some(super::screen_prefs::decoder_choice()),
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        // „Monitor“ and „Auflösung“ (plan 2026-11-17 P5).
        virtual_display: Some(super::screen_prefs::virtual_display_choice()),
    };
    announced(request, GivenUp::default())
}

// The production opener binds capabilities to its concrete renderer and lease.

/// `SET_MODE` to the full view with the viewer's settings.
pub fn full_set_mode(source: Option<SourceId>, display: FrameSize, prefs: &ViewerPrefs) -> SetMode {
    SetMode {
        mode: StreamMode::Full,
        source,
        size: Some(display),
        fps: Some(full_fps()),
        // „Scharf (GPU)“ sets the bit the switch „Schärfe vor Flüssigkeit“ set (P9).
        sharpness: prefs.codec_setting().sharpness(),
        // „Codec: Verlustfrei“ keeps the meaning of the old switch (4.10).
        lossless: prefs.codec_setting().lossless(),
    }
}

/// `SET_MODE` back to a preview.
/// The rate of `interval_ms` alone, size and codec unchanged: a new rate
/// keeps the encoder (host `same_but_rate`). A preview opens at the host's
/// own rate (one picture a second) until it hears another.
pub fn preview_rate_mode(interval_ms: u64) -> SetMode {
    SetMode {
        size: None,
        ..preview_set_mode(PREVIEW_MIN_EDGE, interval_ms)
    }
}

pub fn preview_set_mode(edge: u32, interval_ms: u64) -> SetMode {
    SetMode {
        mode: StreamMode::Preview,
        source: None,
        size: Some(FrameSize::new(edge, edge)),
        fps: Some(preview_fps(interval_ms)),
        sharpness: false,
        // „Codec: Verlustfrei“ stays in the preview too (fix 2026-11-19):
        // before, a view lowered to its tile left it for lossy tiles.
        lossless: super::screen_prefs::codec_setting().lossless(),
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
    if let Some(session) = tile {
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
    session.send(ViewerFrame::SetMode(preview_set_mode(edge, interval_ms)));
    session.set_prefer_yuv(false);
    session.set_present_gap_ms(preview_gap_ms(interval_ms));
    session.mode = SessionMode::Preview;
    Some(session)
}
