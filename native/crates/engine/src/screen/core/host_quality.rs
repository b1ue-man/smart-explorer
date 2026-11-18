//! Requested quality survives temporary preview, lossless and cadence limits.
//! Merge W2 (bk7b-bericht U1): an encoder that fails for its codec gives way
//! to the next negotiated pairing of another video codec in rank order
//! (`codec_fallbacks`, e.g. HEVC → H.264) before the lossless tiles; the
//! viewer learns the codec from the header of the new picture group
//! (`wire_binary::GopHeader::codec`) and opens the matching decoder, as it
//! does for the switch to the tiles. A viewer of 0.5.196 names only H.264
//! and the tiles (`decoders`), so its session keeps H.264 → tiles.
//!
//! Plan 2026-11-18 „Netzrate“ (user rule of 2026-11-18): a failed encoder
//! first gives way to another encoder of the same codec (hardware, then
//! software; `codec_fallbacks` lists those pairings first), then, only under
//! „Automatisch“, to another video codec, and the tiles only when no lossy
//! encoder is left. A chosen codec (H.264, HEVC, AV1) never changes; „Scharf
//! (GPU)“ stays on the GPU encoders of H.264 and HEVC (`FallbackScope`).
//! Without an encoder of its scope the session ends with a clear error.
#[cfg(test)]
#[path = "batches_quality_task_tests.rs"]
mod batches_task_tests;
#[cfg(test)]
#[path = "bildkette_w2_codec_fallback_task_tests.rs"]
mod codec_fallback_tests;

#[cfg(test)]
#[path = "netzrate_task_tests.rs"]
mod netzrate_task_tests;

use super::{CodecChoice, CodecKind, FrameSize, Pipeline, StreamKind};
use crate::screen::core::first_picture::FirstPictureWatch;
use crate::screen::core::negotiate::CodecPreference;
use crate::screen::core::sharp_mode;

/// Complete coalesced state: omitting fps must not discard a pending rate
/// request, including a full -> preview -> full sequence during capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct QualityMode {
    pub kind: StreamKind,
    full_size: Option<FrameSize>,
    preview_size: Option<FrameSize>,
    pub full_fps: u32,
    pub preview_fps: u32,
    pub sharpness: bool,
    pub lossless: bool,
    /// The codec the viewer chose (`set_codec_preference`); a `SET_MODE`
    /// keeps it. „Automatisch“ until the opening names another.
    pub chosen: CodecPreference,
}
impl QualityMode {
    pub(super) fn update(
        mut self,
        kind: StreamKind,
        size: Option<FrameSize>,
        fps: Option<u16>,
        sharpness: bool,
        lossless: bool,
    ) -> Self {
        self.kind = kind;
        if let Some(size) = size {
            if kind == StreamKind::Preview {
                self.preview_size = Some(size);
            } else {
                self.full_size = Some(size);
            }
        }
        self.sharpness = sharpness;
        self.lossless = lossless;
        if let Some(fps) = fps {
            let fps = u32::from(fps).max(1);
            if kind == StreamKind::Preview {
                self.preview_fps = fps;
            } else {
                self.full_fps = fps;
            }
        }
        self
    }
    /// Equal to `other` apart from the wanted rates: a new rate alone keeps
    /// the encoder (plan 2026-11-14 „Bildkette“ B-03).
    pub(super) fn same_but_rate(&self, other: &Self) -> bool {
        let rates = Self {
            full_fps: other.full_fps,
            preview_fps: other.preview_fps,
            ..*self
        };
        rates == *other
    }
    pub(super) fn size(&self) -> Option<FrameSize> {
        if self.kind == StreamKind::Preview {
            self.preview_size
        } else {
            self.full_size
        }
    }
}

pub(super) struct QualityPolicy {
    pub mode: QualityMode,
    original: CodecChoice,
    /// The 4:4:4 pairing of the same codec and stages (plan 2026-11-14
    /// „Bildkette“ B-32), used under „Schärfe vor Flüssigkeit“.
    sharp: Option<CodecChoice>,
    viewer_fps: u32,
    /// A UI toggle cannot revive a codec that really failed to encode.
    pub encoder_failed: bool,
    /// The negotiated pairings of the other video codecs, best first
    /// (`codec_fallbacks`); a failed codec gives way to the first.
    pub(super) fallbacks: Vec<CodecChoice>,
    /// Video codecs whose encoder stages all failed in this session (W4, B3).
    pub(super) failed_codecs: Vec<CodecKind>,
    /// Silent answers before the first picture of a video encoder.
    pub(super) first_picture: FirstPictureWatch,
    /// A chosen codec lost its last pairing (`SET_DECODERS`): the next tick
    /// ends the session with this reason (plan 2026-11-18).
    pub(super) pending_error: Option<String>,
}

