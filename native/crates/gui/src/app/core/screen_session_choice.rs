//! What a session keeps of the viewer's „Codec“ (user rule of 2026-11-18:
//! „jede Einstellung, die der Nutzer wählt, wird genau so verwendet“).
//!
//! - The opening sends the stored choice: „Verlustfrei“ and „Scharf (GPU)“
//!   reach the host only through `SET_MODE` (`lossless`, `sharpness`), so a
//!   full view whose request names one of them sends that `SET_MODE` once
//!   the host accepted (`opening_mode`), as the Android viewer does.
//! - The viewer's own switch to the lossless tiles (a codec without
//!   picture, a hybrid picture the decoder cannot place) happens only under
//!   „Automatisch“ (and under „Verlustfrei“, which is that choice); any other
//!   choice keeps its codec and shows the error (`request_lossless` gives
//!   `None`). A new codec choice ends such a switch (`codec_chosen`), so the
//!   switch never overrules what the user chose later.
//! - Fix of 2026-11-18: a codec the viewer gave up under „Automatisch“ is
//!   out of this session only (`give_up_codec`); a new codec choice takes
//!   the session's give-ups back (`codec_chosen`).
//! - A session that began with the tiles cannot be moved to a video codec
//!   by the host (`SET_DECODERS` keeps its pairing): a new choice other than
//!   „Verlustfrei“ opens it again (`needs_reopen`).
use std::sync::atomic::Ordering;

use super::{lock, Shared, ViewerSession};
use crate::screen::core::decoder_health::GivenUp;
use crate::screen::core::negotiate::{CodecKind, CodecSetting};
use crate::screen::core::wire_control::{SetMode, StreamMode, ViewerFrame};

impl Shared {
    /// Keeps the mode of every `SET_MODE` going out; the viewer's switch to
    /// the tiles stays in it until a new codec choice.
    pub(crate) fn decoder_mode(&self, frame: &mut ViewerFrame) {
        if let ViewerFrame::SetMode(mode) = frame {
            mode.lossless |= self.lossless_fallback.load(Ordering::Acquire);
            *lock(&self.decoder_mode) = mode.clone();
        }
    }

    /// The `SET_MODE` of the viewer's switch to the tiles, once; `None`
    /// when it was asked before or the choice is a named codec.
    pub(crate) fn request_lossless(&self) -> Option<SetMode> {
        if !tiles_allowed(self.codec_choice()) {
            return None;
        }
        if self.lossless_fallback.swap(true, Ordering::AcqRel) {
            return None;
        }
        let mut mode = lock(&self.decoder_mode);
        mode.lossless = true;
        mode.source = None;
        Some(mode.clone())
    }

    /// Notes that this session gave `codec` up: the codecs given up so far
    /// when it is new, `None` when it was given up before.
    pub(crate) fn give_up_codec(&self, codec: CodecKind) -> Option<GivenUp> {
        let mut given_up = lock(&self.given_up);
        let new = given_up.insert(codec);
        new.then_some(*given_up)
    }

    /// The „Codec“ this session runs for.
    pub fn codec_choice(&self) -> CodecSetting {
        *lock(&self.codec)
    }

    /// The `SET_MODE` the opening sends after the host accepted: only for
    /// a full view whose codec choice needs it (`lossless`, `sharpness`).
    pub(crate) fn opening_mode(&self) -> Option<SetMode> {
        let mode = lock(&self.decoder_mode).clone();
        (mode.mode == StreamMode::Full && (mode.lossless || mode.sharpness)).then_some(mode)
    }

    /// The mode a session opens with: the request's kind, size and rate and
    /// the codec choice's two bits.
    pub(crate) fn set_opening(&self, mode: SetMode, codec: CodecSetting) {
        let full = mode.mode == StreamMode::Full;
        *lock(&self.decoder_mode) = SetMode {
            sharpness: full && codec.sharpness(),
            lossless: full && codec.lossless(),
            ..mode
        };
        *lock(&self.codec) = codec;
        let lossless = codec == CodecSetting::Lossless;
        self.local
            .opened_lossless
            .store(lossless, Ordering::Release);
    }
}

/// Whether the viewer may switch the session to the tiles itself: under
/// „Automatisch“, and under „Verlustfrei“, which chose them.
pub fn tiles_allowed(codec: CodecSetting) -> bool {
    matches!(codec, CodecSetting::Auto | CodecSetting::Lossless)
}

impl Shared {
    /// The host reported a stream of `codec`; the first one is kept.
    pub(crate) fn note_stream(&self, codec: CodecKind) {
        let _ = lock(&self.first_codec).get_or_insert(codec);
    }
}

/// Whether a session whose first stream was `first` must open again for
/// the choice `codec`: one that began with the tiles takes no video codec.
pub fn reopen_for(first: Option<CodecKind>, codec: CodecSetting) -> bool {
    first == Some(CodecKind::Tiles) && codec != CodecSetting::Lossless
}

impl ViewerSession {
    /// Whether the choice `codec` needs a new session (`reopen_for`). A
    /// session that runs for „Verlustfrei“ offers the tiles alone
    /// (`StreamRequest::chosen_decoders`), so it is a tile session before
    /// the host reported its first stream too (Codex 11: the choice went
    /// out as `SET_DECODERS` and the host ended the session).
    pub fn needs_reopen(&self, codec: CodecSetting) -> bool {
        // The choice at the opening counts, not the last one (U1: a video
        // session switched to „Verlustfrei“ by `SET_DECODERS` goes back to a
        // video codec the same way).
        let opened = &self.shared.local.opened_lossless;
        let lossless = opened.load(Ordering::Acquire);
        reopen_for(*lock(&self.shared.first_codec), codec)
            || (lossless && codec != CodecSetting::Lossless)
    }

    /// The host ended the session because it was opened for the tiles and a
    /// video codec was chosen (`set_decoders::TILES_SESSION`): the view opens
    /// it again with its source and choice (`ViewState::ensure_session`).
    pub fn reopen_wanted(&self) -> bool {
        self.shared.local.reopen.load(Ordering::Acquire)
    }

    /// A new „Codec“ was chosen: the viewer's earlier switch to the tiles
    /// and its give-ups of codecs end, and the choice counts for the
    /// session's later decisions. The caller sends the new `SET_MODE` after
    /// this.
    pub fn codec_chosen(&self, codec: CodecSetting) {
        self.shared
            .lossless_fallback
            .store(false, Ordering::Release);
        *lock(&self.shared.given_up) = GivenUp::default();
        *lock(&self.shared.codec) = codec;
    }
}

#[cfg(test)]
#[path = "wahlgilt_session_task_tests.rs"]
mod wahlgilt_session_task_tests;
