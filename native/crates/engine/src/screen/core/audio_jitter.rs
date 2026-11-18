//! The viewer's jitter buffer for the host's sound (plan 2026-11-17 P9
//! „Audio“). Frames arrive as datagrams, possibly late, twice, out of order
//! or not at all; the output device takes one frame every 5 ms. The buffer
//! holds as few frames as the measured arrival jitter needs:
//! - the interarrival jitter `J` is estimated as in RTP (RFC 3550, 6.4.1):
//!   the transit time of each packet is its arrival minus the host clock of
//!   its last sample, `J += (|D| - J) / 16` with `D` the change of transit
//!   between two packets;
//! - the target depth is `ceil(2 J / frame)` frames, at least `MIN_TARGET`
//!   (one frame: the frame in play needs no reserve of its own, the output
//!   device holds it) and at most `MAX_TARGET`, so a quiet network costs one
//!   frame (5 ms) and only measured jitter adds more;
//! - playback starts (and starts again after running dry) once the target
//!   depth is buffered; a missing frame between buffered ones is played as
//!   lost (the decoder conceals it), running dry is silence;
//! - a buffer that stays more than two frames above its target for
//!   `TRIM_AFTER` frames drops its oldest frame, so the delay of a past
//!   burst does not stay (Moonlight drops queued audio the same way);
//! - a packet older than the next frame to play is late and dropped; a
//!   sequence jump of more than `RESET_GAP` frames (a new capture process
//!   on the host) starts the buffer anew.
use std::collections::BTreeMap;

use super::audio_wire::{AudioReport, FRAME_US};

/// Smallest target depth in frames.
pub const MIN_TARGET: usize = 1;
/// Largest target depth in frames (100 ms).
pub const MAX_TARGET: usize = 20;
/// Most frames held; older ones are dropped first (320 ms).
pub const MAX_FRAMES: usize = 64;
/// Frames a buffer may stay above its target before it trims (100 ms).
pub const TRIM_AFTER: u32 = 20;
/// A sequence jump of more frames (1 s) restarts the buffer.
pub const RESET_GAP: u64 = 200;

/// What the output plays next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Playout<T> {
    Frame(T),
    /// The frame is missing while later ones are here: conceal it.
    Lost,
    /// Nothing to play (not started or ran dry).
    Silence,
}

/// Counters for the statistics and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitterStats {
    pub received: u64,
    pub late: u64,
    pub lost: u64,
    pub underruns: u64,
    pub trimmed: u64,
    pub resets: u64,
}

pub struct JitterBuffer<T> {
    frames: BTreeMap<u64, T>,
    /// Extended sequence of the next frame to play.
    next: Option<u64>,
    highest: Option<u64>,
    playing: bool,
    jitter_us: f64,
    last_transit: Option<i64>,
    above_target: u32,
    stats: JitterStats,
}

impl<T> Default for JitterBuffer<T> {
    fn default() -> Self {
        Self {
            frames: BTreeMap::new(),
            next: None,
            highest: None,
            playing: false,
            jitter_us: 0.0,
            last_transit: None,
            above_target: 0,
            stats: JitterStats::default(),
        }
    }
}

impl<T> JitterBuffer<T> {
    /// The 32-bit sequence extended by the wraps seen so far.
    fn extend(&self, sequence: u32) -> u64 {
        match self.highest {
            None => u64::from(sequence) + (1 << 32),
            Some(highest) => {
                // Truncation to the low 32 bits is the wrap arithmetic.
                let delta = i64::from(sequence.wrapping_sub(highest as u32) as i32);
                highest.saturating_add_signed(delta)
            }
        }
    }

