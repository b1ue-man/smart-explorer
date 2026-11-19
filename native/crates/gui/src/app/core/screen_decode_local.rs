//! What the desktop viewer decides locally about its decoders (plan
//! 2026-11-14 „Bildkette“ V1):
//!
//! - The storage of a shown picture goes back to the decoder
//!   (`PicturePool`): the decoder hands its picture over instead of
//!   copying it (`ScreenDecoder::take_yuv_picture`, `take_picture`), the
//!   draw code gives the planes back when the GPU path dropped them
//!   (`YuvFrame`'s drop) or the BGRA picture right after its upload, and
//!   the decode thread passes them to the decoder before the next record
//!   (`recycle_*`). While size and format stay, no picture is allocated.
//! - B-36: the viewer announces `gop_continue` only when every decoder
//!   that may open for its offers keeps its references over a continuation
//!   (`CodecRegistry::keeps_references`), and names the offers whose
//!   decoders do in `gop_continue_decoders` (`continuing_offers`, P1): the
//!   host then continues when the negotiated decoder is one of them, so
//!   „Codec: Automatisch“ continues once HEVC or AV1 is negotiated.
//! - P1: a new „Codec“ or „Dekodierer“ goes out as `SET_DECODERS` when the
//!   host announced it (`ViewerSession::set_decoders`); otherwise the
//!   session opens again. P9: the offers come from the register at that
//!   moment (`current_offers`), so a new „Dekodierer-Bibliothek“ counts.
//! - „Dekodierer“ changes without a new session when a decoder of the new
//!   choice decodes the running stream (`CodecRegistry::decodes_with`):
//!   the decode thread asks for a key frame and opens the new decoder for
//!   it, the old one decodes until then. Otherwise the session opens again.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::screen_session::PictureData;
use crate::daemon::StreamRequest;
use crate::screen::core::codec_registry::CodecRegistry;
use crate::screen::core::decoder_health::GivenUp;
use crate::screen::core::host_backend::{Chroma, ScreenDecoder, YuvPicture};
use crate::screen::core::negotiate::{
    decoder_preference, CodecKind, CodecOffer, DecoderPreference,
};
use crate::screen::core::pixel_convert::{BgraImage, YuvMatrix};
use crate::screen::core::set_decoders::SetDecoders;
use crate::screen::core::wire_control::ViewerFrame;

/// Pictures kept per kind: one on screen and one in the slot cover the
/// steady state; more would only hold memory.
pub const KEPT: usize = 2;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Stored {
    yuv: Vec<YuvPicture>,
    bgra: Vec<BgraImage>,
}

/// Storage of shown pictures on its way back to the session's decoder.
#[derive(Clone, Default)]
pub struct PicturePool(Arc<Mutex<Stored>>);

impl PicturePool {
    pub fn give_yuv(&self, planes: YuvPicture) {
        let mut stored = lock(&self.0);
        if stored.yuv.len() < KEPT {
            stored.yuv.push(planes);
        }
    }

    pub fn give_bgra(&self, image: BgraImage) {
        let mut stored = lock(&self.0);
        if stored.bgra.len() < KEPT {
            stored.bgra.push(image);
        }
    }

    /// A picture that nobody showed (replaced in the slot).
    pub fn give(&self, data: PictureData) {
        match data {
            PictureData::Yuv(planes) => self.give_yuv(planes),
            PictureData::Bgra(image) => self.give_bgra(image),
        }
    }

    /// Hands everything given back to `decoder`; it keeps what fits.
    pub fn feed(&self, decoder: &mut dyn ScreenDecoder) {
        let (yuv, bgra) = {
            let mut stored = lock(&self.0);
            (
                std::mem::take(&mut stored.yuv),
                std::mem::take(&mut stored.bgra),
            )
        };
        for planes in yuv {
            decoder.recycle_yuv(planes);
        }
        for image in bgra {
            decoder.recycle_picture(image);
        }
    }

    /// Planes and BGRA pictures waiting (tests).
    #[cfg(test)]
    pub fn held(&self) -> (usize, usize) {
        let stored = lock(&self.0);
        (stored.yuv.len(), stored.bgra.len())
    }
}

/// Planes without storage: what a dropped `YuvFrame` leaves behind.
pub fn empty_planes() -> YuvPicture {
    YuvPicture {
        y: Vec::new(),
        y_stride: 0,
        uv: Vec::new(),
        uv_stride: 0,
        chroma: Chroma::Nv12,
        matrix: YuvMatrix::Bt709,
    }
}

/// The session's share of this module (`Shared::local`).
#[derive(Default)]
pub struct DecodeLocal {
    pub pool: PicturePool,
    /// Counts the „Dekodierer“ choices taken over without a new session.
    pub switches: AtomicU64,
    /// The host announced `set_decoders_v1` (`StreamEvent::Accepted`, P1).
    pub set_decoders: AtomicBool,
    /// The session announced a continuation for some decoder (B-36): the
    /// decode thread then waits for the host's next stream after `Broken`.
    pub continuation: AtomicBool,
    /// The session's number for the decoder libraries' memory
    /// (`decoder_health::begin_session`, entered by its decode thread).
    pub epoch: AtomicU64,
    /// The host ended the session for a video codec chosen in a tile
    /// session (`ViewerSession::reopen_wanted`).
    pub reopen: AtomicBool,
}