/// Which pairings may stand in for a failed encoder (plan 2026-11-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackScope {
    /// „Automatisch“: every video codec, the tiles last.
    Automatic,
    /// A chosen codec: its other encoders only, never the tiles.
    Codec(CodecKind),
    /// „Scharf (GPU)“: the GPU encoders of H.264 and HEVC only.
    Sharp,
}

impl FallbackScope {
    /// The scope of the viewer's choice; `sharpness` is „Scharf (GPU)“ as
    /// `SET_MODE` sends it.
    pub fn of(chosen: CodecPreference, sharpness: bool) -> Self {
        if sharpness || chosen == CodecPreference::Sharp {
            Self::Sharp
        } else {
            chosen.codec().map_or(Self::Automatic, Self::Codec)
        }
    }

    /// Whether `choice` may stand in for a failed encoder.
    pub fn admits(self, choice: &CodecChoice) -> bool {
        match self {
            Self::Automatic => choice.codec.is_video(),
            Self::Codec(codec) => choice.codec == codec,
            Self::Sharp => sharp_mode::sharp_codec(choice.codec) && choice.encoder_hardware,
        }
    }

    /// Whether the tiles may take over when no encoder of the scope is left.
    pub fn tiles_allowed(self) -> bool {
        self == Self::Automatic
    }

    /// The choice as the error names it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Codec(codec) => codec.key(),
            Self::Sharp => sharp_mode::SHARP_KEY,
        }
    }
}

/// Whether the stage tried the encoders of `b` when it failed for `a`: the
/// same codec, profile and limits (`CodecRegistry::matching_encoders` takes
/// every encoder of the codec within them, hardware first or software
/// first after the pairing's flag).
fn same_encoders(a: &CodecChoice, b: &CodecChoice) -> bool {
    a.codec == b.codec
        && a.profile == b.profile
        && a.max_macroblocks_per_second == b.max_macroblocks_per_second
        && a.max_size == b.max_size
        && a.max_pixels == b.max_pixels
}

/// The pairing that stands in for `failed` within `scope`, taken out of
/// `fallbacks` (their order: `codec_fallbacks`). `stage_tried`: the stage
/// already tried every encoder of `failed`'s limits (an encoder error);
/// pairings with the same limits are given up with it. A silent encoder
/// (`first_picture`) was not refused by the stage: the pairing of the other
/// stage opens the encoders in the other order. Pairings outside the scope
/// stay in the list.
pub fn next_fallback(
    fallbacks: &mut Vec<CodecChoice>,
    failed: &CodecChoice,
    stage_tried: bool,
    scope: FallbackScope,
) -> Option<CodecChoice> {
    fallbacks.retain(|choice| choice != failed && !(stage_tried && same_encoders(choice, failed)));
    let index = fallbacks.iter().position(|choice| scope.admits(choice))?;
    Some(fallbacks.remove(index))
}

/// The pairings a session may fall back to when the encoder of `choice`
/// fails, in this order (plan 2026-11-18): the other pairings of the same
/// codec and chroma with another encoder stage or other limits (hardware
/// before software, the negotiation's rank, B-30), then per other video
/// codec in rank order its 4:2:0 pairings likewise. Empty for a viewer that
/// named no other pairing (0.5.196: H.264 and tiles).
pub fn codec_fallbacks(choices: &[CodecChoice], choice: &CodecChoice) -> Vec<CodecChoice> {
    let key = |c: &CodecChoice| {
        (
            c.codec,
            c.encoder_hardware,
            c.max_macroblocks_per_second,
            c.max_size,
            c.max_pixels,
        )
    };
    let mut codecs = vec![choice.codec];
    for other in choices {
        if other.codec.is_video() && !codecs.contains(&other.codec) {
            codecs.push(other.codec);
        }
    }
    let mut seen = vec![key(choice)];
    let mut fallbacks = Vec::new();
    for codec in codecs {
        for other in choices.iter().filter(|other| other.codec == codec) {
            let chroma = if codec == choice.codec {
                choice.codec.chroma_444(choice.profile)
            } else {
                false
            };
            if other.codec.chroma_444(other.profile) != chroma || seen.contains(&key(other)) {
                continue;
            }
            seen.push(key(other));
            fallbacks.push(*other);
        }
    }
    fallbacks
}
impl QualityPolicy {
    pub(super) fn new(original: CodecChoice, viewer_fps: u32) -> Self {
        let viewer_fps = viewer_fps.clamp(1, super::super::pacing::MAX_FPS);
        Self {
            original,
            sharp: None,
            viewer_fps,
            encoder_failed: false,
            fallbacks: Vec::new(),
            failed_codecs: Vec::new(),
            first_picture: FirstPictureWatch::default(),
            pending_error: None,
            mode: QualityMode {
                kind: StreamKind::Video,
                full_size: None,
                preview_size: None,
                full_fps: viewer_fps,
                preview_fps: 1,
                sharpness: false,
                lossless: false,
                chosen: CodecPreference::Auto,
            },
        }
    }
    pub(super) fn max_fps(&self) -> u32 {
        let requested = if self.mode.kind == StreamKind::Preview {
            self.mode.preview_fps
        } else {
            self.mode.full_fps
        };
        // P9: sharpness (lossless) no longer caps the rate at 15 frames/s;
        // the hardware is the limit (encoding capacity, line, viewer).
        requested.clamp(1, self.viewer_fps)
    }
    /// Which pairings may stand in for a failed encoder.
    pub(super) fn scope(&self) -> FallbackScope {
        FallbackScope::of(self.mode.chosen, self.mode.sharpness)
    }
    pub(super) fn note_failed_codec(&mut self, codec: CodecKind) {
        if codec.is_video() && !self.failed_codecs.contains(&codec) {
            self.failed_codecs.push(codec);
        }
    }
}

