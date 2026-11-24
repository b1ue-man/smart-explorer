//! The request and the replayed frames of a session built again after
//! `session_changed` (moved out of `screen_ipc_reconnect.rs`, round 12).
//!
//! Round 12 (H3): the host decides a display („Monitor“/„Auflösung“) only at
//! a monitor (`host_visibility.rs`: a display change always ends at one). A
//! session built again therefore carries the viewer's display in its hello,
//! at the chosen monitor or the host's own one, and a window chosen after
//! the display follows with one `SET_MODE`; before, the new session opened on
//! the window, `SET_DISPLAY` moved it to a monitor and `SET_MODE` back to the
//! window (two needless source changes). A display chosen after the source
//! is in the hello as well and is not sent a second time.
//!
//! Round 12 (H4): a source the new session does not have (a window of the
//! session before, a monitor that went with it) is refused with
//! `no_display`; the next attempt opens on the host's own source
//! (`without_source`) and the replayed `SET_MODE` asks for the chosen one
//! again, so the viewer sees the host's error for it (as for a source chosen
//! in a running session) instead of the end of the view.
//!
//! Round 13: only a refusal of the source itself falls back (`no_display`
//! with a reason that names the source gone, `source_refused`); a bare
//! `no_display` (no graphical session yet) or a transient reason keeps the
//! chosen source for the next attempt (H3a). The source in force is the
//! one chosen last, else the first request's (`chosen_source`, H4: a
//! source named only at the opening was lost once a display was chosen). A
//! fallback that opened elsewhere asks for a chosen monitor again also when
//! the display came after it, and the display follows it once more (H3b).
//! The `SET_MODE` that changes the source is repeated without it, so a
//! refused source does not take the viewer's sizes, rates and modes with it
//! (`host_visibility.rs` drops the rest of a `SET_MODE` whose source it
//! refuses). A viewer that never sent a `SET_MODE` has no frame to carry
//! the source: the fallback then stays on the host's own source.
use super::ModeMemory;
use crate::screen::core::viewer_client::{ViewerError, ViewerRequest};
use crate::screen::core::virtual_display::VirtualDisplayWish;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ViewerFrame;
use crate::screen::core::wire_hello::ScreenErrorCode;

fn monitor(source: &SourceId) -> bool {
    matches!(source, SourceId::Monitor(_))
}

/// The source in force: the one the viewer chose last, else the one its
/// first request named (round 13, H4).
fn chosen_source(modes: &ModeMemory, first: &ViewerRequest) -> Option<SourceId> {
    modes.source().or_else(|| first.source.clone())
}

/// The reasons of a `no_display` that hold for the host's own source as
/// well, not only for the chosen one: no graphical session, capture not
/// ready or not reached, a host process that did not answer. Every other
/// reason names the chosen source (`window_not_found`, an X11 error for a
/// window gone with its X server, `portal_selects_source`,
/// `stale_or_foreign_family_selection`, `ambiguous_monitor_id`, …;
/// round 14, B1: a list of the source reasons missed several).
const SESSION_WIDE: [&str; 6] = [
    "no_display",
    "capture_unavailable",
    "session_inactive",
    "android_host_app_not_running",
    "capture_metadata_pending",
    "capture_session_mismatch",
];

/// Whether the host refused the request's source itself: `no_display` with
/// a reason that is not session-wide (`host_server.rs`, `Failure::Source`).
/// A bare `no_display` names no graphical session or no monitor, and a
/// session-wide reason holds for the host's own source too: the chosen
/// source stays for the next attempt (round 13, H3a).
pub(super) fn source_refused(error: &ViewerError) -> bool {
    let ViewerError::Refused(text) = error else {
        return false;
    };
    if error.code() != ScreenErrorCode::NoDisplay.key() {
        return false;
    }
    let Some((_, reason)) = text.split_once(": ") else {
        return false;
    };
    let reason = reason.trim();
    !reason.is_empty()
        && !reason.starts_with("host_process")
        && !reason
            .split(':')
            .next()
            .is_some_and(|first| SESSION_WIDE.contains(&first.trim()))
}

