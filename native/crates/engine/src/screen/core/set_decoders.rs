//! `SET_DECODERS` (plan 2026-11-14 „Bildkette“ P1): a change of „Codec“ or
//! „Dekodierer“ within a running session.
//!
//! Before, `SET_MODE` had no codec field and no frame carried new decoder
//! offers, so the viewer opened a new session for every such change (BK6
//! E6); on an Android host that asked the system dialog again. With the
//! capability:
//!
//! - The host announces `set_decoders: Some(1)` in `SERVER_HELLO`. A viewer
//!   sends `SET_DECODERS` only then (`viewer_client_tasks.rs`): a host of
//!   v0.5.196, or of this batch before the frame, ends the session on an
//!   unknown frame (`decode_viewer_frame`). Without the capability the
//!   viewer opens the session again, as before.
//! - The frame replaces the viewer's decoder offers. Its „Codec“ and
//!   „Dekodierer“ travel the way they travel in the hello: as the offers
//!   that remain under the choice (`StreamRequest::chosen_decoders`, BK6
//!   E5), H.264, the tiles, HEVC and AV1 in one list (only a host of this
//!   version reads the frame, so the split of B-31 is not needed). It also
//!   carries the continuation announcement of the new decoders
//!   (`gop_continue.rs`, `ContinueOffers`).
//! - The host negotiates again with the ranking of the opening
//!   (`renegotiate`, the same `negotiate_for_contract` with
//!   `CodecPreference::Auto` as `host_server_start.rs`). A changed pairing
//!   starts a new picture group with a key frame in the new codec
//!   (`Pipeline::set_decoders`); an unchanged one changes nothing.
//! - Plan 2026-11-18 „Netzrate“: the ranking follows the viewer's choice as
//!   its decoders name it (`negotiate::preference_named_by`); a chosen codec
//!   negotiates its own pairings only (`negotiate_chosen`) and one the host
//!   lacks ends the session with the reason (`renegotiate_for`).
use std::io;

use serde::{Deserialize, Serialize};

use super::frame_budget::FrameContract;
use super::gop_continue::ContinueOffers;
use super::host_pipeline::codec_fallbacks;
use super::negotiate::{
    negotiate_chosen, negotiate_for_contract, preference_named_by, sharp_alternative,
    ChosenUnavailable, CodecChoice, CodecKind, CodecOffer, CodecPreference, CodecTrust,
    MAX_COMBINED_OFFERS,
};
use super::pixel_convert::FrameSize;
use super::wire::invalid;
use super::wire_hello::ScreenServerHello;

/// Viewer to host: JSON `SetDecoders`. The next free tag after
/// `RATE_CEILING` (50).
pub const SET_DECODERS: u8 = 51;
/// Version of `SET_DECODERS`, announced in `ScreenServerHello`.
pub const SET_DECODERS_V1: u16 = 1;

/// Whether the viewer may send `SET_DECODERS` to this host.
pub fn negotiated(server: &ScreenServerHello) -> bool {
    server.set_decoders == Some(SET_DECODERS_V1)
}

/// The viewer's decoders after a new „Codec“ or „Dekodierer“.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetDecoders {
    /// Every decoder the viewer offers under its choice, in its order.
    #[serde(default)]
    pub decoders: Vec<CodecOffer>,
    /// Every one of `decoders` keeps its references over a continuation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gop_continue: bool,
    /// The decoders that keep their references over a continuation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gop_continue_decoders: Vec<CodecOffer>,
}

impl SetDecoders {
    pub fn validate(&self) -> io::Result<()> {
        if self.decoders.len() > MAX_COMBINED_OFFERS {
            return Err(invalid("set decoders lists more than 64 decoders"));
        }
        if self.gop_continue_decoders.len() > MAX_COMBINED_OFFERS
            || self
                .gop_continue_decoders
                .iter()
                .any(|offer| !offer.codec.is_video())
        {
            return Err(invalid("set decoders names an invalid continuing decoder"));
        }
        Ok(())
    }

    /// The continuation announcement this frame carries.
    pub fn continuation(&self) -> ContinueOffers {
        ContinueOffers::new(self.gop_continue, self.gop_continue_decoders.clone())
    }
}

