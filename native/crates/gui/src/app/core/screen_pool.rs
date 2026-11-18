//! Requests and the switch between the group view and the full view
//! (SCR-M7, U25-3/10). A tile that streams a preview under „Bildschirm
//! ansehen = Erlauben“ becomes the full view over the same connection
//! (`SET_MODE` to `full`), and the full view goes back to a preview of its
//! tile the same way. Under „Fragen“ a preview has no picture and the full
//! view needs a new session, because the host asks its user for it
//! (`screen_task_grid_switch_reuses_connection`).
use crate::daemon::StreamRequest;

use super::screen_prefs::ViewerPrefs;
use super::screen_session::{Backend, ViewerSession};
use super::screen_target::ScreenTarget;
use crate::screen::core::negotiate::CodecOffer;
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
    announced(StreamRequest {
        expected_node: None,
        mode: SessionMode::Preview,
        source: None,
        decoders: offers.to_vec(),
        display_size: Some(FrameSize::new(edge, edge)),
        max_fps: preview_fps(interval_ms),
        frame_capabilities: None,
        max_rate_bps: super::screen_prefs::max_rate_bps(),
        codec: Some(super::screen_prefs::codec_setting()),
        decoder: Some(super::screen_prefs::decoder_choice()),
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        // A preview never changes the host's display (plan 2026-11-17 P5).
        virtual_display: None,
    })
}

/// B-36: `gop_continue` only when every decoder that may open for the
/// request's offers keeps its references, and per codec the offers whose
/// decoders do (`screen_decode_local`); a preview raised to the full view
/// keeps its session, so both ask.
fn announced(mut request: StreamRequest) -> StreamRequest {
    // Codecs that gave no picture in this process stay out (fix of
    // 2026-11-17, `decoder_health`).
    // P9: the register's offers now („Dekodierer-Bibliothek“ at run time).
    let offers = super::screen_decode_local::current_offers(&request.decoders);
    request.decoders = crate::screen::core::decoder_health::usable_offers(&offers);
    request.gop_continue = super::screen_decode_local::announces(&request);
    request.gop_continue_decoders = super::screen_decode_local::continuing(&request);
    request
}

/// `SET_DECODERS` with the viewer's current „Codec“ and „Dekodierer“ for
/// `offers` (P1): the decoders and the announcement a new session's hello
/// would carry.
pub fn decoders_frame(offers: &[CodecOffer]) -> SetDecoders {
    preview_request(offers, PREVIEW_MIN_EDGE, DEFAULT_PREVIEW_INTERVAL_MS).decoders_frame()
}

pub fn full_request(
    offers: &[CodecOffer],
    source: Option<SourceId>,
    display: FrameSize,
    control: bool,
) -> StreamRequest {
    announced(StreamRequest {
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
    })
}

// The production opener binds capabilities to its concrete renderer and lease.

/// `SET_MODE` to the full view with the viewer's settings.
pub fn full_set_mode(source: Option<SourceId>, display: FrameSize, prefs: &ViewerPrefs) -> SetMode {
    SetMode {
        mode: StreamMode::Full,
        source,
        size: Some(display),
        fps: Some(full_fps()),
        sharpness: prefs.sharpness,
        // „Codec: Verlustfrei“ keeps the meaning of the old switch (4.10).
        lossless: prefs.codec_setting().lossless(),
    }
}

/// `SET_MODE` back to a preview.
pub fn preview_set_mode(edge: u32, interval_ms: u64) -> SetMode {
    SetMode {
        mode: StreamMode::Preview,
        source: None,
        size: Some(FrameSize::new(edge, edge)),
        fps: Some(preview_fps(interval_ms)),
        sharpness: false,
        lossless: false,
    }
}

/// How the full view opens.
pub struct ViewOpen {
    pub display: FrameSize,
    pub prefs: ViewerPrefs,
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

/// Group → view: the tile's preview session becomes the full view when the
/// host allows viewing without asking and no control is wanted; any other
/// case ends it and opens a new session in the mode `view` (or `control`),
/// for which the host asks if it has to.
pub fn to_view(
    backend: &Backend,
    tile: Option<ViewerSession>,
    target: &ScreenTarget,
    open: &ViewOpen,
) -> ViewerSession {
    if let Some(mut session) = tile {
        if !target.control && session.mode == SessionMode::Preview && view_allowed(&session) {
            let source = target.source.clone();
            session.send(ViewerFrame::SetMode(full_set_mode(
                source,
                open.display,
                &open.prefs,
            )));
            // The full view draws the host's pointer itself again (P6); the
            // frame only leaves for a host whose session negotiated that.
            session.send(ViewerFrame::CursorOverlay(true));
            session.set_prefer_yuv(open.prefer_yuv);
            session.set_present_gap_ms(0);
            session.mode = SessionMode::View;
            return session;
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
    session.send(ViewerFrame::SetMode(preview_set_mode(edge, interval_ms)));
    session.set_prefer_yuv(false);
    session.set_present_gap_ms(preview_gap_ms(interval_ms));
    session.mode = SessionMode::Preview;
    Some(session)
}