/// The request of a session built again: the ceiling (Gemini 5/10) and the
/// decoders with their „Codec“ (round 8 B4) as the viewer last chose them go
/// in the hello itself, so the new session never starts above the ceiling
/// nor with an encoder of the first request's codec that `SET_DECODERS`
/// would replace at once; so do the source and display in force (Codex
/// 11/4: the first source may be gone), the display at a monitor (H3).
pub(super) fn request(
    modes: &ModeMemory,
    first: &ViewerRequest,
    ceiling: Option<u64>,
) -> ViewerRequest {
    let mut request = first.clone();
    request.max_rate_bps = ceiling;
    match modes.display() {
        None => {
            if let Some(source) = modes.source() {
                request.source = Some(source);
            }
        }
        Some(choice) => {
            request.virtual_display = Some(choice);
            // The chosen monitor, else the host's own one: a window or a
            // family follows by `SET_MODE` (`frames`). Round 13 (H4): also a
            // monitor named only in the first request.
            request.source = chosen_source(modes, first).filter(monitor);
        }
    }
    if let Some(set) = modes.decoders() {
        request.decoders = set.decoders;
        request.gop_continue = set.gop_continue;
        request.gop_continue_decoders = set.gop_continue_decoders;
        request.codec_choice = set.codec_choice.or(first.codec_choice);
    }
    request
}

/// `request` on the host's own source (H4); `None` when it names none.
pub(super) fn without_source(request: &ViewerRequest) -> Option<ViewerRequest> {
    request.source.as_ref()?;
    let mut fallback = request.clone();
    fallback.source = None;
    Some(fallback)
}

/// What the viewer chose since the first opening (`first`), in the order
/// to send it to a session opened with `opened` (the new one starts from
/// that request): its modes, ceiling, decoders, display, sound (`audio`: the
/// host answered it), control and visibility (`visibility`: the host reads
/// it, with a frame contract as the first session's did; Gemini 3/9).
pub(super) fn frames(
    modes: &ModeMemory,
    first: &ViewerRequest,
    opened: &ViewerRequest,
    audio: bool,
    visibility: bool,
) -> Vec<ViewerFrame> {
    let display = modes.display();
    // H3: a display the hello carried is decided already.
    let carried = display.is_some() && opened.virtual_display == display;
    let last = modes.display_last();
    let chosen = chosen_source(modes, first);
    // Round 13 (H3b): a display chosen after a monitor was decided at that
    // monitor; a fallback that opened elsewhere asks for the monitor again
    // and then for the display. Not for se's own display: it shows that
    // display at whatever monitor, as the hello already did.
    let again = last
        && carried
        && modes.chose_mode()
        && display.is_some_and(|choice| choice.wish != VirtualDisplayWish::Request)
        && chosen
            .as_ref()
            .is_some_and(|source| monitor(source) && opened.source.as_ref() != Some(source));
    let mut frames = Vec::new();
    // Codex 11/4: a display choice older than the last source goes first,
    // so it never undoes a later window choice.
    if let Some(choice) = display.filter(|_| !last && !carried) {
        frames.push(ViewerFrame::SetDisplay(choice));
    }
    // A display chosen after the source and carried by the hello put the
    // session at its monitor already: the older source does not move it.
    let source = if last && carried && !again {
        None
    } else {
        chosen
    };
    let mut sets = modes.replay(source.clone());
    // Round 13: a refused source drops the rest of its `SET_MODE`
    // (`host_visibility.rs`); the same frame without the source follows
    // (an unchanged mode changes nothing on the host).
    if source.is_some() && source != opened.source {
        if let Some(mut plain) = sets.first().cloned() {
            plain.source = None;
            sets.insert(1, plain);
        }
    }
    for mode in sets {
        frames.push(ViewerFrame::SetMode(mode));
    }
    if let Some(ceiling) = modes.rate_ceiling() {
        frames.push(ViewerFrame::RateCeiling(ceiling));
    }
    // The decoders of the viewer's last „Codec“/„Dekodierer“ (P1).
    if let Some(set) = modes.decoders() {
        frames.push(ViewerFrame::SetDecoders(set));
    }
    // „Monitor“/„Auflösung“ chosen in the session (P9); the viewer task drops
    // it for a host without `live`. Round 13 (H3b): again after the monitor
    // it was decided at.
    if let Some(choice) = display.filter(|_| last && (!carried || again)) {
        frames.push(ViewerFrame::SetDisplay(choice));
    }
    if let Some(on) = modes.audio().filter(|_| audio) {
        frames.push(ViewerFrame::Audio(on));
    }
    // A control the viewer released stays released, a taken one is taken.
    if let Some(action) = modes.control() {
        let request = crate::screen::core::wire_control::ControlRequest { action };
        frames.push(ViewerFrame::ControlRequest(request));
    }
    if let Some(report) = modes.visibility().filter(|_| visibility) {
        frames.push(ViewerFrame::Visibility(report));
    }
    frames
}
