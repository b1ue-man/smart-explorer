//! Codec negotiation v2 and `rate_ceiling_v1` in the hellos (plan
//! 2026-11-14 „Bildkette“ B-31, BK6).
//!
//! A peer of v0.5.196 deserializes `decoders` and `encoders` into its
//! `CodecKind { H264, Tiles }`, which has no `#[serde(other)]`: one HEVC or
//! AV1 entry there would fail its whole hello. So the old lists keep only
//! H.264 and the tiles, in both roles and always. The host announces
//! `codecs_v2: Some(1)` in `SERVER_HELLO`; only then does the viewer list its
//! HEVC and AV1 decoders in the new field `decoders_v2`, and only for a
//! viewer that sent that field does the host list its HEVC and AV1 encoders
//! in `encoders_v2` of `HELLO_OK`. Serde ignores fields a struct does not
//! name (none of the hellos has `deny_unknown_fields`), so an old peer reads
//! a new hello as it read one of v0.5.196.
//!
//! `rate_ceiling_v1`: the host announces `rate_ceiling: Some(1)`; a viewer
//! sends `RATE_CEILING` only then, because an unknown viewer frame ends the
//! session at an old host (P6 report „Entscheidungen“).
use std::io;

use super::super::negotiate::{CodecOffer, MAX_CODEC_OFFERS};
use super::super::wire::invalid;
use super::{ScreenClientHello, ScreenHelloOk, ScreenServerHello};

/// Version of the v2 codec lists, announced in `ScreenServerHello`.
pub const CODECS_V2: u16 = 1;
/// Version of `RATE_CEILING`, announced in `ScreenServerHello`.
pub const RATE_CEILING_V1: u16 = 1;

/// The offers a peer of v0.5.196 knows, and the others.
pub(super) fn split(all: &[CodecOffer]) -> (Vec<CodecOffer>, Vec<CodecOffer>) {
    all.iter().partition(|offer| offer.codec.is_legacy())
}

/// A v2 list: at most `MAX_CODEC_OFFERS`, HEVC and AV1 only.
pub(super) fn validate_v2(list: Option<&[CodecOffer]>) -> io::Result<()> {
    let Some(list) = list else {
        return Ok(());
    };
    if list.len() > MAX_CODEC_OFFERS {
        return Err(invalid("screen hello lists more than 32 v2 codecs"));
    }
    if list.iter().any(|offer| offer.codec.is_legacy()) {
        return Err(invalid("a v2 codec list repeats a legacy codec"));
    }
    Ok(())
}

/// `gop_continue_decoders` (B-36 per codec): at most one entry per offer of
/// both lists, video codecs only (the tiles have no reference chain).
pub(super) fn validate_continuing(list: Option<&[CodecOffer]>) -> io::Result<()> {
    let Some(list) = list else {
        return Ok(());
    };
    if list.len() > super::super::negotiate::MAX_COMBINED_OFFERS {
        return Err(invalid(
            "screen hello lists more than 64 continuing decoders",
        ));
    }
    if list.iter().any(|offer| !offer.codec.is_video()) {
        return Err(invalid("a continuing decoder must be a video decoder"));
    }
    Ok(())
}

/// Whether the viewer may send `RATE_CEILING` to this host.
pub fn rate_ceiling_negotiated(server: &ScreenServerHello) -> bool {
    server.rate_ceiling == Some(RATE_CEILING_V1)
}

impl ScreenServerHello {
    /// Whether this host reads `decoders_v2`.
    pub fn codecs_v2(&self) -> bool {
        self.codecs_v2 == Some(CODECS_V2)
    }
}

impl ScreenClientHello {
    /// Places the viewer's decoders: H.264 and the tiles in `decoders`,
    /// HEVC and AV1 in `decoders_v2` only for a host with `codecs_v2`.
    pub fn offer_decoders(&mut self, all: &[CodecOffer], server: &ScreenServerHello) {
        let (legacy, v2) = split(all);
        self.decoders = legacy;
        self.decoders_v2 = server
            .codecs_v2()
            .then(|| v2.into_iter().take(MAX_CODEC_OFFERS).collect());
    }

    /// Every decoder the viewer named, the legacy list first.
    pub fn all_decoders(&self) -> Vec<CodecOffer> {
        let mut all = self.decoders.clone();
        all.extend(self.decoders_v2.iter().flatten().copied());
        all
    }

    /// The viewer's choice: `codec_choice` where it names one, else as its
    /// decoders name it (`negotiate::preference_of`).
    pub fn codec_preference(&self) -> super::super::negotiate::CodecPreference {
        super::super::negotiate::preference_of(self.codec_choice, &self.all_decoders())
    }
}

impl ScreenHelloOk {
    /// Places the host's encoders: H.264 and the tiles in `encoders`, HEVC
    /// and AV1 in `encoders_v2` only for a viewer that sent `decoders_v2`.
    pub fn offer_encoders(&mut self, all: &[CodecOffer], viewer: &ScreenClientHello) {
        let (legacy, v2) = split(all);
        self.encoders = legacy;
        self.encoders_v2 = viewer
            .decoders_v2
            .is_some()
            .then(|| v2.into_iter().take(MAX_CODEC_OFFERS).collect());
    }

    /// Every encoder the host named, the legacy list first.
    pub fn all_encoders(&self) -> Vec<CodecOffer> {
        let mut all = self.encoders.clone();
        all.extend(self.encoders_v2.iter().flatten().copied());
        all
    }
}

#[cfg(test)]
#[path = "bildkette_codec_task_tests.rs"]
mod bildkette_codec_task_tests;
