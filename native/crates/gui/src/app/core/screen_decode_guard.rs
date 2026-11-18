//! The decode thread's watch over the negotiated codec (fix of 2026-11-17:
//! a Windows viewer stayed black while the Windows host showed its
//! indicator). A codec that brings no picture (`DecoderGuard`,
//! `decoder_health`) is given up for this process: a host that announced
//! `set_decoders_v1` negotiates again without it (`SET_DECODERS`, P1), any
//! other host switches to the lossless tiles (`SET_MODE`, as the hybrid
//! fallback does). The window names the change. A picture that came out of
//! a later decoder call than its own counts as a failure (plan 2026-11-17
//! „Windows-Bild“ P3, `DecoderGuard::on_picture`).
//!
//! User rule of 2026-11-18: only „Codec: Automatisch“ gives a codec up and
//! moves on. Under a named codec the session keeps it and the window shows
//! the error with the guard's reason (`named_text`); under „Verlustfrei“
//! the switch to the tiles is that choice and marks nothing.
use std::sync::atomic::Ordering;
use std::time::Instant;

use super::DecodeState;
use crate::screen::core::decoder_health;
use crate::screen::core::negotiate::{CodecKind, CodecSetting};
use crate::screen::core::wire_control::ViewerFrame;

fn label(codec: CodecKind) -> &'static str {
    match codec {
        CodecKind::H264 => "H.264",
        CodecKind::Hevc => "HEVC",
        CodecKind::Av1 => "AV1",
        CodecKind::Tiles => "Kacheln",
    }
}

/// The error of a named codec that brings no picture: the codec stays.
pub fn named_text(codec: CodecKind, chosen: CodecSetting, reason: &str) -> String {
    format!(
        "{} liefert auf diesem Gerät kein Bild ({reason}). Gewählt ist „{}“; die Verbindung \
         wechselt nicht, nur „Codec: Automatisch“ weicht auf einen anderen Codec aus.",
        label(codec),
        chosen.label()
    )
}

impl DecodeState {
    pub(super) fn guard_record(&mut self, codec: CodecKind) {
        if let Some(codec) = self.guard.on_record(codec, Instant::now()) {
            self.give_up(codec);
        }
    }

    pub(super) fn guard_failure(&mut self, codec: CodecKind) {
        if let Some(codec) = self.guard.on_failure(codec, Instant::now()) {
            self.give_up(codec);
        }
    }

    /// A decoded picture of `codec`; `lag` pictures submitted after it are
    /// still in the decoder.
    pub(super) fn guard_picture(&mut self, codec: CodecKind, lag: usize) {
        if let Some(codec) = self.guard.on_picture(codec, Instant::now(), lag) {
            self.give_up(codec);
        }
    }

    pub(super) fn guard_tick(&mut self) {
        if let Some(codec) = self.guard.on_tick(Instant::now()) {
            self.give_up(codec);
        }
    }

    fn give_up(&mut self, codec: CodecKind) {
        let chosen = self.shared.codec_choice();
        if chosen != CodecSetting::Auto {
            let reason = self.guard.reason();
            eprintln!(
                "screen viewer: {} gave no picture ({reason}); chosen codec kept",
                label(codec)
            );
            if chosen == CodecSetting::Lossless {
                // „Verlustfrei“ chose the tiles: that is no fallback.
                if let Some(mode) = self.shared.request_lossless() {
                    if (self.send)(&ViewerFrame::SetMode(mode)).is_err() {
                        self.connection_lost();
                    }
                }
                return;
            }
            let text = named_text(codec, chosen, &reason);
            self.shared.update(|info| info.notice = Some(text));
            return;
        }
        decoder_health::mark_unusable(codec);
        eprintln!(
            "screen viewer: {} gave no picture; codec given up",
            label(codec)
        );
        if self.shared.local.set_decoders.load(Ordering::Acquire) {
            if let Some(frame) = super::super::screen_decode_local::usable_decoders_frame() {
                let continues = frame.gop_continue || !frame.gop_continue_decoders.is_empty();
                self.shared
                    .local
                    .continuation
                    .store(continues, Ordering::Release);
                if (self.send)(&ViewerFrame::SetDecoders(frame)).is_err() {
                    self.connection_lost();
                    return;
                }
                let text = format!(
                    "{} liefert auf diesem Gerät kein Bild. Die Verbindung wechselt den Codec.",
                    label(codec)
                );
                self.shared.update(|info| info.notice = Some(text));
                return;
            }
        }
        if let Some(mode) = self.shared.request_lossless() {
            let text = format!(
                "{} liefert auf diesem Gerät kein Bild. Die Verbindung wechselt zur verlustfreien Kachelübertragung.",
                label(codec)
            );
            self.shared.update(|info| info.notice = Some(text));
            if (self.send)(&ViewerFrame::SetMode(mode)).is_err() {
                self.connection_lost();
            }
        }
    }
}

#[cfg(test)]
#[path = "wahlgilt_guard_task_tests.rs"]
mod wahlgilt_guard_task_tests;
