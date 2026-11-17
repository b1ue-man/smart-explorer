//! `screen.viewSetDecoders {session, decoders, codec?, decoder?}` →
//! `{"switched": bool}` (plan 2026-11-14 „Bildkette“ P1): a new „Codec“ or
//! „Dekodierer“ of a running session without a new session.
//!
//! The fields are those of `screen.viewOpen` (`screen_view_codecs.rs`):
//! `decoders[]` of the app, `codec` and `decoder` of the viewer. With a host
//! that announced `set_decoders_v1` (the event `accepted` names it as
//! `setDecoders`) the core sends `SET_DECODERS` with the offers that remain
//! under the choice, and the host starts a picture group in a changed codec
//! (the event `stream` names it); the session, and on an Android host the
//! consent to share its screen, stay. `switched: false` means the host does
//! not take the frame (v0.5.196 or older), and the app opens the session
//! again as before. „Verlustfrei“ stays the `lossless` of
//! `screen.viewSetMode`. The Android sink opens a decoder per picture group
//! (`viewer_sink.rs`), so the frame announces no continuation.
use serde_json::{json, Value};

use super::codecs::{codec_args, decoders_arg};
use super::session_arg;
use crate::daemon::StreamRequest;
use crate::mobile::ApiError;
use crate::screen::core::wire_control::ViewerFrame;

pub(super) fn set_decoders(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    if !session
        .set_decoders
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(json!({ "switched": false }));
    }
    let (codec, decoder) = codec_args(args)?;
    let request = StreamRequest {
        expected_node: None,
        mode: session.mode,
        source: None,
        decoders: decoders_arg(args, session.mode)?,
        display_size: None,
        frame_capabilities: None,
        max_fps: 1,
        max_rate_bps: None,
        codec,
        decoder,
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        virtual_display: None,
    };
    session.send(&ViewerFrame::SetDecoders(request.decoders_frame()))?;
    Ok(json!({ "switched": true }))
}
