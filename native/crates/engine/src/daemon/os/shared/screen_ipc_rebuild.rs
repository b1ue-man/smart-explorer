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
use super::ModeMemory;
use crate::screen::core::viewer_client::ViewerRequest;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ViewerFrame;

fn monitor(source: &SourceId) -> bool {
    matches!(source, SourceId::Monitor(_))
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
            // family follows by `SET_MODE` (`frames`).
            request.source = modes.source().filter(monitor);
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

/// What the viewer chose since the first opening, in the order to send it
/// to a session opened with `opened` (the new one starts from that request):
/// its modes, ceiling, decoders, display, sound (`audio`: the host answered
/// it), control and visibility (`visibility`: the host reads it, with a
/// frame contract as the first session's did; Gemini 3/9).
pub(super) fn frames(
    modes: &ModeMemory,
    opened: &ViewerRequest,
    audio: bool,
    visibility: bool,
) -> Vec<ViewerFrame> {
    let display = modes.display();
    // H3: a display the hello carried is decided already.
    let carried = display.is_some() && opened.virtual_display == display;
    let last = modes.display_last();
    let mut frames = Vec::new();
    // Codex 11/4: a display choice older than the last source goes first,
    // so it never undoes a later window choice.
    if let Some(choice) = display.filter(|_| !last && !carried) {
        frames.push(ViewerFrame::SetDisplay(choice));
    }
    // A display chosen after the source and carried by the hello put the
    // session at its monitor already: the older source does not move it.
    let with_source = !(last && carried);
    for mode in modes.replay(with_source) {
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
    // it for a host without `live`.
    if let Some(choice) = display.filter(|_| last && !carried) {
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