impl DecodeLocal {
    /// Puts `picture` into the session's slot; one nobody took goes back.
    pub fn replace(
        &self,
        slot: &Mutex<Option<super::screen_session::Picture>>,
        picture: super::screen_session::Picture,
    ) {
        if let Some(unshown) = lock(slot).replace(picture) {
            self.pool.give(unshown.data);
        }
    }
}

/// The decoders of this process (`Backend::production`); without it (task
/// tests with fake decoders) nothing is announced and every decoder choice
/// opens the session again.
static REGISTRY: Mutex<Option<Arc<CodecRegistry>>> = Mutex::new(None);

pub fn install_registry(registry: Arc<CodecRegistry>) {
    *lock(&REGISTRY) = Some(registry);
}

fn registry() -> Option<Arc<CodecRegistry>> {
    lock(&REGISTRY).clone()
}

/// `SET_DECODERS` with the decoders this process offers without the codecs
/// the session gave up (`given_up`), after a codec gave no picture
/// (`screen_decode_guard.rs`); `None` without the register (task tests with
/// fake decoders).
pub fn usable_decoders_frame(given_up: GivenUp) -> Option<SetDecoders> {
    registry().map(|registry| {
        super::screen_pool::decoders_frame_without(&registry.decoder_offers(), given_up)
    })
}

/// P9: the decoders this process offers now. „Dekodierer-Bibliothek“
/// changes them while the viewer runs (the backend's offers are those of
/// the viewer's start), so every request and `SET_DECODERS` asks the
/// register; without it (task tests with fake decoders) `fallback`.
pub fn current_offers(fallback: &[CodecOffer]) -> Vec<CodecOffer> {
    registry().map_or_else(|| fallback.to_vec(), |registry| registry.decoder_offers())
}

/// B-36: whether `request` may announce `gop_continue`.
pub fn announces(request: &StreamRequest) -> bool {
    registry().is_some_and(|registry| announces_with(&registry, request))
}

pub fn announces_with(registry: &CodecRegistry, request: &StreamRequest) -> bool {
    registry.keeps_references(
        &request.chosen_decoders(),
        request.decoder.unwrap_or_default(),
    )
}

/// B-36 per codec: the offers of `request` whose decoders continue.
pub fn continuing(request: &StreamRequest) -> Vec<CodecOffer> {
    registry().map_or_else(Vec::new, |registry| continuing_with(&registry, request))
}

pub fn continuing_with(registry: &CodecRegistry, request: &StreamRequest) -> Vec<CodecOffer> {
    registry.continuing_offers(
        &request.chosen_decoders(),
        request.decoder.unwrap_or_default(),
    )
}

impl super::screen_session::ViewerSession {
    pub(super) fn picture_pool(&self) -> PicturePool {
        self.shared.local.pool.clone()
    }

    /// The BGRA picture after its upload.
    pub(super) fn recycle_bgra(&self, image: BgraImage) {
        self.shared.local.pool.give_bgra(image);
    }

    /// P1: the new decoders of a „Codec“ or „Dekodierer“ choice as
    /// `SET_DECODERS`, when the host announced it; `false` when the session
    /// must open again. The host negotiates again and starts a group in a
    /// changed codec; a changed decoder of the same codec takes over at the
    /// next key frame (`switch_decoder`).
    pub fn set_decoders(&self, frame: SetDecoders) -> bool {
        if !self.shared.local.set_decoders.load(Ordering::Acquire) {
            return false;
        }
        let continues = frame.gop_continue || !frame.gop_continue_decoders.is_empty();
        self.shared
            .local
            .continuation
            .store(continues, Ordering::Release);
        self.send(ViewerFrame::SetDecoders(frame));
        true
    }

    /// V1: takes a new „Dekodierer“ choice (already the process's,
    /// `screen_prefs::set_codec_choice`) over without a new session;
    /// `false` when the session must open again.
    pub fn switch_decoder(&self) -> bool {
        registry().is_some_and(|registry| self.switch_decoder_with(&registry, decoder_preference()))
    }

    pub fn switch_decoder_with(
        &self,
        registry: &CodecRegistry,
        preference: DecoderPreference,
    ) -> bool {
        let takes_over = {
            let info = self.shared.info();
            let Some(stream) = info.stream.as_ref() else {
                return false;
            };
            if stream.codec == CodecKind::Tiles {
                // No video decoder runs; the next one opens under the choice.
                return true;
            }
            let contract = info.hello.as_ref().and_then(|hello| hello.frame_contract);
            registry.decodes_with(
                (stream.codec, stream.profile),
                stream.size,
                contract,
                preference,
            )
        };
        if takes_over {
            self.shared.local.switches.fetch_add(1, Ordering::AcqRel);
        }
        takes_over
    }
}

#[cfg(test)]
#[path = "bildkette_viewer_announce_task_tests.rs"]
mod bildkette_viewer_announce_task_tests;