impl Pipeline {
    fn tile_choice(&self) -> CodecChoice {
        CodecChoice {
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: self.tile_limit.unwrap_or_else(|| {
                self.frame_contract
                    .map_or(FrameSize::MAX, |c| c.limits.max_size)
            }),
            max_pixels: self
                .frame_contract
                .map_or(super::super::pixel_convert::MAX_FRAME_PIXELS, |c| {
                    c.limits.max_pixels
                }),
            max_macroblocks_per_second: None,
        }
    }

    /// Both choices were negotiated at session start. A temporary geometry
    /// limit may fall back again at the next tick; a real encoder error sticks.
    pub(super) fn quality_choice(&self) -> CodecChoice {
        let policy = &self.quality_policy;
        if policy.mode.lossless || policy.encoder_failed {
            self.tile_choice()
        } else if let Some(sharp) = policy.sharp.filter(|_| policy.mode.sharpness) {
            sharp
        } else {
            policy.original
        }
    }

    /// The negotiated 4:4:4 alternative (B-32); applies at the next mode.
    pub fn set_sharp_choice(&mut self, sharp: Option<CodecChoice>) {
        self.quality_policy.sharp = sharp.filter(|choice| choice.codec == self.choice.codec);
    }

    /// The pairings of the other video codecs (`codec_fallbacks`).
    pub fn set_codec_fallbacks(&mut self, fallbacks: Vec<CodecChoice>) {
        self.quality_policy.fallbacks = fallbacks;
    }

    /// The codec the viewer chose (plan 2026-11-18): at the opening, before
    /// the session's actor copies the mode. „Automatisch“ when not called.
    pub fn set_codec_preference(&mut self, chosen: CodecPreference) {
        self.quality_policy.mode.chosen = chosen;
    }

    /// The encoder of the session's pairing failed and the stage tried every
    /// encoder of its limits: the next pairing of the scope
    /// (`next_fallback`), with a new picture group (whose header names the
    /// codec); `None` when none is left (then the tiles under
    /// „Automatisch“, else an error).
    pub(super) fn fall_back_to_next_codec(&mut self) -> Option<CodecChoice> {
        self.fall_back_within(true)
    }

    /// The encoder opened but delivered no first picture (`first_picture`).
    pub(super) fn fall_back_after_silence(&mut self) -> Option<CodecChoice> {
        self.fall_back_within(false)
    }

    fn fall_back_within(&mut self, stage_tried: bool) -> Option<CodecChoice> {
        let failed = self.choice;
        let scope = self.quality_policy.scope();
        let policy = &mut self.quality_policy;
        policy.first_picture.restart();
        // A failed 4:4:4 pairing (B-32) or a pairing taken for the size gives
        // way to the session's own pairing first.
        if failed != policy.original
            && policy.original.codec.is_video()
            && !policy.fallbacks.contains(&policy.original)
        {
            policy.fallbacks.insert(0, policy.original);
        }
        let exhausted = policy.failed_codecs.clone();
        policy
            .fallbacks
            .retain(|choice| !exhausted.contains(&choice.codec));
        let next = next_fallback(&mut policy.fallbacks, &failed, stage_tried, scope);
        let same_left = next.is_some_and(|next| next.codec == failed.codec)
            || policy
                .fallbacks
                .iter()
                .any(|choice| choice.codec == failed.codec);
        if !same_left {
            policy.note_failed_codec(failed.codec);
        }
        let next = next?;
        policy.original = next;
        policy.sharp = None;
        if next.codec != failed.codec {
            self.pairing.on_codec_fallback(failed.codec, next);
        }
        self.choice = next;
        self.quality.size = self.fitted_size();
        self.invalidate_reference();
        Some(next)
    }

    /// Every fallback starts a complete GOP and a fresh presentation reference.
    pub(super) fn fall_back_to_tiles(&mut self) {
        self.choice = self.tile_choice();
        self.quality.size = self.fitted_size();
        self.invalidate_reference();
    }
}
