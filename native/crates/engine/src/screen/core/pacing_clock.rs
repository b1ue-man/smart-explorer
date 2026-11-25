//! The frame clock and the codec's frame rate cap (moved out of
//! `pacing.rs`).
use super::MAX_FPS;

/// The highest frame rate the codec pairing allows for `size`: `requested`
/// within `MAX_FPS` and the macroblock rate both report; `None` when not even
/// one frame per second fits. The encoder opens with this rate (plan
/// 2026-11-14 „Bildkette“ B-03).
pub fn codec_cap(
    requested: u32,
    size: super::super::pixel_convert::FrameSize,
    max_blocks_per_second: Option<u64>,
) -> Option<u32> {
    let blocks = super::super::frame_size_caps::macroblocks(size);
    if blocks == 0 {
        return None;
    }
    let max = max_blocks_per_second
        .map_or(u64::from(MAX_FPS), |n| n / blocks)
        .min(u64::from(MAX_FPS));
    if max == 0 {
        return None;
    }
    Some(requested.clamp(1, MAX_FPS).min(max as u32))
}

/// Takes captured frames at the wanted rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameClock {
    interval_us: u64,
    next_due_us: Option<u64>,
}

fn interval_us(fps: u32) -> u64 {
    1_000_000 / u64::from(fps.clamp(1, MAX_FPS))
}

impl FrameClock {
    pub fn new(fps: u32) -> Self {
        Self {
            interval_us: interval_us(fps),
            next_due_us: None,
        }
    }

    pub fn set_fps(&mut self, fps: u32) {
        self.interval_us = interval_us(fps);
    }

    /// Sets the cadence to `codec_cap` and returns it.
    pub fn set_codec_fps(
        &mut self,
        requested: u32,
        size: super::super::pixel_convert::FrameSize,
        max_blocks_per_second: Option<u64>,
    ) -> Option<u32> {
        let fps = codec_cap(requested, size, max_blocks_per_second)?;
        self.set_fps(fps);
        Some(fps)
    }

    pub fn interval_us(&self) -> u64 {
        self.interval_us
    }

    /// Whether the frame captured at `now_us` is taken. A frame up to a
    /// quarter interval early counts as due, so a 60 Hz source at 30
    /// frames/s yields every second frame despite jitter; after a stall of
    /// more than one interval the cadence starts over instead of bursting.
    pub fn take(&mut self, now_us: u64) -> bool {
        let interval = self.interval_us;
        let next = match self.next_due_us {
            Some(due) if now_us + interval / 4 < due => return false,
            Some(due) if due + 2 * interval >= now_us => due + interval,
            _ => now_us + interval,
        };
        self.next_due_us = Some(next);
        true
    }
}
