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
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::{
    ControlAction, ControlRequest, SetMode, StreamMode, ViewerFrame,
};
use crate::screen::core::wire_hello::SessionMode;
use crate::share::access_grant::CapabilityDecision;

/// Previews are asked for in multiples of this many pixels of the long edge
/// (the host scales to what fits; steps keep resizing a tile from asking
/// again at every pixel).
pub(crate) const PREVIEW_STEP: u32 = 160;
/// The long edge of a preview of the smallest tile (decision C6).
pub(crate) const PREVIEW_MIN_EDGE: u32 = 320;
/// „Aktualisierung“ of the group view at the start, in ms.
pub(crate) const DEFAULT_PREVIEW_INTERVAL_MS: u64 = 1000;
/// Frames per second of the full view.
pub(crate) const FULL_FPS: u16 = 60;

/// The long edge of a preview for a tile `width_points` wide.
pub(crate) fn preview_edge(width_points: f32, pixels_per_point: f32) -> u32 {
    let wanted = (width_points * pixels_per_point).ceil().max(1.0) as u32;
    (wanted.div_ceil(PREVIEW_STEP) * PREVIEW_STEP).max(PREVIEW_MIN_EDGE)
}

/// Frames per second the host is asked for to get a picture every
/// `interval_ms` at best; `SET_MODE` knows whole frames per second only.
pub(crate) fn preview_fps(interval_ms: u64) -> u16 {
    u16::try_from((1000 / interval_ms.max(1)).max(1)).unwrap_or(u16::MAX)
}

/// Shortest time between two shown previews. An interval longer than a
/// second cannot be asked of the host, so the viewer shows every n-th
/// picture itself (each one is still decoded: tile frames build on each
/// other).
pub(crate) fn preview_gap_ms(interval_ms: u64) -> u64 {
    if interval_ms > 1000 {
        interval_ms
    } else {
        0
    }
}

pub(crate) fn preview_request(offers: &[CodecOffer], edge: u32, interval_ms: u64) -> StreamRequest {
    StreamRequest {
            expected_node: None,
        mode: SessionMode::Preview,
        source: None,
        decoders: offers.to_vec(),
        display_size: Some(FrameSize::new(edge, edge)),
        max_fps: preview_fps(interval_ms),
        frame_capabilities: renderer_capabilities(),
    }
}

pub(crate) fn full_request(
    offers: &[CodecOffer],
    source: Option<SourceId>,
    display: FrameSize,
    control: bool,
) -> StreamRequest {
    StreamRequest {
            expected_node: None,
        mode: if control {
            SessionMode::Control
        } else {
            SessionMode::View
        },
        source,
        decoders: offers.to_vec(),
        display_size: Some(display),
        max_fps: FULL_FPS,
        frame_capabilities: renderer_capabilities(),
    }
}

fn renderer_capabilities() -> Option<crate::screen::core::frame_budget::FrameCapabilities> {
    let side = super::screen_wgpu_yuv::texture_limit()?;
    Some(crate::screen::core::frame_budget::FrameCapabilities::legacy_resources(
        FrameSize::new(side, side),
    ))
}

/// `SET_MODE` to the full view with the viewer's settings.
pub(crate) fn full_set_mode(
    source: Option<SourceId>,
    display: FrameSize,
    prefs: &ViewerPrefs,
) -> SetMode {
    SetMode {
        mode: StreamMode::Full,
        source,
        size: Some(display),
        fps: Some(FULL_FPS),
        sharpness: prefs.sharpness,
        lossless: prefs.lossless,
    }
}

/// `SET_MODE` back to a preview.
pub(crate) fn preview_set_mode(edge: u32, interval_ms: u64) -> SetMode {
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
pub(crate) struct ViewOpen {
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
pub(crate) fn to_view(
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
pub(crate) fn to_tile(
    mut session: ViewerSession,
    edge: u32,
    interval_ms: u64,
) -> Option<ViewerSession> {
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
    session.send(ViewerFrame::SetMode(preview_set_mode(edge, interval_ms)));
    session.set_prefer_yuv(false);
    session.set_present_gap_ms(preview_gap_ms(interval_ms));
    session.mode = SessionMode::Preview;
    Some(session)
}