/// The pairing a host takes after `SET_DECODERS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub choice: CodecChoice,
    /// The 4:4:4 alternative of the same codec and stages (B-32).
    pub sharp: Option<CodecChoice>,
    /// Fallback pairings of the other negotiated video codecs (W2).
    pub fallbacks: Vec<CodecChoice>,
    /// The tile codec's size limit for the new offers.
    pub tile_limit: Option<FrameSize>,
    /// The viewer's choice its decoders name (plan 2026-11-18).
    pub preference: CodecPreference,
}

/// Negotiates the host's `encoders` against the viewer's new `decoders`
/// with the ranking of the opening. A session that opened with the tiles
/// keeps them (`video_session` false): its capture and output plan was made
/// for the tiles (`HostCodecPlan::for_session`); a video session may take
/// any pairing, the tiles included (its plan already carries the fallback
/// to the tiles). `None` when no pairing exists.
pub fn renegotiate(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    contract: Option<FrameContract>,
    video_session: bool,
) -> Option<Pairing> {
    renegotiate_trusted(encoders, decoders, contract, video_session, CodecTrust::ALL)
}

/// `renegotiate` with the host's trust of the newer codecs (D6, plan
/// 2026-11-17 P3).
pub fn renegotiate_trusted(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    contract: Option<FrameContract>,
    video_session: bool,
    trust: CodecTrust,
) -> Option<Pairing> {
    renegotiate_for(encoders, decoders, contract, video_session, trust)
        .ok()
        .flatten()
}

/// The reason a session opened for the tiles gives a chosen video codec:
/// the viewer opens the session anew with its choice.
pub const TILES_SESSION: &str = "the session was opened for the tiles; reopen it for this codec";

/// `renegotiate_trusted` under the viewer's choice (plan 2026-11-18): `Err`
/// names why a chosen codec has no pairing; `Ok(None)` when no pairing
/// exists for this session.
pub fn renegotiate_for(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    contract: Option<FrameContract>,
    video_session: bool,
    trust: CodecTrust,
) -> Result<Option<Pairing>, ChosenUnavailable> {
    // Review 2026-11-19: only a viewer that took `set_decoders_v1` sends
    // this frame, and such a viewer names the tiles under „Automatisch“ and
    // „Verlustfrei“; an empty offer is a chosen codec it has no decoder of
    // (`CodecPreference::filter_decoders`), read as „Automatisch“ it got the
    // tiles. A hello keeps the old reading: a viewer before codec
    // negotiation sends no decoders (`ScreenHello::decoders` defaults) and
    // gets the tiles as before.
    if decoders.is_empty() {
        return Err(ChosenUnavailable {
            preference: CodecPreference::Auto,
            reason: super::negotiate::EMPTY_OFFER.into(),
        });
    }
    let preference = preference_named_by(decoders);
    let choices = negotiate_chosen(encoders, decoders, preference, contract, trust)?;
    let Some(choice) = choices
        .iter()
        .copied()
        .find(|choice| video_session || choice.codec == CodecKind::Tiles)
    else {
        // Review 2026-11-19 (Codex 11): a session opened for the tiles
        // cannot take a chosen video codec (its output plan is the tiles'),
        // and keeping the tiles ignored the choice without a word.
        if !choices.is_empty() {
            return Err(ChosenUnavailable {
                preference,
                reason: TILES_SESSION.into(),
            });
        }
        return Ok(None);
    };
    let sharp = sharp_alternative(&choices, &choice);
    let fallbacks = if video_session {
        codec_fallbacks(&choices, &choice)
    } else {
        Vec::new()
    };
    let tile_limit = negotiate_for_contract(encoders, decoders, CodecPreference::Tiles, contract)
        .first()
        .map(|tiles| tiles.max_size);
    Ok(Some(Pairing {
        choice,
        sharp,
        fallbacks,
        tile_limit,
        preference,
    }))
}

#[cfg(test)]
#[path = "bildkette_proto_task_tests.rs"]
mod bildkette_proto_task_tests;