    /// One arrived frame: `host_us` is the host clock of its last sample,
    /// `arrival_us` this side's clock at its arrival.
    pub fn push(&mut self, sequence: u32, host_us: u64, arrival_us: u64, frame: T) {
        let extended = self.extend(sequence);
        if let Some(next) = self.next {
            if extended > next.saturating_add(RESET_GAP)
                || next > extended.saturating_add(RESET_GAP)
            {
                self.restart();
            } else if extended < next {
                self.stats.late += 1;
                return;
            }
        }
        // Clock values are far below 2^63 µs; wrapping keeps it total.
        let transit = (arrival_us as i64).wrapping_sub(host_us as i64);
        if let Some(last) = self.last_transit {
            let change = transit.wrapping_sub(last).unsigned_abs() as f64;
            self.jitter_us += (change - self.jitter_us) / 16.0;
        }
        self.last_transit = Some(transit);
        self.stats.received += 1;
        self.highest = Some(self.highest.map_or(extended, |high| high.max(extended)));
        self.frames.insert(extended, frame);
        while self.frames.len() > MAX_FRAMES {
            self.frames.pop_first();
            self.stats.trimmed += 1;
        }
    }

    fn restart(&mut self) {
        self.frames.clear();
        self.next = None;
        self.highest = None;
        self.playing = false;
        self.last_transit = None;
        self.above_target = 0;
        self.stats.resets += 1;
    }

    /// The frames playback waits for before it starts.
    pub fn target(&self) -> usize {
        let frames = (2.0 * self.jitter_us / f64::from(FRAME_US)).ceil();
        // `frames` is a non-negative finite value of at most a few thousand.
        (frames as usize).clamp(MIN_TARGET, MAX_TARGET)
    }

    /// The statistics for `AUDIO_REPORT`; `output_us` is the device's.
    pub fn report(&self, output_us: u32) -> AudioReport {
        let us = |frames: usize| {
            u32::try_from(frames)
                .unwrap_or(u32::MAX)
                .saturating_mul(FRAME_US)
        };
        AudioReport {
            depth_us: us(self.frames.len()),
            target_us: us(self.target()),
            // A finite non-negative estimate; saturates far above any path.
            jitter_us: self.jitter_us.min(f64::from(u32::MAX)) as u32,
            output_us,
            received: self.stats.received,
            lost: self.stats.lost,
            late: self.stats.late,
            underruns: self.stats.underruns,
            trimmed: self.stats.trimmed,
        }
    }

    pub fn jitter_us(&self) -> f64 {
        self.jitter_us
    }

    pub fn buffered(&self) -> usize {
        self.frames.len()
    }

    pub fn stats(&self) -> JitterStats {
        self.stats
    }

    /// Forgets every frame (the viewer switched its sound off).
    pub fn clear(&mut self) {
        let stats = self.stats;
        *self = Self::default();
        self.stats = stats;
    }

    /// The next frame for the output, once per frame interval.
    pub fn pop(&mut self) -> Playout<T> {
        let target = self.target();
        if !self.playing {
            if self.frames.len() < target {
                return Playout::Silence;
            }
            self.playing = true;
            let first = self.frames.keys().next().copied();
            if self
                .next
                .is_none_or(|next| first.is_some_and(|first| first > next))
            {
                self.next = first;
            }
        }
        let Some(next) = self.next else {
            self.playing = false;
            return Playout::Silence;
        };
        if let Some(frame) = self.frames.remove(&next) {
            self.next = Some(next + 1);
            self.trim(target);
            return Playout::Frame(frame);
        }
        if self.frames.is_empty() {
            self.stats.underruns += 1;
            self.playing = false;
            return Playout::Silence;
        }
        self.stats.lost += 1;
        self.next = Some(next + 1);
        Playout::Lost
    }

    /// Drops the oldest frame of a buffer that stayed too deep.
    fn trim(&mut self, target: usize) {
        if self.frames.len() <= target + 2 {
            self.above_target = 0;
            return;
        }
        self.above_target += 1;
        if self.above_target < TRIM_AFTER {
            return;
        }
        self.above_target = 0;
        if let Some((oldest, _)) = self.frames.pop_first() {
            self.next = Some(oldest + 1);
            self.stats.trimmed += 1;
        }
    }
}

#[cfg(test)]
#[path = "audio_jitter_tests.rs"]
mod tests;
