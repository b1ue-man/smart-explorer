//! The decode thread's watch over the negotiated codec (fix of 2026-11-17:
//! a Windows viewer stayed black while the Windows host showed its
//! indicator). A codec that brings no picture (`DecoderGuard`,
//! `decoder_health`) is given up for this session: a host that announced
//! `set_decoders_v1` negotiates again without it (`SET_DECODERS`, P1), any
//! other host switches to the lossless tiles (`SET_MODE`, as the hybrid
//! fallback does). The window names the change with its reason. A picture
//! that came out of a later decoder call than its own is a picture; only
//! late pictures in a row give the codec up (plan 2026-11-17 „Windows-Bild“
//! P3, `DecoderGuard::on_picture`).
//!
//! Fix of 2026-11-18 („Kacheln immer“, v0.5.200): the give-up was a mark for
//! the whole process, so a long-running viewer offered fewer codecs in every
//! later session until only the tiles were left. It now lasts for the
//! session (`Shared::give_up_codec`); a new session, or a new codec choice,
//! offers every codec again. Every decision („Codec aufgegeben“, „Kachel-
//! Rückfall“, a named codec kept) goes with its reason to the viewer log
//! (`host_log::viewer_note`, `<app data>/logs/screen-viewer.log`).
//!
//! User rule of 2026-11-18: only „Codec: Automatisch“ gives a codec up and
//! moves on. Under a named codec the session keeps it and the window shows
//! the error with the guard's reason (`named_text`); under „Verlustfrei“
//! the switch to the tiles is that choice.
use std::sync::atomic::Ordering;
use std::time::Instant;

use super::DecodeState;
use crate::screen::core::negotiate::{CodecKind, CodecSetting};
use crate::screen::core::wire_control::ViewerFrame;

/// A line of the viewer log; task tests write no log file.
fn note(text: &str) {
    if !cfg!(test) {
        crate::screen::os::host_worker_client::host_log::viewer_note(text);
    }
}

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

/// The window's text when „Automatisch“ gives `codec` up for the session.
pub fn given_up_text(codec: CodecKind, reason: &str, tiles: bool) -> String {
    if tiles {
        format!(
            "{} liefert auf diesem Gerät kein Bild ({reason}). Die Verbindung wechselt für \
             diese Sitzung zur verlustfreien Kachelübertragung.",
            label(codec)
        )
    } else {
        format!(
            "{} liefert auf diesem Gerät kein Bild ({reason}). Die Verbindung wechselt für \
             diese Sitzung den Codec.",
            label(codec)
        )
    }
}

/// The viewer-log line of a codec given up under „Automatisch“: the codecs
/// the session negotiates with next (`None`: the switch to the tiles).
pub fn given_up_line(codec: CodecKind, reason: &str, next: Option<&[CodecKind]>) -> String {
    match next {
        Some(codecs) => {
            let keys: Vec<&str> = codecs.iter().map(|codec| codec.key()).collect();
            format!(
                "Codec aufgegeben für diese Sitzung: {} ({reason}); neu verhandelt mit [{}]",
                codec.key(),
                keys.join(", ")
            )
        }
        None => format!(
            "Kachel-Rückfall dieser Sitzung: {} ({reason}); der Gastgeber verhandelt nicht neu",
            codec.key()
        ),
    }
}

impl DecodeState {
    /// A choice made since the last event starts the watch afresh.
    fn renew_guard(&mut self) {
        let choice = self.shared.local.choices.load(Ordering::Acquire);
        self.guard.renew(choice);
    }

    pub(super) fn guard_record(&mut self, codec: CodecKind) {
        self.renew_guard();
        if let Some(codec) = self.guard.on_record(codec, Instant::now()) {
            self.give_up(codec);
        }
    }

    pub(super) fn guard_failure(&mut self, codec: CodecKind) {
        self.renew_guard();
        if let Some(codec) = self.guard.on_failure(codec, Instant::now()) {
            self.give_up(codec);
        }
    }

    /// A decoded picture of `codec`; `lag` pictures submitted after it are
    /// still in the decoder.
    pub(super) fn guard_picture(&mut self, codec: CodecKind, lag: usize) {
        self.renew_guard();
        if let Some(codec) = self.guard.on_picture(codec, Instant::now(), lag) {
            self.give_up(codec);
        }
    }

    pub(super) fn guard_tick(&mut self) {
        self.renew_guard();
        if let Some(codec) = self.guard.on_tick(Instant::now()) {
            self.give_up(codec);
        }
    }

