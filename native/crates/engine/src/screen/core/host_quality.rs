//! Requested quality survives temporary preview, lossless and cadence limits.
//! Merge W2 (bk7b-bericht U1): an encoder that fails for its codec gives way
//! to the next negotiated pairing of another video codec in rank order
//! (`codec_fallbacks`, e.g. HEVC → H.264) before the lossless tiles; the
//! viewer learns the codec from the header of the new picture group
//! (`wire_binary::GopHeader::codec`) and opens the matching decoder, as it
//! does for the switch to the tiles. A viewer of 0.5.196 names only H.264
//! and the tiles (`decoders`), so its session keeps H.264 → tiles.
#[cfg(test)]
#[path = "batches_quality_task_tests.rs"]
mod batches_task_tests;
#[cfg(test)]
#[path = "bildkette_w2_codec_fallback_task_tests.rs"]
mod codec_fallback_tests;

use super::{CodecChoice, CodecKind, FrameSize, Pipeline, StreamKind};

const SHARPNESS_FPS: u32 = 15;

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
}

/// The pairings a session may fall back to when the encoder of `choice`'s
/// codec fails: per other video codec its best pairing among `choices` (the
/// negotiation's rank order, B-30), 4:2:0 only, in that order. Empty for a
/// viewer that named no other video codec (0.5.196: H.264 and tiles).
pub fn codec_fallbacks(choices: &[CodecChoice], choice: &CodecChoice) -> Vec<CodecChoice> {
    let mut codecs = vec![choice.codec];
    let mut fallbacks = Vec::new();
    for other in choices {
        if !other.codec.is_video()
            || other.codec.chroma_444(other.profile)
            || codecs.contains(&other.codec)
        {
            continue;
        }
        codecs.push(other.codec);
        fallbacks.push(*other);
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
            mode: QualityMode {
                kind: StreamKind::Video,
                full_size: None,
                preview_size: None,
                full_fps: viewer_fps,
                preview_fps: 1,
                sharpness: false,
                lossless: false,
            },
        }
    }
    pub(super) fn max_fps(&self) -> u32 {
        let requested = if self.mode.kind == StreamKind::Preview {
            self.mode.preview_fps
        } else {
            self.mode.full_fps
        };
        let limit = if self.mode.sharpness {
            self.viewer_fps.min(SHARPNESS_FPS)
        } else {
            self.viewer_fps
        };
        requested.clamp(1, limit)
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

    /// The encoder of the session's codec failed: the next pairing of
    /// another video codec, with a new picture group (whose header names
    /// the codec); `None` when no other one was negotiated (then the tiles).
    pub(super) fn fall_back_to_next_codec(&mut self) -> Option<CodecChoice> {
        let failed = self.choice.codec;
        let policy = &mut self.quality_policy;
        policy.note_failed_codec(failed);
        let exhausted = &policy.failed_codecs;
        policy
            .fallbacks
            .retain(|choice| !exhausted.contains(&choice.codec));
        if policy.fallbacks.is_empty() {
            return None;
        }
        let next = policy.fallbacks.remove(0);
        policy.original = next;
        policy.sharp = None;
        self.pairing.on_codec_fallback(failed, next);
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
