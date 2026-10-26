//! Requested quality survives temporary preview, lossless and cadence limits.
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
    pub(super) fn update(mut self, kind: StreamKind, size: Option<FrameSize>,
        fps: Option<u16>, sharpness: bool, lossless: bool) -> Self {
        self.kind = kind;
        if let Some(size) = size {
            if kind == StreamKind::Preview { self.preview_size = Some(size); }
            else { self.full_size = Some(size); }
        }
        self.sharpness = sharpness;
        self.lossless = lossless;
        if let Some(fps) = fps {
            let fps = u32::from(fps).max(1);
            if kind == StreamKind::Preview { self.preview_fps = fps; }
            else { self.full_fps = fps; }
        }
        self
    }
    pub(super) fn size(&self) -> Option<FrameSize> {
        if self.kind == StreamKind::Preview { self.preview_size }
        else { self.full_size }
    }
}

pub(super) struct QualityPolicy {
    pub mode: QualityMode,
    original: CodecChoice,
    viewer_fps: u32,
    /// A UI toggle cannot revive a codec that really failed to encode.
    pub encoder_failed: bool,
}
impl QualityPolicy {
    pub(super) fn new(original: CodecChoice, viewer_fps: u32) -> Self {
        let viewer_fps = viewer_fps.clamp(1, super::super::pacing::MAX_FPS);
        Self { original, viewer_fps, encoder_failed: false,
            mode: QualityMode { kind: StreamKind::Video, full_size: None, preview_size: None,
                full_fps: viewer_fps, preview_fps: 1, sharpness: false, lossless: false } }
    }
    pub(super) fn max_fps(&self) -> u32 {
        let requested = if self.mode.kind == StreamKind::Preview {
            self.mode.preview_fps
        } else { self.mode.full_fps };
        let limit = if self.mode.sharpness { self.viewer_fps.min(SHARPNESS_FPS) }
            else { self.viewer_fps };
        requested.clamp(1, limit)
    }
}

impl Pipeline {
    fn tile_choice(&self) -> CodecChoice {
        CodecChoice {
            codec: CodecKind::Tiles, profile: None,
            encoder_hardware: false, decoder_hardware: false,
            max_size: self.tile_limit.unwrap_or_else(|| self.frame_contract
                .map_or(FrameSize::MAX, |c| c.limits.max_size)),
            max_pixels: self.frame_contract.map_or(super::super::pixel_convert::MAX_FRAME_PIXELS,
                |c| c.limits.max_pixels),
            max_macroblocks_per_second: None,
        }
    }

    /// Both choices were negotiated at session start. A temporary geometry
    /// limit may fall back again at the next tick; a real encoder error sticks.
    pub(super) fn quality_choice(&self) -> CodecChoice {
        if self.quality_policy.mode.lossless || self.quality_policy.encoder_failed {
            self.tile_choice()
        } else { self.quality_policy.original }
    }

    /// Every fallback starts a complete GOP and a fresh presentation reference.
    pub(super) fn fall_back_to_tiles(&mut self) {
        self.choice = self.tile_choice();
        self.quality.size = self.fitted_size();
        self.invalidate_reference();
    }
}
