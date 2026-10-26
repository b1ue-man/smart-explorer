//! Portable image geometry and derived resource/packet bounds.
use super::wire::invalid;
use serde::{Deserialize, Serialize};
use std::io;

/// Preserved v1 side bound; versioned capabilities may establish a larger limit.
pub(crate) const MAX_FRAME_SIDE: u32 = 4096;
/// Preserved v1 pixel budget: 4096 × 2304 = 36 864 AVC macroblocks.
pub(crate) const MAX_FRAME_PIXELS: u64 = 4096 * 2304;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct FrameSize {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    /// Largest output in landscape orientation (C2 „Grenzen mit Grund“).
    pub(crate) const MAX: Self = Self::new(4096, 2304);

    pub(crate) const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub(crate) fn pixels(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// A frame that may be allocated for data from the network.
    pub(crate) fn is_valid_frame(self) -> bool {
        (1..=MAX_FRAME_SIDE).contains(&self.width)
            && (1..=MAX_FRAME_SIDE).contains(&self.height)
            && self.pixels() <= MAX_FRAME_PIXELS
    }

    pub(crate) fn check_frame(self) -> io::Result<()> {
        if self.is_valid_frame() {
            Ok(())
        } else {
            Err(invalid(
                "screen frame size is outside 1..=4096 or above 4096 x 2304 pixels",
            ))
        }
    }

    /// Bytes of a tight BGRA frame; only meaningful for valid frames.
    pub(crate) fn bgra_len(self) -> usize {
        self.checked_bgra_len().unwrap_or(0)
    }

    pub(crate) fn checked_bgra_len(self) -> io::Result<usize> {
        usize::try_from(self.width)
            .ok()
            .and_then(|w| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|h| w.checked_mul(h))
            })
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid("BGRA byte size overflow"))
    }

    /// The largest size with the same aspect ratio within `max`; never larger
    /// than `self` and never below 1 × 1.
    pub(crate) fn fit_within(self, max: Self) -> Self {
        if self.width <= max.width && self.height <= max.height {
            return self;
        }
        if self.width == 0 || self.height == 0 || max.width == 0 || max.height == 0 {
            return Self::new(self.width.min(max.width), self.height.min(max.height));
        }
        let (width, height) = (u64::from(self.width), u64::from(self.height));
        let (max_width, max_height) = (u64::from(max.width), u64::from(max.height));
        if width * max_height >= height * max_width {
            Self::new(max.width.max(1), (height * max_width / width).max(1) as u32)
        } else {
            Self::new(
                (width * max_height / height).max(1) as u32,
                max.height.max(1),
            )
        }
    }

    /// `self` × `numerator` / `denominator`, at least 1 × 1.
    pub(crate) fn scale(self, numerator: u32, denominator: u32) -> Self {
        let scale = |side: u32| {
            let scaled = u64::from(side) * u64::from(numerator) / u64::from(denominator.max(1));
            scaled.clamp(1, u64::from(u32::MAX)) as u32
        };
        Self::new(scale(self.width), scale(self.height))
    }

    /// Rounded down to even sides (4:2:0 encoders), at least 2 × 2.
    pub(crate) fn even(self) -> Self {
        Self::new((self.width & !1).max(2), (self.height & !1).max(2))
    }

    /// Size of a 4:2:0 chroma plane.
    pub(crate) fn chroma(self) -> Self {
        Self::new(self.width.div_ceil(2), self.height.div_ceil(2))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub(crate) fn fits(&self, size: FrameSize) -> bool {
        self.width > 0
            && self.height > 0
            && u64::from(self.x) + u64::from(self.width) <= u64::from(size.width)
            && u64::from(self.y) + u64::from(self.height) <= u64::from(size.height)
    }
}

#[path = "frame_size_avc.rs"]
mod avc;
#[path = "frame_size_packet.rs"]
mod packet;

pub(crate) use avc::{avc_limits, check_avc_dimensions, fit_macroblocks, macroblocks, AvcLimits};
pub(crate) use packet::{validate_encoded_budget, PacketBudget, RECORD_BUFFER_OVERHEAD};