    fn give_up(&mut self, codec: CodecKind) {
        let reason = self.guard.reason();
        let _ = self.leave_codec(codec, &reason);
    }

    /// `codec` brings no usable picture for `reason`: under „Automatisch“
    /// the session leaves it, under „Verlustfrei“ it goes to the tiles it
    /// chose, under a named codec the error is shown and the codec stays.
    /// `true` when a `SET_DECODERS` or `SET_MODE` went out (the host starts
    /// a new stream; no key frame of the old one is asked for).
    /// No other decoder of the chosen „Dekodierer“/„Dekodier-Bibliothek“:
    /// the tiles are not taken, the error is shown (Codex 5/13, 6/13).
    fn hardware_stays(&mut self, codec: CodecKind, reason: &str) {
        let decoder = super::super::screen_prefs::decoder_choice();
        let library = super::super::screen_prefs::decoder_library_choice();
        let text = format!(
            "{} liefert auf diesem Gerät kein Bild ({reason}). Gewählt sind „Dekodierer: {}“ \
             und „Dekodier-Bibliothek: {}“, kein anderer passender Dekodierer bleibt; die \
             Kacheln werden nicht genommen (nur „Automatisch“ weicht aus).",
            label(codec),
            decoder.label(),
            library.label()
        );
        self.shared.update(|info| info.notice = Some(text));
    }

    pub(super) fn leave_codec(&mut self, codec: CodecKind, reason: &str) -> bool {
        let chosen = self.shared.codec_choice();
        if chosen == CodecSetting::Lossless {
            // „Verlustfrei“ chose the tiles: that is no fallback.
            let Some(mode) = self.shared.request_lossless() else {
                return false;
            };
            note(&format!(
                "{} ohne Bild ({reason}); „Verlustfrei“ gewählt: Kachelübertragung",
                codec.key()
            ));
            if (self.send)(&ViewerFrame::SetMode(mode)).is_err() {
                self.connection_lost();
            }
            return true;
        }
        if chosen != CodecSetting::Auto {
            if self.reported.insert(codec) {
                note(&format!(
                    "{} ohne Bild ({reason}); gewählter Codec „{}“ bleibt",
                    codec.key(),
                    chosen.label()
                ));
            }
            let text = named_text(codec, chosen, reason);
            self.shared.update(|info| info.notice = Some(text));
            return false;
        }
        let Some(given_up) = self.shared.give_up_codec(codec) else {
            return false;
        };
        if self.shared.local.set_decoders.load(Ordering::Acquire) {
            let frame = super::super::screen_decode_local::usable_decoders_frame(given_up);
            if let Some(frame) = frame {
                let continues = frame.gop_continue || !frame.gop_continue_decoders.is_empty();
                let mut next: Vec<CodecKind> = Vec::new();
                for offer in frame.decoders.iter().filter(|offer| offer.codec.is_video()) {
                    if !next.contains(&offer.codec) {
                        next.push(offer.codec);
                    }
                }
                let tiles = next.is_empty();
                if tiles && super::super::screen_session::hardware_bars_tiles(chosen) {
                    self.hardware_stays(codec, reason);
                    return false;
                }
                note(&given_up_line(codec, reason, Some(&next)));
                if tiles {
                    note(&format!(
                        "Kachel-Rückfall dieser Sitzung: alle Video-Codecs aufgegeben ({})",
                        given_up.keys().join(", ")
                    ));
                }
                self.shared
                    .local
                    .continuation
                    .store(continues, Ordering::Release);
                if (self.send)(&ViewerFrame::SetDecoders(frame)).is_err() {
                    self.connection_lost();
                    return true;
                }
                let text = given_up_text(codec, reason, tiles);
                self.shared.update(|info| info.notice = Some(text));
                return true;
            }
        }
        if super::super::screen_session::hardware_bars_tiles(chosen) {
            // The local switch to the tiles is barred too (H4: it fell
            // silent then).
            self.hardware_stays(codec, reason);
            return false;
        }
        let Some(mode) = self.shared.request_lossless() else {
            return false;
        };
        note(&given_up_line(codec, reason, None));
        let text = given_up_text(codec, reason, true);
        self.shared.update(|info| info.notice = Some(text));
        if (self.send)(&ViewerFrame::SetMode(mode)).is_err() {
            self.connection_lost();
        }
        true
    }
}

#[cfg(test)]
#[path = "wahlgilt_guard_task_tests.rs"]
mod wahlgilt_guard_task_tests;

#[cfg(test)]
#[path = "kachelfix_guard_task_tests.rs"]
mod kachelfix_guard_task_tests;
